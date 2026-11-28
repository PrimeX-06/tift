//! Lossless source snapshots and source-ordered semantic events.
//!
//! Every byte belongs to exactly one lexical token. Semantic rules annotate
//! those tokens; they never replace the original spelling or discard recovery
//! text. Definitions, scope, conditional branches and environment boundaries
//! share this index, including the project loader's exported bindings.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap, HashMap, HashSet};
use std::ops::Range;

use crate::config::{EnvironmentAlias, EnvironmentArguments};
use crate::lexical_index::{
    AppendLexicalIndex, AppendProtectedTokens, LexicalIndex, PrefixGroupPairs, RawTokenStream,
    SourceTokens,
};
use crate::semantic_index::SemanticIndex;
use crate::source_document::SourceDocument;
use crate::{CompiledConfig, Config, Diagnostic, Severity, syntax};

pub use crate::lexical_index::{MappedToken, SourceToken, SourceTokenKind};

mod named_wrapper;
use named_wrapper::NamedWrapper;

pub(crate) fn printed_command(name: &str) -> bool {
    matches!(
        name,
        "emph"
            | "textbf"
            | "textit"
            | "textmd"
            | "textnormal"
            | "textrm"
            | "textsc"
            | "textsf"
            | "textsl"
            | "texttt"
            | "textup"
    )
}

pub(crate) fn paragraph_environment(name: &str) -> bool {
    matches!(
        name,
        "quote" | "quotation" | "center" | "flushleft" | "flushright"
    )
}

pub(crate) type LoaderPositions = HashMap<(String, usize), Range<usize>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleOrigin {
    Builtin,
    Configuration,
    Source,
}

/// Extension meanings are independent of their declaration syntax.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleMeaning {
    Environment(EnvironmentAlias),
    Arguments(EnvironmentArguments),
    Condition,
    /// A declaration shadows a package rule whose meaning is unavailable.
    Unknown,
}

#[derive(Debug, Clone)]
pub struct SyntaxRule {
    pub name: String,
    pub meaning: RuleMeaning,
    pub origin: RuleOrigin,
    pub span: Option<Range<usize>>,
}

#[derive(Debug, Clone)]
pub struct Declaration {
    pub name: String,
    pub span: Range<usize>,
    pub body: Range<usize>,
    pub signature: Option<EnvironmentArguments>,
}

#[derive(Debug, Clone)]
pub struct EnvironmentDeclaration {
    pub name: String,
    pub span: Range<usize>,
    pub bodies: Vec<Range<usize>>,
    pub signature: Option<EnvironmentArguments>,
    pub provide: bool,
}

#[derive(Debug, Clone)]
pub struct Conditional {
    pub span: Range<usize>,
    pub branches: Vec<Range<usize>>,
}

#[derive(Debug, Clone)]
pub(crate) enum Replacement {
    Boundary(EnvironmentAlias),
    VerbatimEnvironment(crate::config::VerbatimTerminator),
    WrappedEnvironment(String),
    RestartEnvironment(String),
    RestartArgument,
    RestartVia {
        environment: String,
        opener: String,
    },
    GroupBegin,
    InlineMathShift,
    /// A single expansion produces only ordinary characters.
    LiteralText,
    ConstructedWrapper(String),
    NamedWrapper(NamedWrapper),
    Forward(String),
    /// Literal boundaries balance, provided the supplied arguments do too.
    NeutralWrapper,
    FlatWrapper,
    Unknown,
}

#[derive(Debug, Clone)]
struct Binding {
    replacement: Replacement,
    candidate: bool,
    signature: Option<EnvironmentArguments>,
    condition: Option<String>,
    literal_arguments: bool,
    literal_slots: u16,
    stored_slots: u16,
    math_signature: Option<(&'static str, &'static str)>,
    global: bool,
}

#[derive(Default)]
struct BindingScope {
    environment: Option<String>,
    previous: HashMap<String, Option<Binding>>,
}

impl BindingScope {
    fn environment(name: &str) -> Self {
        Self {
            environment: Some(name.into()),
            ..Self::default()
        }
    }
}

type ConditionalProofs = HashMap<
    usize,
    HashMap<String, BTreeMap<usize, (Option<EnvironmentArguments>, u16, bool, bool)>>,
>;

fn join_conditional_data(
    source: &str,
    at: usize,
    branches: &[Range<usize>],
    proofs: &mut ConditionalProofs,
    bindings: &mut HashMap<String, Binding>,
) {
    let Some(names) = proofs.remove(&at) else {
        return;
    };
    if !branches.last().is_some_and(|range| {
        syntax::command_at(source, range.end).is_some_and(|(name, _)| name == "else")
    }) {
        return;
    }
    for (name, variants) in names {
        if variants.len() != branches.len() + 1 {
            continue;
        }
        let Some(Some(signature)) = variants
            .first_key_value()
            .map(|(_, value)| value.0.as_ref())
        else {
            continue;
        };
        if variants
            .values()
            .any(|(other, _, _, candidate)| *candidate || other.as_ref() != Some(signature))
        {
            continue;
        }
        if let Some(binding) = bindings.get_mut(&name) {
            binding.signature = Some(signature.clone());
            binding.literal_arguments = variants.values().all(|(_, _, literal, _)| *literal);
            binding.literal_slots = variants
                .values()
                .fold(u16::MAX, |mask, (_, slots, _, _)| mask & slots);
            binding.stored_slots = binding.literal_slots;
        }
    }
}

struct SourceImportEvent {
    at: usize,
    aliases: BTreeMap<String, Option<EnvironmentAlias>>,
    arguments: BTreeMap<String, Option<EnvironmentArguments>>,
    commands: BTreeMap<String, crate::config::ImportedCommand>,
}

/// Owned, immutable text with a complete lexical partition and semantic index.
#[derive(Debug, Clone)]
pub struct SourceModel {
    source: SourceDocument,
    tokens: LexicalIndex,
    reader_windows: Vec<syntax::ReaderWindow>,
    diagnostics: Vec<Diagnostic>,
    pub(crate) mismatch: Option<Range<usize>>,
    semantic: SemanticIndex,
    pub(crate) structure: Option<(Vec<syntax::Token>, Vec<syntax::SharedBlock>)>,
}

#[derive(Clone, Copy)]
enum IndexPurpose<'a> {
    Document,
    Definitions,
    Exports,
    IndexedExports {
        readers: &'a [usize],
        transparent: &'a [usize],
        closing_group: &'a dyn Fn(usize) -> Option<usize>,
        tables: ExportTables,
    },
}

#[derive(Clone, Copy)]
enum ExportTables {
    Aliases,
    All,
}

mod export_reuse;

#[derive(Default)]
struct ExportReaderPositions {
    readers: Vec<usize>,
    transparent: Vec<usize>,
    groups: PrefixGroupPairs,
}

fn opaque_verbatim_declarations(
    source: &str,
    token: &SourceToken,
) -> Option<Vec<crate::package_environments::VerbatimDeclaration>> {
    if token.kind != SourceTokenKind::Opaque {
        return None;
    }
    let mut cursor = token.span.start;
    let mut declarations = Vec::new();
    while cursor < token.span.end {
        let (command, end) = syntax::command_at(source, cursor)?;
        let declaration = crate::package_environments::verbatim_declaration(source, command, end)?;
        if declaration.end > token.span.end {
            return None;
        }
        cursor = declaration.end;
        declarations.push(declaration);
    }
    Some(declarations)
}

impl ExportReaderPositions {
    fn observe(&mut self, source: &str, index: usize, token: &SourceToken) {
        if opaque_verbatim_declarations(source, token).is_some() {
            self.readers.push(index);
            return;
        }
        let Some((name, _)) = token.control(source) else {
            return;
        };
        let name = name.trim_end_matches('*');
        if syntax::project_export_reader(name)
            || name.starts_with("csname")
            || syntax::expl3_definition_variant(name).is_some()
        {
            self.readers.push(index);
        }
        if matches!(
            name,
            "uppercase" | "lowercase" | "expanded" | "precattl_exec:n"
        ) {
            self.transparent.push(index);
        }
    }

    fn truncate(&mut self, keep: usize) {
        self.groups.retain_prefix(keep);
        self.readers
            .truncate(self.readers.partition_point(|&index| index < keep));
        self.transparent
            .truncate(self.transparent.partition_point(|&index| index < keep));
    }

    fn aliases(
        &mut self,
        source: &str,
        config: &Config,
        tokens: &[SourceToken],
        ranges: &[Range<usize>],
    ) -> (BTreeMap<String, EnvironmentAlias>, usize) {
        let mut loader_bytes = 0;
        let loaders = SourceModel::has_anchored_loader_events(config).then(|| {
            if ranges.is_empty() {
                SourceModel::loader_positions(source, Some(tokens))
            } else {
                loader_bytes = source.len();
                SourceModel::loader_positions(source, None)
            }
        });
        let constructed = syntax::constant_constructed_commands(source, config, ranges);
        let semantic = self.analyze(
            source,
            config,
            tokens,
            loaders.as_ref(),
            constructed,
            ExportTables::Aliases,
        );
        (semantic.exported, loader_bytes)
    }

    fn analyze(
        &mut self,
        source: &str,
        config: &Config,
        tokens: &[SourceToken],
        loaders: Option<&LoaderPositions>,
        constructed: HashMap<usize, (String, usize)>,
        tables: ExportTables,
    ) -> SemanticIndex {
        let mut semantic = SemanticIndex::for_source(config, loaders);
        semantic.constructed = constructed;
        let groups = std::cell::RefCell::new(&mut self.groups);
        SemanticReader {
            source,
            tokens,
            semantic: &mut semantic,
            diagnostics: &mut Vec::new(),
        }
        .index(
            config,
            loaders,
            IndexPurpose::IndexedExports {
                readers: &self.readers,
                transparent: &self.transparent,
                closing_group: &|open| groups.borrow_mut().closing_group(tokens, open),
                tables,
            },
        );
        semantic
    }
}

#[doc(hidden)]
pub type ProjectExports = (
    BTreeMap<String, Option<EnvironmentAlias>>,
    BTreeMap<String, Option<EnvironmentArguments>>,
    BTreeMap<String, bool>,
    BTreeMap<String, crate::config::ImportedCommand>,
    BTreeMap<String, Option<crate::config::VerbatimTerminator>>,
);

/// Alias snapshots over successive exact prefixes under a fixed configuration.
#[doc(hidden)]
#[derive(Default)]
pub struct ProjectAliasIndex {
    lexical: AppendProtectedTokens,
    positions: ExportReaderPositions,
    last_lexed_bytes: usize,
    total_lexed_bytes: usize,
}

impl ProjectAliasIndex {
    /// Ranges must be the original-byte protection for this complete prefix.
    pub fn aliases(
        &mut self,
        source: &str,
        config: &Config,
        ranges: &[Range<usize>],
    ) -> BTreeMap<String, EnvironmentAlias> {
        let (tokens, keep, scanned) = self.lexical.update(source, ranges);
        self.positions.truncate(keep);
        for (index, token) in tokens.iter().enumerate().skip(keep) {
            self.positions.observe(source, index, token);
        }
        let (aliases, loader_bytes) = self.positions.aliases(source, config, tokens, ranges);
        self.last_lexed_bytes = scanned.saturating_add(loader_bytes);
        self.total_lexed_bytes = self.total_lexed_bytes.saturating_add(self.last_lexed_bytes);
        aliases
    }

    /// Bytes submitted to the protected partition lexer and any raw loader scan
    /// by the last call. Excludes protection computation and semantic analysis.
    pub fn last_lexed_bytes(&self) -> usize {
        self.last_lexed_bytes
    }

    /// Cumulative last-call byte counts, saturating at `usize::MAX`.
    pub fn total_lexed_bytes(&self) -> usize {
        self.total_lexed_bytes
    }
}

/// Project exports over successive prefixes of one source with a fixed configuration.
#[derive(Default)]
pub struct ProjectExportIndex {
    lexical: AppendLexicalIndex,
    protection: syntax::ExportProtectionCursor,
    positions: ExportReaderPositions,
    projection: export_reuse::ExportProjection,
    raw_loaders: Vec<usize>,
    last_lexed_bytes: usize,
    may_construct: bool,
    pending_restart: Option<usize>,
}

impl ProjectExportIndex {
    /// Raw source bytes submitted to the append lexer by the last snapshot,
    /// including its restarted tail token. Excludes protection and semantic projection.
    pub fn last_lexed_bytes(&self) -> usize {
        self.last_lexed_bytes
    }

    /// The previous source must remain an exact prefix. Supplied ranges must be
    /// the successful declaration protection for this exact source and configuration.
    pub fn snapshot(
        &mut self,
        source: &str,
        config: &Config,
        protection: Option<&[Range<usize>]>,
    ) -> ProjectExports {
        let previous_len = self.lexical.raw().last().map_or(0, |token| token.span.end);
        let (restart, scanned_bytes) = self.lexical.append_raw(source);
        self.last_lexed_bytes = scanned_bytes;
        let raw = self.lexical.raw();
        let loaders = SourceModel::has_anchored_loader_events(config).then(|| {
            let keep = raw.partition_point(|token| token.span.end <= restart);
            self.raw_loaders
                .truncate(self.raw_loaders.partition_point(|&index| index < keep));
            self.raw_loaders
                .extend(
                    raw.iter()
                        .enumerate()
                        .skip(keep)
                        .filter_map(|(index, token)| {
                            token
                                .control(source)
                                .filter(|(name, _)| SourceModel::loader_command(name))
                                .map(|_| index)
                        }),
                );
            SourceModel::loader_positions_from_tokens(
                source,
                self.raw_loaders.iter().map(|&index| &raw[index]),
            )
        });
        let restart = self
            .pending_restart
            .map_or(restart, |pending| pending.min(restart));
        if !self.may_construct {
            let overlap =
                source.floor_char_boundary(previous_len.saturating_sub(r"\csname".len() - 1));
            self.may_construct = source[overlap..].contains(r"\csname");
        }
        let scanned;
        let ranges = match protection {
            Some(ranges) => ranges,
            None => {
                let raw =
                    std::cell::RefCell::new(RawTokenStream::buffered(source, self.lexical.raw()));
                match self.protection.ranges(
                    source,
                    previous_len,
                    config,
                    loaders.as_ref().map(|positions| syntax::RawReaderFacts {
                        tokens: &raw,
                        loaders: Some(positions),
                    }),
                ) {
                    Ok(ranges) => {
                        scanned = ranges;
                        &scanned
                    }
                    Err(error) => {
                        self.pending_restart = Some(restart);
                        return SourceModel::failed_document(
                            SourceDocument::new(source),
                            config,
                            error,
                        )
                        .semantic
                        .into_exports();
                    }
                }
            }
        };
        self.pending_restart = None;
        let (tokens, keep) = self.lexical.project(source, ranges, restart);
        let (tokens, keep) = self.projection.update(source, config, tokens, keep);
        self.positions.truncate(keep);
        for (index, token) in tokens.iter().enumerate().skip(keep) {
            self.positions.observe(source, index, token);
        }
        let constructed = if self.may_construct {
            syntax::constant_constructed_commands(source, config, ranges)
        } else {
            HashMap::new()
        };
        self.positions
            .analyze(
                source,
                config,
                tokens,
                loaders.as_ref(),
                constructed,
                ExportTables::All,
            )
            .into_exports()
    }
}

impl SourceModel {
    pub fn parse(source: &str) -> Self {
        let config = Config::default();
        Self::from_config(source, &config)
    }

    pub fn analyze(source: &str, config: &CompiledConfig) -> Self {
        Self::build(source, &config.config, Some(config))
    }

    pub fn from_config(source: &str, config: &Config) -> Self {
        Self::build(source, config, None)
    }

    /// Source metadata for the project loader without formatting validation.
    #[doc(hidden)]
    pub fn from_declaration_source(source: &str, config: &Config) -> Self {
        let document = SourceDocument::new(source);
        match Self::scan_document(
            document.clone(),
            config,
            None,
            None,
            false,
            IndexPurpose::Document,
        ) {
            Ok(model) => model,
            Err(error) => Self::failed_document(document, config, error),
        }
    }

    #[doc(hidden)]
    pub fn project_exports(source: &str, config: &Config) -> ProjectExports {
        let document = SourceDocument::new(source);
        let model = Self::scan_document(
            document.clone(),
            config,
            None,
            None,
            false,
            IndexPurpose::Exports,
        )
        .unwrap_or_else(|error| Self::failed_document(document, config, error));
        model.semantic.into_exports()
    }

    fn failed_document(
        document: SourceDocument,
        config: &Config,
        error: crate::FormatError,
    ) -> Self {
        let range = 0..document.len();
        let mut model = Self::with_document(document.clone(), config, std::slice::from_ref(&range));
        model.record_error(document.as_str(), error);
        model
    }

    fn scan_document(
        document: SourceDocument,
        config: &Config,
        compiled: Option<&syntax::CompiledSyntax>,
        previous: Option<&Self>,
        collect_readers: bool,
        purpose: IndexPurpose<'_>,
    ) -> Result<Self, crate::FormatError> {
        let source = document.as_str();
        if !Self::has_anchored_loader_events(config) {
            let mut windows = Vec::new();
            let (ranges, diagnostics) = syntax::protection_snapshot(
                source,
                config,
                compiled,
                collect_readers.then_some(&mut windows),
                None,
            )?;
            let tokens = match previous {
                Some(model) => model.tokens.relex(model.source(), source, &ranges),
                None => LexicalIndex::new(source, &ranges),
            };
            let mut model =
                Self::with_lexical_index(document, config, &ranges, tokens, None, purpose);
            model.reader_windows = windows;
            model.diagnostics.extend(diagnostics);
            return Ok(model);
        }
        let previous = previous.map(|model| model.tokens.relex(model.source(), source, &[]));
        let mut raw = std::cell::RefCell::new(RawTokenStream::new(source, previous));
        let loaders = Some(Self::loader_positions_from_tokens(
            source,
            raw.get_mut().complete(),
        ));
        let mut windows = Vec::new();
        let (ranges, diagnostics) = syntax::protection_snapshot(
            source,
            config,
            compiled,
            collect_readers.then_some(&mut windows),
            Some(syntax::RawReaderFacts {
                tokens: &raw,
                loaders: loaders.as_ref(),
            }),
        )?;
        let tokens = raw.into_inner().finish().with_protection(source, &ranges);
        let mut model =
            Self::with_lexical_index(document, config, &ranges, tokens, loaders, purpose);
        model.reader_windows = windows;
        model.diagnostics.extend(diagnostics);
        Ok(model)
    }

    fn build(source: &str, config: &Config, compiled: Option<&CompiledConfig>) -> Self {
        let document = SourceDocument::new(source);
        match Self::scan_document(
            document.clone(),
            config,
            compiled.map(|c| c.compiled_syntax()),
            None,
            true,
            IndexPurpose::Document,
        ) {
            Ok(mut model) => {
                let owned = compiled
                    .is_none()
                    .then(|| CompiledConfig::new(config.clone()).ok())
                    .flatten();
                if let Some(compiled) = compiled.or(owned.as_ref()) {
                    let structure = {
                        let ranges = model.protected_ranges();
                        let analysis = crate::source_analysis::SourceAnalysis::for_model(
                            source, config, ranges, &model,
                        );
                        syntax::shared_blocks_with_analysis(source, compiled, ranges, &analysis)
                    };
                    match structure {
                        Ok(structure) => {
                            // Use the structural groups: literal operands and
                            // groups paired across arguments are already resolved.
                            for block in &structure.1 {
                                if block.kind == syntax::BlockKind::Group && block.end.is_empty() {
                                    model.diagnostics.push(Diagnostic {
                                        severity: Severity::Error,
                                        message: "Unclosed group before an opaque region; source retained".into(),
                                        span: block.begin.clone(),
                                    });
                                }
                            }
                            model.structure = Some(structure);
                        }
                        Err(error) => {
                            model.record_error(source, error);
                        }
                    }
                }
                model
            }
            Err(error) => Self::failed_document(document, config, error),
        }
    }

    fn record_error(&mut self, source: &str, error: crate::FormatError) {
        if let crate::FormatError::StructuralMismatch { opener, closer, .. } = &error {
            self.mismatch = Some(*opener..closer.end);
        }
        self.diagnostics.push(located_error(source, error));
    }

    pub fn parse_bytes(source: &[u8]) -> ByteSourceModel {
        ByteSourceModel::parse(source)
    }

    /// Reader candidates use the same lexical partition even inside delayed
    /// declaration bodies; the declaration reader controls their visibility.
    pub(crate) fn reader_positions(source: &str) -> impl Iterator<Item = usize> + '_ {
        SourceTokens::new(source, &[])
            .filter(|t| {
                matches!(
                    t.kind,
                    SourceTokenKind::Control
                        | SourceTokenKind::Comment
                        | SourceTokenKind::OpenGroup
                        | SourceTokenKind::CloseGroup
                )
            })
            .map(|t| t.span.start)
    }

    /// Literal group ends from the common lexical partition. Consumers that
    /// need scoped lookahead build this once instead of rescanning nested bodies.
    pub(crate) fn literal_group_ends(
        tokens: impl IntoIterator<Item = SourceToken>,
    ) -> HashMap<usize, usize> {
        let mut groups = Vec::new();
        let mut ends = HashMap::new();
        for token in tokens {
            match token.kind {
                SourceTokenKind::OpenGroup => groups.push(token.span.start),
                SourceTokenKind::CloseGroup => {
                    if let Some(start) = groups.pop() {
                        ends.insert(start, token.span.end);
                    }
                }
                _ => (),
            }
        }
        ends
    }

    #[doc(hidden)]
    pub fn loader_occurrences(source: &str) -> HashMap<usize, (String, usize)> {
        Self::loaders(source)
            .map(|(span, name, occurrence)| (span.start, (name.to_owned(), occurrence)))
            .collect()
    }

    fn loaders(source: &str) -> impl Iterator<Item = (Range<usize>, &str, usize)> {
        let mut counts = HashMap::new();
        SourceTokens::new(source, &[]).filter_map(move |token| {
            let (name, _) = token.control(source)?;
            Self::loader_command(name).then(|| {
                let occurrence = counts.entry(name).or_insert(0);
                let loader = (token.span.clone(), name, *occurrence);
                *occurrence += 1;
                loader
            })
        })
    }

    fn loader_command(name: &str) -> bool {
        matches!(
            name,
            "input"
                | "include"
                | "documentclass"
                | "LoadClass"
                | "LoadClassWithOptions"
                | "usepackage"
                | "RequirePackage"
                | "RequirePackageWithOptions"
        )
    }

    fn loader_positions_from_tokens<'a>(
        source: &str,
        tokens: impl IntoIterator<Item = &'a SourceToken>,
    ) -> LoaderPositions {
        let mut counts = HashMap::<&str, usize>::new();
        let mut positions = HashMap::new();
        for token in tokens {
            let Some((name, _)) = token.control(source) else {
                continue;
            };
            if !Self::loader_command(name) {
                continue;
            }
            let occurrence = counts.entry(name).or_insert(0);
            positions.insert((name.to_owned(), *occurrence), token.span.clone());
            *occurrence += 1;
        }
        positions
    }

    pub(crate) fn has_anchored_loader_events(config: &Config) -> bool {
        if config
            .syntax
            .verbatim_environment_events
            .iter()
            .any(|event| event.loader.is_some())
        {
            return true;
        }
        config
            .syntax
            .package_load_events
            .iter()
            .any(|event| event.loader.is_some())
            || config
                .syntax
                .command_import_events
                .iter()
                .any(|event| event.loader.is_some())
            || config
                .syntax
                .environment_alias_events
                .iter()
                .any(|event| event.loader.is_some())
            || config
                .syntax
                .environment_argument_events
                .iter()
                .any(|event| event.loader.is_some())
    }

    pub(crate) fn loader_positions(
        source: &str,
        tokens: Option<&[SourceToken]>,
    ) -> LoaderPositions {
        tokens.map_or_else(
            || {
                Self::loaders(source)
                    .map(|(span, name, occurrence)| ((name.to_owned(), occurrence), span))
                    .collect()
            },
            |tokens| Self::loader_positions_from_tokens(source, tokens),
        )
    }

    fn import_events(
        source: &str,
        config: &Config,
        loader_positions: Option<&LoaderPositions>,
    ) -> Vec<SourceImportEvent> {
        if config.syntax.environment_alias_events.is_empty()
            && config.syntax.environment_argument_events.is_empty()
            && config.syntax.command_import_events.is_empty()
        {
            return Vec::new();
        }
        let offset = |at: usize, loader: &Option<(String, usize)>| -> Option<usize> {
            let Some(loader) = loader else {
                return Some(at.min(source.len()));
            };
            let loader = loader_positions?.get(loader)?;
            let end = loader.end;
            let mut start = skip(source, end);
            if source.as_bytes().get(start) == Some(&b'[') {
                start = syntax::balanced_end(source, start, b'[', b']')
                    .map_or(source.len(), |end| skip(source, end));
            }
            Some(if source.as_bytes().get(start) == Some(&b'{') {
                syntax::balanced_end(source, start, b'{', b'}').unwrap_or(source.len())
            } else {
                source[start..]
                    .find(|c: char| c.is_whitespace() || "%{}".contains(c))
                    .map_or(source.len(), |n| start + n)
            })
        };
        let mut result = Vec::new();
        for event in &config.syntax.environment_alias_events {
            if let Some(at) = offset(event.at, &event.loader) {
                result.push(SourceImportEvent {
                    at,
                    aliases: event.aliases.clone(),
                    arguments: BTreeMap::new(),
                    commands: BTreeMap::new(),
                });
            }
        }
        for event in &config.syntax.environment_argument_events {
            if let Some(at) = offset(event.at, &event.loader) {
                result.push(SourceImportEvent {
                    at,
                    aliases: BTreeMap::new(),
                    arguments: event.arguments.clone(),
                    commands: BTreeMap::new(),
                });
            }
        }
        for event in &config.syntax.command_import_events {
            if let Some(at) = offset(event.at, &event.loader) {
                result.push(SourceImportEvent {
                    at,
                    aliases: BTreeMap::new(),
                    arguments: BTreeMap::new(),
                    commands: event.commands.clone(),
                });
            }
        }
        result.sort_by_key(|event| event.at);
        let mut merged: Vec<SourceImportEvent> = Vec::new();
        for event in result {
            if let Some(previous) = merged.last_mut().filter(|previous| previous.at == event.at) {
                previous.aliases.extend(event.aliases);
                previous.arguments.extend(event.arguments);
                previous.commands.extend(event.commands);
            } else {
                merged.push(event);
            }
        }
        merged
    }

    pub(crate) fn command_imports(
        source: &str,
        config: &Config,
        loader_positions: Option<&LoaderPositions>,
    ) -> Vec<crate::config::CommandImportEvent> {
        config
            .syntax
            .command_import_events
            .iter()
            .filter_map(|event| {
                let mut event = event.clone();
                event.at =
                    Self::import_offset(source, event.at, event.loader.as_ref(), loader_positions)?;
                Some(event)
            })
            .collect()
    }

    pub(crate) fn import_offset(
        source: &str,
        at: usize,
        loader: Option<&(String, usize)>,
        positions: Option<&LoaderPositions>,
    ) -> Option<usize> {
        let Some(loader) = loader else {
            return Some(at);
        };
        let mut cursor = skip(source, positions?.get(loader)?.end);
        if source.as_bytes().get(cursor) == Some(&b'[') {
            cursor = skip(source, syntax::balanced_end(source, cursor, b'[', b']')?);
        }
        if source.as_bytes().get(cursor) == Some(&b'{') {
            syntax::balanced_end(source, cursor, b'{', b'}')
        } else {
            Some(
                source[cursor..]
                    .find(|c: char| c.is_whitespace() || "%{}".contains(c))
                    .map_or(source.len(), |n| cursor + n),
            )
        }
    }

    pub(crate) fn with_ranges(source: &str, config: &Config, ranges: &[Range<usize>]) -> Self {
        Self::with_document(SourceDocument::new(source), config, ranges)
    }

    pub(crate) fn with_document(
        source: SourceDocument,
        config: &Config,
        ranges: &[Range<usize>],
    ) -> Self {
        Self::index_ranges(source, config, ranges, IndexPurpose::Document)
    }

    pub(crate) fn for_definition_scan(
        source: &str,
        config: &Config,
        ranges: &[Range<usize>],
    ) -> Self {
        Self::index_ranges(
            SourceDocument::new(source),
            config,
            ranges,
            IndexPurpose::Definitions,
        )
    }

    pub(crate) fn aliases_with_ranges(
        source: &str,
        config: &Config,
        ranges: &[Range<usize>],
    ) -> BTreeMap<String, EnvironmentAlias> {
        Self::index_ranges(
            SourceDocument::new(source),
            config,
            ranges,
            IndexPurpose::Exports,
        )
        .semantic
        .exported
    }

    pub(crate) fn indexed_aliases_with_ranges(
        source: &str,
        config: &Config,
        ranges: &[Range<usize>],
    ) -> BTreeMap<String, EnvironmentAlias> {
        let mut positions = ExportReaderPositions::default();
        let tokens: Vec<_> = SourceTokens::new(source, ranges)
            .enumerate()
            .map(|(index, token)| {
                positions.observe(source, index, &token);
                token
            })
            .collect();
        positions.aliases(source, config, &tokens, ranges).0
    }

    fn index_ranges(
        source: SourceDocument,
        config: &Config,
        ranges: &[Range<usize>],
        purpose: IndexPurpose<'_>,
    ) -> Self {
        let tokens = LexicalIndex::new(source.as_str(), ranges);
        let loaders = Self::has_anchored_loader_events(config)
            .then(|| Self::loader_positions(source.as_str(), None));
        Self::with_lexical_index(source, config, ranges, tokens, loaders, purpose)
    }

    fn with_lexical_index(
        source: SourceDocument,
        config: &Config,
        ranges: &[Range<usize>],
        tokens: LexicalIndex,
        loader_positions: Option<LoaderPositions>,
        purpose: IndexPurpose<'_>,
    ) -> Self {
        let source = syntax::preserve_literal_translations(source, ranges);
        let mut semantic = SemanticIndex::for_source(config, loader_positions.as_ref());
        semantic.protected = ranges.to_vec();
        semantic.constructed = syntax::constant_constructed_commands(&source, config, ranges);
        let mut model = Self {
            source,
            tokens,
            reader_windows: Vec::new(),
            diagnostics: Vec::new(),
            mismatch: None,
            semantic,
            structure: None,
        };
        SemanticReader {
            source: &model.source,
            tokens: &model.tokens,
            semantic: &mut model.semantic,
            diagnostics: &mut model.diagnostics,
        }
        .index(config, loader_positions.as_ref(), purpose);
        model
            .semantic
            .protected
            .extend(model.semantic.opaque.iter().cloned());
        model.semantic.protected.sort_unstable_by_key(|r| r.start);
        model.semantic.protected.dedup_by(|next, previous| {
            if previous.end < next.start {
                return false;
            }
            previous.end = previous.end.max(next.end);
            true
        });
        model
    }

    pub fn source(&self) -> &str {
        self.source.as_str()
    }
    pub fn source_bytes(&self) -> &[u8] {
        self.source.as_bytes()
    }
    pub(crate) fn document(&self) -> &SourceDocument {
        &self.source
    }

    pub(crate) fn reader_windows(&self) -> &[syntax::ReaderWindow] {
        &self.reader_windows
    }

    pub(crate) fn set_reader_windows(&mut self, windows: Vec<syntax::ReaderWindow>) {
        self.reader_windows = windows;
    }

    pub(crate) fn rescan_document(
        document: SourceDocument,
        config: &CompiledConfig,
        previous: Option<&Self>,
    ) -> Result<Self, crate::FormatError> {
        Self::scan_document(
            document,
            &config.config,
            Some(config.compiled_syntax()),
            previous,
            true,
            IndexPurpose::Document,
        )
    }
    pub fn tokens(&self) -> &[SourceToken] {
        self.tokens.as_slice()
    }

    /// TeX character-translated tokens before syntax-specific protection.
    pub fn analysis_tokens(&self) -> impl Iterator<Item = MappedToken> + '_ {
        SourceTokens::mapped(&self.source, &[])
    }

    pub(crate) fn project_text(
        &self,
        document: SourceDocument,
        change: &crate::source_document::SourceChange,
    ) -> Self {
        self.project_semantics(document, change, |_| {})
    }

    fn project_semantics(
        &self,
        document: SourceDocument,
        change: &crate::source_document::SourceChange,
        remove_local: impl FnOnce(&mut SemanticIndex),
    ) -> Self {
        let mut ranges = self.lexical_protection().to_vec();
        for range in &mut ranges {
            change.project(range);
        }
        let document = syntax::preserve_literal_translations(document, &ranges);
        let tokens = self.tokens.relex(self.source(), document.as_str(), &ranges);
        let mut semantic = self.semantic.clone();
        remove_local(&mut semantic);
        semantic.project_text(change);
        let mut diagnostics = self.diagnostics.clone();
        for diagnostic in &mut diagnostics {
            change.project(&mut diagnostic.span);
        }
        Self {
            source: document,
            tokens,
            reader_windows: self
                .reader_windows
                .iter()
                .cloned()
                .map(|mut window| {
                    change.project(&mut window.span);
                    window
                })
                .collect(),
            semantic,
            diagnostics,
            mismatch: self.mismatch.clone().map(|mut range| {
                change.project(&mut range);
                range
            }),
            structure: None,
        }
    }

    /// A closed paragraph of builtin printed arguments emits invocation rules
    /// and ordinary reader windows. Neither escapes into the next paragraph.
    pub(crate) fn project_printed_commands(
        &self,
        document: SourceDocument,
        change: &crate::source_document::SourceChange,
        body: Range<usize>,
        old: &Self,
        new: &Self,
    ) -> Option<Self> {
        let outside = self.invocation_context_at(body.start);
        if self
            .tokens
            .iter()
            .filter_map(|t| t.control(self.source()))
            .any(|(name, _)| {
                !matches!(
                    name,
                    "documentclass"
                        | "usepackage"
                        | "RequirePackage"
                        | "begin"
                        | "end"
                        | "section"
                        | "subsection"
                ) && !printed_command(name)
            })
        {
            // Declarations, including scoped assignments and constructed names,
            // can replace the meaning of a formatting command at this offset.
            return None;
        }
        for local in [old, new] {
            if !local.protected_ranges().is_empty()
                || !local.diagnostics().is_empty()
                || !local.semantic.invocation_arguments.is_empty()
                || !local.stored_argument_ranges().is_empty()
                || local.invocation_context_at(local.source.len()).environment().is_some()
                || local.rules().iter().any(|r| r.span.is_some() && !matches!(&r.meaning,
                    RuleMeaning::Environment(alias) if paragraph_environment(&alias.environment) && r.origin == RuleOrigin::Builtin))
            {
                return None;
            }
            for token in local.tokens.iter() {
                let Some((name, end)) = token.control(local.source()) else {
                    continue;
                };
                if matches!(name, "begin" | "end") {
                    let (environment, _, _) = syntax::env_after_command(local.source(), name, end)?;
                    if !paragraph_environment(environment)
                        || self.has_environment_declaration(environment)
                        || self
                            .resolved_rule(name)
                            .is_some_and(|r| r.origin != RuleOrigin::Builtin)
                        || crate::packages::context_ambiguous(environment, &outside).is_some()
                        || crate::packages::context_header_signature(environment, &outside)
                            .is_some()
                        || local.environment_arguments_at(token.span.start).is_some()
                    {
                        return None;
                    }
                    continue;
                }
                if !printed_command(name)
                    || self
                        .resolved_rule(name)
                        .is_some_and(|r| r.origin != RuleOrigin::Builtin)
                    || crate::packages::context_ambiguous(name, &outside).is_some()
                    || crate::packages::command_signature_in_context(
                        local.source(),
                        name,
                        end,
                        &outside,
                    ) != Some("m")
                    || crate::packages::context_invocation_ranges(
                        local.source(),
                        name,
                        end,
                        &outside,
                    )
                    .is_some()
                    || local
                        .invocation_rule(token.span.start)
                        .is_none_or(|r| r.origin != RuleOrigin::Builtin)
                {
                    return None;
                }
            }
        }
        let intersecting = self
            .reader_windows
            .iter()
            .filter(|w| w.span.start < body.end && body.start < w.span.end);
        let mut local = old.reader_windows.iter();
        for window in intersecting {
            let entry = local.next()?;
            let clipped = window.span.start.max(body.start) - body.start
                ..window.span.end.min(body.end) - body.start;
            if entry.span != clipped || !entry.same_state(window) {
                return None;
            }
        }
        if local.next().is_some() || old.reader_windows.is_empty() {
            return None;
        }
        let mut updated = self.project_semantics(document, change, |semantic| {
            semantic.invocations.retain(|at, _| !body.contains(at));
            semantic
                .rules
                .retain(|r| r.span.as_ref().is_none_or(|r| !body.contains(&r.start)));
            semantic.context.remove_inner_environments(&body);
        });
        updated.semantic.context.insert_inner_environments(
            body.start,
            &new.semantic.context,
            outside.environment(),
        );
        for r in new.rules().iter().filter(|r| r.span.is_some()) {
            let mut rule = r.clone();
            let span = rule.span.as_mut().expect("source rule");
            *span = body.start + span.start..body.start + span.end;
            updated.semantic.rules.push(rule);
        }
        for (&at, rule) in &new.semantic.invocations {
            let mut rule = rule.clone();
            if let Some(span) = &mut rule.span {
                *span = body.start + span.start..body.start + span.end;
            }
            updated.semantic.invocations.insert(body.start + at, rule);
        }
        let affected: HashSet<_> = old
            .semantic
            .builtin_first
            .keys()
            .chain(new.semantic.builtin_first.keys())
            .collect();
        for name in affected {
            let rule = old
                .rules()
                .iter()
                .chain(new.rules())
                .find(|r| &r.name == name)?
                .clone();
            updated
                .semantic
                .rules
                .retain(|r| r.name != *name || r.span.is_some());
            updated.semantic.builtin_first.remove(name);
            updated.semantic.registry.remove(name);
            if let Some(first) = updated
                .semantic
                .invocations
                .iter()
                .filter_map(|(at, r)| (r.name == *name).then_some(*at))
                .min()
            {
                updated.semantic.builtin_first.insert(name.clone(), first);
                updated.semantic.rules.push(rule.clone());
                register(&mut updated.semantic.registry, rule);
            }
        }
        updated.semantic.rules.sort_by_key(|r| {
            r.span
                .as_ref()
                .map(|span| span.start)
                .or_else(|| updated.semantic.builtin_first.get(&r.name).copied())
                .unwrap_or(0)
        });
        if [old, new].iter().any(|m| {
            m.rules()
                .iter()
                .any(|r| matches!(r.meaning, RuleMeaning::Environment(_)))
        }) {
            for name in ["begin", "end"] {
                updated.semantic.registry.remove(name);
                for rule in updated.semantic.rules.iter().filter(|r| r.name == name) {
                    register(&mut updated.semantic.registry, rule.clone());
                }
            }
        }
        let mut windows: Vec<syntax::ReaderWindow> = Vec::new();
        let mut add = |window: &syntax::ReaderWindow, span: Range<usize>| {
            if span.is_empty() {
                return;
            }
            if let Some(last) = windows.last_mut()
                && last.span.end == span.start
                && last.same_state(window)
            {
                last.span.end = span.end;
            } else {
                let mut entry = window.clone();
                entry.span = span;
                windows.push(entry);
            }
        };
        for w in self
            .reader_windows
            .iter()
            .take_while(|w| w.span.start < body.start)
        {
            add(w, w.span.start..w.span.end.min(body.start));
        }
        for w in &new.reader_windows {
            add(w, body.start + w.span.start..body.start + w.span.end);
        }
        for w in self.reader_windows.iter().filter(|w| w.span.end > body.end) {
            add(
                w,
                change.position(w.span.start.max(body.end))..change.position(w.span.end),
            );
        }
        updated.reader_windows = windows;
        Some(updated)
    }
    pub(crate) fn lexical_protection(&self) -> &[Range<usize>] {
        self.tokens.protected_ranges()
    }
    pub(crate) fn control_at(&self, at: usize) -> Option<(&str, usize)> {
        let index = self.tokens.partition_point(|token| token.span.end <= at);
        self.tokens
            .get(index)
            .filter(|token| token.span.start == at)
            .and_then(|token| token.control(self.source.as_str()))
    }
    pub fn declarations(&self) -> &[Declaration] {
        &self.semantic.declarations
    }
    pub fn environment_declarations(&self) -> &[EnvironmentDeclaration] {
        &self.semantic.environment_declarations
    }
    /// Header signatures resolved where an environment begins. A begin offset
    /// takes priority when adjacent begin tokens share a boundary.
    pub fn environment_arguments_at(&self, at: usize) -> Option<&EnvironmentArguments> {
        self.environment_arguments_at_begin(at)
            .or_else(|| self.environment_arguments_after(at))
    }

    pub(crate) fn environment_arguments_at_begin(
        &self,
        at: usize,
    ) -> Option<&EnvironmentArguments> {
        self.semantic.environment_arguments.get(&at)
    }

    pub(crate) fn environment_arguments_after(&self, at: usize) -> Option<&EnvironmentArguments> {
        self.semantic
            .environment_arguments
            .get(self.semantic.environment_header_starts.get(&at)?)
    }
    pub(crate) fn invocation_context_at(
        &self,
        at: usize,
    ) -> crate::packages::context::InvocationContext<'_> {
        self.semantic.context.at(at)
    }
    pub fn conditionals(&self) -> &[Conditional] {
        &self.semantic.conditionals
    }
    pub fn rules(&self) -> &[SyntaxRule] {
        &self.semantic.rules
    }
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }
    pub fn document_class(&self) -> Option<&str> {
        self.semantic.document_class.as_deref()
    }
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error)
    }
    pub fn reconstruct(&self) -> String {
        let mut result = String::with_capacity(self.source.len());
        for token in self.tokens.iter() {
            result.push_str(&self.source[token.span.clone()]);
        }
        result
    }
    pub fn boundary_at(&self, at: usize) -> Option<&EnvironmentAlias> {
        self.semantic.boundaries.get(&at)
    }
    pub(crate) fn explicit_wrapper_end(&self, at: usize) -> Option<usize> {
        self.semantic.explicit_wrapper_ends.get(&at).copied()
    }
    pub(crate) fn opens_group_at(&self, at: usize) -> bool {
        self.semantic.group_openers.contains(&at)
    }
    pub(crate) fn inline_math_shift_at(&self, at: usize) -> bool {
        self.semantic.inline_math_shifts.contains(&at)
    }
    pub fn command_arguments(&self, name: &str) -> Option<&EnvironmentArguments> {
        match &self.semantic.registry.get(name)?.meaning {
            RuleMeaning::Arguments(arguments) => Some(arguments),
            _ => None,
        }
    }
    pub fn resolved_rule(&self, name: &str) -> Option<&SyntaxRule> {
        self.semantic.registry.get(name)
    }
    pub fn invocation_rule(&self, at: usize) -> Option<&SyntaxRule> {
        self.semantic.invocations.get(&at)
    }
    pub(crate) fn invocation_argument_ranges_at(
        &self,
        at: usize,
    ) -> Option<&[syntax::xparse::ArgumentSlot]> {
        self.semantic
            .invocation_arguments
            .get(&at)
            .map(Vec::as_slice)
    }
    /// Operands proven to be stored or ignored by their source definition.
    /// Delimiters remain part of the range so printers can preserve their
    /// contents while retaining the surrounding argument structure.
    pub(crate) fn stored_argument_ranges(&self) -> &[Range<usize>] {
        &self.semantic.stored_arguments
    }
    pub(crate) fn inferred_math_signature_at(
        &self,
        at: usize,
    ) -> Option<crate::packages::math::Signature> {
        self.semantic
            .inferred_math_signatures
            .get(&at)
            .map(|&(shape, domains)| crate::packages::math::Signature { shape, domains })
    }
    pub(crate) fn constructed_commands(&self) -> &HashMap<usize, (String, usize)> {
        &self.semantic.constructed
    }
    pub fn exported_aliases(&self) -> &BTreeMap<String, EnvironmentAlias> {
        &self.semantic.exported
    }
    pub fn exported_unknown_aliases(&self) -> &HashSet<String> {
        &self.semantic.exported_unknown
    }
    pub fn exported_environment_arguments(
        &self,
    ) -> &BTreeMap<String, Option<EnvironmentArguments>> {
        &self.semantic.exported_environments
    }
    pub(crate) fn has_environment_declaration(&self, name: &str) -> bool {
        self.semantic.environment_declaration_names.contains(name)
    }
    #[doc(hidden)]
    pub fn exported_packages(&self) -> BTreeMap<String, bool> {
        self.semantic.context.export_packages()
    }
    #[doc(hidden)]
    pub fn exported_commands(&self) -> &BTreeMap<String, crate::config::ImportedCommand> {
        &self.semantic.exported_commands
    }
    pub fn opaque_ranges(&self) -> &[Range<usize>] {
        &self.semantic.opaque
    }
    pub fn protected_ranges(&self) -> &[Range<usize>] {
        &self.semantic.protected
    }
}

impl SemanticIndex {
    fn into_exports(self) -> ProjectExports {
        (
            self.exported
                .into_iter()
                .map(|(name, alias)| (name, Some(alias)))
                .chain(self.exported_unknown.into_iter().map(|name| (name, None)))
                .collect(),
            self.exported_environments,
            self.context.export_packages(),
            self.exported_commands,
            self.exported_verbatim,
        )
    }

    fn for_source(config: &Config, loaders: Option<&LoaderPositions>) -> Self {
        Self {
            declarations: Vec::new(),
            environment_declarations: Vec::new(),
            environment_declaration_names: HashSet::new(),
            environment_arguments: HashMap::new(),
            environment_header_starts: HashMap::new(),
            explicit_wrapper_ends: HashMap::new(),
            context: crate::packages::context::ContextIndex::with_config(config, loaders),
            conditionals: Vec::new(),
            rules: Vec::new(),
            builtin_first: HashMap::new(),
            boundaries: HashMap::new(),
            group_openers: HashSet::new(),
            inline_math_shifts: HashSet::new(),
            exported: BTreeMap::new(),
            exported_unknown: HashSet::new(),
            exported_environments: BTreeMap::new(),
            exported_verbatim: BTreeMap::new(),
            exported_commands: BTreeMap::new(),
            registry: HashMap::new(),
            invocations: HashMap::new(),
            invocation_arguments: HashMap::new(),
            stored_arguments: Vec::new(),
            inferred_math_signatures: HashMap::new(),
            constructed: HashMap::new(),
            opaque: Vec::new(),
            protected: Vec::new(),
            document_class: None,
        }
    }
}

struct SemanticReader<'a> {
    source: &'a str,
    tokens: &'a [SourceToken],
    semantic: &'a mut SemanticIndex,
    diagnostics: &'a mut Vec<Diagnostic>,
}

impl SemanticReader<'_> {
    fn index(
        &mut self,
        config: &Config,
        loader_positions: Option<&LoaderPositions>,
        purpose: IndexPurpose<'_>,
    ) {
        let record_invocations = !matches!(purpose, IndexPurpose::IndexedExports { .. });
        let export_reader = |token: &SourceToken| {
            opaque_verbatim_declarations(self.source, token).is_some()
                || token.control(self.source).is_some_and(|(name, end)| {
                    let name = name.trim_end_matches('*');
                    self.semantic.constructed.contains_key(&token.span.start)
                        || match name {
                            "include" => false,
                            "input" => crate::packages::context::input_package_at(self.source, end)
                                .is_some(),
                            _ => syntax::project_export_reader(name),
                        }
                })
        };
        let stop_after = if matches!(purpose, IndexPurpose::Document)
            || !config.syntax.command_import_events.is_empty()
            || !config.syntax.environment_alias_events.is_empty()
            || !config.syntax.environment_argument_events.is_empty()
            || !config.syntax.package_load_events.is_empty()
        {
            self.tokens.len()
        } else if let IndexPurpose::IndexedExports { readers, .. } = purpose {
            readers
                .iter()
                .rev()
                .copied()
                .find(|&index| export_reader(&self.tokens[index]))
                .map_or(0, |last| last + 1)
        } else {
            self.tokens
                .iter()
                .rposition(export_reader)
                .map_or(0, |last| last + 1)
        };
        let transparent_group = |token: &SourceToken| {
            let (name, end) = token.control(self.source)?;
            let range = syntax::token_list_argument_range(self.source, name, end)?;
            Some([range.start, range.end - 1])
        };
        let transparent: HashSet<_> =
            if let IndexPurpose::IndexedExports { transparent, .. } = purpose {
                transparent
                    .iter()
                    .filter_map(|&index| transparent_group(&self.tokens[index]))
                    .flatten()
                    .collect()
            } else {
                self.tokens
                    .iter()
                    .filter_map(transparent_group)
                    .flatten()
                    .collect()
            };
        // Only foreach lookahead and grouped primitive definition targets need
        // matching token indices. Ordinary argument groups need no lookup table.
        let pairs = std::cell::LazyCell::new(|| {
            let mut pairs = HashMap::new();
            let mut groups = Vec::new();
            for (index, token) in self.tokens.iter().enumerate() {
                match token.kind {
                    SourceTokenKind::OpenGroup => groups.push(index),
                    SourceTokenKind::CloseGroup => {
                        if let Some(open) = groups.pop() {
                            pairs.insert(open, index);
                        }
                    }
                    _ => (),
                }
            }
            pairs
        });
        let closing_group = |index| match purpose {
            IndexPurpose::IndexedExports { closing_group, .. } => closing_group(index),
            _ => pairs.get(&index).copied(),
        };
        let mut bindings = HashMap::<String, Binding>::new();
        let mut unresolved_alias_suffix: Option<usize> = None;
        let mut unconditional_document = false;
        if config.syntax.environment_argument_events.is_empty() {
            for (name, signature) in &config.syntax.inferred_environment_arguments {
                bindings.insert(
                    format!("environment:{name}"),
                    Binding {
                        replacement: Replacement::Unknown,
                        candidate: false,
                        signature: Some(signature.clone()),
                        condition: None,
                        literal_arguments: false,
                        literal_slots: 0,
                        stored_slots: 0,
                        math_signature: None,
                        global: false,
                    },
                );
            }
        }
        let local_targets: HashSet<_> = if config.syntax.inferred_environment_aliases.is_empty() {
            HashSet::new()
        } else {
            let target = |at: usize| {
                let token = &self.tokens[at];
                (token.kind == SourceTokenKind::Control)
                    .then(|| definition(self.source, self.tokens, &closing_group, at))
                    .flatten()
                    .map(|(d, _, _, _)| d.name)
            };
            if let IndexPurpose::IndexedExports { readers, .. } = purpose {
                readers.iter().copied().filter_map(target).collect()
            } else {
                (0..self.tokens.len()).filter_map(target).collect()
            }
        };
        for (name, alias) in &config.syntax.inferred_environment_aliases {
            // Definitions within this file use their exact source lifetime.
            // A final project snapshot cannot move them before their declaration.
            if !local_targets.contains(name) {
                bindings.insert(
                    name.clone(),
                    Binding {
                        replacement: Replacement::Boundary(alias.clone()),
                        candidate: true,
                        signature: None,
                        condition: None,
                        literal_arguments: false,
                        literal_slots: 0,
                        stored_slots: 0,
                        math_signature: None,
                        global: false,
                    },
                );
            }
        }
        for (name, alias) in &config.syntax.environment_aliases {
            bindings.insert(
                name.clone(),
                Binding {
                    replacement: Replacement::Boundary(alias.clone()),
                    candidate: true,
                    signature: None,
                    condition: None,
                    literal_arguments: false,
                    literal_slots: 0,
                    stored_slots: 0,
                    math_signature: None,
                    global: false,
                },
            );
            self.semantic.rules.push(SyntaxRule {
                name: name.clone(),
                meaning: RuleMeaning::Environment(alias.clone()),
                origin: RuleOrigin::Configuration,
                span: None,
            });
        }
        for (name, rule) in &config.syntax.commands {
            self.semantic.rules.push(SyntaxRule {
                name: name.clone(),
                meaning: RuleMeaning::Arguments(EnvironmentArguments {
                    arguments: rule.arguments.unwrap_or(0),
                    optional_arguments: rule.optional_arguments,
                    signature: rule.signature.clone(),
                }),
                origin: RuleOrigin::Configuration,
                span: None,
            });
        }
        let mut scopes = vec![BindingScope::default()];
        let mut conditions: Vec<(usize, Vec<Range<usize>>, usize)> = Vec::new();
        let mut pending_conditions = BTreeMap::<usize, Vec<usize>>::new();
        let mut pending_operands = BTreeMap::<usize, usize>::new();
        let mut active_conditions = BinaryHeap::<Reverse<usize>>::new();
        let mut seen_builtins = HashSet::new();
        let mut foreach_variables = HashMap::<String, usize>::new();
        let mut global_prefix = false;
        let mut conditional_proofs = ConditionalProofs::new();
        let imports = SourceModel::import_events(self.source, config, loader_positions);
        let mut import_index = 0;
        let mut restore_depth = None;
        let mut pos = 0;
        'tokens: while pos < self.tokens.len() {
            if pos >= stop_after
                && match purpose {
                    IndexPurpose::Document => false,
                    IndexPurpose::Definitions => conditions.is_empty(),
                    IndexPurpose::Exports | IndexPurpose::IndexedExports { .. } => {
                        let depth = restore_depth.get_or_insert_with(|| {
                            scopes
                                .iter()
                                .enumerate()
                                .skip(1)
                                .find(|(_, scope)| !scope.previous.is_empty())
                                .map_or(usize::MAX, |(depth, _)| depth)
                        });
                        conditional_proofs.is_empty() && scopes.len() <= *depth
                    }
                }
            {
                break;
            }
            let token = &self.tokens[pos];
            let start = token.span.start;
            while pending_operands
                .first_key_value()
                .is_some_and(|(&at, _)| at <= start)
            {
                let (_, end) = pending_operands.pop_first().unwrap();
                if start < end {
                    pos = token_after_read(self.tokens, pos, end);
                    continue 'tokens;
                }
            }
            while pending_conditions
                .first_key_value()
                .is_some_and(|(&at, _)| at <= start)
            {
                let (_, ends) = pending_conditions.pop_first().unwrap();
                active_conditions.extend(ends.into_iter().map(Reverse));
            }
            while active_conditions.peek().is_some_and(|end| end.0 <= start) {
                active_conditions.pop();
            }
            let certain = conditions.is_empty() && active_conditions.is_empty();
            while imports
                .get(import_index)
                .is_some_and(|event| event.at <= start)
            {
                for (name, alias) in &imports[import_index].aliases {
                    if !config.syntax.environment_aliases.contains_key(name) {
                        let command = imports[import_index].commands.get(name);
                        let replacement = match alias {
                            Some(alias) => Replacement::Boundary(alias.clone()),
                            None if command.is_some_and(|c| c.composable_environment_wrapper) => {
                                Replacement::NeutralWrapper
                            }
                            None if command.is_some_and(|c| c.neutral_environment_wrapper) => {
                                Replacement::FlatWrapper
                            }
                            None => Replacement::Unknown,
                        };
                        assign(
                            &mut bindings,
                            &mut scopes,
                            name,
                            Binding {
                                replacement,
                                candidate: true,
                                signature: command.and_then(|command| command.arguments.clone()),
                                condition: None,
                                literal_arguments: command
                                    .is_some_and(|command| command.literal_arguments),
                                stored_slots: 0,
                                literal_slots: command.map_or(0, |command| command.literal_slots),
                                math_signature: None,
                                global: false,
                            },
                            command.is_some_and(|command| command.global),
                        );
                    }
                }
                for (name, signature) in &imports[import_index].arguments {
                    assign(
                        &mut bindings,
                        &mut scopes,
                        &format!("environment:{name}"),
                        Binding {
                            replacement: Replacement::Unknown,
                            candidate: false,
                            signature: signature.clone(),
                            condition: None,
                            literal_arguments: false,
                            literal_slots: 0,
                            stored_slots: 0,
                            math_signature: None,
                            global: false,
                        },
                        false,
                    );
                }
                for (name, command) in &imports[import_index].commands {
                    if !config.syntax.commands.contains_key(name)
                        && !imports[import_index].aliases.contains_key(name)
                    {
                        assign(
                            &mut bindings,
                            &mut scopes,
                            name,
                            Binding {
                                replacement: if command.composable_environment_wrapper {
                                    Replacement::NeutralWrapper
                                } else if command.neutral_environment_wrapper {
                                    Replacement::FlatWrapper
                                } else {
                                    Replacement::Unknown
                                },
                                candidate: command.neutral_environment_wrapper,
                                signature: command.arguments.clone(),
                                condition: None,
                                literal_arguments: command.literal_arguments,
                                stored_slots: 0,
                                literal_slots: command.literal_slots,
                                math_signature: None,
                                global: false,
                            },
                            command.global,
                        );
                    }
                }
                import_index += 1;
            }
            if token.kind == SourceTokenKind::OpenGroup {
                if transparent.contains(&token.span.start) {
                    pos += 1;
                    continue;
                }
                scopes.push(BindingScope::default());
                pos += 1;
                continue;
            }
            if token.kind == SourceTokenKind::CloseGroup {
                if transparent.contains(&token.span.start) {
                    pos += 1;
                    continue;
                }
                restore_scope(&mut bindings, &mut scopes);
                pos += 1;
                continue;
            }
            let Some((control, end)) = token.control(self.source) else {
                if let Some(declarations) = opaque_verbatim_declarations(self.source, token) {
                    for declaration in declarations {
                        for name in declaration.names {
                            assign(
                                &mut bindings,
                                &mut scopes,
                                &format!("environment:{name}"),
                                Binding {
                                    replacement: if certain {
                                        Replacement::VerbatimEnvironment(declaration.terminator)
                                    } else {
                                        Replacement::Unknown
                                    },
                                    candidate: false,
                                    signature: None,
                                    condition: None,
                                    literal_arguments: false,
                                    literal_slots: 0,
                                    stored_slots: 0,
                                    math_signature: None,
                                    global: false,
                                },
                                global_prefix,
                            );
                        }
                    }
                    global_prefix = false;
                }
                pos += 1;
                continue;
            };
            let constructed = self.semantic.constructed.get(&start);
            let end = constructed.map_or(end, |(_, finish)| *finish);
            let control = constructed.map_or(control, |(name, _)| name.as_str());
            let name = control.trim_end_matches('*');
            if foreach_variables
                .get(name)
                .is_some_and(|&finish| start < finish)
            {
                if record_invocations {
                    self.semantic.invocations.insert(
                        start,
                        SyntaxRule {
                            name: name.to_owned(),
                            meaning: RuleMeaning::Unknown,
                            origin: RuleOrigin::Source,
                            span: Some(start..end),
                        },
                    );
                }
                pos += 1;
                continue;
            }
            let current_binding = bindings.get(name);
            let configured_rule = config.syntax.commands.get(name);
            let unshadowed = current_binding.is_none() && configured_rule.is_none();
            if name == "foreach"
                && unshadowed
                && let Some(binding) = syntax::arguments::foreach_binding(self.source, end, |at| {
                    let index = token_after_read(self.tokens, pos, at);
                    closing_group(index).map(|close| self.tokens[close].span.end)
                })
            {
                for variable in binding.names {
                    foreach_variables
                        .entry(variable.to_owned())
                        .and_modify(|finish| *finish = (*finish).max(binding.end))
                        .or_insert(binding.end);
                }
            }
            if unshadowed
                && let Some(environment) = crate::packages::scoped_environment_for_command(control)
            {
                self.semantic.context.observe_environment(
                    environment,
                    control.strip_prefix("end") != Some(environment),
                    end,
                );
            }
            if unshadowed
                && let Some(branches) =
                    crate::packages::command_branch_ranges(self.source, control, end)
            {
                for branch in branches {
                    pending_conditions
                        .entry(branch.start)
                        .or_default()
                        .push(branch.end);
                }
            }
            // A terminal branch never reaches its \fi. The continuing branch
            // keeps the same document open, including its package context.
            if name == "end"
                && unconditional_document
                && !conditions.is_empty()
                && !bindings.contains_key("end")
                && !bindings.contains_key("fi")
                && self.semantic.context.at(start).environment() == Some("document")
                && let Some(("document", false, finish)) =
                    syntax::env_after_command(self.source, name, end)
                && syntax::command_at(self.source, skip(self.source, finish))
                    .is_some_and(|(command, _)| command == "fi")
            {
                self.semantic.protected.push(start..finish);
                pos = token_after_read(self.tokens, pos, finish);
                continue;
            }
            self.semantic
                .context
                .observe(self.source, control, start, end, certain);
            if unshadowed
                && let Some((alias, finish)) =
                    crate::packages::documents::branch_environment_boundary(self.source, name, end)
            {
                self.semantic
                    .context
                    .observe_environment(&alias.environment, true, finish);
                scopes.push(BindingScope::environment(&alias.environment));
                self.semantic.boundaries.insert(start, alias);
                self.semantic.protected.push(end..finish);
                pos = token_after_read(self.tokens, pos, finish);
                continue;
            }
            if name == "documentclass" && self.semantic.document_class.is_none() {
                let arguments = crate::layout::signature_argument_ranges(
                    self.source,
                    end..self.source.len(),
                    "om".bytes(),
                );
                if let Some(argument) = arguments.last()
                    && self.source.as_bytes()[argument.start] == b'{'
                {
                    let class = self.source[argument.start + 1..argument.end - 1].trim();
                    if !class.is_empty() && !class.contains(['\\', '#', '{', '}']) {
                        self.semantic.document_class = Some(class.into());
                    }
                }
            }
            if name == "global" && current_binding.is_none() {
                global_prefix = true;
                pos += 1;
                continue;
            }
            if matches!(name, "long" | "outer" | "protected") && current_binding.is_none() {
                pos += 1;
                continue;
            }
            if matches!(name, "begingroup" | "bgroup") {
                scopes.push(BindingScope::default());
                pos += 1;
                continue;
            }
            if matches!(name, "endgroup" | "egroup") {
                restore_scope(&mut bindings, &mut scopes);
                pos += 1;
                continue;
            }
            if unshadowed
                && let Some(reason) =
                    crate::packages::context_ambiguous(control, &self.semantic.context.at(start))
            {
                let finish = adjacent_arguments_end(self.source, end);
                self.semantic.opaque.push(start..finish);
                self.diagnostics.push(Diagnostic {
                    severity: Severity::Warning,
                    message: format!("Cannot choose the grammar of \\{control}: {reason}; this invocation is preserved"),
                    span: start..end,
                });
                pos = token_after_read(self.tokens, pos, finish);
                continue;
            }
            let invocation = if !record_invocations {
                None
            } else if let Some(rule) = configured_rule {
                Some(SyntaxRule {
                    name: name.to_owned(),
                    meaning: RuleMeaning::Arguments(EnvironmentArguments {
                        arguments: rule.arguments.unwrap_or(0),
                        optional_arguments: rule.optional_arguments,
                        signature: rule.signature.clone(),
                    }),
                    origin: RuleOrigin::Configuration,
                    span: Some(start..end),
                })
            } else if let Some(binding) = current_binding {
                let meaning = binding_meaning(name, binding, &bindings);
                Some(SyntaxRule {
                    name: name.to_owned(),
                    meaning,
                    origin: RuleOrigin::Source,
                    span: Some(start..end),
                })
            } else {
                crate::packages::command_signature_in_context(
                    self.source,
                    control,
                    end,
                    &self.semantic.context.at(start),
                )
                .map(|signature| SyntaxRule {
                    name: name.to_owned(),
                    meaning: RuleMeaning::Arguments(EnvironmentArguments {
                        signature: Some(signature.into()),
                        ..Default::default()
                    }),
                    origin: RuleOrigin::Builtin,
                    span: Some(start..end),
                })
                .or_else(|| {
                    if !config.arguments.align_consecutive {
                        return None;
                    }
                    let arguments = if let Some(signature) =
                        crate::package_commands::declaration_signature(self.source, control, end)
                    {
                        EnvironmentArguments {
                            signature: Some(signature.into()),
                            ..Default::default()
                        }
                    } else {
                        let rule = crate::package_commands::builtin_command_rule(name)?;
                        EnvironmentArguments {
                            arguments: rule.arguments?,
                            optional_arguments: rule.optional_arguments,
                            signature: rule.signature.clone(),
                        }
                    };
                    Some(SyntaxRule {
                        name: name.to_owned(),
                        meaning: RuleMeaning::Arguments(arguments),
                        origin: RuleOrigin::Builtin,
                        span: Some(start..end),
                    })
                })
            };
            if let Some(rule) = invocation {
                if rule.origin == RuleOrigin::Source
                    && let Some(signature) = bindings
                        .get(name)
                        .and_then(|binding| binding.math_signature)
                {
                    self.semantic
                        .inferred_math_signatures
                        .insert(start, signature);
                }
                if rule.origin == RuleOrigin::Builtin
                    && let Some(ranges) = crate::packages::context_invocation_ranges(
                        self.source,
                        control,
                        end,
                        &self.semantic.context.at(start),
                    )
                {
                    match ranges {
                        Ok(ranges) => {
                            self.semantic.invocation_arguments.insert(start, ranges);
                        }
                        Err(span) => {
                            self.semantic.opaque.push(start..span.end);
                            self.diagnostics.push(Diagnostic {
                                severity: Severity::Error,
                                message: format!(
                                    "Unclosed argument of \\{control}; source retained"
                                ),
                                span,
                            });
                        }
                    }
                }
                if let RuleMeaning::Arguments(arguments) = &rule.meaning
                    && !self.semantic.invocation_arguments.contains_key(&start)
                    && let Some(signature) = arguments.signature.as_deref()
                    && let Some(span) = syntax::arguments::unclosed_argument(
                        self.source,
                        end,
                        signature,
                        rule.origin == RuleOrigin::Builtin && control == "item",
                        |at| {
                            let index = token_after_read(self.tokens, pos, at);
                            self.tokens.get(index).filter(|token| {
                                token.span.start == at && token.kind == SourceTokenKind::OpenGroup
                            })?;
                            closing_group(index).map(|close| self.tokens[close].span.end)
                        },
                    )
                    && !(rule.origin == RuleOrigin::Builtin
                        && crate::packages::command_data_end(self.source, control, end).is_some())
                {
                    self.diagnostics.push(Diagnostic {
                        severity: Severity::Error,
                        message: format!("Unclosed argument of \\{control}; source retained"),
                        span,
                    });
                }
                self.semantic.invocations.insert(start, rule);
            }
            if matches!(name, "begin" | "end") {
                let argument = skip(self.source, end);
                if let Some(finish) = syntax::balanced_end(self.source, argument, b'{', b'}') {
                    let mut alias = EnvironmentAlias {
                        environment: self.source[argument + 1..finish - 1].trim().into(),
                        begin: name == "begin",
                    };
                    let mut finish = finish;
                    if alias.begin && alias.environment == "document" {
                        unconditional_document = certain;
                    }
                    // The lexical index skips quoted commands, comments and
                    // delayed bodies. Keep a final executable document closer
                    // visible after an opaque environment wrapper.
                    if !alias.begin
                        && alias.environment == "document"
                        && certain
                        && !bindings.contains_key("end")
                        && self
                            .semantic
                            .context
                            .at(start.saturating_sub(1))
                            .environment()
                            == Some("document")
                        && skip(self.source, finish) == self.source.len()
                        && let Some(index) = unresolved_alias_suffix.take()
                        && self.semantic.rules.iter().any(|rule| {
                            rule.span.as_ref().is_some_and(|span| span.end <= self.semantic.opaque[index].start)
                                && matches!(&rule.meaning, RuleMeaning::Environment(alias) if alias.begin && alias.environment == "document")
                        })
                    {
                        self.semantic.opaque[index].end = start;
                    }
                    if !alias.begin
                        && let Some(outer) = self.semantic.context.at(start).environment()
                        && outer != alias.environment
                        && let Some(binding) = bindings.get(&format!("environment:{outer}"))
                        && matches!(&binding.replacement, Replacement::WrappedEnvironment(inner) if *inner == alias.environment)
                        && let Some(("endgroup", after)) =
                            syntax::command_at(self.source, skip(self.source, finish))
                        && !bindings.contains_key("endgroup")
                    {
                        alias.environment = outer.to_owned();
                        self.semantic
                            .context
                            .skip_environment(&alias.environment, after);
                        self.semantic.boundaries.insert(start, alias.clone());
                        self.semantic.explicit_wrapper_ends.insert(start, after);
                        finish = after;
                    }
                    if alias.begin
                        && !config
                            .syntax
                            .environment_arguments
                            .contains_key(&alias.environment)
                        && !bindings.contains_key(&format!("environment:{}", alias.environment))
                        && let Some(reason) = crate::packages::context_ambiguous(
                            &alias.environment,
                            &self.semantic.context.at(start),
                        )
                    {
                        self.diagnostics.push(Diagnostic {
                            severity: Severity::Warning,
                            message: format!(
                                "Cannot choose the header grammar of environment {:?}: {reason}",
                                alias.environment
                            ),
                            span: start..finish,
                        });
                    }
                    if alias.begin {
                        let key = format!("environment:{}", alias.environment);
                        let arguments = config
                            .syntax
                            .environment_arguments
                            .get(&alias.environment)
                            .filter(|_| {
                                !config
                                    .syntax
                                    .inferred_environment_arguments
                                    .contains_key(&alias.environment)
                            })
                            .or_else(|| {
                                bindings
                                    .get(&key)
                                    .and_then(|binding| binding.signature.as_ref())
                            });
                        if let Some(arguments) = arguments {
                            self.semantic
                                .environment_arguments
                                .insert(start, arguments.clone());
                            self.semantic
                                .environment_header_starts
                                .insert(finish, start);
                        } else if bindings.contains_key(&key) {
                            let invocation_end = syntax::inferred_environment_end(
                                self.source,
                                finish,
                                &alias.environment,
                                config,
                            );
                            self.semantic.opaque.push(start..invocation_end);
                            self.diagnostics.push(Diagnostic {
                                severity: Severity::Warning,
                                message: format!("Cannot safely infer the header of environment {:?}; this invocation is preserved", alias.environment),
                                span: start..finish,
                            });
                            pos = token_after_read(self.tokens, pos, invocation_end);
                            continue;
                        }
                    }
                    // Environment invocation groups have the same lifetime as
                    // brace groups; the document wrapper supplies file context.
                    if alias.environment != "document" {
                        if alias.begin {
                            scopes.push(BindingScope::environment(&alias.environment));
                        } else {
                            restore_scope(&mut bindings, &mut scopes);
                        }
                    }
                    self.semantic.rules.push(SyntaxRule {
                        name: name.to_owned(),
                        meaning: RuleMeaning::Environment(alias),
                        origin: RuleOrigin::Builtin,
                        span: Some(start..finish),
                    });
                    pos = token_after_read(self.tokens, pos, finish);
                    continue;
                }
            }
            if current_binding.is_none()
                && let Some(declaration) =
                    syntax::source_environment_declaration(self.source, start, control, end)
            {
                let key = format!("environment:{}", declaration.name);
                let provided_builtin = declaration.provide
                    && !bindings.contains_key(&key)
                    && crate::packages::header_signature(&declaration.name).is_some();
                if !declaration.provide || !bindings.contains_key(&key) && !provided_builtin {
                    let signature = if certain {
                        declaration.signature.clone()
                    } else {
                        bindings
                            .get(&key)
                            .and_then(|binding| binding.signature.as_ref())
                            .filter(|prior| Some(*prior) == declaration.signature.as_ref())
                            .cloned()
                    };
                    assign(
                        &mut bindings,
                        &mut scopes,
                        &key,
                        Binding {
                            replacement: if certain {
                                wrapped_environment(self.source, &declaration)
                                    .map_or(Replacement::Unknown, Replacement::WrappedEnvironment)
                            } else {
                                Replacement::Unknown
                            },
                            candidate: false,
                            signature,
                            condition: None,
                            literal_arguments: false,
                            literal_slots: 0,
                            stored_slots: 0,
                            math_signature: None,
                            global: false,
                        },
                        global_prefix,
                    );
                }
                let finish = declaration.span.end;
                if !provided_builtin {
                    self.semantic
                        .environment_declaration_names
                        .insert(declaration.name.to_owned());
                    self.semantic.environment_declarations.push(declaration);
                }
                global_prefix = false;
                pos = token_after_read(self.tokens, pos, finish);
                continue;
            }
            if current_binding.is_none()
                && (name.ends_with("environment")
                    || name.ends_with("DocumentEnvironment")
                    || matches!(name, "NewEnviron" | "RenewEnviron"))
                && let Some(finish) = syntax::delayed_data_end(self.source, control, end)
            {
                self.semantic.opaque.push(start..finish);
                self.diagnostics.push(Diagnostic {
                    severity: Severity::Warning,
                    message: "Cannot infer a literal environment name from this declaration; the declaration is preserved".into(),
                    span: start..finish,
                });
                pos = token_after_read(self.tokens, pos, finish);
                continue;
            }
            if current_binding.is_none()
                && let Some(definition) = definition(self.source, self.tokens, &closing_group, pos)
            {
                let (mut declaration, finish, global, provide) = definition;
                // Expl3 definitions share the delayed-body semantics of
                // classic declarations. The code printer can expose their
                // body separately; ordinary structural/math passes must not
                // execute stored begin/end tokens within that replacement.
                if config.protect.macro_bodies && name.starts_with("cs_") {
                    self.semantic.protected.push(declaration.body.clone());
                }
                let body = &self.source[declaration.body.clone()];
                let discardable_slots = if certain {
                    discarded_parameter_mask(
                        name,
                        declaration
                            .signature
                            .as_ref()
                            .and_then(|arguments| arguments.signature.as_deref()),
                        body,
                    )
                } else {
                    0
                };
                let zero = declaration.signature.as_ref().is_some_and(|s| {
                    s.arguments == 0
                        && !s.optional_arguments
                        && s.signature.as_deref().is_none_or(str::is_empty)
                });
                let mut forwarded_data = false;
                if zero {
                    let start = skip(body, 0);
                    if let Some((target, after)) = syntax::command_at(body, start)
                        && let Some(signature) = bindings
                            .get(target)
                            .filter(|binding| binding.literal_arguments)
                            .and_then(|binding| binding.signature.as_ref())
                            .and_then(|s| s.signature.as_deref())
                            .or_else(|| {
                                (!bindings.contains_key(target)
                                    && crate::packages::commands::is_metadata_command(target))
                                .then(|| crate::packages::command_signature(body, target, after))
                                .flatten()
                            })
                        && signature.bytes().all(|slot| slot == b'm')
                    {
                        let mut cursor = skip(body, after);
                        let mut supplied = 0usize;
                        while cursor < body.len() && body.as_bytes()[cursor] == b'{' {
                            let Some(end) = syntax::balanced_end(body, cursor, b'{', b'}') else {
                                break;
                            };
                            supplied += 1;
                            cursor = skip(body, end);
                        }
                        if cursor == body.len() && supplied < signature.len() {
                            let remaining = signature.len() - supplied;
                            declaration.signature = Some(EnvironmentArguments {
                                arguments: remaining,
                                signature: Some("m".repeat(remaining)),
                                ..Default::default()
                            });
                            forwarded_data = true;
                        }
                    }
                }
                let previous_candidate =
                    bindings.get(&declaration.name).is_some_and(|b| b.candidate);
                // Declaration bodies may be opaque in the document partition.
                // Read their lexical children with the same scanner before
                // interpreting complete command names.
                let contains_boundary = body_has_environment_effect(body, config, false, false);
                let neutral_composition = !contains_boundary
                    && zero
                    && SourceTokens::new(body, &[]).all(|token| {
                        if token.kind != SourceTokenKind::Control {
                            return true;
                        }
                        let Some((name, end)) = token.control(body) else {
                            return false;
                        };
                        bindings.get(name).is_none_or(|binding| {
                            !binding.candidate
                                || matches!(
                                    binding.replacement,
                                    Replacement::NeutralWrapper | Replacement::FlatWrapper
                                ) && binding.signature.as_ref().is_some_and(|signature| {
                                    flat_wrapper_arguments(
                                        body, end, signature, config, &bindings, 0,
                                    )
                                })
                        })
                    });
                let parameterized_wrapper = body.contains('#')
                    && if contains_boundary {
                        !body_has_environment_effect(body, config, true, false)
                    } else {
                        references_environment_candidate(body, &bindings)
                    }
                    && SourceTokens::new(body, &[]).all(|token| {
                        token.control(body).is_none_or(|(control, _)| {
                            let name = control.trim_end_matches('*');
                            (contains_boundary
                                || !matches!(
                                    name,
                                    "csname" | "endcsname" | "expandafter" | "scantokens"
                                ))
                                && bindings.get(name).is_none_or(|binding| {
                                    !binding.candidate
                                        || matches!(
                                            binding.replacement,
                                            Replacement::NeutralWrapper | Replacement::FlatWrapper
                                        )
                                })
                        })
                    });
                // A definite redefinition replaces the old grammar. Only
                // uncertain assignments retain the previous boundary candidate.
                let candidate = !certain && previous_candidate
                    || contains_boundary
                    || !neutral_composition && references_environment_candidate(body, &bindings);
                let literal_arguments = certain
                    && !candidate
                    && (forwarded_data
                        || parameters_only_in_data(
                            body,
                            &bindings,
                            Some(&self.semantic.context.at(start)),
                        )
                        || name.starts_with("cs_") && !body.contains('#'));
                let mut replacement = if certain
                    && contains_boundary
                    && declaration
                        .signature
                        .as_ref()
                        .and_then(|s| s.signature.as_deref())
                        == Some("m")
                    && (syntax::classic_command_definition(name)
                        || matches!(
                            name,
                            "def"
                                | "gdef"
                                | "NewDocumentCommand"
                                | "RenewDocumentCommand"
                                | "ProvideDocumentCommand"
                                | "DeclareDocumentCommand"
                        ))
                    && !config.syntax.commands.contains_key(name)
                    && parameter_restart_body(body)
                {
                    Replacement::RestartArgument
                } else if certain
                    && !zero
                    && contains_boundary
                    && !config.syntax.commands.contains_key(name)
                    && let Some(proof) =
                        NamedWrapper::classify(name, body, declaration.signature.as_ref())
                {
                    Replacement::NamedWrapper(proof)
                } else if certain && parameterized_wrapper {
                    if composable_wrapper_definition(self.source, &declaration, config) {
                        Replacement::NeutralWrapper
                    } else {
                        Replacement::FlatWrapper
                    }
                } else if certain
                    && zero
                    && (body.bytes().all(|b| b.is_ascii_alphanumeric())
                        || matches!(name, "edef" | "xdef")
                            && !bindings.contains_key("arabic")
                            && body
                                .trim()
                                .strip_prefix(r"\arabic{")
                                .and_then(|s| s.strip_suffix('}'))
                                .is_some_and(|s| {
                                    !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphabetic())
                                }))
                {
                    Replacement::LiteralText
                } else if certain && zero {
                    replacement(body)
                } else {
                    Replacement::Unknown
                };
                if matches!(name, "edef" | "xdef")
                    && let Replacement::Forward(target) = &replacement
                {
                    replacement = resolve(target, &bindings)
                        .map_or(Replacement::Unknown, Replacement::Boundary);
                }
                if certain
                    && candidate
                    && matches!(replacement, Replacement::Unknown)
                    && body.contains("\\csname")
                {
                    replacement = Replacement::ConstructedWrapper(body.to_owned());
                }
                let stored_slots = stored_parameter_mask_with_bindings(
                    body,
                    &self.semantic.context.at(start),
                    &bindings,
                );
                if active_conditions.is_empty()
                    && let Some((at, branches, _)) = conditions.last()
                {
                    conditional_proofs
                        .entry(*at)
                        .or_default()
                        .entry(declaration.name.clone())
                        .or_default()
                        .insert(
                            branches.len(),
                            (
                                declaration.signature.clone(),
                                if forwarded_data {
                                    u16::MAX
                                } else {
                                    stored_slots
                                },
                                forwarded_data || literal_arguments,
                                candidate,
                            ),
                        );
                }
                if !(config
                    .syntax
                    .environment_aliases
                    .contains_key(&declaration.name)
                    || provide
                        && (bindings.contains_key(&declaration.name)
                            || syntax::context_builtin_command(&declaration.name)))
                {
                    assign(
                        &mut bindings,
                        &mut scopes,
                        &declaration.name,
                        Binding {
                            replacement,
                            candidate,
                            signature: if certain {
                                declaration.signature.clone()
                            } else {
                                None
                            },
                            condition: syntax::opens_final_condition(body).then(|| "iftrue".into()),
                            literal_arguments,
                            stored_slots,
                            literal_slots: stored_slots
                                | if certain && !forwarded_data {
                                    discardable_slots
                                } else {
                                    0
                                },
                            math_signature: certain
                                .then(|| crate::packages::scientific::factory_math_signature(name))
                                .flatten()
                                .map(|signature| (signature.shape, signature.domains)),
                            global: false,
                        },
                        global || global_prefix,
                    );
                }
                if let Some(signature) = &declaration.signature {
                    self.semantic.rules.push(SyntaxRule {
                        name: declaration.name.to_owned(),
                        meaning: RuleMeaning::Arguments(signature.clone()),
                        origin: RuleOrigin::Source,
                        span: Some(declaration.span.clone()),
                    });
                } else {
                    self.diagnostics.push(Diagnostic { severity: Severity::Warning, message: format!("Cannot infer the argument signature of \\{}; its definition is preserved", declaration.name), span: declaration.span.clone() });
                }
                self.semantic.declarations.push(declaration);
                global_prefix = false;
                pos = finish;
                continue;
            }
            if let Some((target, value, finish, global)) =
                syntax::command_copy(self.source, name, end)
            {
                let mut binding = bindings.get(value).cloned().unwrap_or(Binding {
                    replacement: Replacement::Forward(value.to_owned()),
                    candidate: resolve(value, &bindings).is_some(),
                    signature: None,
                    condition: (syntax::is_condition_command(value, config)
                        || matches!(value, "fi" | "else" | "or"))
                    .then(|| value.to_owned()),
                    literal_arguments: false,
                    literal_slots: 0,
                    stored_slots: 0,
                    math_signature: None,
                    global: false,
                });
                if !certain {
                    binding.replacement = Replacement::Unknown;
                    binding.signature = None;
                    binding.literal_arguments = false;
                    binding.stored_slots = 0;
                    binding.literal_slots = 0;
                    binding.math_signature = None;
                }
                assign(
                    &mut bindings,
                    &mut scopes,
                    target,
                    binding,
                    global || global_prefix,
                );
                global_prefix = false;
                pos = token_after_read(self.tokens, pos, finish);
                continue;
            }
            if matches!(name, "let" | "futurelet") {
                let target = next(self.tokens, pos + 1);
                if let Some(target_token) = self.tokens.get(target)
                    && let Some((target_name, _)) =
                        syntax::command_at(self.source, target_token.span.start)
                {
                    let mut rhs = next(self.tokens, target + 1);
                    if self
                        .tokens
                        .get(rhs)
                        .is_some_and(|t| self.source[t.span.clone()] == *"=")
                    {
                        rhs = next(self.tokens, rhs + 1);
                    }
                    if let Some(rhs_token) = self.tokens.get(rhs)
                        && (rhs_token.kind == SourceTokenKind::Control
                            || bindings.contains_key(target_name)
                            || matches!(target_name, "newpage" | "Vbreak" | "verse" | "endverse")
                            || named_wrapper::dependency(target_name))
                    {
                        let rhs_control = rhs_token.control(self.source).map(|(name, _)| name);
                        let mut binding = rhs_control
                            .and_then(|name| bindings.get(name))
                            .cloned()
                            .unwrap_or(Binding {
                                replacement: Replacement::Unknown,
                                candidate: false,
                                signature: None,
                                condition: rhs_control
                                    .filter(|name| {
                                        syntax::is_condition_command(name, config)
                                            || matches!(*name, "fi" | "else" | "or")
                                    })
                                    .map(str::to_owned),
                                literal_arguments: false,
                                literal_slots: 0,
                                stored_slots: 0,
                                math_signature: None,
                                global: false,
                            });
                        if !certain || name == "futurelet" {
                            binding.replacement = Replacement::Unknown;
                            binding.signature = None;
                            binding.literal_arguments = false;
                            binding.stored_slots = 0;
                            binding.literal_slots = 0;
                            binding.math_signature = None;
                            if name == "futurelet" {
                                binding.condition = None;
                            }
                        }
                        if !config.syntax.environment_aliases.contains_key(target_name) {
                            assign(
                                &mut bindings,
                                &mut scopes,
                                target_name,
                                binding,
                                global_prefix,
                            );
                        }
                        global_prefix = false;
                        pos = if name == "futurelet" {
                            target + 1
                        } else {
                            rhs + 1
                        };
                        continue;
                    }
                }
            }
            if matches!(name, "newif" | "ltx@newif" | "newife") {
                let target = next(self.tokens, pos + 1);
                if let Some(token) = self.tokens.get(target)
                    && let Some((name, _)) = syntax::command_at(self.source, token.span.start)
                {
                    assign(
                        &mut bindings,
                        &mut scopes,
                        name,
                        Binding {
                            replacement: Replacement::Unknown,
                            candidate: false,
                            signature: None,
                            condition: Some("iftrue".into()),
                            literal_arguments: false,
                            literal_slots: 0,
                            stored_slots: 0,
                            math_signature: None,
                            global: false,
                        },
                        false,
                    );
                    pos = target + 1;
                    continue;
                }
            }
            let configured_condition = config
                .syntax
                .conditions
                .iter()
                .any(|condition| condition == name);
            if let Some(reader) = current_binding.and_then(|binding| binding.condition.as_deref()) {
                if matches!(reader, "else" | "or") {
                    if let Some((_, branches, branch_start)) = conditions.last_mut() {
                        branches.push(*branch_start..start);
                        *branch_start = end;
                    }
                    pos += 1;
                    continue;
                }
                if reader == "fi" {
                    if let Some((at, mut branches, branch_start)) = conditions.pop() {
                        join_conditional_data(
                            self.source,
                            at,
                            &branches,
                            &mut conditional_proofs,
                            &mut bindings,
                        );
                        branches.push(branch_start..start);
                        self.semantic.conditionals.push(Conditional {
                            span: at..end,
                            branches,
                        });
                    }
                    pos += 1;
                    continue;
                }
            }
            let operand_slots = current_binding.map_or(0, |binding| {
                if binding.literal_arguments {
                    u16::MAX
                } else if configured_rule.is_some() {
                    binding.stored_slots
                } else {
                    binding.literal_slots
                }
            });
            if operand_slots != 0
                && let Some(arguments) =
                    current_binding.and_then(|binding| binding.signature.as_ref())
            {
                let cursor = end
                    - usize::from(
                        control.len() > 1 && control.ends_with('*') && constructed.is_none(),
                    );
                let ranges = if let Some(shape) = &arguments.signature {
                    syntax::xparse::Signature::parse(shape).and_then(|signature| {
                        signature.masked_ranges(self.source, cursor, operand_slots)
                    })
                } else if operand_slots == u16::MAX {
                    invocation_arguments(self.source, cursor, arguments)
                } else {
                    None
                };
                if let Some(ranges) = ranges {
                    for range in ranges {
                        pending_operands.insert(range.start, range.end);
                        self.semantic.stored_arguments.push(range.clone());
                        self.semantic.protected.push(range);
                    }
                }
            }
            if configured_condition
                || current_binding.map_or_else(
                    || syntax::is_condition_command(name, config),
                    |binding| binding.condition.is_some(),
                )
            {
                let reader = current_binding
                    .and_then(|binding| binding.condition.as_deref())
                    .unwrap_or(control);
                let operand_end = syntax::test_operand_end(self.source, reader, end).unwrap_or(end);
                if reader != control && operand_end > end {
                    self.semantic.protected.push(end..operand_end);
                }
                conditions.push((start, Vec::new(), operand_end));
                let origin = if configured_condition {
                    RuleOrigin::Configuration
                } else if current_binding.is_some() {
                    RuleOrigin::Source
                } else {
                    RuleOrigin::Builtin
                };
                self.semantic.rules.push(SyntaxRule {
                    name: name.to_owned(),
                    meaning: RuleMeaning::Condition,
                    origin,
                    span: Some(start..end),
                });
                pos = token_after_read(self.tokens, pos, operand_end);
                continue;
            }
            if matches!(name, "else" | "or") {
                if let Some((_, branches, branch_start)) = conditions.last_mut() {
                    branches.push(*branch_start..start);
                    *branch_start = end;
                }
                pos += 1;
                continue;
            }
            if name == "fi" {
                if let Some((at, mut branches, branch_start)) = conditions.pop() {
                    join_conditional_data(
                        self.source,
                        at,
                        &branches,
                        &mut conditional_proofs,
                        &mut bindings,
                    );
                    branches.push(branch_start..start);
                    self.semantic.conditionals.push(Conditional {
                        span: at..end,
                        branches,
                    });
                }
                pos += 1;
                continue;
            }
            let replacement = current_binding
                .map(|binding| &binding.replacement)
                .or_else(|| {
                    (name == "Vbreak" && self.semantic.context.at(start).has_package("altverse"))
                        .then_some(&Replacement::RestartArgument)
                });
            if matches!(replacement, Some(Replacement::GroupBegin)) {
                self.semantic.group_openers.insert(start);
                scopes.push(BindingScope::default());
            } else if matches!(replacement, Some(Replacement::InlineMathShift)) {
                self.semantic.inline_math_shifts.insert(start);
            } else if let Some(restart) = replacement.filter(|r| r.is_restart())
                && certain && control == name && configured_rule.is_none()
                && !config.protect.commands.iter().any(|entry| entry == name)
                && let Some(scope) = scopes.last()
                && let Some(top) = scope.environment.as_deref()
                && self.semantic.context.at(start).environment() == Some(top)
                && let Some(finish) = restart.restart_end(
                    self.source, end, top, config, |key, outer| {
                        if outer && let Some(saved) = scope.previous.get(key) {
                            return saved.as_ref().map(|b| &b.replacement);
                        }
                        bindings.get(key).map(|b| &b.replacement)
                    },
                )
            {
                let fresh = BindingScope::environment(top);
                restore_scope(&mut bindings, &mut scopes);
                scopes.push(fresh);
                pos = token_after_read(self.tokens, pos, finish);
                continue;
            } else if current_binding.is_some() && let Some(alias) = resolve(name, &bindings) {
                self.semantic.context
                    .observe_environment(&alias.environment, alias.begin, end);
                if alias.begin {
                    scopes.push(BindingScope::environment(&alias.environment));
                } else {
                    restore_scope(&mut bindings, &mut scopes);
                }
                self.semantic.boundaries.insert(start, alias.clone());
                self.semantic.rules.push(SyntaxRule {
                    name: name.to_owned(),
                    meaning: RuleMeaning::Environment(alias),
                    origin: RuleOrigin::Source,
                    span: Some(start..end),
                });
            } else if record_invocations && (replacement.is_some_and(Replacement::is_restart) || current_binding.is_some_and(|binding| {
                binding.candidate
                    && !(matches!(binding.replacement, Replacement::NeutralWrapper | Replacement::FlatWrapper)
                        && binding.signature.as_ref().is_some_and(|signature| {
                            let argument_start = if pos.checked_sub(1).is_some_and(|i| {
                                self.tokens[i].control(self.source).is_some_and(|(n, at)| n == "expandafter" && at == start)
                            }) && !bindings.contains_key("expandafter") {
                                expanded_text_group(self.source, end, &bindings).unwrap_or(end)
                            } else {
                                end
                            };
                            neutral_wrapper_arguments(
                                self.source,
                                argument_start,
                                signature,
                                config,
                                &bindings,
                                binding,
                            )
                        }) || matches!(&binding.replacement, Replacement::NamedWrapper(proof)
                            if control == name && constructed.is_none()
                                && proof.accepts(self.source, end, config, &bindings, &self.semantic.context.at(start)))
                            || matches!(&binding.replacement, Replacement::ConstructedWrapper(body)
                            if neutral_constructed_wrapper(body, self.source, end, binding.signature.as_ref(), config, &bindings)))
            })) {
                self.diagnostics.push(Diagnostic { severity: Severity::Warning, message: format!("Cannot safely resolve environment alias \\{name}; the following source is preserved"), span: start..end });
                if self.semantic.opaque.is_empty()
                    || replacement.is_some_and(Replacement::is_restart) && unresolved_alias_suffix.is_none()
                {
                    unresolved_alias_suffix = Some(self.semantic.opaque.len());
                    self.semantic.opaque.push(start..self.source.len());
                }
            } else if record_invocations && seen_builtins.insert(name)
                && let Some(rule) = crate::package_commands::builtin_command_rule(name)
            {
                self.semantic.builtin_first.insert(name.to_owned(), start);
                self.semantic.rules.push(SyntaxRule {
                    name: name.to_owned(),
                    meaning: RuleMeaning::Arguments(EnvironmentArguments {
                        arguments: rule.arguments.unwrap_or(0),
                        optional_arguments: rule.optional_arguments,
                        signature: rule.signature.clone(),
                    }),
                    origin: RuleOrigin::Builtin,
                    span: None,
                });
            }
            pos = if constructed.is_some() {
                token_after_read(self.tokens, pos, end)
            } else {
                pos + 1
            };
        }
        if matches!(purpose, IndexPurpose::Definitions) {
            return;
        }
        if let IndexPurpose::IndexedExports {
            tables: ExportTables::Aliases,
            ..
        } = purpose
        {
            self.semantic.exported = bindings
                .into_iter()
                .filter_map(|(name, binding)| {
                    if !name.starts_with("environment:")
                        && let Replacement::Boundary(alias) = binding.replacement
                    {
                        Some((name, alias))
                    } else {
                        None
                    }
                })
                .collect();
            return;
        }
        for name in bindings.keys() {
            if let Some(environment) = name.strip_prefix("environment:") {
                if let Replacement::VerbatimEnvironment(terminator) = bindings[name].replacement {
                    self.semantic
                        .exported_verbatim
                        .insert(environment.to_owned(), Some(terminator));
                    continue;
                }
                self.semantic
                    .exported_environments
                    .insert(environment.to_owned(), bindings[name].signature.clone());
                continue;
            }
            self.semantic.exported_commands.insert(
                name.clone(),
                crate::config::ImportedCommand {
                    arguments: bindings[name].signature.clone(),
                    literal_arguments: bindings[name].literal_arguments,
                    literal_slots: bindings[name].literal_slots & !bindings[name].stored_slots,
                    neutral_environment_wrapper: matches!(
                        bindings[name].replacement,
                        Replacement::NeutralWrapper | Replacement::FlatWrapper
                    ),
                    composable_environment_wrapper: matches!(
                        bindings[name].replacement,
                        Replacement::NeutralWrapper
                    ),
                    global: bindings[name].global,
                },
            );
            if let Replacement::Boundary(alias) = &bindings[name].replacement {
                self.semantic.exported.insert(name.clone(), alias.clone());
            } else if bindings[name].candidate || resolve(name, &bindings).is_some() {
                self.semantic.exported_unknown.insert(name.clone());
            }
        }
        if matches!(
            purpose,
            IndexPurpose::Exports | IndexPurpose::IndexedExports { .. }
        ) {
            return;
        }
        // Every provider produces the same rule shape and enters the same
        // precedence resolver. Source scope has settled before export.
        for rule in self
            .semantic
            .rules
            .iter()
            .filter(|r| r.origin != RuleOrigin::Source)
        {
            register(&mut self.semantic.registry, rule.clone());
        }
        for (name, binding) in &bindings {
            if name.starts_with("environment:") {
                continue;
            }
            let meaning = binding_meaning(name, binding, &bindings);
            register(
                &mut self.semantic.registry,
                SyntaxRule {
                    name: name.clone(),
                    meaning,
                    origin: RuleOrigin::Source,
                    span: None,
                },
            );
        }
    }
}

fn binding_meaning(
    name: &str,
    binding: &Binding,
    bindings: &HashMap<String, Binding>,
) -> RuleMeaning {
    if binding.condition.is_some() {
        RuleMeaning::Condition
    } else {
        binding
            .signature
            .as_ref()
            .map(|s| RuleMeaning::Arguments(s.clone()))
            .or_else(|| resolve(name, bindings).map(RuleMeaning::Environment))
            .unwrap_or(RuleMeaning::Unknown)
    }
}

fn invocation_arguments(
    source: &str,
    end: usize,
    arguments: &EnvironmentArguments,
) -> Option<Vec<Range<usize>>> {
    if let Some(signature) = &arguments.signature {
        if syntax::xparse::extended(signature) {
            let cursor = syntax::arguments::invocation_cursor(source, end, signature);
            syntax::xparse::Signature::parse(signature)?.ranges(source, cursor..source.len())
        } else {
            Some(crate::layout::signature_argument_ranges(
                source,
                end..source.len(),
                signature.bytes(),
            ))
        }
    } else {
        let shape = std::iter::once(b'o')
            .take(usize::from(arguments.optional_arguments))
            .chain(std::iter::repeat_n(b'm', arguments.arguments));
        Some(crate::layout::signature_argument_ranges(
            source,
            end..source.len(),
            shape,
        ))
    }
}

/// A source wrapper can forward opaque data without making it executable.
/// Prove every parameter occurrence belongs to an established data argument;
/// one use outside such a prefix leaves the invocation available to TeX parsing.
pub(crate) fn parameters_are_stored(body: &str) -> bool {
    parameters_only_in_data(body, &HashMap::new(), None)
}

pub(crate) fn parameters_are_stored_in_context(
    body: &str,
    context: &crate::packages::context::InvocationContext<'_>,
) -> bool {
    parameters_only_in_data(body, &HashMap::new(), Some(context))
}

pub(crate) fn stored_parameter_mask(
    body: &str,
    context: &crate::packages::context::InvocationContext<'_>,
) -> u16 {
    stored_parameter_mask_with_bindings(body, context, &HashMap::new())
}

fn stored_parameter_mask_with_bindings(
    body: &str,
    context: &crate::packages::context::InvocationContext<'_>,
    bindings: &HashMap<String, Binding>,
) -> u16 {
    let (stored, used) = parameter_usage(body, bindings, Some(context));
    if used & UNNUMBERED_PARAMETER != 0 {
        0
    } else {
        stored & !used & 0x1ff
    }
}

const UNNUMBERED_PARAMETER: u16 = 1 << 9;

pub(crate) fn discarded_parameter_mask(command: &str, signature: Option<&str>, body: &str) -> u16 {
    if !(matches!(command, "def" | "gdef") || syntax::classic_command_definition(command)) {
        return 0;
    }
    let Some(shape) = signature.filter(|shape| {
        !shape.is_empty()
            && shape.len() <= 9
            && shape
                .strip_prefix('o')
                .unwrap_or(shape)
                .bytes()
                .all(|slot| slot == b'm')
    }) else {
        return 0;
    };
    if body.contains("^^") || body.contains("##") {
        return 0;
    }
    let declared = (1 << shape.len()) - 1;
    if !body.contains('#') {
        return declared;
    }
    let referenced = SourceTokens::new(body, &[])
        .filter(|token| token.kind == SourceTokenKind::Text)
        .fold(0, |bits, token| bits | parameter_bits(body, token.span));
    if referenced & !declared == 0 {
        declared & !referenced
    } else {
        0
    }
}

fn parameter_bits(body: &str, range: Range<usize>) -> u16 {
    let bytes = body.as_bytes();
    memchr::memchr_iter(b'#', &bytes[range.clone()]).fold(0, |mask, at| {
        mask | match bytes.get(range.start + at + 1) {
            Some(digit @ b'1'..=b'9') => 1 << (digit - b'1'),
            _ => UNNUMBERED_PARAMETER,
        }
    })
}

fn parameters_only_in_data(
    body: &str,
    bindings: &HashMap<String, Binding>,
    context: Option<&crate::packages::context::InvocationContext<'_>>,
) -> bool {
    let (stored, used) = parameter_usage(body, bindings, context);
    stored != 0 && used == 0
}

// Keep separate bitsets for uses inside data and uses in executed text.
// All nine argument slots share the same token scan.
fn parameter_usage(
    body: &str,
    bindings: &HashMap<String, Binding>,
    context: Option<&crate::packages::context::InvocationContext<'_>>,
) -> (u16, u16) {
    if !body.contains('#') {
        return (0, 0);
    }
    let mut cursor = 0;
    let mut stored = 0;
    let mut used = 0;
    for token in SourceTokens::new(body, &[]) {
        if token.span.start < cursor || token.kind == SourceTokenKind::Comment {
            continue;
        }
        if let Some((name, end)) = token.control(body) {
            let finish = match name {
                "scantokens" => syntax::balanced_end(body, skip(body, end), b'{', b'}'),
                // TeX reads this parameter for its character code.
                "catcode" => {
                    let at = skip(body, end);
                    body.as_bytes()
                        .get(at..at + 3)
                        .filter(|operand| matches!(*operand, [b'`', b'#', b'1'..=b'9']))
                        .map(|_| at + 3)
                }
                _ => None,
            }
            .or_else(|| syntax::delayed_data_end(body, name, end))
            .or_else(|| syntax::primitive_data_end(body, name, end))
            .or_else(|| syntax::test_operand_end(body, name, end))
            .or_else(|| syntax::let_declaration_end(body, name, end))
            .or_else(|| {
                context
                    .and_then(|context| {
                        crate::packages::command_data_ranges_in_context(body, name, end, context)
                    })
                    .and_then(|ranges| ranges.last().map(|range| range.end))
            })
            .or_else(|| {
                syntax::message_data_arguments(body, name, end)
                    .and_then(|arguments| arguments.last().map(|argument| argument.end))
            })
            .or_else(|| {
                if let Some(binding) = bindings.get(name) {
                    if !binding.literal_arguments && binding.stored_slots != 0 {
                        let shape = binding.signature.as_ref()?.signature.as_deref()?;
                        let signature = syntax::xparse::Signature::parse(shape)?;
                        let cursor = end - usize::from(name.len() > 1 && name.ends_with('*'));
                        return signature
                            .slot_ranges(body, cursor..body.len())?
                            .iter()
                            .take_while(|argument| binding.stored_slots & (1 << argument.slot) != 0)
                            .last()
                            .map(|argument| argument.range.end);
                    }
                    binding
                        .literal_arguments
                        .then_some(binding.signature.as_ref())
                        .flatten()
                        .and_then(|signature| invocation_arguments(body, end, signature))
                        .and_then(|arguments| arguments.last().map(|argument| argument.end))
                } else {
                    crate::packages::commands::is_recording_command(name)
                        .then(|| crate::packages::command_data_end(body, name, end))
                        .flatten()
                }
            })
            .or_else(|| expl3_stored_data_end(body, name, end));
            if let Some(finish) = finish {
                stored |= parameter_bits(body, end..finish);
                cursor = finish;
            }
        } else {
            used |= parameter_bits(body, token.span);
        }
    }
    (stored, used)
}

/// These kernel operations store their token-list operand. Their declaration
/// spellings describe the preceding variable/name operand; other expl3 `n`
/// arguments remain executable unless a source definition proves otherwise.
pub(crate) fn expl3_stored_arguments(source: &str, name: &str, end: usize) -> Option<Range<usize>> {
    let (family, variant) = name.split_once(':')?;
    if !matches!(
        family,
        "tl_set"
            | "tl_gset"
            | "tl_const"
            | "tl_put_left"
            | "tl_put_right"
            | "tl_gput_left"
            | "tl_gput_right"
            | "seq_put_left"
            | "seq_put_right"
            | "seq_gput_left"
            | "seq_gput_right"
            | "regex_const"
            | "regex_set"
            | "regex_gset"
    ) || !matches!(variant, "Nn" | "cn")
    {
        return None;
    }
    let arguments =
        crate::layout::signature_argument_ranges(source, end..source.len(), b"mm".iter().copied());
    (arguments.len() == 2).then(|| arguments[1].clone())
}

fn expl3_stored_data_end(source: &str, name: &str, end: usize) -> Option<usize> {
    expl3_stored_arguments(source, name, end).map(|argument| argument.end)
}

fn register(registry: &mut HashMap<String, SyntaxRule>, rule: SyntaxRule) {
    let priority = |origin| match origin {
        RuleOrigin::Builtin => 0,
        RuleOrigin::Source => 1,
        RuleOrigin::Configuration => 2,
    };
    if registry
        .get(&rule.name)
        .is_none_or(|old| priority(old.origin) <= priority(rule.origin))
    {
        registry.insert(rule.name.clone(), rule);
    }
}

fn located_error(source: &str, error: crate::FormatError) -> Diagnostic {
    let mut at = error.byte_offset().unwrap_or(0).min(source.len());
    let message = error.to_string();
    while !source.is_char_boundary(at) {
        at -= 1;
    }
    let end = source[at..]
        .chars()
        .next()
        .map_or(at, |c| at + c.len_utf8());
    Diagnostic {
        severity: Severity::Error,
        message,
        span: at..end,
    }
}

fn restore_scope(bindings: &mut HashMap<String, Binding>, scopes: &mut Vec<BindingScope>) {
    if scopes.len() > 1 {
        for (name, previous) in scopes.pop().unwrap().previous {
            if let Some(previous) = previous {
                bindings.insert(name, previous);
            } else {
                bindings.remove(&name);
            }
        }
    }
}

fn assign(
    bindings: &mut HashMap<String, Binding>,
    scopes: &mut [BindingScope],
    name: &str,
    mut binding: Binding,
    global: bool,
) {
    binding.global = global;
    if global {
        for scope in scopes.iter_mut().skip(1) {
            scope.previous.remove(name);
        }
    } else if scopes.len() > 1 {
        scopes
            .last_mut()
            .unwrap()
            .previous
            .entry(name.into())
            .or_insert_with(|| bindings.get(name).cloned());
    }
    bindings.insert(name.into(), binding);
}

fn resolve(name: &str, bindings: &HashMap<String, Binding>) -> Option<EnvironmentAlias> {
    resolve_with(name, |key| bindings.get(key).map(|b| &b.replacement)).cloned()
}

fn resolve_with<'a>(
    name: &str,
    lookup: impl Fn(&str) -> Option<&'a Replacement>,
) -> Option<&'a EnvironmentAlias> {
    let mut slow = name;
    let mut fast = name;
    loop {
        slow = match lookup(slow)? {
            Replacement::Boundary(alias) => return Some(alias),
            Replacement::Forward(target) => target,
            _ => return None,
        };
        // Two cursors detect a cycle in work proportional to the chain length,
        // including short cycles in a file with many unrelated declarations.
        for _ in 0..2 {
            fast = match lookup(fast)? {
                Replacement::Boundary(alias) => return Some(alias),
                Replacement::Forward(target) => target,
                _ => return None,
            };
        }
        if slow == fast {
            return None;
        }
    }
}

fn references_environment_candidate(body: &str, bindings: &HashMap<String, Binding>) -> bool {
    let mut tokens = SourceTokens::new(body, &[]);
    while let Some(token) = tokens.next() {
        let Some((name, end)) = token.control(body) else {
            continue;
        };
        if let Some(finish) = syntax::primitive_data_end(body, name, end)
            .or_else(|| syntax::delayed_data_end(body, name, end))
            .or_else(|| syntax::let_declaration_end(body, name, end))
            .or_else(|| crate::packages::command_data_end(body, name, end))
        {
            tokens.position = finish;
        } else if bindings.get(name).is_some_and(|binding| binding.candidate) {
            return true;
        }
    }
    false
}

fn parameter_restart_body(body: &str) -> bool {
    if !body.contains("{#1}") || !body.contains(r"\newpage") {
        return false;
    }
    let mut cursor = skip(body, 0);
    for control in ["end", "newpage", "begin"] {
        let Some((name, end)) = syntax::command_at(body, cursor) else {
            return false;
        };
        if name != control {
            return false;
        }
        cursor = skip(body, end);
        if control != "newpage" {
            if !body[cursor..].starts_with("{#1}") {
                return false;
            }
            cursor = skip(body, cursor + 4);
        }
    }
    cursor == body.len()
}

impl Replacement {
    pub(crate) fn reader_restart(shape: &str, body: &str) -> Option<Self> {
        if shape == "m" && parameter_restart_body(body) {
            Some(Self::RestartArgument)
        } else if shape.is_empty() {
            let body = body[skip(body, 0)..].trim_end();
            let (first, end) = syntax::command_at(body, 0)?;
            if skip(body, end) == body.len() {
                return Some(Self::Forward(first.into()));
            }
            layout_environment_wrapper(body)
        } else {
            None
        }
    }

    fn is_restart(&self) -> bool {
        matches!(
            self,
            Self::RestartArgument | Self::RestartEnvironment(_) | Self::RestartVia { .. }
        )
    }

    pub(crate) fn restart_end<'a>(
        &self,
        source: &str,
        end: usize,
        top: &str,
        config: &Config,
        lookup: impl Fn(&str, bool) -> Option<&'a Replacement>,
    ) -> Option<usize> {
        let (environment, finish) = match self {
            Self::RestartArgument => {
                if !(paragraph_environment(top) || top == "verse") {
                    return None;
                }
                let open = skip(source, end);
                if source[end..open]
                    .lines()
                    .skip(1)
                    .any(|line| line.trim().is_empty())
                {
                    return None;
                }
                let rest = source[open..]
                    .strip_prefix('{')?
                    .strip_prefix(top)?
                    .strip_prefix('}')?;
                let environment_key = format!("environment:{top}");
                if config.syntax.environment_arguments.contains_key(top)
                    || config
                        .syntax
                        .inferred_environment_arguments
                        .contains_key(top)
                    || config
                        .protect
                        .environments
                        .iter()
                        .chain(&config.protect.file_contents)
                        .any(|env| env == top)
                    || lookup(&environment_key, false).is_some()
                    || lookup(&environment_key, true).is_some()
                {
                    return None;
                }
                (top, source.len() - rest.len())
            }
            Self::RestartEnvironment(env)
            | Self::RestartVia {
                environment: env, ..
            } => (env.as_str(), end),
            _ => return None,
        };
        let shadowed = |key: &str, outer| {
            config.syntax.commands.contains_key(key)
                || config.syntax.environment_aliases.contains_key(key)
                || config.protect.commands.iter().any(|entry| entry == key)
                || lookup(key, outer).is_some()
        };
        if environment != top
            || top == "document"
            || shadowed("end", false)
            || shadowed("begin", true)
            || shadowed(&format!("end{top}"), false)
            || shadowed(top, true)
            || matches!(self, Self::RestartArgument) && shadowed("newpage", true)
        {
            return None;
        }
        if let Self::RestartVia { opener, .. } = self {
            let alias = resolve_with(opener, |key| {
                (!config.syntax.commands.contains_key(key))
                    .then(|| lookup(key, true))
                    .flatten()
            })?;
            if !alias.begin || alias.environment != top {
                return None;
            }
        }
        Some(finish)
    }
}

fn replacement(body: &str) -> Replacement {
    let mut body = body[skip(body, 0)..].trim_end();
    if let Some(("noexpand", end)) = syntax::command_at(body, 0) {
        body = &body[skip(body, end)..];
    }
    if let Some(wrapper) = layout_environment_wrapper(body) {
        return wrapper;
    }
    let mut cursor = 0;
    let mut shifts = 0;
    let mut safe_math = true;
    while cursor < body.len() {
        cursor = skip(body, cursor);
        if cursor >= body.len() {
            break;
        }
        if let Some((name, end)) = syntax::command_at(body, cursor) {
            safe_math &= matches!(name, "hfill" | "relax" | "%" | "space" | "quad" | "qquad");
            cursor = end;
        } else {
            safe_math &= body.as_bytes()[cursor] == b'$';
            shifts += usize::from(body.as_bytes()[cursor] == b'$');
            cursor += body[cursor..].chars().next().unwrap().len_utf8();
        }
    }
    if safe_math && shifts == 1 {
        return Replacement::InlineMathShift;
    }
    let Some((name, end)) = syntax::command_at(body, 0) else {
        return Replacement::Unknown;
    };
    if matches!(name, "begingroup" | "group_begin:") {
        let mut cursor = end;
        let mut groups = 1usize;
        let mut safe = true;
        while cursor < body.len() {
            cursor = skip(body, cursor);
            if cursor == body.len() {
                break;
            }
            if let Some((name, end)) = syntax::command_at(body, cursor) {
                if let Some(finish) = syntax::primitive_data_end(body, name, end)
                    .or_else(|| syntax::delayed_data_end(body, name, end))
                    .or_else(|| syntax::let_declaration_end(body, name, end))
                {
                    cursor = finish;
                    continue;
                }
                if matches!(name, "begingroup" | "group_begin:") {
                    groups += 1;
                } else if matches!(name, "endgroup" | "group_end:") {
                    groups = groups.saturating_sub(1);
                } else if !matches!(name, "escapechar" | "relax" | "cC" | "cA" | "~") {
                    safe = false;
                }
                cursor = end;
            } else {
                cursor += body[cursor..].chars().next().unwrap().len_utf8();
            }
        }
        if safe && groups == 1 {
            return Replacement::GroupBegin;
        }
    }
    if matches!(name, "begin" | "end") {
        let start = skip(body, end);
        if let Some(finish) = syntax::balanced_end(body, start, b'{', b'}')
            && (skip(body, finish) == body.len()
                || name == "begin"
                    && syntax::balanced_end(body, skip(body, finish), b'[', b']')
                        .is_some_and(|end| skip(body, end) == body.len()))
        {
            let environment = body[start + 1..finish - 1].trim();
            if !environment.is_empty() && !environment.contains(['\\', '#', '{', '}', '%']) {
                return Replacement::Boundary(EnvironmentAlias {
                    environment: environment.into(),
                    begin: name == "begin",
                });
            }
        }
    } else if skip(body, end) == body.len() {
        return Replacement::Forward(name.into());
    }
    Replacement::Unknown
}

fn wrapped_environment(source: &str, declaration: &EnvironmentDeclaration) -> Option<String> {
    let [begin, end] = declaration.bodies.as_slice() else {
        return None;
    };
    let close = &source[end.clone()];
    let (inner, false, after) = syntax::env_at(close, skip(close, 0))? else {
        return None;
    };
    if skip(close, after) != close.len()
        || inner.is_empty()
        || inner.contains(['#', '\\', '{', '}', '%'])
    {
        return None;
    }
    let open = &source[begin.clone()];
    let mut tokens = SourceTokens::new(open, &[]);
    let mut found = false;
    while let Some(token) = tokens.next() {
        if token.kind == SourceTokenKind::OpenGroup {
            tokens.position = syntax::balanced_end(open, token.span.start, b'{', b'}')?;
        } else if token.kind == SourceTokenKind::Control {
            let (name, _) = syntax::command_at(open, token.span.start)?;
            if name == "begin" {
                if found || syntax::env_at(open, token.span.start)?.0 != inner {
                    return None;
                }
                found = true;
            } else if matches!(name, "end" | "begingroup" | "endgroup" | "else" | "fi")
                || name.starts_with("if")
            {
                return None;
            }
        }
    }
    found.then(|| inner.to_owned())
}

/// Layout-only commands may surround a literal boundary. A switch between
/// adjacent minipages ends one local scope and opens another at the same depth.
fn layout_environment_wrapper(body: &str) -> Option<Replacement> {
    let mut cursor = 0;
    let mut boundary = None;
    while skip(body, cursor) < body.len() {
        let at = skip(body, cursor);
        let (name, end) = syntax::command_at(body, at)?;
        cursor = end;
        if matches!(
            name,
            "bigskip" | "medskip" | "smallskip" | "noindent" | "par" | "hline"
        ) {
            continue;
        }
        let Some((environment, begin, end)) = syntax::env_after_command(body, name, end) else {
            if let Some(Replacement::Boundary(previous)) = boundary
                && !previous.begin
            {
                boundary = Some(Replacement::RestartVia {
                    environment: previous.environment,
                    opener: name.to_owned(),
                });
                continue;
            }
            return None;
        };
        if environment.is_empty()
            || !environment
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'*')
        {
            return None;
        }
        cursor = end;
        if begin && environment == "tabular" {
            let spec = skip(body, end);
            cursor = syntax::balanced_end(body, spec, b'{', b'}')?;
            let spec = &body[spec + 1..cursor - 1];
            if !SourceTokens::new(spec, &[]).all(|token| match token.control(spec) {
                Some((name, _)) => matches!(
                    name,
                    "tt" | "hangindent" | "linewidth" | "textwidth" | "columnwidth"
                ),
                None => spec[token.span].bytes().all(|b| {
                    b.is_ascii_alphanumeric() || b.is_ascii_whitespace() || b"{}|.".contains(&b)
                }),
            }) {
                return None;
            }
        }
        if begin && environment == "minipage" {
            cursor = syntax::arguments::signature_end(body, end, "ooom")?;
            let mut header = SourceTokens::new(&body[end..cursor], &[]);
            if header.any(|token| {
                token.kind == SourceTokenKind::Control
                    && !matches!(
                        token.control(&body[end..cursor]).map(|(name, _)| name),
                        Some(
                            "linewidth"
                                | "textwidth"
                                | "columnwidth"
                                | "hsize"
                                | "textheight"
                                | "baselineskip"
                        )
                    )
            }) {
                return None;
            }
        }
        boundary = Some(match boundary {
            None => Replacement::Boundary(EnvironmentAlias {
                environment: environment.into(),
                begin,
            }),
            Some(Replacement::Boundary(previous))
                if begin && !previous.begin && previous.environment == environment =>
            {
                Replacement::RestartEnvironment(environment.into())
            }
            _ => return None,
        });
    }
    boundary
}

/// A complete static wrapper leaves the caller's environment stack untouched.
/// Conditional branches must each restore their entry stack. Parameters can
/// stand for neutral arguments only after each invocation proves those operands;
/// dynamic names prevent that proof and delayed declarations remain data.
fn body_has_environment_effect(
    body: &str,
    config: &Config,
    neutral_parameters: bool,
    complete: bool,
) -> bool {
    let mut normalized = String::new();
    let mut copied = 0;
    let mut cursor = 0;
    while cursor < body.len() {
        if body.as_bytes()[cursor] == b'%' {
            cursor = skip(body, cursor);
            continue;
        }
        if let Some((name, end)) = syntax::command_at(body, cursor) {
            if matches!(name, "IfBooleanTF" | "IfNoValueTF" | "IfValueTF") {
                let args =
                    crate::layout::signature_argument_ranges(body, end..body.len(), "mmm".bytes());
                if args.len() == 3 && args.iter().all(|arg| body.as_bytes()[arg.start] == b'{') {
                    let boundaries = |range: &Range<usize>| {
                        let mut events = Vec::new();
                        let text = &body[range.start + 1..range.end - 1];
                        for token in SourceTokens::new(text, &[]) {
                            if let Some((name, end)) = token.control(text)
                                && matches!(name, "begin" | "end")
                            {
                                let start = skip(text, end);
                                let finish = syntax::balanced_end(text, start, b'{', b'}')?;
                                let environment = &text[start + 1..finish - 1];
                                if environment.contains(['#', '\\']) {
                                    return None;
                                }
                                events.push((name.to_owned(), environment.to_owned()));
                            }
                        }
                        Some(events)
                    };
                    if let (Some(first), Some(second)) =
                        (boundaries(&args[1]), boundaries(&args[2]))
                        && !first.is_empty()
                        && first == second
                    {
                        normalized.push_str(&body[copied..cursor]);
                        normalized.push_str(&body[args[1].start + 1..args[1].end - 1]);
                        copied = args[2].end;
                        cursor = copied;
                        continue;
                    }
                }
            }
            cursor = end;
        } else {
            cursor += body[cursor..].chars().next().unwrap().len_utf8();
        }
    }
    if copied > 0 {
        normalized.push_str(&body[copied..]);
        return body_has_environment_effect(&normalized, config, neutral_parameters, complete);
    }
    if !complete && !body.contains("\\begin") && !body.contains("\\end") {
        return false;
    }
    let mut tokens = SourceTokens::new(body, &[]);
    let mut environments = Vec::new();
    let mut conditions = Vec::new();
    let mut seen_boundary = false;
    let mut uncertain = false;
    let mut invalid_condition = false;
    let mut loops = Vec::new();
    while let Some(token) = tokens.next() {
        if !neutral_parameters
            && token.kind == SourceTokenKind::Text
            && body[token.span.clone()].contains('#')
        {
            uncertain = true;
        }
        let Some((name, end)) = token.control(body) else {
            continue;
        };
        if let Some(finish) = syntax::primitive_data_end(body, name, end)
            .or_else(|| syntax::delayed_data_end(body, name, end))
            .or_else(|| syntax::let_declaration_end(body, name, end))
            .or_else(|| crate::packages::command_data_end(body, name, end))
        {
            tokens.position = finish;
            continue;
        }
        // Common one-step expansion into a braced display argument. Keep
        // scanning its contents so literal environment effects remain visible.
        if name == "expandafter"
            && let Some((reader, after)) = syntax::command_at(body, skip(body, end))
            && matches!(reader, "url" | "email" | "path")
            && let Some(("expandafter", brace)) = syntax::command_at(body, skip(body, after))
            && body.as_bytes().get(skip(body, brace)) == Some(&b'{')
        {
            tokens.position = brace;
            continue;
        }
        if let Some(close) = syntax::loop_end(name) {
            loops.push((close, conditions.len()));
        } else if loops.last().is_some_and(|&(close, _)| name == close)
            && let Some((_, depth)) = loops.pop()
        {
            invalid_condition |=
                conditions.len() != depth + 1 || conditions.pop() != Some(environments.len());
        } else if matches!(name, "csname" | "endcsname" | "expandafter" | "scantokens") {
            if complete {
                return true;
            }
            uncertain = true;
        } else if matches!(name, "begin" | "end") {
            let start = skip(body, end);
            let Some(finish) = syntax::balanced_end(body, start, b'{', b'}') else {
                // A bare \end is also used as a delimited macro sentinel.
                // Only complete-argument proofs need to reject that ambiguity.
                if complete {
                    return true;
                }
                continue;
            };
            seen_boundary = true;
            let environment = body[start + 1..finish - 1].trim();
            if environment.is_empty() || environment.contains(['\\', '#', '{', '}', '%']) {
                return true;
            }
            if name == "begin" {
                environments.push(environment);
            } else {
                if conditions
                    .last()
                    .is_some_and(|&depth| environments.len() <= depth)
                    || environments.pop() != Some(environment)
                {
                    return true;
                }
            }
            tokens.position = finish;
        } else if syntax::is_condition_command(name, config)
            || syntax::inferred_condition(name, body, end)
        {
            conditions.push(environments.len());
            tokens.position = syntax::test_operand_end(body, name, end).unwrap_or(end);
        } else if matches!(name, "else" | "or") {
            invalid_condition |= conditions.last().copied() != Some(environments.len());
        } else if name == "fi" {
            invalid_condition |= conditions.pop() != Some(environments.len());
        } else if matches!(name, "newif" | "ltx@newif" | "newife") {
            tokens.position = syntax::command_at(body, skip(body, end)).map_or(end, |(_, end)| end);
        } else if let Some(finish) = syntax::test_operand_end(body, name, end) {
            tokens.position = finish;
        }
    }
    (complete || seen_boundary)
        && (uncertain || invalid_condition || !conditions.is_empty() || !loops.is_empty())
        || !environments.is_empty()
}

fn neutral_constructed_wrapper(
    body: &str,
    source: &str,
    end: usize,
    signature: Option<&EnvironmentArguments>,
    config: &Config,
    bindings: &HashMap<String, Binding>,
) -> bool {
    // Environment names must already be literal before substituting values.
    for token in SourceTokens::new(body, &[]) {
        if let Some((name, end)) = token.control(body)
            && matches!(name, "begin" | "end")
        {
            let start = skip(body, end);
            let Some(end) = syntax::balanced_end(body, start, b'{', b'}') else {
                return false;
            };
            if body[start + 1..end - 1].contains(['#', '\\']) {
                return false;
            }
        }
    }
    let Some(arguments) =
        signature.and_then(|signature| invocation_arguments(source, end, signature))
    else {
        return false;
    };
    if arguments.is_empty() || arguments.len() > 9 {
        return false;
    }
    let mut expanded = body.to_owned();
    for (index, argument) in arguments.iter().enumerate() {
        if source.as_bytes().get(argument.start) != Some(&b'{') {
            return false;
        }
        let value = &source[argument.start + 1..argument.end - 1];
        if !value
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        {
            return false;
        }
        expanded = expanded.replace(&format!("#{}", index + 1), value);
    }
    if expanded.contains('#') {
        return false;
    }
    let mut normalized = String::new();
    let mut cursor = 0;
    while cursor < expanded.len() {
        if let Some(("csname", after)) = syntax::command_at(&expanded, cursor) {
            let Some(offset) = expanded[after..].find("\\endcsname") else {
                return false;
            };
            let name: String = expanded[after..after + offset]
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect();
            if !bindings
                .get(&name)
                .is_some_and(|binding| !binding.candidate)
            {
                return false;
            }
            normalized.push_str("\\relax ");
            cursor = after + offset + "\\endcsname".len();
        } else {
            let c = expanded[cursor..].chars().next().unwrap();
            normalized.push(c);
            cursor += c.len_utf8();
        }
    }
    !body_has_environment_effect(&normalized, config, false, true)
}

/// In \expandafter\wrapper\expandafter{\value}, the inner expansion
/// takes place before the wrapper scans its first braced argument.
fn expanded_text_group(
    source: &str,
    end: usize,
    bindings: &HashMap<String, Binding>,
) -> Option<usize> {
    let ("expandafter", after) = syntax::command_at(source, skip(source, end))? else {
        return None;
    };
    let open = skip(source, after);
    if source.as_bytes().get(open) != Some(&b'{') {
        return None;
    }
    let (value, _) = syntax::command_at(source, open + 1)?;
    matches!(&bindings.get(value)?.replacement, Replacement::LiteralText).then_some(open)
}

const WRAPPER_CONTROLS: &[&str] = &["begin", "end", "multicolumn", "\\", "hline"];
const WRAPPER_ENVIRONMENTS: &[&str] = &[
    "array",
    "quote",
    "quotation",
    "center",
    "flushleft",
    "flushright",
];

fn composable_wrapper_definition(source: &str, declaration: &Declaration, config: &Config) -> bool {
    let Some(shape) = declaration
        .signature
        .as_ref()
        .and_then(|signature| signature.signature.as_deref())
    else {
        return false;
    };
    if shape.is_empty() || !shape.bytes().all(|slot| slot == b'm') {
        return false;
    }
    let header = &source[declaration.span.start..declaration.body.start - 1];
    if SourceTokens::new(header, &[])
        .filter(|token| token.kind == SourceTokenKind::Control)
        .count()
        != 2
        || header.contains(['~', '^', '$'])
        || header.contains('>')
    {
        return false;
    }
    let body = &source[declaration.body.clone()];
    let mut saw_environment = false;
    let fixed_group = |at, accepted: fn(u8) -> bool| {
        let open = skip(body, at);
        let end = syntax::balanced_end(body, open, b'{', b'}')?;
        let text = &body[open + 1..end - 1];
        (!text.is_empty() && text.bytes().all(accepted)).then_some(end)
    };
    let column = |byte| matches!(byte, b'l' | b'c' | b'r' | b'|');
    for token in SourceTokens::new(body, &[]) {
        let Some((name, end)) = token.control(body) else {
            continue;
        };
        if !WRAPPER_CONTROLS.contains(&name) || declaration.name == name {
            return false;
        }
        if matches!(name, "begin" | "end") {
            let open = skip(body, end);
            let Some(finish) = syntax::balanced_end(body, open, b'{', b'}') else {
                return false;
            };
            let environment = &body[open + 1..finish - 1];
            if !WRAPPER_ENVIRONMENTS.contains(&environment)
                || config
                    .protect
                    .environments
                    .iter()
                    .any(|entry| entry == environment)
            {
                return false;
            }
            if name == "begin" && environment == "array" && fixed_group(finish, column).is_none() {
                return false;
            }
            saw_environment = true;
        } else if name == "multicolumn"
            && fixed_group(end, |byte| byte.is_ascii_digit())
                .and_then(|at| fixed_group(at, column))
                .is_none()
        {
            return false;
        }
    }
    saw_environment && !body.contains(['^', '~', '$'])
}

fn neutral_wrapper_arguments(
    source: &str,
    end: usize,
    signature: &EnvironmentArguments,
    config: &Config,
    bindings: &HashMap<String, Binding>,
    binding: &Binding,
) -> bool {
    if flat_wrapper_arguments(
        source,
        end,
        signature,
        config,
        bindings,
        binding.literal_slots & !binding.stored_slots,
    ) {
        return true;
    }
    matches!(binding.replacement, Replacement::NeutralWrapper)
        && WRAPPER_CONTROLS.iter().all(|name| {
            !bindings.contains_key(*name) && !config.syntax.commands.contains_key(*name)
        })
        && WRAPPER_ENVIRONMENTS.iter().all(|name| {
            !bindings.contains_key(*name)
                && !bindings.contains_key(&format!("end{name}"))
                && !bindings.contains_key(&format!("environment:{name}"))
        })
        && nested_wrapper_end(source, end, signature, bindings, 0, &mut 128).is_some()
}

fn nested_wrapper_end(
    source: &str,
    end: usize,
    signature: &EnvironmentArguments,
    bindings: &HashMap<String, Binding>,
    depth: usize,
    remaining: &mut usize,
) -> Option<usize> {
    if depth >= 16 {
        return None;
    }
    *remaining = remaining.checked_sub(1)?;
    let shape = signature.signature.as_deref()?;
    let parsed = syntax::xparse::Signature::parse(shape)?;
    let cursor = if source[..end].ends_with('*') {
        end - 1
    } else {
        end
    };
    let slots = parsed.slot_ranges(source, cursor..source.len())?;
    let finish = slots.last().map_or(end, |slot| slot.range.end.max(end));
    for slot in slots {
        let operand = &source[slot.range.clone()];
        if operand == "*" || operand.len() == 1 && operand.as_bytes()[0].is_ascii_alphanumeric() {
            continue;
        }
        if slot.body == slot.range || !matches!(operand.as_bytes().first(), Some(b'{' | b'[')) {
            return None;
        }
        let body = &source[slot.body];
        let mut tokens = SourceTokens::new(body, &[]);
        while let Some(token) = tokens.next() {
            if let Some((control, after)) = token.control(body) {
                let binding = bindings.get(control.trim_end_matches('*'))?;
                if !matches!(binding.replacement, Replacement::NeutralWrapper) {
                    return None;
                }
                tokens.position = nested_wrapper_end(
                    body,
                    after,
                    binding.signature.as_ref()?,
                    bindings,
                    depth + 1,
                    remaining,
                )?;
            } else if token.kind == SourceTokenKind::Text
                && body[token.span].contains(['#', '^', '~', '$', '&'])
            {
                return None;
            }
        }
    }
    Some(finish)
}

fn flat_wrapper_arguments(
    source: &str,
    end: usize,
    signature: &EnvironmentArguments,
    config: &Config,
    bindings: &HashMap<String, Binding>,
    discarded: u16,
) -> bool {
    // An adjacent star is a separate TeX token for a \newcommand with
    // ordinary positional arguments, including after an absent optional slot.
    let end = if signature
        .signature
        .as_ref()
        .is_none_or(|shape| shape.bytes().all(|s| matches!(s, b'o' | b'm')))
        && source[..end]
            .strip_suffix('*')
            .is_some_and(|prefix| prefix.ends_with(|c: char| c.is_ascii_alphabetic() || c == '@'))
    {
        end - 1
    } else {
        end
    };
    let Some(arguments) = invocation_arguments(source, end, signature) else {
        return false;
    };
    if arguments.len() < signature.arguments {
        return false;
    }
    let boolean_ranges: HashSet<_> = signature
        .signature
        .as_ref()
        .and_then(|shape| syntax::xparse::Signature::parse(shape))
        .and_then(|parsed| {
            let mask = parsed.boolean_slots();
            parsed
                .slot_ranges(
                    source,
                    syntax::arguments::invocation_cursor(
                        source,
                        end,
                        signature.signature.as_ref().unwrap(),
                    )..source.len(),
                )
                .map(|arguments| {
                    arguments
                        .into_iter()
                        .filter(|arg| arg.slot < 9 && mask & (1 << arg.slot) != 0)
                        .map(|arg| (arg.range.start, arg.range.end))
                        .collect()
                })
        })
        .unwrap_or_default();
    let ignored = (discarded != 0)
        .then(|| {
            syntax::xparse::Signature::parse(signature.signature.as_deref()?)?
                .masked_ranges(source, end, discarded)
        })
        .flatten()
        .unwrap_or_default();
    arguments.iter().all(|argument| {
        if boolean_ranges.contains(&(argument.start, argument.end)) || ignored.contains(argument) {
            return true;
        }
        let operand = &source[argument.clone()];
        if operand == "*" || operand.len() == 1 && operand.as_bytes()[0].is_ascii_alphanumeric() {
            return true;
        }
        // An unbraced operand can be a single math shift or a grouping token.
        // Require an explicit operand boundary before proving a replacement
        // independently of the wrapper's surrounding text.
        if !matches!(source.as_bytes().get(argument.start), Some(b'{' | b'[')) {
            return false;
        }
        let body = &source[argument.clone()];
        if body_has_environment_effect(body, config, false, true) {
            return false;
        }
        // A parameter containing an alias or a constructed control sequence
        // can cross the wrapper's literal boundaries without spelling begin/end.
        // Keep that call conservative even when its ordinary tokens balance.
        SourceTokens::new(body, &[]).all(|token| {
            token.control(body).is_none_or(|(control, _)| {
                let name = control.trim_end_matches('*');
                !matches!(name, "csname" | "endcsname" | "expandafter" | "scantokens")
                    && !bindings.get(name).is_some_and(|binding| binding.candidate)
            })
        })
    })
}

fn skip(source: &str, mut pos: usize) -> usize {
    loop {
        while source
            .as_bytes()
            .get(pos)
            .is_some_and(u8::is_ascii_whitespace)
        {
            pos += 1;
        }
        if source.as_bytes().get(pos) == Some(&b'%') {
            pos = source[pos..]
                .find('\n')
                .map_or(source.len(), |n| pos + n + 1);
        } else {
            return pos;
        }
    }
}

fn adjacent_arguments_end(source: &str, mut cursor: usize) -> usize {
    loop {
        let mut start = cursor;
        while matches!(source.as_bytes().get(start), Some(b' ' | b'\t')) {
            start += 1;
        }
        let close = match source.as_bytes().get(start) {
            Some(b'{') => b'}',
            Some(b'[') => b']',
            Some(b'(') => b')',
            _ => break,
        };
        let Some(end) = syntax::balanced_end(source, start, source.as_bytes()[start], close) else {
            break;
        };
        cursor = end;
    }
    cursor
}

/// First token starting at or beyond `end`, which is past the current token's start.
/// Each later token owns at least one byte, so at most `end - current.end`
/// later tokens can start before the target (zero for a read inside the current token).
fn token_after_read(tokens: &[SourceToken], current: usize, end: usize) -> usize {
    let token = &tokens[current];
    debug_assert!(token.span.start < end);
    let next = current + 1;
    let tail = &tokens[next..];
    let limit = end.saturating_sub(token.span.end).min(tail.len());
    next + tail[..limit].partition_point(|token| token.span.start < end)
}

fn next(tokens: &[SourceToken], mut at: usize) -> usize {
    while tokens.get(at).is_some_and(|t| {
        matches!(
            t.kind,
            SourceTokenKind::Space | SourceTokenKind::Newline | SourceTokenKind::Comment
        )
    }) {
        at += 1;
    }
    at
}

fn definition(
    source: &str,
    tokens: &[SourceToken],
    closing_group: &impl Fn(usize) -> Option<usize>,
    at: usize,
) -> Option<(Declaration, usize, bool, bool)> {
    let (command, command_end) = tokens[at].control(source)?;
    let command = command.trim_end_matches('*');
    let primitive = matches!(command, "def" | "gdef" | "edef" | "xdef");
    let classic = syntax::classic_command_definition(command);
    let fixed = syntax::fixed_command_factory_signature(command);
    if let Some(declaration) =
        syntax::expl3_command_declaration(source, tokens[at].span.start, command, command_end)
    {
        let finish = token_after_read(tokens, at, declaration.span.end);
        return Some((declaration, finish, command.starts_with("cs_gset"), false));
    }
    let xparse = matches!(
        command,
        "NewDocumentCommand"
            | "RenewDocumentCommand"
            | "ProvideDocumentCommand"
            | "DeclareDocumentCommand"
            | "NewAcroCommand"
            | "RenewAcroCommand"
            | "ProvideAcroCommand"
            | "DeclareAcroCommand"
    );
    if !primitive && !classic && !xparse && fixed.is_none() {
        return None;
    }
    if !primitive {
        let declaration = syntax::source_command_declaration(
            source,
            tokens[at].span.start,
            command,
            command_end,
        )?;
        let finish = token_after_read(tokens, at, declaration.span.end);
        return Some((
            declaration,
            finish,
            false,
            matches!(
                command,
                "providecommand"
                    | "providerobustcmd"
                    | "ProvideDocumentCommand"
                    | "ProvideAcroCommand"
                    | "ProvideDerivative"
                    | "ProvideDifferential"
            ),
        ));
    }
    let mut cursor = next(tokens, at + 1);
    let target = tokens.get(cursor)?;
    let target_group = target.kind == SourceTokenKind::OpenGroup;
    let name_at = if target_group {
        next(tokens, cursor + 1)
    } else {
        cursor
    };
    let (mut name, mut name_end) = syntax::command_at(source, tokens.get(name_at)?.span.start)?;
    if !target_group
        && name == "csname"
        && syntax::expansion_precedes(source, tokens[at].span.start)
    {
        // The expanded name is the definition target. Registering the
        // primitive itself would shadow every subsequent name construction.
        let finish = syntax::csname_end(source, name_end, syntax::command_at)?;
        let literal = source[name_end..finish]
            .strip_suffix("\\endcsname")?
            .trim_start();
        if literal.is_empty()
            || literal.contains(['\\', '%'])
            || literal.contains(char::is_whitespace)
        {
            return None;
        }
        name = literal;
        name_end = finish;
        cursor = next(tokens, token_after_read(tokens, name_at, finish));
    } else if target_group {
        let finish = closing_group(cursor)?;
        if next(tokens, name_at + 1) != finish {
            return None;
        }
        cursor = next(tokens, finish + 1);
    } else {
        cursor = next(tokens, name_at + 1);
    }
    while source.as_bytes().get(tokens.get(cursor)?.span.start) != Some(&b'{') {
        cursor += 1;
    }
    let parameter_text = &source[name_end..tokens[cursor].span.start];
    let arguments = syntax::arguments::parameter_signature(parameter_text).map(|signature| {
        EnvironmentArguments {
            arguments: parameter_text.bytes().filter(|&byte| byte == b'#').count(),
            signature: Some(signature),
            ..Default::default()
        }
    });
    let body_start = tokens.get(cursor)?.span.start;
    let body_end = syntax::balanced_end(source, body_start, b'{', b'}')?;
    let finish = token_after_read(tokens, cursor, body_end);
    Some((
        Declaration {
            name: name.into(),
            span: tokens[at].span.start..body_end,
            body: body_start + 1..body_end - 1,
            signature: arguments,
        },
        finish,
        matches!(command, "gdef" | "xdef"),
        matches!(
            command,
            "providecommand"
                | "providerobustcmd"
                | "ProvideDocumentCommand"
                | "ProvideAcroCommand"
                | "ProvideDerivative"
                | "ProvideDifferential"
        ),
    ))
}

/// Byte-oriented recovery also admits invalid UTF-8 without lossy conversion.
#[derive(Debug, Clone)]
pub struct ByteSourceModel {
    source: Vec<u8>,
    tokens: Vec<SourceToken>,
    diagnostics: Vec<Diagnostic>,
}

impl ByteSourceModel {
    pub fn parse(source: &[u8]) -> Self {
        if let Ok(text) = std::str::from_utf8(source) {
            let model = SourceModel::parse(text);
            return Self {
                source: source.into(),
                tokens: model.tokens.into_tokens(),
                diagnostics: model.diagnostics,
            };
        }
        let mut tokens = Vec::new();
        let mut diagnostics = Vec::new();
        let mut pos = 0;
        while pos < source.len() {
            match std::str::from_utf8(&source[pos..]) {
                Ok(text) => {
                    tokens.extend(SourceTokens::new(text, &[]).map(|mut t| {
                        t.span = t.span.start + pos..t.span.end + pos;
                        t
                    }));
                    break;
                }
                Err(error) => {
                    let valid = error.valid_up_to();
                    if valid > 0 {
                        let text = std::str::from_utf8(&source[pos..pos + valid])
                            .expect("validated prefix");
                        tokens.extend(SourceTokens::new(text, &[]).map(|mut t| {
                            t.span = t.span.start + pos..t.span.end + pos;
                            t
                        }));
                        pos += valid;
                    }
                    let end =
                        (pos + error.error_len().unwrap_or(source.len() - pos)).min(source.len());
                    diagnostics.push(Diagnostic {
                        severity: Severity::Error,
                        message: "Invalid UTF-8 sequence; original bytes retained".into(),
                        span: pos..end,
                    });
                    tokens.push(SourceToken {
                        kind: SourceTokenKind::Invalid,
                        span: pos..end,
                    });
                    pos = end;
                }
            }
        }
        Self {
            source: source.into(),
            tokens,
            diagnostics,
        }
    }
    pub fn source_bytes(&self) -> &[u8] {
        &self.source
    }
    pub fn tokens(&self) -> &[SourceToken] {
        &self.tokens
    }
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }
    pub fn has_errors(&self) -> bool {
        !self.diagnostics.is_empty()
    }
    pub fn reconstruct(&self) -> Vec<u8> {
        self.tokens
            .iter()
            .flat_map(|t| self.source[t.span.clone()].iter().copied())
            .collect()
    }
}

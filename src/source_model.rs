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
use crate::{CompiledConfig, Config, Diagnostic, Severity, syntax};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceTokenKind {
    Control,
    OpenGroup,
    CloseGroup,
    Space,
    Newline,
    Comment,
    Text,
    Opaque,
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceToken {
    pub kind: SourceTokenKind,
    pub span: Range<usize>,
}

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
enum Replacement {
    Boundary(EnvironmentAlias),
    Forward(String),
    /// Literal boundaries balance, provided the supplied arguments do too.
    NeutralWrapper,
    Unknown,
}

#[derive(Debug, Clone)]
struct Binding {
    replacement: Replacement,
    candidate: bool,
    signature: Option<EnvironmentArguments>,
    condition: Option<String>,
    literal_arguments: bool,
    math_signature: Option<(&'static str, &'static str)>,
}

struct SourceImportEvent {
    at: usize,
    aliases: BTreeMap<String, Option<EnvironmentAlias>>,
    arguments: BTreeMap<String, Option<EnvironmentArguments>>,
}

/// Owned, immutable text with a complete lexical partition and semantic index.
#[derive(Debug, Clone)]
pub struct SourceModel {
    source: String,
    tokens: Vec<SourceToken>,
    diagnostics: Vec<Diagnostic>,
    declarations: Vec<Declaration>,
    environment_declarations: Vec<EnvironmentDeclaration>,
    environment_declaration_names: HashSet<String>,
    environment_arguments: HashMap<usize, EnvironmentArguments>,
    context: crate::packages::context::ContextIndex,
    conditionals: Vec<Conditional>,
    rules: Vec<SyntaxRule>,
    boundaries: HashMap<usize, EnvironmentAlias>,
    exported: BTreeMap<String, EnvironmentAlias>,
    exported_unknown: HashSet<String>,
    exported_environments: BTreeMap<String, Option<EnvironmentArguments>>,
    registry: HashMap<String, SyntaxRule>,
    invocations: HashMap<usize, SyntaxRule>,
    invocation_arguments: HashMap<usize, Vec<syntax::xparse::ArgumentSlot>>,
    stored_arguments: Vec<Range<usize>>,
    inferred_math_signatures: HashMap<usize, (&'static str, &'static str)>,
    constructed: HashMap<usize, (String, usize)>,
    opaque: Vec<Range<usize>>,
    protected: Vec<Range<usize>>,
    document_class: Option<String>,
    pub(crate) structure: Option<(Vec<syntax::Token>, Vec<syntax::SharedBlock>)>,
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
        match syntax::protected_ranges(source, config) {
            Ok(ranges) => Self::with_ranges(source, config, &ranges),
            Err(error) => {
                let mut model =
                    Self::with_ranges(source, config, std::slice::from_ref(&(0..source.len())));
                model.diagnostics.push(located_error(source, error));
                model
            }
        }
    }

    fn build(source: &str, config: &Config, compiled: Option<&CompiledConfig>) -> Self {
        let protected = syntax::protected_ranges_with_diagnostics(
            source,
            config,
            compiled.map(|compiled| compiled.syntax.as_ref()),
        );
        match protected {
            Ok((ranges, diagnostics)) => {
                let mut model = Self::with_ranges(source, config, &ranges);
                model.diagnostics.extend(diagnostics);
                let ranges = model.protected.clone();
                let owned = compiled
                    .is_none()
                    .then(|| CompiledConfig::new(config.clone()).ok())
                    .flatten();
                if let Some(compiled) = compiled.or(owned.as_ref()) {
                    let structure = {
                        let analysis = crate::source_analysis::SourceAnalysis::for_model(
                            source, config, &ranges, &model,
                        );
                        syntax::shared_blocks_with_analysis(source, compiled, &ranges, &analysis)
                    };
                    match structure {
                        Ok(structure) => {
                            let mut open = Vec::new();
                            for token in &structure.0 {
                                match token.kind {
                                    syntax::TokenKind::OpenBrace => open.push(token.span.clone()),
                                    syntax::TokenKind::CloseBrace => {
                                        open.pop();
                                    }
                                    syntax::TokenKind::Command => {
                                        if let Some((name, _)) =
                                            syntax::command_at(source, token.span.start)
                                        {
                                            if matches!(name, "bgroup" | "begingroup") {
                                                open.push(token.span.clone());
                                            } else if matches!(name, "egroup" | "endgroup") {
                                                open.pop();
                                            }
                                        }
                                    }
                                    _ => (),
                                }
                            }
                            for span in open {
                                model.diagnostics.push(Diagnostic {
                                    severity: Severity::Error,
                                    message:
                                        "Unclosed group before an opaque region; source retained"
                                            .into(),
                                    span,
                                });
                            }
                            model.structure = Some(structure);
                        }
                        Err(error) => {
                            model.diagnostics.push(located_error(source, error));
                        }
                    }
                }
                model
            }
            Err(error) => {
                let mut model =
                    Self::with_ranges(source, config, std::slice::from_ref(&(0..source.len())));
                model.diagnostics.push(located_error(source, error));
                model
            }
        }
    }

    pub fn parse_bytes(source: &[u8]) -> ByteSourceModel {
        ByteSourceModel::parse(source)
    }

    /// Reader candidates use the same lexical partition even inside delayed
    /// declaration bodies; the declaration reader controls their visibility.
    pub(crate) fn reader_positions(source: &str) -> Vec<usize> {
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
            .collect()
    }

    /// Literal group ends from the common lexical partition. Consumers that
    /// need scoped lookahead build this once instead of rescanning nested bodies.
    pub(crate) fn literal_group_ends(source: &str) -> HashMap<usize, usize> {
        let mut groups = Vec::new();
        let mut ends = HashMap::new();
        for token in SourceTokens::new(source, &[]) {
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
        let mut counts = HashMap::<String, usize>::new();
        let mut result = HashMap::new();
        for token in SourceTokens::new(source, &[]) {
            if token.kind == SourceTokenKind::Control
                && let Some((name, _)) = syntax::command_at(source, token.span.start)
                && matches!(
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
            {
                let occurrence = counts.entry(name.into()).or_default();
                result.insert(token.span.start, (name.into(), *occurrence));
                *occurrence += 1;
            }
        }
        result
    }

    fn import_events(&self, config: &Config) -> Vec<SourceImportEvent> {
        if config.syntax.environment_alias_events.is_empty()
            && config.syntax.environment_argument_events.is_empty()
        {
            return Vec::new();
        }
        let loaders: HashMap<_, _> = Self::loader_occurrences(&self.source)
            .into_iter()
            .map(|(at, key)| (key, at))
            .collect();
        let offset = |at: usize, loader: &Option<(String, usize)>| -> Option<usize> {
            let Some(loader) = loader else {
                return Some(at.min(self.source.len()));
            };
            let loader_start = *loaders.get(loader)?;
            let (_, end) = syntax::command_at(&self.source, loader_start)?;
            let mut start = skip(&self.source, end);
            if self.source.as_bytes().get(start) == Some(&b'[') {
                start = syntax::balanced_end(&self.source, start, b'[', b']')
                    .map_or(self.source.len(), |end| skip(&self.source, end));
            }
            Some(if self.source.as_bytes().get(start) == Some(&b'{') {
                syntax::balanced_end(&self.source, start, b'{', b'}').unwrap_or(self.source.len())
            } else {
                self.source[start..]
                    .find(|c: char| c.is_whitespace() || "%{}".contains(c))
                    .map_or(self.source.len(), |n| start + n)
            })
        };
        let mut result = Vec::new();
        for event in &config.syntax.environment_alias_events {
            if let Some(at) = offset(event.at, &event.loader) {
                result.push(SourceImportEvent {
                    at,
                    aliases: event.aliases.clone(),
                    arguments: BTreeMap::new(),
                });
            }
        }
        for event in &config.syntax.environment_argument_events {
            if let Some(at) = offset(event.at, &event.loader) {
                result.push(SourceImportEvent {
                    at,
                    aliases: BTreeMap::new(),
                    arguments: event.arguments.clone(),
                });
            }
        }
        result.sort_by_key(|event| event.at);
        result
    }

    pub(crate) fn with_ranges(source: &str, config: &Config, ranges: &[Range<usize>]) -> Self {
        let tokens = tokenize(source, ranges);
        let mut model = Self {
            source: source.into(),
            tokens,
            diagnostics: Vec::new(),
            declarations: Vec::new(),
            environment_declarations: Vec::new(),
            environment_declaration_names: HashSet::new(),
            environment_arguments: HashMap::new(),
            context: crate::packages::context::ContextIndex::with_config(source, config),
            conditionals: Vec::new(),
            rules: Vec::new(),
            boundaries: HashMap::new(),
            exported: BTreeMap::new(),
            exported_unknown: HashSet::new(),
            exported_environments: BTreeMap::new(),
            registry: HashMap::new(),
            invocations: HashMap::new(),
            invocation_arguments: HashMap::new(),
            stored_arguments: Vec::new(),
            inferred_math_signatures: HashMap::new(),
            constructed: syntax::constant_constructed_commands(source, config, ranges),
            opaque: Vec::new(),
            protected: ranges.to_vec(),
            document_class: None,
            structure: None,
        };
        model.index(config);
        model.protected.extend(model.opaque.iter().cloned());
        model.protected.sort_by_key(|r| r.start);
        let mut merged: Vec<Range<usize>> = Vec::new();
        for range in std::mem::take(&mut model.protected) {
            if let Some(last) = merged.last_mut().filter(|last| last.end >= range.start) {
                last.end = last.end.max(range.end);
            } else {
                merged.push(range);
            }
        }
        model.protected = merged;
        model
    }

    pub fn source(&self) -> &str {
        &self.source
    }
    pub fn source_bytes(&self) -> &[u8] {
        self.source.as_bytes()
    }
    pub fn tokens(&self) -> &[SourceToken] {
        &self.tokens
    }
    pub fn declarations(&self) -> &[Declaration] {
        &self.declarations
    }
    pub fn environment_declarations(&self) -> &[EnvironmentDeclaration] {
        &self.environment_declarations
    }
    /// Header signatures resolved where an environment begins. The closing
    /// offset of the begin token is also accepted by layout consumers.
    pub fn environment_arguments_at(&self, at: usize) -> Option<&EnvironmentArguments> {
        self.environment_arguments.get(&at)
    }
    pub(crate) fn invocation_context_at(
        &self,
        at: usize,
    ) -> crate::packages::context::InvocationContext<'_> {
        self.context.at(at)
    }
    pub fn conditionals(&self) -> &[Conditional] {
        &self.conditionals
    }
    pub fn rules(&self) -> &[SyntaxRule] {
        &self.rules
    }
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }
    pub fn document_class(&self) -> Option<&str> {
        self.document_class.as_deref()
    }
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error)
    }
    pub fn reconstruct(&self) -> String {
        let mut result = String::with_capacity(self.source.len());
        for token in &self.tokens {
            result.push_str(&self.source[token.span.clone()]);
        }
        result
    }
    pub fn boundary_at(&self, at: usize) -> Option<&EnvironmentAlias> {
        self.boundaries.get(&at)
    }
    pub fn command_arguments(&self, name: &str) -> Option<&EnvironmentArguments> {
        match &self.registry.get(name)?.meaning {
            RuleMeaning::Arguments(arguments) => Some(arguments),
            _ => None,
        }
    }
    pub fn resolved_rule(&self, name: &str) -> Option<&SyntaxRule> {
        self.registry.get(name)
    }
    pub fn invocation_rule(&self, at: usize) -> Option<&SyntaxRule> {
        self.invocations.get(&at)
    }
    pub(crate) fn invocation_argument_ranges_at(
        &self,
        at: usize,
    ) -> Option<&[syntax::xparse::ArgumentSlot]> {
        self.invocation_arguments.get(&at).map(Vec::as_slice)
    }
    /// Operands proven to be stored or ignored by their source definition.
    /// Delimiters remain part of the range so printers can preserve their
    /// contents while retaining the surrounding argument structure.
    pub(crate) fn stored_argument_ranges(&self) -> &[Range<usize>] {
        &self.stored_arguments
    }
    pub(crate) fn inferred_math_signature_at(
        &self,
        at: usize,
    ) -> Option<crate::packages::math::Signature> {
        self.inferred_math_signatures
            .get(&at)
            .map(|&(shape, domains)| crate::packages::math::Signature { shape, domains })
    }
    pub(crate) fn constructed_commands(&self) -> &HashMap<usize, (String, usize)> {
        &self.constructed
    }
    pub fn exported_aliases(&self) -> &BTreeMap<String, EnvironmentAlias> {
        &self.exported
    }
    pub fn exported_unknown_aliases(&self) -> &HashSet<String> {
        &self.exported_unknown
    }
    pub fn exported_environment_arguments(
        &self,
    ) -> &BTreeMap<String, Option<EnvironmentArguments>> {
        &self.exported_environments
    }
    pub(crate) fn has_environment_declaration(&self, name: &str) -> bool {
        self.environment_declaration_names.contains(name)
    }
    #[doc(hidden)]
    pub fn exported_packages(&self) -> BTreeMap<String, bool> {
        self.context.export_packages()
    }
    pub fn opaque_ranges(&self) -> &[Range<usize>] {
        &self.opaque
    }
    pub fn protected_ranges(&self) -> &[Range<usize>] {
        &self.protected
    }

    fn index(&mut self, config: &Config) {
        let mut pairs = HashMap::new();
        let mut groups = Vec::new();
        for (index, token) in self.tokens.iter().enumerate() {
            match token.kind {
                SourceTokenKind::OpenGroup => groups.push(index),
                SourceTokenKind::CloseGroup => {
                    if let Some(open) = groups.pop() {
                        pairs.insert(open, index);
                    }
                    // The structural parser diagnoses unmatched delimiters after
                    // command operands and conditional protection are resolved.
                }
                _ => (),
            }
        }
        let mut bindings = HashMap::<String, Binding>::new();
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
                        math_signature: None,
                    },
                );
            }
        }
        let local_targets: HashSet<_> = if config.syntax.inferred_environment_aliases.is_empty() {
            HashSet::new()
        } else {
            self.tokens
                .iter()
                .enumerate()
                .filter_map(|(at, token)| {
                    (token.kind == SourceTokenKind::Control)
                        .then(|| definition(&self.source, &self.tokens, &pairs, at))
                        .flatten()
                        .map(|(d, _, _, _)| d.name)
                })
                .collect()
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
                        math_signature: None,
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
                    math_signature: None,
                },
            );
            self.rules.push(SyntaxRule {
                name: name.clone(),
                meaning: RuleMeaning::Environment(alias.clone()),
                origin: RuleOrigin::Configuration,
                span: None,
            });
        }
        for (name, rule) in &config.syntax.commands {
            self.rules.push(SyntaxRule {
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
        let mut scopes = vec![HashMap::<String, Option<Binding>>::new()];
        let mut conditions: Vec<(usize, Vec<Range<usize>>, usize)> = Vec::new();
        let mut pending_conditions = BTreeMap::<usize, Vec<usize>>::new();
        let mut active_conditions = BinaryHeap::<Reverse<usize>>::new();
        let mut seen_builtins = HashSet::new();
        let mut foreach_variables = HashMap::<String, usize>::new();
        let mut global_prefix = false;
        let imports = self.import_events(config);
        let mut import_index = 0;
        let mut pos = 0;
        while pos < self.tokens.len() {
            let token = &self.tokens[pos];
            let start = token.span.start;
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
                        let replacement = alias.as_ref().map_or(Replacement::Unknown, |alias| {
                            Replacement::Boundary(alias.clone())
                        });
                        assign(
                            &mut bindings,
                            &mut scopes,
                            name,
                            Binding {
                                replacement,
                                candidate: true,
                                signature: None,
                                condition: None,
                                literal_arguments: false,
                                math_signature: None,
                            },
                            false,
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
                            math_signature: None,
                        },
                        false,
                    );
                }
                import_index += 1;
            }
            if token.kind == SourceTokenKind::OpenGroup {
                scopes.push(HashMap::new());
                pos += 1;
                continue;
            }
            if token.kind == SourceTokenKind::CloseGroup {
                if scopes.len() > 1 {
                    for (name, previous) in scopes.pop().unwrap() {
                        if let Some(previous) = previous {
                            bindings.insert(name, previous);
                        } else {
                            bindings.remove(&name);
                        }
                    }
                }
                pos += 1;
                continue;
            }
            if token.kind != SourceTokenKind::Control {
                pos += 1;
                continue;
            }
            let Some((control, end)) = syntax::command_at(&self.source, start) else {
                pos += 1;
                continue;
            };
            let constructed = self.constructed.get(&start);
            let end = constructed.map_or(end, |(_, finish)| *finish);
            let control = constructed.map_or(control, |(name, _)| name.as_str());
            let name = control.trim_end_matches('*').to_owned();
            if foreach_variables
                .get(&name)
                .is_some_and(|&finish| start < finish)
            {
                self.invocations.insert(
                    start,
                    SyntaxRule {
                        name,
                        meaning: RuleMeaning::Unknown,
                        origin: RuleOrigin::Source,
                        span: Some(start..end),
                    },
                );
                pos += 1;
                continue;
            }
            if name == "foreach"
                && !config.syntax.commands.contains_key(&name)
                && !bindings.contains_key(&name)
                && let Some(binding) = syntax::arguments::foreach_binding(&self.source, end, |at| {
                    let index = self.tokens.partition_point(|token| token.span.start < at);
                    pairs.get(&index).map(|&close| self.tokens[close].span.end)
                })
            {
                for variable in binding.names {
                    foreach_variables
                        .entry(variable.to_owned())
                        .and_modify(|finish| *finish = (*finish).max(binding.end))
                        .or_insert(binding.end);
                }
            }
            if !config.syntax.commands.contains_key(&name)
                && !bindings.contains_key(&name)
                && let Some(environment) = crate::packages::scoped_environment_for_command(control)
            {
                self.context.observe_environment(
                    environment,
                    control.strip_prefix("end") != Some(environment),
                    end,
                );
            }
            if !config.syntax.commands.contains_key(&name)
                && !bindings.contains_key(&name)
                && let Some(branches) =
                    crate::packages::command_branch_ranges(&self.source, control, end)
            {
                for branch in branches {
                    pending_conditions
                        .entry(branch.start)
                        .or_default()
                        .push(branch.end);
                }
            }
            self.context
                .observe(&self.source, control, start, end, certain);
            if name == "documentclass" && self.document_class.is_none() {
                let arguments = crate::layout::signature_argument_ranges(
                    &self.source,
                    end..self.source.len(),
                    "om".bytes(),
                );
                if let Some(argument) = arguments.last()
                    && self.source.as_bytes()[argument.start] == b'{'
                {
                    let class = self.source[argument.start + 1..argument.end - 1].trim();
                    if !class.is_empty() && !class.contains(['\\', '#', '{', '}']) {
                        self.document_class = Some(class.into());
                    }
                }
            }
            if name == "global" && !bindings.contains_key(&name) {
                global_prefix = true;
                pos += 1;
                continue;
            }
            if matches!(name.as_str(), "long" | "outer" | "protected")
                && !bindings.contains_key(&name)
            {
                pos += 1;
                continue;
            }
            if matches!(name.as_str(), "begingroup" | "bgroup") {
                scopes.push(HashMap::new());
                pos += 1;
                continue;
            }
            if matches!(name.as_str(), "endgroup" | "egroup") {
                restore_scope(&mut bindings, &mut scopes);
                pos += 1;
                continue;
            }
            if !config.syntax.commands.contains_key(&name)
                && !bindings.contains_key(&name)
                && let Some(reason) =
                    crate::packages::context_ambiguous(control, &self.context.at(start))
            {
                let finish = adjacent_arguments_end(&self.source, end);
                self.opaque.push(start..finish);
                self.diagnostics.push(Diagnostic {
                    severity: Severity::Warning,
                    message: format!("Cannot choose the grammar of \\{control}: {reason}; this invocation is preserved"),
                    span: start..end,
                });
                pos = self
                    .tokens
                    .partition_point(|token| token.span.start < finish);
                continue;
            }
            let invocation = if let Some(rule) = config.syntax.commands.get(&name) {
                Some(SyntaxRule {
                    name: name.clone(),
                    meaning: RuleMeaning::Arguments(EnvironmentArguments {
                        arguments: rule.arguments.unwrap_or(0),
                        optional_arguments: rule.optional_arguments,
                        signature: rule.signature.clone(),
                    }),
                    origin: RuleOrigin::Configuration,
                    span: Some(start..end),
                })
            } else if let Some(binding) = bindings.get(&name) {
                let meaning = binding_meaning(&name, binding, &bindings);
                Some(SyntaxRule {
                    name: name.clone(),
                    meaning,
                    origin: RuleOrigin::Source,
                    span: Some(start..end),
                })
            } else {
                crate::packages::command_signature_in_context(
                    &self.source,
                    control,
                    end,
                    &self.context.at(start),
                )
                .map(|signature| SyntaxRule {
                    name: name.clone(),
                    meaning: RuleMeaning::Arguments(EnvironmentArguments {
                        signature: Some(signature.into()),
                        ..Default::default()
                    }),
                    origin: RuleOrigin::Builtin,
                    span: Some(start..end),
                })
            };
            if let Some(rule) = invocation {
                if rule.origin == RuleOrigin::Source
                    && let Some(signature) = bindings
                        .get(&name)
                        .and_then(|binding| binding.math_signature)
                {
                    self.inferred_math_signatures.insert(start, signature);
                }
                if rule.origin == RuleOrigin::Builtin
                    && let Some(ranges) = crate::packages::context_invocation_ranges(
                        &self.source,
                        control,
                        end,
                        &self.context.at(start),
                    )
                {
                    match ranges {
                        Ok(ranges) => {
                            self.invocation_arguments.insert(start, ranges);
                        }
                        Err(span) => {
                            self.opaque.push(start..span.end);
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
                    && !self.invocation_arguments.contains_key(&start)
                    && let Some(signature) = arguments.signature.as_deref()
                    && let Some(span) =
                        syntax::arguments::unclosed_argument(&self.source, end, signature)
                {
                    self.diagnostics.push(Diagnostic {
                        severity: Severity::Error,
                        message: format!("Unclosed argument of \\{control}; source retained"),
                        span,
                    });
                }
                self.invocations.insert(start, rule);
            }
            if matches!(name.as_str(), "begin" | "end") {
                let argument = skip(&self.source, end);
                if let Some(finish) = syntax::balanced_end(&self.source, argument, b'{', b'}') {
                    let alias = EnvironmentAlias {
                        environment: self.source[argument + 1..finish - 1].trim().into(),
                        begin: name == "begin",
                    };
                    if alias.begin
                        && !config
                            .syntax
                            .environment_arguments
                            .contains_key(&alias.environment)
                        && !bindings.contains_key(&format!("environment:{}", alias.environment))
                        && let Some(reason) = crate::packages::context_ambiguous(
                            &alias.environment,
                            &self.context.at(start),
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
                            self.environment_arguments.insert(start, arguments.clone());
                            self.environment_arguments.insert(finish, arguments.clone());
                        } else if bindings.contains_key(&key) {
                            let invocation_end = syntax::inferred_environment_end(
                                &self.source,
                                finish,
                                &alias.environment,
                                config,
                            );
                            self.opaque.push(start..invocation_end);
                            self.diagnostics.push(Diagnostic {
                                severity: Severity::Warning,
                                message: format!("Cannot safely infer the header of environment {:?}; this invocation is preserved", alias.environment),
                                span: start..finish,
                            });
                            pos = self
                                .tokens
                                .partition_point(|token| token.span.start < invocation_end);
                            continue;
                        }
                    }
                    // Environment invocation groups have the same lifetime as
                    // brace groups; the document wrapper supplies file context.
                    if alias.environment != "document" {
                        if alias.begin {
                            scopes.push(HashMap::new());
                        } else {
                            restore_scope(&mut bindings, &mut scopes);
                        }
                    }
                    self.rules.push(SyntaxRule {
                        name,
                        meaning: RuleMeaning::Environment(alias),
                        origin: RuleOrigin::Builtin,
                        span: Some(start..finish),
                    });
                    pos = self.tokens.partition_point(|t| t.span.start < finish);
                    continue;
                }
            }
            if !bindings.contains_key(&name)
                && let Some(declaration) =
                    syntax::source_environment_declaration(&self.source, start, control, end)
            {
                let key = format!("environment:{}", declaration.name);
                if !declaration.provide || !bindings.contains_key(&key) {
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
                            replacement: Replacement::Unknown,
                            candidate: false,
                            signature,
                            condition: None,
                            literal_arguments: false,
                            math_signature: None,
                        },
                        global_prefix,
                    );
                }
                let finish = declaration.span.end;
                self.environment_declaration_names
                    .insert(declaration.name.clone());
                self.environment_declarations.push(declaration);
                global_prefix = false;
                pos = self
                    .tokens
                    .partition_point(|token| token.span.start < finish);
                continue;
            }
            if !bindings.contains_key(&name)
                && (name.ends_with("environment")
                    || name.ends_with("DocumentEnvironment")
                    || matches!(name.as_str(), "NewEnviron" | "RenewEnviron"))
                && let Some(finish) = syntax::delayed_data_end(&self.source, control, end)
            {
                self.opaque.push(start..finish);
                self.diagnostics.push(Diagnostic {
                    severity: Severity::Warning,
                    message: "Cannot infer a literal environment name from this declaration; the declaration is preserved".into(),
                    span: start..finish,
                });
                pos = self
                    .tokens
                    .partition_point(|token| token.span.start < finish);
                continue;
            }
            if !bindings.contains_key(&name)
                && let Some(definition) = definition(&self.source, &self.tokens, &pairs, pos)
            {
                let (declaration, finish, global, provide) = definition;
                // Expl3 definitions share the delayed-body semantics of
                // classic declarations. The code printer can expose their
                // body separately; ordinary structural/math passes must not
                // execute stored begin/end tokens within that replacement.
                if config.protect.macro_bodies && name.starts_with("cs_") {
                    self.protected.push(declaration.body.clone());
                }
                let body = &self.source[declaration.body.clone()];
                let zero = declaration.signature.as_ref().is_some_and(|s| {
                    s.arguments == 0
                        && !s.optional_arguments
                        && s.signature.as_deref().is_none_or(str::is_empty)
                });
                let previous_candidate =
                    bindings.get(&declaration.name).is_some_and(|b| b.candidate);
                // Declaration bodies may be opaque in the document partition.
                // Read their lexical children with the same scanner before
                // interpreting complete command names.
                let contains_boundary = body_has_environment_effect(body, config, false, false);
                let parameterized_wrapper = contains_boundary
                    && body.contains('#')
                    && !body_has_environment_effect(body, config, true, false);
                let candidate = previous_candidate
                    || contains_boundary
                    || syntax::command_at(body, skip(body, 0))
                        .is_some_and(|(n, _)| bindings.get(n).is_some_and(|b| b.candidate));
                let literal_arguments = certain
                    && !candidate
                    && (parameters_only_in_data(body, &bindings)
                        || name.starts_with("cs_") && !body.contains('#'));
                let mut replacement = if certain && parameterized_wrapper {
                    Replacement::NeutralWrapper
                } else if certain && zero {
                    replacement(body)
                } else {
                    Replacement::Unknown
                };
                if matches!(name.as_str(), "edef" | "xdef")
                    && let Replacement::Forward(target) = &replacement
                {
                    replacement = resolve(target, &bindings)
                        .map_or(Replacement::Unknown, Replacement::Boundary);
                }
                if !(config
                    .syntax
                    .environment_aliases
                    .contains_key(&declaration.name)
                    || provide && bindings.contains_key(&declaration.name))
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
                            condition: None,
                            literal_arguments,
                            math_signature: certain
                                .then(|| crate::packages::scientific::factory_math_signature(&name))
                                .flatten()
                                .map(|signature| (signature.shape, signature.domains)),
                        },
                        global || global_prefix,
                    );
                }
                if let Some(signature) = &declaration.signature {
                    self.rules.push(SyntaxRule {
                        name: declaration.name.clone(),
                        meaning: RuleMeaning::Arguments(signature.clone()),
                        origin: RuleOrigin::Source,
                        span: Some(declaration.span.clone()),
                    });
                } else {
                    self.diagnostics.push(Diagnostic { severity: Severity::Warning, message: format!("Cannot infer the argument signature of \\{}; its definition is preserved", declaration.name), span: declaration.span.clone() });
                }
                self.declarations.push(declaration);
                global_prefix = false;
                pos = finish;
                continue;
            }
            if matches!(name.as_str(), "let" | "futurelet") {
                let target = next(&self.tokens, pos + 1);
                if let Some(target_token) = self.tokens.get(target)
                    && let Some((target_name, _)) =
                        syntax::command_at(&self.source, target_token.span.start)
                {
                    let mut rhs = next(&self.tokens, target + 1);
                    if self
                        .tokens
                        .get(rhs)
                        .is_some_and(|t| self.source[t.span.clone()] == *"=")
                    {
                        rhs = next(&self.tokens, rhs + 1);
                    }
                    if let Some(rhs_token) = self.tokens.get(rhs)
                        && let Some((rhs_name, _)) =
                            syntax::command_at(&self.source, rhs_token.span.start)
                    {
                        let mut binding = bindings.get(rhs_name).cloned().unwrap_or(Binding {
                            replacement: Replacement::Unknown,
                            candidate: false,
                            signature: None,
                            condition: syntax::is_condition_command(rhs_name, config)
                                .then(|| rhs_name.to_owned()),
                            literal_arguments: false,
                            math_signature: None,
                        });
                        if !certain || name == "futurelet" {
                            binding.replacement = Replacement::Unknown;
                            binding.signature = None;
                            binding.literal_arguments = false;
                            binding.math_signature = None;
                            if name == "futurelet" {
                                binding.condition = None;
                            }
                        }
                        if !config.syntax.environment_aliases.contains_key(target_name) {
                            assign(&mut bindings, &mut scopes, target_name, binding, false);
                        }
                        pos = rhs + 1;
                        continue;
                    }
                }
            }
            if name == "newif" {
                let target = next(&self.tokens, pos + 1);
                if let Some(token) = self.tokens.get(target)
                    && let Some((name, _)) = syntax::command_at(&self.source, token.span.start)
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
                            math_signature: None,
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
                .any(|condition| condition == &name);
            if let Some(binding) = bindings
                .get(&name)
                .filter(|binding| binding.literal_arguments)
                && let Some(signature) = &binding.signature
                && let Some(arguments) = invocation_arguments(&self.source, end, signature)
                && let Some(last) = arguments.last()
            {
                let finish = last.end;
                self.stored_arguments.extend(arguments.iter().cloned());
                self.protected.extend(arguments);
                pos = self
                    .tokens
                    .partition_point(|token| token.span.start < finish);
                continue;
            }
            if configured_condition
                || bindings.get(&name).map_or_else(
                    || syntax::is_condition_command(&name, config),
                    |binding| binding.condition.is_some(),
                )
            {
                let reader = bindings
                    .get(&name)
                    .and_then(|binding| binding.condition.as_deref())
                    .unwrap_or(control);
                let operand_end =
                    syntax::test_operand_end(&self.source, reader, end).unwrap_or(end);
                if reader != control && operand_end > end {
                    self.protected.push(end..operand_end);
                }
                conditions.push((start, Vec::new(), operand_end));
                let origin = if configured_condition {
                    RuleOrigin::Configuration
                } else if bindings.contains_key(&name) {
                    RuleOrigin::Source
                } else {
                    RuleOrigin::Builtin
                };
                self.rules.push(SyntaxRule {
                    name,
                    meaning: RuleMeaning::Condition,
                    origin,
                    span: Some(start..end),
                });
                pos = self.tokens.partition_point(|t| t.span.start < operand_end);
                continue;
            }
            if matches!(name.as_str(), "else" | "or") {
                if let Some((_, branches, branch_start)) = conditions.last_mut() {
                    branches.push(*branch_start..start);
                    *branch_start = end;
                }
                pos += 1;
                continue;
            }
            if name == "fi" {
                if let Some((at, mut branches, branch_start)) = conditions.pop() {
                    branches.push(branch_start..start);
                    self.conditionals.push(Conditional {
                        span: at..end,
                        branches,
                    });
                }
                pos += 1;
                continue;
            }
            if let Some(alias) = resolve(&name, &bindings) {
                self.context
                    .observe_environment(&alias.environment, alias.begin, end);
                if alias.begin {
                    scopes.push(HashMap::new());
                } else {
                    restore_scope(&mut bindings, &mut scopes);
                }
                self.boundaries.insert(start, alias.clone());
                self.rules.push(SyntaxRule {
                    name,
                    meaning: RuleMeaning::Environment(alias),
                    origin: RuleOrigin::Source,
                    span: Some(start..end),
                });
            } else if bindings.get(&name).is_some_and(|binding| {
                binding.candidate
                    && !(matches!(binding.replacement, Replacement::NeutralWrapper)
                        && binding.signature.as_ref().is_some_and(|signature| {
                            neutral_wrapper_arguments(
                                &self.source,
                                end,
                                signature,
                                config,
                                &bindings,
                            )
                        }))
            }) {
                self.diagnostics.push(Diagnostic { severity: Severity::Warning, message: format!("Cannot safely resolve environment alias \\{name}; the following source is preserved"), span: start..end });
                if self.opaque.is_empty() {
                    self.opaque.push(start..self.source.len());
                }
            } else if seen_builtins.insert(name.clone())
                && let Some(rule) = crate::package_commands::builtin_command_rule(&name)
            {
                self.rules.push(SyntaxRule {
                    name,
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
                self.tokens.partition_point(|t| t.span.start < end)
            } else {
                pos + 1
            };
        }
        for name in bindings.keys() {
            if let Some(environment) = name.strip_prefix("environment:") {
                self.exported_environments
                    .insert(environment.to_owned(), bindings[name].signature.clone());
                continue;
            }
            if let Replacement::Boundary(alias) = &bindings[name].replacement {
                self.exported.insert(name.clone(), alias.clone());
            } else if bindings[name].candidate || resolve(name, &bindings).is_some() {
                self.exported_unknown.insert(name.clone());
            }
        }
        // Every provider produces the same rule shape and enters the same
        // precedence resolver. Source scope has settled before export.
        for rule in self.rules.iter().filter(|r| r.origin != RuleOrigin::Source) {
            register(&mut self.registry, rule.clone());
        }
        for (name, binding) in &bindings {
            if name.starts_with("environment:") {
                continue;
            }
            let meaning = binding_meaning(name, binding, &bindings);
            register(
                &mut self.registry,
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
fn parameters_only_in_data(body: &str, bindings: &HashMap<String, Binding>) -> bool {
    if !body.contains('#') {
        return false;
    }
    let mut cursor = 0;
    let mut saw_parameter = false;
    for token in SourceTokens::new(body, &[]) {
        if token.span.start < cursor || token.kind == SourceTokenKind::Comment {
            continue;
        }
        if token.kind == SourceTokenKind::Control
            && let Some((name, end)) = syntax::command_at(body, token.span.start)
        {
            let finish = syntax::delayed_data_end(body, name, end)
                .or_else(|| {
                    if let Some(binding) = bindings.get(name) {
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
                saw_parameter |= body[end..finish].contains('#');
                cursor = finish;
            }
        } else if body[token.span].contains('#') {
            return false;
        }
    }
    saw_parameter
}

/// These kernel operations store their token-list operand. Their declaration
/// spellings describe the preceding variable/name operand; other expl3 `n`
/// arguments remain executable unless a source definition proves otherwise.
fn expl3_stored_data_end(source: &str, name: &str, end: usize) -> Option<usize> {
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
    ) || !matches!(variant, "Nn" | "cn")
    {
        return None;
    }
    syntax::arguments::signature_end(source, end, "mm")
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
    let message = error.to_string();
    let mut at = message
        .rsplit(" at byte ")
        .next()
        .and_then(|s| s.split_whitespace().next())
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0)
        .min(source.len());
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

fn restore_scope(
    bindings: &mut HashMap<String, Binding>,
    scopes: &mut Vec<HashMap<String, Option<Binding>>>,
) {
    if scopes.len() > 1 {
        for (name, previous) in scopes.pop().unwrap() {
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
    scopes: &mut [HashMap<String, Option<Binding>>],
    name: &str,
    binding: Binding,
    global: bool,
) {
    if global {
        for scope in scopes.iter_mut().skip(1) {
            scope.remove(name);
        }
    } else if scopes.len() > 1 {
        scopes
            .last_mut()
            .unwrap()
            .entry(name.into())
            .or_insert_with(|| bindings.get(name).cloned());
    }
    bindings.insert(name.into(), binding);
}

fn resolve(name: &str, bindings: &HashMap<String, Binding>) -> Option<EnvironmentAlias> {
    let mut slow = name;
    let mut fast = name;
    loop {
        slow = match &bindings.get(slow)?.replacement {
            Replacement::Boundary(alias) => return Some(alias.clone()),
            Replacement::Forward(target) => target,
            Replacement::Unknown | Replacement::NeutralWrapper => return None,
        };
        // Two cursors detect a cycle in work proportional to the chain length,
        // including short cycles in a file with many unrelated declarations.
        for _ in 0..2 {
            fast = match &bindings.get(fast)?.replacement {
                Replacement::Boundary(alias) => return Some(alias.clone()),
                Replacement::Forward(target) => target,
                Replacement::Unknown | Replacement::NeutralWrapper => return None,
            };
        }
        if slow == fast {
            return None;
        }
    }
}

fn replacement(body: &str) -> Replacement {
    let body = body[skip(body, 0)..].trim_end();
    let Some((name, end)) = syntax::command_at(body, 0) else {
        return Replacement::Unknown;
    };
    if matches!(name, "begin" | "end") {
        let start = skip(body, end);
        if let Some(finish) = syntax::balanced_end(body, start, b'{', b'}')
            && skip(body, finish) == body.len()
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
    if !complete && !body.contains("\\begin") && !body.contains("\\end") {
        return false;
    }
    let mut tokens = SourceTokens::new(body, &[]);
    let mut environments = Vec::new();
    let mut conditions = Vec::new();
    let mut seen_boundary = false;
    let mut uncertain = false;
    let mut invalid_condition = false;
    while let Some(token) = tokens.next() {
        if !neutral_parameters
            && token.kind == SourceTokenKind::Text
            && body[token.span.clone()].contains('#')
        {
            uncertain = true;
        }
        if token.kind != SourceTokenKind::Control {
            continue;
        }
        let Some((name, end)) = syntax::command_at(body, token.span.start) else {
            continue;
        };
        if let Some(finish) = syntax::primitive_data_end(body, name, end)
            .or_else(|| syntax::delayed_data_end(body, name, end))
            .or_else(|| syntax::let_declaration_end(body, name, end))
        {
            tokens.position = finish;
            continue;
        }
        if matches!(name, "csname" | "endcsname" | "expandafter" | "scantokens") {
            if complete {
                return true;
            }
            uncertain = true;
        } else if matches!(name, "begin" | "end") {
            seen_boundary = true;
            let start = skip(body, end);
            let Some(finish) = syntax::balanced_end(body, start, b'{', b'}') else {
                return true;
            };
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
        } else if name == "newif" {
            tokens.position = syntax::command_at(body, skip(body, end)).map_or(end, |(_, end)| end);
        } else if let Some(finish) = syntax::test_operand_end(body, name, end) {
            tokens.position = finish;
        }
    }
    seen_boundary && uncertain
        || invalid_condition
        || !environments.is_empty()
        || !conditions.is_empty()
}

fn neutral_wrapper_arguments(
    source: &str,
    end: usize,
    signature: &EnvironmentArguments,
    config: &Config,
    bindings: &HashMap<String, Binding>,
) -> bool {
    let Some(arguments) = invocation_arguments(source, end, signature) else {
        return false;
    };
    if arguments.len() < signature.arguments {
        return false;
    }
    arguments.iter().all(|argument| {
        // An unbraced operand can be a single math shift or a grouping token.
        // Require an explicit operand boundary before proving a replacement
        // independently of the wrapper's surrounding text.
        if source.as_bytes().get(argument.start) != Some(&b'{') {
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
            if token.kind != SourceTokenKind::Control {
                return true;
            }
            let Some((name, _)) = syntax::command_at(body, token.span.start) else {
                return false;
            };
            !matches!(name, "csname" | "endcsname" | "expandafter" | "scantokens")
                && !bindings.get(name).is_some_and(|binding| binding.candidate)
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
    pairs: &HashMap<usize, usize>,
    at: usize,
) -> Option<(Declaration, usize, bool, bool)> {
    let (command, command_end) = syntax::command_at(source, tokens[at].span.start)?;
    let command = command.trim_end_matches('*');
    let primitive = matches!(command, "def" | "gdef" | "edef" | "xdef");
    let classic = syntax::classic_command_definition(command);
    let fixed = syntax::fixed_command_factory_signature(command);
    if let Some(declaration) =
        syntax::expl3_command_declaration(source, tokens[at].span.start, command, command_end)
    {
        let finish = tokens.partition_point(|token| token.span.start < declaration.span.end);
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
        let finish = tokens.partition_point(|token| token.span.start < declaration.span.end);
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
    let (name, name_end) = syntax::command_at(source, tokens.get(name_at)?.span.start)?;
    if target_group {
        let finish = *pairs.get(&cursor)?;
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
    let finish = tokens.partition_point(|t| t.span.start < body_end);
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

/// Streaming lexical partition shared by stored snapshots and filtered readers.
struct SourceTokens<'a> {
    source: &'a str,
    ranges: &'a [Range<usize>],
    position: usize,
    protected_index: usize,
}

impl<'a> SourceTokens<'a> {
    fn new(source: &'a str, ranges: &'a [Range<usize>]) -> Self {
        Self {
            source,
            ranges,
            position: 0,
            protected_index: 0,
        }
    }
}

impl Iterator for SourceTokens<'_> {
    type Item = SourceToken;

    fn next(&mut self) -> Option<Self::Item> {
        let source = self.source;
        let start = self.position;
        if start == source.len() {
            return None;
        }
        while self
            .ranges
            .get(self.protected_index)
            .is_some_and(|r| r.end <= start)
        {
            self.protected_index += 1;
        }
        let protected = self.ranges.get(self.protected_index);
        let limit = protected.map_or(source.len(), |r| r.start.max(start));
        let mut pos = start;
        let kind = if let Some(range) = protected.filter(|r| r.start <= start) {
            pos = range.end.min(source.len());
            SourceTokenKind::Opaque
        } else {
            match source.as_bytes()[pos] {
                b'\\' => {
                    pos = syntax::command_at(source, pos).map_or(pos + 1, |(_, end)| end);
                    SourceTokenKind::Control
                }
                b'%' => {
                    pos = source[pos..].find('\n').map_or(source.len(), |n| pos + n);
                    SourceTokenKind::Comment
                }
                b'{' => {
                    pos += 1;
                    SourceTokenKind::OpenGroup
                }
                b'}' => {
                    pos += 1;
                    SourceTokenKind::CloseGroup
                }
                b'\r' | b'\n' => {
                    pos += if source[pos..].starts_with("\r\n") {
                        2
                    } else {
                        1
                    };
                    SourceTokenKind::Newline
                }
                b' ' | b'\t' => {
                    pos += 1;
                    while pos < limit && matches!(source.as_bytes()[pos], b' ' | b'\t') {
                        pos += 1;
                    }
                    SourceTokenKind::Space
                }
                b'=' => {
                    pos += 1;
                    SourceTokenKind::Text
                }
                _ => {
                    // ASCII delimiters and validated range boundaries delimit
                    // whole UTF-8 scalars, so ordinary text can be one slice.
                    pos += 1;
                    while pos < limit
                        && !matches!(
                            source.as_bytes()[pos],
                            b'\\' | b'%' | b'{' | b'}' | b'\r' | b'\n' | b' ' | b'\t' | b'='
                        )
                    {
                        pos += 1;
                    }
                    SourceTokenKind::Text
                }
            }
        };
        self.position = pos;
        Some(SourceToken {
            kind,
            span: start..pos,
        })
    }
}

fn tokenize(source: &str, ranges: &[Range<usize>]) -> Vec<SourceToken> {
    SourceTokens::new(source, ranges).collect()
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
                tokens: model.tokens,
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

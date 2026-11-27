//! Persistent declaration state and checkpoint inference for project preparation.

use super::{
    CompiledSyntax, ContextDeclaration, DeclarationReuseBoundary, balanced_end, command_at,
    compile, context_argument, context_box_declaration, context_builtin_command,
    context_declaration_command, context_factory_range, context_literal_name,
    context_macro_declaration_with_config, context_primitive_declaration,
    context_theorem_declaration, declaration_export_protection,
    declaration_protection_validation_needed, declaration_reader_complete,
    declaration_reuse_boundary_with_ranges, declaration_reuse_config_supported, env_at,
    has_context_declaration, infer_list_environments_with_config, inferred_condition,
    inferred_protected_environments, is_condition_command, latex_macro_definition,
    let_declaration_end, line_end, literal_csname, macro_body, next_character_test_end,
    primitive_condition, primitive_data_end, primitive_test_end, protected_ranges, skip_trivia,
    source_environment_declaration, token_data_range,
};

use crate::config::{CommandRule, EnvironmentArguments, ProtectConfig, SyntaxConfig};
use crate::{Config, FormatError};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    ops::Range,
    sync::Arc,
};

/// Augment explicit settings with supported literal declarations from project
/// sources. The source is used only as declaration context; formatting and
/// diagnostic offsets continue to refer to the original input file.
///
/// Definitions are read in source order outside groups, conditional branches,
/// comments and protected content. Explicit configuration always wins. Dynamic
/// or unsupported argument specifications do not inherit an earlier signature.
pub fn infer_declaration_config(source: &str, config: &Config) -> Result<Config, FormatError> {
    infer_declaration_config_impl(source, config, false, false, None, None)
        .map(|(config, _, _)| config)
}

/// Infer project declarations and retain a completed scanner boundary for an
/// independently balanced, declaration-free append. Ordinary open scopes keep
/// their existing lifetime; pending declarations or data operands cannot reuse
/// the result. The public declaration reuse policy remains a separate gate.
#[doc(hidden)]
pub fn infer_declaration_config_with_continuation(
    source: &str,
    config: &Config,
) -> Result<(Config, bool), FormatError> {
    infer_declaration_config_impl(source, config, true, false, None, None)
        .map(|(config, safe, _)| (config, safe))
}

/// Share protection analysis between declaration inference and its reuse gate.
#[doc(hidden)]
pub fn infer_declaration_config_with_reuse(
    source: &str,
    config: &Config,
) -> Result<(Config, bool, DeclarationReuseBoundary), FormatError> {
    infer_declaration_config_impl(source, config, true, true, None, None)
}

/// Append-only declaration analysis for one immutable base configuration.
#[doc(hidden)]
#[derive(Clone, Default)]
pub struct DeclarationIndex {
    compiled: Option<Arc<CompiledSyntax>>,
    protection_config: Option<Arc<ProtectionConfig>>,
    source_len: usize,
    protection: Option<ProtectionSnapshot>,
    markers: super::protection_tape::MarkerCursor,
    state: DeclarationScanState,
    output: Option<Arc<InferredSettings>>,
    aliases_match_base: bool,
    reusable: bool,
    last_scan_bytes: usize,
    last_incremental: bool,
    last_protection_scan_bytes: usize,
}

#[derive(Clone)]
struct ProtectionSnapshot {
    end: usize,
    digest: blake3::Hash,
    ranges: Arc<Vec<Range<usize>>>,
    plain_append: bool,
}

struct InferredSettings {
    commands: BTreeMap<String, CommandRule>,
    arguments: BTreeMap<String, EnvironmentArguments>,
    inferred_arguments: BTreeMap<String, EnvironmentArguments>,
    lists: Vec<String>,
    raw: Vec<String>,
}

impl InferredSettings {
    fn between(base: &Config, output: &Config) -> Self {
        fn changed<T: PartialEq + Clone>(
            base: &BTreeMap<String, T>,
            output: &BTreeMap<String, T>,
        ) -> BTreeMap<String, T> {
            output
                .iter()
                .filter(|(name, value)| base.get(*name) != Some(*value))
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect()
        }
        Self {
            commands: changed(&base.syntax.commands, &output.syntax.commands),
            arguments: changed(
                &base.syntax.environment_arguments,
                &output.syntax.environment_arguments,
            ),
            inferred_arguments: changed(
                &base.syntax.inferred_environment_arguments,
                &output.syntax.inferred_environment_arguments,
            ),
            lists: output.syntax.list_environments[base.syntax.list_environments.len()..].to_vec(),
            raw: output.protect.environments[base.protect.environments.len()..].to_vec(),
        }
    }

    fn apply(&self, base: &Config) -> Config {
        let mut output = base.clone();
        output.syntax.commands.extend(self.commands.clone());
        output
            .syntax
            .environment_arguments
            .extend(self.arguments.clone());
        output
            .syntax
            .inferred_environment_arguments
            .extend(self.inferred_arguments.clone());
        output
            .syntax
            .list_environments
            .extend_from_slice(&self.lists);
        output.protect.environments.extend_from_slice(&self.raw);
        output
    }
}

fn plain_protection_text(text: &str) -> bool {
    !text.contains(['\\', '%', '^', '`', '!', '{', '}', '[', ']'])
}

fn has_reader_events(syntax: &SyntaxConfig) -> bool {
    !syntax.package_load_events.is_empty()
        || !syntax.command_import_events.is_empty()
        || !syntax.environment_alias_events.is_empty()
        || !syntax.environment_argument_events.is_empty()
}

fn after_plain_definition<'a>(source: &'a str, config: &Config) -> Option<&'a str> {
    let (name @ ("newcommand" | "renewcommand" | "providecommand" | "def"), end) =
        command_at(source, 0)?
    else {
        return None;
    };
    if config.syntax.commands.contains_key(name)
        || config.protect.commands.iter().any(|cmd| cmd == name)
    {
        return None;
    }
    let header = &source[end..];
    let header = if name == "def" {
        header
    } else {
        header.strip_prefix('{')?
    };
    let (target, end) = command_at(header, 0)?;
    if !target.bytes().all(|byte| byte.is_ascii_alphabetic())
        || !super::declaration_ordinary_binding_name(target)
    {
        return None;
    }
    let mut tail = &header[end..];
    if name == "def" {
        while let [b'#', b'1'..=b'9', ..] = tail.as_bytes() {
            tail = &tail[2..];
        }
    } else {
        tail = tail.strip_prefix('}')?;
        if let [b'[', b'0'..=b'9', b']', ..] = tail.as_bytes() {
            tail = &tail[3..];
        }
    }
    let (body, rest) = tail.strip_prefix('{')?.split_once('}')?;
    plain_protection_text(body).then_some(rest)
}

fn plain_protection_boundary(mut source: &str, config: &Config) -> bool {
    let syntax = &config.syntax;
    if !declaration_reuse_config_supported(config)
        || has_reader_events(syntax)
        || !syntax.environment_aliases.is_empty()
        || !syntax.inferred_environment_aliases.is_empty()
        || !(source.is_empty() || source.ends_with('\n'))
    {
        return false;
    }
    while let Some(at) = source.find('\\') {
        if !plain_protection_text(&source[..at]) {
            return false;
        }
        let Some(rest) = after_plain_definition(&source[at..], config) else {
            return false;
        };
        source = rest;
    }
    plain_protection_text(source)
}

struct ProtectionConfig {
    syntax: SyntaxConfig,
    protect: ProtectConfig,
    code_algorithm2e: bool,
    code_expl3: bool,
    breaks_enabled: bool,
    breaks_pgfplots_tables: bool,
    breaks_pgfplots_named_tables: bool,
    align_consecutive: bool,
}

impl ProtectionConfig {
    fn matches(&self, config: &Config) -> bool {
        self.syntax == config.syntax
            && self.protect == config.protect
            && self.code_algorithm2e == config.code.algorithm2e
            && self.code_expl3 == config.code.expl3
            && self.breaks_enabled == config.breaks.enabled
            && self.breaks_pgfplots_tables == config.breaks.pgfplots_tables
            && self.breaks_pgfplots_named_tables == config.breaks.pgfplots_named_tables
            && self.align_consecutive == config.arguments.align_consecutive
    }
}

impl From<&Config> for ProtectionConfig {
    fn from(config: &Config) -> Self {
        Self {
            syntax: config.syntax.clone(),
            protect: config.protect.clone(),
            code_algorithm2e: config.code.algorithm2e,
            code_expl3: config.code.expl3,
            breaks_enabled: config.breaks.enabled,
            breaks_pgfplots_tables: config.breaks.pgfplots_tables,
            breaks_pgfplots_named_tables: config.breaks.pgfplots_named_tables,
            align_consecutive: config.arguments.align_consecutive,
        }
    }
}

#[derive(Clone, Default)]
struct DeclarationScanState {
    selected: HashMap<(bool, String), ContextDeclaration>,
    ambiguous: HashSet<(bool, String)>,
    depth: usize,
    conditional_depth: usize,
    environment_depth: usize,
    readable_factory_end: Option<usize>,
    continuation_conditions: Vec<(bool, bool)>,
    continuation_operand_end: usize,
    continuation_groups: Vec<bool>,
    append_safe: bool,
}

impl DeclarationIndex {
    #[doc(hidden)]
    pub fn infer_without_exported_aliases(
        &mut self,
        source: &str,
        config: &Config,
    ) -> Result<(Config, bool, DeclarationReuseBoundary), FormatError> {
        infer_declaration_config_impl(
            source,
            config,
            true,
            true,
            Some(self),
            Some(ExportRequest::FileOptions),
        )
    }

    #[doc(hidden)]
    pub fn infer_with_export_index(
        &mut self,
        source: &str,
        config: &Config,
        exports: &mut crate::source_model::ProjectExportIndex,
        snapshot: &mut Option<crate::source_model::ProjectExports>,
    ) -> Result<(Config, bool, DeclarationReuseBoundary), FormatError> {
        *snapshot = None;
        infer_declaration_config_impl(
            source,
            config,
            true,
            true,
            Some(self),
            Some(ExportRequest::All(exports, snapshot)),
        )
    }

    #[doc(hidden)]
    pub fn infer_with_alias_index(
        &mut self,
        source: &str,
        config: &Config,
        aliases: &mut crate::source_model::ProjectAliasIndex,
    ) -> Result<(Config, bool, DeclarationReuseBoundary), FormatError> {
        infer_declaration_config_impl(
            source,
            config,
            true,
            true,
            Some(self),
            Some(ExportRequest::Aliases(aliases)),
        )
    }

    #[doc(hidden)]
    pub fn reader_settled(&self) -> bool {
        self.reusable
    }

    #[doc(hidden)]
    pub fn scan_work(&self) -> (usize, bool) {
        (self.last_scan_bytes, self.last_incremental)
    }

    /// Bytes submitted to a full protection scan by the most recent call.
    /// Cached protection reuse reports zero.
    #[doc(hidden)]
    pub fn protection_scan_bytes(&self) -> usize {
        self.last_protection_scan_bytes
    }

    /// Borrow original-byte protection for the same append-only source and base settings.
    #[doc(hidden)]
    pub fn protected_prefix(&self, source: &str) -> Option<&[Range<usize>]> {
        let snapshot = self.protection.as_ref()?;
        let suffix = source.get(snapshot.end..)?;
        if !suffix.is_empty()
            && !(snapshot.plain_append
                && suffix
                    .bytes()
                    .all(|byte| matches!(byte, b'\n' | b'\r' | b' ' | b'\t')))
        {
            return None;
        }
        (blake3::hash(&source.as_bytes()[..snapshot.end]) == snapshot.digest)
            .then_some(snapshot.ranges.as_slice())
    }
}

/// The source must retain its existing prefix; changed base settings invalidate cached state.
#[doc(hidden)]
pub fn infer_declaration_config_indexed(
    source: &str,
    config: &Config,
    index: &mut DeclarationIndex,
) -> Result<(Config, bool, DeclarationReuseBoundary), FormatError> {
    infer_declaration_config_impl(source, config, true, true, Some(index), None)
}

enum ExportRequest<'a> {
    FileOptions,
    All(
        &'a mut crate::source_model::ProjectExportIndex,
        &'a mut Option<crate::source_model::ProjectExports>,
    ),
    Aliases(&'a mut crate::source_model::ProjectAliasIndex),
}

fn infer_declaration_config_impl(
    source: &str,
    config: &Config,
    continuation: bool,
    reuse: bool,
    mut index: Option<&mut DeclarationIndex>,
    exports: Option<ExportRequest<'_>>,
) -> Result<(Config, bool, DeclarationReuseBoundary), FormatError> {
    if let Some(index) = index.as_mut()
        && index
            .protection_config
            .as_ref()
            .is_none_or(|base| !base.matches(config))
    {
        **index = DeclarationIndex {
            protection_config: Some(Arc::new(config.into())),
            ..Default::default()
        };
    }
    let compiled = if let Some(index) = index.as_mut() {
        if index.compiled.is_none() && memchr::memchr2(b'\\', b'%', source.as_bytes()).is_some() {
            index.compiled = compile(&config.syntax).ok().map(Arc::new);
        }
        index.compiled.clone()
    } else {
        None
    };
    let mut previous = index.as_mut().and_then(|index| index.protection.take());
    let digest = index.as_ref().map(|_| {
        let mut hash = blake3::Hasher::new();
        if let Some(snapshot) = &previous
            && let Some(suffix) = source.get(snapshot.end..)
        {
            hash.update(&source.as_bytes()[..snapshot.end]);
            if hash.finalize() != snapshot.digest {
                previous = None;
            }
            hash.update(suffix.as_bytes());
        } else {
            previous = None;
            hash.update(source.as_bytes());
        }
        hash.finalize()
    });
    if previous.is_none()
        && let Some(index) = index.as_mut()
    {
        index.markers = Default::default();
    }
    let protection_incremental = previous.as_ref().is_some_and(|snapshot| {
        snapshot.plain_append
            && source
                .get(snapshot.end..)
                .is_some_and(|suffix| !suffix.is_empty() && plain_protection_text(suffix))
    });
    let indexed = index.is_some();
    let protection = |index: &mut Option<&mut DeclarationIndex>| {
        if protection_incremental {
            Ok(Arc::clone(&previous.as_ref().unwrap().ranges))
        } else if let Some(index) = index.as_mut() {
            if source.len() < 32 * 1024 {
                return declaration_export_protection(source, config, compiled.as_deref())
                    .map(Arc::new);
            }
            let markers = index
                .markers
                .update(source, previous.as_ref().map(|snapshot| snapshot.end));
            super::protected_ranges_indexed(
                source,
                config,
                compiled.as_deref(),
                Some(&mut Vec::new()),
                Default::default(),
                markers,
            )
            .map(Arc::new)
        } else {
            protected_ranges(source, config).map(Arc::new)
        }
    };
    let snapshot = |ranges: Arc<Vec<Range<usize>>>| ProtectionSnapshot {
        end: source.len(),
        digest: digest.expect("only indexed scans publish protection"),
        plain_append: ranges.last().is_none_or(|range| range.end < source.len())
            && (if protection_incremental {
                source.ends_with('\n')
            } else {
                let checked = previous
                    .as_ref()
                    .filter(|snapshot| snapshot.plain_append)
                    .map_or(0, |snapshot| snapshot.end);
                plain_protection_boundary(&source[checked..], config)
            }),
        ranges,
    };
    let was_reusable = index.as_mut().is_some_and(|index| {
        index.last_scan_bytes = 0;
        index.last_incremental = false;
        std::mem::take(&mut index.reusable)
    });
    let has_declaration = has_context_declaration(source);
    let validate_protection =
        has_declaration || declaration_protection_validation_needed(source, config);
    if !has_declaration {
        if let Some(index) = index.as_mut() {
            index.last_protection_scan_bytes =
                usize::from(!protection_incremental && (validate_protection || reuse))
                    * source.len();
        }
        // The original no-declaration inference only validates protection
        // when requested by its established gate. Conservative false metadata
        // needs no extra reader/protection scan and cannot conceal an error.
        let protected = if validate_protection {
            Some(protection(&mut index)?)
        } else if reuse {
            protection(&mut index).ok()
        } else {
            None
        };
        let boundary = protected
            .as_ref()
            .filter(|_| reuse)
            .map_or(DeclarationReuseBoundary::Pending, |ranges| {
                declaration_reuse_boundary_with_ranges(source, config, ranges)
            });
        if let Some(index) = index {
            index.protection = protected.map(snapshot);
        }
        return Ok((config.clone(), false, boundary));
    }
    if let Some(index) = index.as_mut() {
        index.last_protection_scan_bytes = if protection_incremental {
            0
        } else {
            source.len()
        };
    }
    let protected = protection(&mut index)?;
    let boundary = if reuse {
        declaration_reuse_boundary_with_ranges(source, config, &protected)
    } else {
        DeclarationReuseBoundary::Pending
    };
    let initial_append_safe = continuation
        && declaration_reuse_config_supported(config)
        && (source.is_empty() || source.ends_with('\n'));
    let incremental = index.as_ref().is_some_and(|index| {
        was_reusable
            && previous.is_some()
            && index.source_len < source.len()
            && declaration_reuse_config_supported(config)
            && !has_reader_events(&config.syntax)
            && (protection_incremental
                || protected
                    .iter()
                    .take_while(|range| range.start < index.source_len)
                    .eq(previous.as_ref().unwrap().ranges.iter()))
    });
    let mut cursor = if incremental {
        index.as_ref().unwrap().source_len
    } else {
        0
    };
    let cached_output = if incremental {
        index.as_mut().unwrap().output.take()
    } else {
        None
    };
    let state = if incremental {
        std::mem::take(&mut index.as_mut().unwrap().state)
    } else {
        DeclarationScanState {
            append_safe: initial_append_safe,
            ..Default::default()
        }
    };
    if let Some(index) = index.as_mut() {
        index.reusable = false;
        index.last_scan_bytes = source.len() - cursor;
        index.last_incremental = incremental;
    }
    let DeclarationScanState {
        mut selected,
        mut ambiguous,
        mut depth,
        mut conditional_depth,
        mut environment_depth,
        mut readable_factory_end,
        mut continuation_conditions,
        mut continuation_operand_end,
        mut continuation_groups,
        mut append_safe,
    } = state;
    let mut declarations_changed = !incremental;
    let mut reader_complete = source.is_empty() || source.ends_with('\n');
    let reader_start = cursor;
    let mut reader_positions =
        crate::source_model::SourceModel::reader_positions(&source[reader_start..])
            .map(|at| at + reader_start)
            .peekable();
    let mut protected_index = 0usize;
    while cursor < source.len() {
        while protected
            .get(protected_index)
            .is_some_and(|range| range.end <= cursor)
        {
            protected_index += 1;
        }
        let range = protected
            .get(protected_index)
            .filter(|range| range.start <= cursor);
        if let Some(range) = range.filter(|range| range.start == cursor)
            && (context_factory_range(source, range) || token_data_range(source, range))
        {
            readable_factory_end = Some(range.end);
        }
        if let Some(range) = range.filter(|range| readable_factory_end != Some(range.end)) {
            cursor = range.end;
            continue;
        }
        if source.as_bytes()[cursor] == b'%' {
            cursor = line_end(source, cursor);
            continue;
        }
        if let Some((command, end)) = command_at(source, cursor) {
            let base = command.trim_end_matches('*');
            let complete = !(indexed || append_safe)
                || declaration_reader_complete(source, command, cursor, end);
            if indexed {
                reader_complete &= complete
                    && !(matches!(base, "string" | "noexpand")
                        && skip_trivia(source, end) == source.len())
                    && !(inferred_condition(base, source, end)
                        && !primitive_condition(base)
                        && skip_trivia(source, end) == source.len());
            }
            append_safe &= complete;
            if append_safe && cursor >= continuation_operand_end {
                let constructed = (base == "csname")
                    .then(|| literal_csname(source, cursor))
                    .flatten();
                let (executed, executed_end) = constructed
                    .as_ref()
                    .map_or((base, end), |(name, finish)| (name.as_str(), *finish));
                if base == "newif" {
                    continuation_operand_end = command_at(source, skip_trivia(source, end))
                        .map_or(end, |(_, finish)| finish);
                } else if inferred_condition(executed, source, executed_end) {
                    continuation_conditions.push((executed == "ifcase", false));
                    continuation_operand_end = primitive_test_end(source, command, end)
                        .or_else(|| next_character_test_end(source, command, end))
                        .unwrap_or(executed_end);
                } else if executed == "fi" {
                    if continuation_conditions.pop().is_none() {
                        if let Some(unknown_branch) = continuation_groups.last_mut() {
                            // Library-specific condition names can occur in
                            // delayed argument data. Require their surrounding
                            // ordinary brace group to close in this scan.
                            *unknown_branch = true;
                        } else {
                            append_safe = false;
                        }
                    }
                } else if matches!(executed, "else" | "or") {
                    if let Some((case, had_else)) = continuation_conditions.last_mut() {
                        append_safe = !*had_else && (executed != "or" || *case);
                        *had_else = executed == "else";
                    } else if let Some(unknown_branch) = continuation_groups.last_mut() {
                        *unknown_branch = true;
                    } else {
                        append_safe = false;
                    }
                } else if let Some(finish) = next_character_test_end(source, command, end) {
                    continuation_operand_end = finish;
                }
            }
            if let Some(finish) = primitive_data_end(source, command, end) {
                cursor = finish;
                continue;
            }
            if matches!(base, "begingroup" | "bgroup") {
                depth += 1;
            } else if matches!(base, "endgroup" | "egroup") {
                depth = depth.saturating_sub(1);
            }
            if is_condition_command(base, config) {
                conditional_depth += 1;
            } else if base == "fi" {
                conditional_depth = conditional_depth.saturating_sub(1);
            }
            if let Some((name, begin, finish)) = env_at(source, cursor) {
                if name != "document" {
                    if begin {
                        environment_depth += 1;
                    } else {
                        environment_depth = environment_depth.saturating_sub(1);
                    }
                }
                cursor = finish;
                continue;
            }
            if base == "string" || base == "noexpand" {
                let next = skip_trivia(source, end);
                cursor = command_at(source, next).map_or_else(
                    || next + source[next..].chars().next().map_or(0, char::len_utf8),
                    |(_, end)| end,
                );
                continue;
            }
            let at_top_level = depth == 0 && conditional_depth == 0 && environment_depth == 0;
            let in_top_level_condition =
                depth == 0 && conditional_depth > 0 && environment_depth == 0;
            let mut declarations = Vec::new();
            let mut finish = end;
            if let Some(declaration) = source_environment_declaration(source, cursor, command, end)
            {
                finish = declaration.span.end;
                declarations.push(ContextDeclaration {
                    name: declaration.name,
                    environment: true,
                    span: declaration.span,
                    arguments: declaration.signature,
                    raw: false,
                });
            } else if latex_macro_definition(base) {
                if let Some(declaration) =
                    context_macro_declaration_with_config(source, cursor, base, end, Some(config))
                {
                    finish = declaration.span.end;
                    declarations.push(declaration);
                }
            } else if matches!(base, "def" | "gdef" | "edef" | "xdef" | "let" | "futurelet") {
                if let Some(declaration) = context_primitive_declaration(source, cursor, base, end)
                {
                    finish = declaration.span.end;
                    if selected.contains_key(&(true, declaration.name.clone())) {
                        let mut environment = declaration.clone();
                        environment.environment = true;
                        declarations.push(environment);
                    }
                    if let Some(name) = declaration.name.strip_prefix("end")
                        && selected.contains_key(&(true, name.to_owned()))
                    {
                        let mut environment = declaration.clone();
                        environment.environment = true;
                        environment.name = name.to_owned();
                        declarations.push(environment);
                    }
                    declarations.push(declaration);
                }
            } else if (at_top_level || in_top_level_condition)
                && context_declaration_command(base)
                && !config.syntax.commands.contains_key(base)
                && !selected.contains_key(&(false, base.to_owned()))
                && !ambiguous.contains(&(false, base.to_owned()))
            {
                if base == "newtheorem" {
                    if let Some(declaration) =
                        context_theorem_declaration(source, cursor, command, end)
                    {
                        finish = declaration.span.end;
                        declarations.push(declaration);
                    }
                } else if let Some(declaration) = context_box_declaration(source, cursor, base, end)
                {
                    finish = declaration.span.end;
                    declarations.push(declaration);
                } else if let Some(declaration) =
                    crate::package_environments::verbatim_declaration(source, base, end)
                {
                    finish = declaration.end;
                    if declaration.terminator
                        == crate::package_environments::VerbatimTerminator::AnyPosition
                    {
                        for name in declaration.names {
                            declarations.push(ContextDeclaration {
                                name,
                                environment: true,
                                span: cursor..finish,
                                arguments: None,
                                raw: true,
                            });
                        }
                    }
                } else if base == "newlist" {
                    let mut after = end;
                    let parsed = (|| {
                        let name = context_literal_name(
                            &source[context_argument(source, &mut after, b'{', b'}')?],
                        )?;
                        context_literal_name(
                            &source[context_argument(source, &mut after, b'{', b'}')?],
                        )?;
                        source[context_argument(source, &mut after, b'{', b'}')?]
                            .trim()
                            .parse::<usize>()
                            .ok()?;
                        Some(name)
                    })();
                    if let Some(name) = parsed {
                        finish = after;
                        declarations.push(ContextDeclaration {
                            name,
                            environment: true,
                            span: cursor..finish,
                            arguments: Some(crate::config::EnvironmentArguments {
                                signature: Some("o".into()),
                                ..Default::default()
                            }),
                            raw: false,
                        });
                    }
                }
            }
            if append_safe && finish == end && context_declaration_command(command) {
                // Failed inference can mean an unsupported complete signature,
                // or a declaration waiting for a later segment. Only a fully
                // read declaration is a stable append boundary.
                append_safe = if latex_macro_definition(base)
                    || matches!(base, "def" | "gdef" | "edef" | "xdef")
                {
                    macro_body(source, base, end).is_some()
                } else if matches!(base, "let" | "futurelet") {
                    let_declaration_end(source, base, end).is_some()
                } else if base == "newtheorem" {
                    context_theorem_declaration(source, cursor, command, end).is_some()
                } else if base == "newlist" {
                    let mut after = end;
                    (0..3).all(|_| context_argument(source, &mut after, b'{', b'}').is_some())
                } else {
                    context_box_declaration(source, cursor, base, end).is_some()
                        || crate::package_environments::verbatim_declaration(source, base, end)
                            .is_some()
                };
            }
            if (at_top_level || in_top_level_condition) && !declarations.is_empty() {
                declarations_changed = true;
            }
            if indexed && finish == end && context_declaration_command(command) {
                reader_complete &=
                    declaration_context_reader_complete(source, command, cursor, end, complete);
            }
            if at_top_level {
                let provide = base.starts_with("provide") || base.starts_with("Provide");
                for declaration in declarations {
                    let key = (declaration.environment, declaration.name.clone());
                    let configured = if declaration.environment {
                        config
                            .syntax
                            .environment_arguments
                            .contains_key(&declaration.name)
                            || config.protect.environments.contains(&declaration.name)
                            || config.syntax.list_environments.contains(&declaration.name)
                            || crate::layout::environment_signature(&declaration.name).is_some()
                    } else {
                        config.syntax.commands.contains_key(&declaration.name)
                            || config.syntax.headings.contains_key(&declaration.name)
                            || config.protect.commands.contains(&declaration.name)
                            || context_builtin_command(&declaration.name)
                    };
                    if !provide
                        || !configured && !selected.contains_key(&key) && !ambiguous.contains(&key)
                    {
                        ambiguous.remove(&key);
                        selected.insert(key, declaration);
                    }
                }
            } else if in_top_level_condition {
                // An unevaluated branch can replace prior syntax. Dropping that
                // inference keeps later calls conservative without executing TeX.
                for declaration in declarations {
                    let key = (declaration.environment, declaration.name);
                    selected.remove(&key);
                    ambiguous.insert(key);
                }
            }
            cursor = finish;
            continue;
        }
        match source.as_bytes()[cursor] {
            b'{' => {
                depth += 1;
                if append_safe {
                    continuation_groups.push(false);
                }
            }
            b'}' => {
                depth = depth.saturating_sub(1);
                if append_safe {
                    continuation_groups.pop();
                }
            }
            b'\\' => {
                append_safe = false;
                reader_complete = false;
            }
            _ => (),
        }
        cursor += source[cursor..]
            .chars()
            .next()
            .expect("valid position")
            .len_utf8();
        // Declaration state changes only at these delimiters. A forward range
        // cursor bounds the byte search so protected text still participates
        // in exactly the same declaration/reader decisions.
        let next_range = protected[protected_index..]
            .iter()
            .find(|range| range.start >= cursor)
            .map_or(source.len(), |range| range.start);
        while reader_positions.next_if(|&at| at < cursor).is_some() {}
        cursor = reader_positions
            .peek()
            .map_or(next_range, |&at| at.min(next_range));
    }
    let independent_append_safe = initial_append_safe
        && append_safe
        && protected
            .last()
            .is_none_or(|range| range.end != source.len())
        && conditional_depth == 0
        && continuation_conditions.is_empty()
        && !continuation_groups
            .iter()
            .any(|unknown_branch| *unknown_branch);
    let cached_output = cached_output.filter(|_| !declarations_changed);
    let mut output = if let Some(cached) = &cached_output {
        cached.apply(config)
    } else {
        declaration_config_from_state(source, config, &selected, &ambiguous)?
    };
    let file_options = matches!(exports, Some(ExportRequest::FileOptions));
    let reuse_base_aliases = incremental
        && protection_incremental
        && index.as_ref().is_some_and(|index| index.aliases_match_base);
    if !reuse_base_aliases {
        apply_declaration_aliases(source, config, &protected, indexed, exports, &mut output);
    }
    if let Some(index) = index {
        index.source_len = source.len();
        index.protection = Some(snapshot(protected));
        index.state = DeclarationScanState {
            selected,
            ambiguous,
            depth,
            conditional_depth,
            environment_depth,
            readable_factory_end,
            continuation_conditions,
            continuation_operand_end,
            continuation_groups,
            append_safe,
        };
        index.aliases_match_base = !file_options
            && output.syntax.inferred_environment_aliases
                == config.syntax.inferred_environment_aliases;
        index.output = cached_output;
        index
            .output
            .get_or_insert_with(|| Arc::new(InferredSettings::between(config, &output)));
        index.reusable = reader_complete;
    }
    Ok((output, independent_append_safe, boundary))
}

fn declaration_config_from_state(
    source: &str,
    config: &Config,
    selected: &HashMap<(bool, String), ContextDeclaration>,
    ambiguous: &HashSet<(bool, String)>,
) -> Result<Config, FormatError> {
    let mut declarations: Vec<_> = selected.values().collect();
    declarations.sort_by(|left, right| {
        (left.span.start, left.environment, &left.name).cmp(&(
            right.span.start,
            right.environment,
            &right.name,
        ))
    });
    // Signature configuration does not imply list membership or raw safety.
    // Keep declaration bodies when either property still needs inference, and
    // avoid copying/reparsing a command-only context or fully covered targets.
    let infer_lists = config.syntax.infer_list_environments
        && declarations.iter().any(|declaration| {
            declaration.environment && !config.syntax.list_environments.contains(&declaration.name)
        });
    let infer_raw = declarations.iter().any(|declaration| {
        declaration.environment
            && !declaration.raw
            && !config.protect.environments.contains(&declaration.name)
    });
    let need_declaration_source = infer_lists || infer_raw;
    let mut declaration_source = String::new();
    let mut last_span = None;
    let mut output = config.clone();
    for declaration in &declarations {
        if need_declaration_source && last_span.as_ref() != Some(&declaration.span) {
            declaration_source.push_str(&source[declaration.span.clone()]);
            declaration_source.push('\n');
            last_span = Some(declaration.span.clone());
        }
        if declaration.environment {
            if declaration.raw && !output.protect.environments.contains(&declaration.name) {
                output.protect.environments.push(declaration.name.clone());
            }
            if let Some(arguments) = &declaration.arguments {
                if !config
                    .syntax
                    .environment_arguments
                    .contains_key(&declaration.name)
                    || config
                        .syntax
                        .inferred_environment_arguments
                        .contains_key(&declaration.name)
                {
                    output
                        .syntax
                        .inferred_environment_arguments
                        .insert(declaration.name.clone(), arguments.clone());
                }
                output
                    .syntax
                    .environment_arguments
                    .entry(declaration.name.clone())
                    .or_insert_with(|| arguments.clone());
            }
        } else if declaration.name != "pie"
            && !output.syntax.commands.contains_key(&declaration.name)
        {
            // TikZ installs its picture-local pie alias on every picture entry.
            // A source binding outside that scope cannot become a global
            // formatting rule. Explicit user syntax is retained in the clone.
            let rule = declaration
                .arguments
                .as_ref()
                .and_then(|arguments| {
                    let signature = arguments.signature.as_deref()?;
                    let required = signature.strip_prefix('o').unwrap_or(signature);
                    Some(if required.bytes().all(|slot| slot == b'm') {
                        crate::config::CommandRule {
                            arguments: Some(required.len()),
                            optional_arguments: signature.starts_with('o'),
                            ..Default::default()
                        }
                    } else {
                        crate::config::CommandRule {
                            signature: Some(signature.to_owned()),
                            ..Default::default()
                        }
                    })
                })
                .or_else(|| {
                    context_builtin_command(&declaration.name).then(|| crate::config::CommandRule {
                        // The original built-in grammar is unavailable. This generic
                        // rule retains ordinary argument grouping and disables package
                        // or project-loader assumptions about the overwritten command.
                        optional_arguments: true,
                        ..Default::default()
                    })
                });
            if let Some(rule) = rule {
                output
                    .syntax
                    .commands
                    .entry(declaration.name.clone())
                    .or_insert(rule);
            }
        }
    }
    for (environment, name) in ambiguous {
        if !*environment && context_builtin_command(name) {
            output
                .syntax
                .commands
                .entry(name.clone())
                .or_insert(crate::config::CommandRule {
                    optional_arguments: true,
                    ..Default::default()
                });
        }
    }
    if infer_lists {
        output
            .syntax
            .list_environments
            .extend(infer_list_environments_with_config(
                &declaration_source,
                &config.syntax.list_environments,
                config,
            ));
    }
    if infer_raw {
        let declaration_ranges = protected_ranges(&declaration_source, &output)?;
        for name in
            inferred_protected_environments(&declaration_source, &output, &declaration_ranges)
        {
            if !output.protect.environments.contains(&name) {
                output.protect.environments.push(name);
            }
        }
    }
    Ok(output)
}

fn apply_declaration_aliases(
    source: &str,
    config: &Config,
    ranges: &[Range<usize>],
    indexed: bool,
    exports: Option<ExportRequest<'_>>,
    output: &mut Config,
) {
    // The same source-ordered binding index supplies aliases to imported files.
    // Explicit configuration wins over source declarations.
    let aliases = match exports {
        Some(ExportRequest::FileOptions) => return,
        Some(ExportRequest::All(index, snapshot)) => {
            let exports = index.snapshot(source, config, Some(ranges));
            for (name, alias) in &exports.0 {
                if let Some(alias) = alias {
                    output
                        .syntax
                        .inferred_environment_aliases
                        .entry(name.clone())
                        .or_insert_with(|| alias.clone());
                }
            }
            *snapshot = Some(exports);
            return;
        }
        Some(ExportRequest::Aliases(index)) => index.aliases(source, config, ranges),
        None if indexed => {
            crate::source_model::SourceModel::indexed_aliases_with_ranges(source, config, ranges)
        }
        None => crate::source_model::SourceModel::aliases_with_ranges(source, config, ranges),
    };
    for (name, alias) in aliases {
        output
            .syntax
            .inferred_environment_aliases
            .entry(name)
            .or_insert(alias);
    }
}

fn declaration_context_reader_complete(
    source: &str,
    command: &str,
    cursor: usize,
    end: usize,
    complete: bool,
) -> bool {
    if source_environment_declaration(source, cursor, command, end).is_some() {
        return true;
    }
    let base = command.trim_end_matches('*');
    if latex_macro_definition(base) || matches!(base, "def" | "gdef" | "edef" | "xdef") {
        macro_body(source, base, end).is_some()
            || unsupported_macro_reader_settled(source, base, end)
    } else if matches!(base, "let" | "futurelet") {
        complete
    } else if base == "newtheorem" {
        context_theorem_declaration(source, cursor, command, end).is_some()
            || unsupported_braced_reader_settled(source, end, 2)
    } else if base == "newlist" {
        let mut after = end;
        (0..3).all(|_| context_argument(source, &mut after, b'{', b'}').is_some())
    } else {
        context_box_declaration(source, cursor, base, end).is_some()
            || crate::package_environments::verbatim_declaration(source, base, end).is_some()
    }
}

// Optional brackets may occur between the required groups. A closed but
// unsupported slot is stable; a missing or unclosed slot needs more source.
fn unsupported_braced_reader_settled(source: &str, end: usize, groups: usize) -> bool {
    let mut cursor = skip_trivia(source, end);
    for _ in 0..groups {
        while source.as_bytes().get(cursor) == Some(&b'[') {
            let Some(end) = balanced_end(source, cursor, b'[', b']') else {
                return false;
            };
            cursor = skip_trivia(source, end);
        }
        let Some(&token) = source.as_bytes().get(cursor) else {
            return false;
        };
        if token != b'{' {
            return true;
        }
        let Some(end) = balanced_end(source, cursor, b'{', b'}') else {
            return false;
        };
        cursor = skip_trivia(source, end);
    }
    while source.as_bytes().get(cursor) == Some(&b'[') {
        let Some(end) = balanced_end(source, cursor, b'[', b']') else {
            return false;
        };
        cursor = skip_trivia(source, end);
    }
    true
}

// A non-group token in a required braced slot makes the curated declaration
// permanently unsupported. Appending later text cannot repair that slot.
// Missing or unclosed operands retain complete-history fallback.
fn unsupported_macro_reader_settled(source: &str, name: &str, end: usize) -> bool {
    if matches!(name, "def" | "gdef" | "edef" | "xdef") {
        return false;
    }
    let mut cursor = skip_trivia(source, end);
    let Some(&token) = source.as_bytes().get(cursor) else {
        return false;
    };
    cursor = if token == b'{' {
        let Some(end) = balanced_end(source, cursor, b'{', b'}') else {
            return false;
        };
        end
    } else if token == b'\\' {
        let Some((_, end)) = command_at(source, cursor) else {
            return false;
        };
        end
    } else {
        return true;
    };
    cursor = skip_trivia(source, cursor);
    while source.as_bytes().get(cursor) == Some(&b'[') {
        let Some(end) = balanced_end(source, cursor, b'[', b']') else {
            return false;
        };
        cursor = skip_trivia(source, end);
    }
    let groups = 1
        + usize::from(name.contains("Document"))
        + usize::from(name.to_ascii_lowercase().contains("environment"));
    for _ in 0..groups {
        let Some(&token) = source.as_bytes().get(cursor) else {
            return false;
        };
        if token != b'{' {
            return true;
        }
        let Some(end) = balanced_end(source, cursor, b'{', b'}') else {
            return false;
        };
        cursor = skip_trivia(source, end);
    }
    true
}

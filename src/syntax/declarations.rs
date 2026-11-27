//! Persistent declaration state and checkpoint inference for project preparation.

use super::{
    CompiledSyntax, ContextDeclaration, DeclarationReuseBoundary, balanced_end, command_at,
    compile, context_argument, context_box_declaration, context_builtin_command,
    context_declaration_command, context_factory_range, context_literal_name,
    context_macro_declaration_with_config, context_primitive_declaration,
    context_theorem_declaration, declaration_protection_validation_needed,
    declaration_reader_complete, declaration_reuse_boundary_with_ranges,
    declaration_reuse_config_supported, env_at, has_context_declaration,
    infer_list_environments_with_config, inferred_condition, inferred_protected_environments,
    is_condition_command, latex_macro_definition, let_declaration_end, line_end, literal_csname,
    macro_body, next_character_test_end, primitive_condition, primitive_data_end,
    primitive_test_end, protected_ranges, protected_ranges_with_syntax, skip_trivia,
    source_environment_declaration, token_data_range,
};

use crate::{Config, FormatError};
use std::{
    collections::{HashMap, HashSet},
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
    infer_declaration_config_impl(source, config, false, false, None).map(|(config, _, _)| config)
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
    infer_declaration_config_impl(source, config, true, false, None)
        .map(|(config, safe, _)| (config, safe))
}

/// Share protection analysis between declaration inference and its reuse gate.
#[doc(hidden)]
pub fn infer_declaration_config_with_reuse(
    source: &str,
    config: &Config,
) -> Result<(Config, bool, DeclarationReuseBoundary), FormatError> {
    infer_declaration_config_impl(source, config, true, true, None)
}

/// Append-only declaration analysis for one immutable base configuration.
#[doc(hidden)]
#[derive(Clone, Default)]
pub struct DeclarationIndex {
    compiled: Option<Arc<CompiledSyntax>>,
    source_len: usize,
    protected: Vec<Range<usize>>,
    state: DeclarationScanState,
    output: Option<Config>,
    reusable: bool,
    last_scan_bytes: usize,
    last_incremental: bool,
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
    pub fn reader_settled(&self) -> bool {
        self.reusable
    }

    #[doc(hidden)]
    pub fn scan_work(&self) -> (usize, bool) {
        (self.last_scan_bytes, self.last_incremental)
    }
}

/// The source must retain its existing prefix and use the same base settings.
#[doc(hidden)]
pub fn infer_declaration_config_indexed(
    source: &str,
    config: &Config,
    index: &mut DeclarationIndex,
) -> Result<(Config, bool, DeclarationReuseBoundary), FormatError> {
    infer_declaration_config_impl(source, config, true, true, Some(index))
}

fn infer_declaration_config_impl(
    source: &str,
    config: &Config,
    continuation: bool,
    reuse: bool,
    mut index: Option<&mut DeclarationIndex>,
) -> Result<(Config, bool, DeclarationReuseBoundary), FormatError> {
    let compiled = if let Some(index) = index.as_mut() {
        if index.compiled.is_none() && memchr::memchr2(b'\\', b'%', source.as_bytes()).is_some() {
            index.compiled = compile(&config.syntax).ok().map(Arc::new);
        }
        index.compiled.clone()
    } else {
        None
    };
    let protection = |source: &str| {
        compiled.as_ref().map_or_else(
            || protected_ranges(source, config),
            |compiled| protected_ranges_with_syntax(source, config, compiled),
        )
    };
    let has_declaration = has_context_declaration(source);
    let validate_protection =
        has_declaration || declaration_protection_validation_needed(source, config);
    if !has_declaration {
        if let Some(index) = index.as_mut() {
            index.last_scan_bytes = 0;
            index.last_incremental = false;
            index.reusable = false;
        }
        // The original no-declaration inference only validates protection
        // when requested by its established gate. Conservative false metadata
        // needs no extra reader/protection scan and cannot conceal an error.
        let protected = if validate_protection {
            Some(protection(source)?)
        } else if reuse {
            protection(source).ok()
        } else {
            None
        };
        let boundary = protected
            .as_ref()
            .filter(|_| reuse)
            .map_or(DeclarationReuseBoundary::Pending, |ranges| {
                declaration_reuse_boundary_with_ranges(source, config, ranges)
            });
        return Ok((config.clone(), false, boundary));
    }
    let protected = protection(source)?;
    let boundary = if reuse {
        declaration_reuse_boundary_with_ranges(source, config, &protected)
    } else {
        DeclarationReuseBoundary::Pending
    };
    let initial_append_safe = continuation
        && declaration_reuse_config_supported(config)
        && (source.is_empty() || source.ends_with('\n'));
    let indexed = index.is_some();
    let incremental = index.as_ref().is_some_and(|index| {
        index.reusable
            && index.source_len <= source.len()
            && declaration_reuse_config_supported(config)
            && protected
                .iter()
                .take_while(|range| range.start < index.source_len)
                .eq(index.protected.iter())
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
            if indexed {
                let complete = declaration_reader_complete(source, command, cursor, end)
                    && !(matches!(base, "string" | "noexpand")
                        && skip_trivia(source, end) == source.len())
                    && !(inferred_condition(base, source, end)
                        && !primitive_condition(base)
                        && skip_trivia(source, end) == source.len());
                reader_complete &= complete;
            }
            if append_safe && !declaration_reader_complete(source, command, cursor, end) {
                append_safe = false;
            }
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
                let complete = declaration_context_reader_complete(source, command, cursor, end);
                reader_complete &= complete;
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
    let independent_append_safe = append_safe
        && protected
            .last()
            .is_none_or(|range| range.end != source.len())
        && conditional_depth == 0
        && continuation_conditions.is_empty()
        && !continuation_groups
            .iter()
            .any(|unknown_branch| *unknown_branch);
    let output = if !declarations_changed && let Some(output) = cached_output {
        output
    } else {
        declaration_config_from_state(source, config, &selected, &ambiguous, &protected)?
    };
    if let Some(index) = index {
        index.source_len = source.len();
        index.protected = protected;
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
        index.output = Some(output.clone());
        index.reusable = reader_complete;
    }
    Ok((output, independent_append_safe, boundary))
}

fn declaration_config_from_state(
    source: &str,
    config: &Config,
    selected: &HashMap<(bool, String), ContextDeclaration>,
    ambiguous: &HashSet<(bool, String)>,
    ranges: &[Range<usize>],
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
    // The same source-ordered binding index supplies aliases to imported files.
    // Explicit configuration wins over source declarations.
    for (name, alias) in
        crate::source_model::SourceModel::aliases_with_ranges(source, config, ranges)
    {
        output
            .syntax
            .inferred_environment_aliases
            .entry(name)
            .or_insert(alias);
    }
    Ok(output)
}

fn declaration_context_reader_complete(
    source: &str,
    command: &str,
    cursor: usize,
    end: usize,
) -> bool {
    if source_environment_declaration(source, cursor, command, end).is_some() {
        return true;
    }
    let base = command.trim_end_matches('*');
    if latex_macro_definition(base) || matches!(base, "def" | "gdef" | "edef" | "xdef") {
        macro_body(source, base, end).is_some()
            || unsupported_macro_reader_settled(source, base, end)
    } else if matches!(base, "let" | "futurelet") {
        declaration_reader_complete(source, command, cursor, end)
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

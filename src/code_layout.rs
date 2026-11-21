//! Dedicated layout for algorithm2e and expl3 source.

use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::Range;

pub(crate) mod expl3;
mod printer;
mod scanner;

use crate::{CompiledConfig, FileKind, FormatError, syntax};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct CodeConfig {
    pub algorithm2e: bool,
    pub expl3: bool,
}

impl Default for CodeConfig {
    fn default() -> Self {
        Self {
            algorithm2e: true,
            expl3: true,
        }
    }
}

pub(crate) fn is_active(source: &str, config: &CompiledConfig) -> bool {
    config.config.code.algorithm2e
        && source.contains("\\begin")
        && (source.contains("algorithm") || source.contains("algocf"))
        || config.config.code.expl3 && has_expl3_marker(source)
}

pub(crate) fn has_expl3_marker(source: &str) -> bool {
    scanner::commands(source, &[])
        .into_iter()
        .any(|(_, name, _)| {
            matches!(
                name,
                "ExplSyntaxOn" | "ProvidesExplPackage" | "ProvidesExplClass" | "ProvidesExplFile"
            )
        })
}

fn overlap(left: &Range<usize>, right: &Range<usize>) -> bool {
    left.start < right.end && right.start < left.end
}

fn overlapping_ranges<'a>(ranges: &'a [Range<usize>], range: &Range<usize>) -> &'a [Range<usize>] {
    let first = ranges.partition_point(|entry| entry.end <= range.start);
    let count = ranges[first..].partition_point(|entry| entry.start < range.end);
    &ranges[first..first + count]
}

pub(crate) fn merged(mut ranges: Vec<Range<usize>>) -> Vec<Range<usize>> {
    ranges.sort_by_key(|range| (range.start, std::cmp::Reverse(range.end)));
    let mut result: Vec<Range<usize>> = Vec::new();
    for range in ranges.into_iter().filter(|range| !range.is_empty()) {
        if let Some(previous) = result.last_mut()
            && range.start <= previous.end
        {
            previous.end = previous.end.max(range.end);
        } else {
            result.push(range);
        }
    }
    result
}

/// Capture complete formulas and curated math-domain commands as opaque source.
pub(crate) fn math_ranges(source: &str, protected: &[Range<usize>]) -> Vec<Range<usize>> {
    scanner::math_ranges(source, protected)
}

fn mathematical_command(name: &str) -> bool {
    crate::packages::math::command(name).is_some_and(|signature| signature.domains.contains('M'))
        || matches!(
            name,
            "ensuremath"
                | "frac"
                | "dfrac"
                | "tfrac"
                | "binom"
                | "dbinom"
                | "tbinom"
                | "overset"
                | "underset"
                | "stackrel"
                | "sqrt"
                | "mathchoice"
                | "boxed"
                | "prescript"
                | "tensor"
                | "sideset"
                | "mathrel"
                | "mathbin"
                | "mathord"
                | "mathop"
                | "mathopen"
                | "mathclose"
                | "mathpunct"
                | "mathinner"
                | "smash"
                | "vphantom"
                | "hphantom"
                | "phantom"
        )
}

pub(crate) fn algorithm_environment(name: &str) -> bool {
    matches!(
        name.trim_end_matches('*'),
        "algorithm" | "algorithm2e" | "algocf"
    )
}

pub(crate) fn expl3_regions(source: &str, protected: &[Range<usize>]) -> Vec<Range<usize>> {
    scanner::expl3_regions(source, protected)
}

fn math_environment(name: &str) -> bool {
    crate::packages::math::is_math_environment(name)
        || matches!(
            name.trim_end_matches('*'),
            "math"
                | "displaymath"
                | "equation"
                | "align"
                | "alignat"
                | "aligned"
                | "alignedat"
                | "flalign"
                | "gather"
                | "gathered"
                | "multline"
                | "multlined"
                | "split"
                | "eqnarray"
                | "array"
                | "subarray"
                | "cases"
                | "dcases"
                | "rcases"
                | "drcases"
                | "matrix"
                | "pmatrix"
                | "bmatrix"
                | "Bmatrix"
                | "vmatrix"
                | "Vmatrix"
                | "smallmatrix"
                | "NiceMatrix"
                | "pNiceMatrix"
                | "bNiceMatrix"
                | "BNiceMatrix"
                | "vNiceMatrix"
                | "VNiceMatrix"
                | "NiceArray"
                | "pNiceArray"
                | "bNiceArray"
                | "BNiceArray"
                | "vNiceArray"
                | "VNiceArray"
                | "subequations"
                | "empheq"
                | "IEEEeqnarray"
                | "IEEEeqnarraybox"
        )
}

pub(crate) struct Opaque {
    source: String,
    marker_prefix: String,
    entries: Vec<(String, String, bool, bool)>,
}

impl Opaque {
    pub(crate) fn new(source: &str, ranges: Vec<Range<usize>>) -> Self {
        let mut prefix = "TiftOpaque".to_owned();
        while source.contains(&prefix) {
            prefix.push('X');
        }
        let marker_prefix = format!("\\{prefix}");
        let mut output = String::with_capacity(source.len());
        let mut entries = Vec::new();
        let mut cursor = 0;
        for (index, range) in merged(ranges).into_iter().enumerate() {
            output.push_str(&source[cursor..range.start]);
            // A control word and empty group form one indivisible, collision-free atom.
            let mut number = index;
            let mut suffix = String::new();
            loop {
                suffix.push(char::from(b'A' + (number % 26) as u8));
                number /= 26;
                if number == 0 {
                    break;
                }
            }
            let original = &source[range.clone()];
            let display = if original.starts_with(r"\[") || original.starts_with("$$") {
                "DISPLAY"
            } else {
                ""
            };
            let marker = format!("{marker_prefix}{display}{suffix}{{}}");
            output.push_str(&marker);
            let line_tail = original.trim_end_matches([' ', '\t']);
            if line_tail.ends_with("\r\n") {
                output.push_str("\r\n");
            } else if line_tail.ends_with('\n') {
                output.push('\n');
            }
            let external_indent = syntax::command_at(original, 0)
                .is_some_and(|(name, _)| mathematical_command(name) || name == "directlua");
            let starts_line =
                !external_indent && (range.start == 0 || source[..range.start].ends_with('\n'));
            let newline_before = source[..range.start].ends_with('\n');
            entries.push((marker, original.to_owned(), starts_line, newline_before));
            cursor = range.end;
        }
        output.push_str(&source[cursor..]);
        Self {
            source: output,
            marker_prefix,
            entries,
        }
    }

    pub(crate) fn source(&self) -> &str {
        &self.source
    }

    pub(crate) fn restore(&self, formatted: &str) -> Option<String> {
        if self.entries.is_empty() {
            return Some(formatted.to_owned());
        }
        let markers = self
            .entries
            .iter()
            .enumerate()
            .map(|(index, (marker, _, _, _))| (marker.as_str(), index))
            .collect::<HashMap<_, _>>();
        let mut seen = vec![false; self.entries.len()];
        let mut positions = Vec::with_capacity(self.entries.len());
        for (position, _) in formatted.match_indices(&self.marker_prefix) {
            let mut end = position + self.marker_prefix.len();
            while formatted
                .as_bytes()
                .get(end)
                .is_some_and(u8::is_ascii_uppercase)
            {
                end += 1;
            }
            let word_end = end;
            while formatted
                .as_bytes()
                .get(end)
                .is_some_and(u8::is_ascii_whitespace)
            {
                end += 1;
            }
            if formatted.as_bytes().get(end) != Some(&b'{') {
                continue;
            }
            end += 1;
            while formatted
                .as_bytes()
                .get(end)
                .is_some_and(u8::is_ascii_whitespace)
            {
                end += 1;
            }
            if formatted.as_bytes().get(end) != Some(&b'}') {
                continue;
            }
            end += 1;
            let key = format!("{}{{}}", &formatted[position..word_end]);
            let Some(&index) = markers.get(key.as_str()) else {
                continue;
            };
            if std::mem::replace(&mut seen[index], true) {
                return None;
            }
            positions.push((position, end, index));
        }
        if seen.contains(&false) {
            return None;
        }
        // A printer normally preserves marker order. Retain the original
        // restoration order for a reordered result, whose inserted line ends
        // can affect another marker's indentation boundary.
        if positions.windows(2).any(|pair| pair[0].2 >= pair[1].2) {
            let mut normalized = formatted.to_owned();
            for &(start, end, index) in positions.iter().rev() {
                normalized.replace_range(start..end, &self.entries[index].0);
            }
            return self.restore_reordered(&normalized);
        }
        let mut output = String::with_capacity(formatted.len());
        let mut cursor = 0;
        let mut line_start = 0;
        for (position, mut end, index) in positions {
            let (_, original, starts_line, newline_before) = &self.entries[index];
            append_tracking_line(&mut output, &formatted[cursor..position], &mut line_start);
            if *starts_line
                && output[line_start..]
                    .bytes()
                    .all(|byte| matches!(byte, b' ' | b'\t'))
            {
                output.truncate(line_start);
            }
            if original.trim_end_matches([' ', '\t']).ends_with('\n') {
                if formatted[end..].starts_with("\r\n") {
                    end += 2;
                } else if formatted[end..].starts_with('\n') {
                    end += 1;
                }
                if !original.ends_with('\n') {
                    while matches!(formatted.as_bytes().get(end), Some(b' ' | b'\t')) {
                        end += 1;
                    }
                }
            }
            // A marker that starts after a printer newline can represent a
            // range whose original bytes start with that same newline. Keep
            // one boundary so repeated formatting does not grow blank lines.
            let original = if original.starts_with("\r\n")
                && !newline_before
                && line_start > 0
                && output[line_start..]
                    .bytes()
                    .all(|byte| byte == b' ' || byte == b'\t')
            {
                output.truncate(line_start);
                &original[2..]
            } else if original.starts_with('\n')
                && !newline_before
                && line_start > 0
                && output[line_start..]
                    .bytes()
                    .all(|byte| byte == b' ' || byte == b'\t')
            {
                output.truncate(line_start);
                &original[1..]
            } else {
                original
            };
            append_tracking_line(&mut output, original, &mut line_start);
            cursor = end;
        }
        output.push_str(&formatted[cursor..]);
        Some(output)
    }

    fn restore_reordered(&self, formatted: &str) -> Option<String> {
        let mut output = formatted.to_owned();
        for (marker, original, starts_line, _) in &self.entries {
            let position = output.find(marker)?;
            let mut start = position;
            if *starts_line {
                let line_start = output[..position]
                    .rfind('\n')
                    .map_or(0, |position| position + 1);
                if output[line_start..position]
                    .bytes()
                    .all(|byte| matches!(byte, b' ' | b'\t'))
                {
                    start = line_start;
                }
            }
            let mut end = position + marker.len();
            if original.trim_end_matches([' ', '\t']).ends_with('\n') {
                if output[end..].starts_with("\r\n") {
                    end += 2;
                } else if output[end..].starts_with('\n') {
                    end += 1;
                }
                if !original.ends_with('\n') {
                    while matches!(output.as_bytes().get(end), Some(b' ' | b'\t')) {
                        end += 1;
                    }
                }
            }
            output.replace_range(start..end, original);
        }
        Some(output)
    }
}

fn append_tracking_line(output: &mut String, text: &str, line_start: &mut usize) {
    if let Some(newline) = text.rfind('\n') {
        *line_start = output.len() + newline + 1;
    }
    output.push_str(text);
}

pub(crate) fn protected_ranges(
    source: &str,
    config: &CompiledConfig,
    allow_expl3: bool,
) -> Result<Vec<Range<usize>>, FormatError> {
    let mut protection = config.config.clone();
    if allow_expl3 {
        protection.protect.expl3 = false;
        if protection.protect.macro_bodies {
            let mut bodies = Vec::new();
            let mut cursor = 0;
            while cursor < source.len() {
                if source.as_bytes()[cursor] == b'%' {
                    cursor += source[cursor..]
                        .find('\n')
                        .map_or(source.len() - cursor, |offset| offset + 1);
                } else if let Some((name, end)) = syntax::command_at(source, cursor) {
                    if let Some((environment, true, after)) =
                        syntax::env_after_command(source, name, end)
                        && (config
                            .config
                            .protect
                            .environments
                            .iter()
                            .any(|name| name == environment)
                            || config
                                .config
                                .protect
                                .file_contents
                                .iter()
                                .any(|name| name == environment))
                    {
                        let terminator = format!("\\end{{{environment}}}");
                        cursor = source[after..]
                            .find(&terminator)
                            .map_or(source.len(), |offset| after + offset + terminator.len());
                    } else if let Some(body) = syntax::expl3_definition_body(source, name, end) {
                        cursor = body.end;
                        bodies.push(body);
                    } else {
                        cursor = syntax::primitive_data_end(source, name, end)
                            .or_else(|| syntax::delayed_data_end(source, name, end))
                            .unwrap_or(end);
                    }
                } else {
                    cursor += source[cursor..].chars().next().unwrap().len_utf8();
                }
            }
            if !bodies.is_empty() {
                let mut ranges =
                    syntax::protected_ranges_for_expl3_outer(source, &protection, &config.syntax)?;
                ranges.retain(|range| !bodies.contains(range));
                for body in bodies {
                    let interior = body.start + 1..body.end - 1;
                    match protected_ranges(&source[interior.clone()], config, true) {
                        Ok(local) => {
                            ranges.extend(local.into_iter().map(|range| {
                                interior.start + range.start..interior.start + range.end
                            }))
                        }
                        Err(_) => ranges.push(body),
                    }
                }
                return Ok(merged(ranges));
            }
        }
    }
    syntax::protected_ranges_with_syntax(source, &protection, &config.syntax)
}

fn matching_ending(formatted: &str, original: &str) -> String {
    let mut formatted = formatted.trim_end_matches(['\r', '\n']).to_owned();
    let tail = original.trim_end_matches([' ', '\t']);
    if tail.ends_with("\r\n") {
        formatted.push_str("\r\n");
    } else if tail.ends_with('\n') {
        formatted.push('\n');
    }
    formatted
}

fn indent_fragment(formatted: &str, original: &str, prefix: &str) -> String {
    let formatted = matching_ending(formatted, original);
    let mut output = String::with_capacity(formatted.len());
    for (index, line) in formatted.split_inclusive('\n').enumerate() {
        if index != 0 && !line.trim_matches(['\r', '\n']).is_empty() {
            output.push_str(prefix);
        }
        output.push_str(line);
    }
    output
}

pub(crate) struct CodeOutput {
    pub text: String,
    pub math_recheck: bool,
}

pub(crate) fn format_code(
    source: &str,
    kind: FileKind,
    config: &CompiledConfig,
) -> Result<CodeOutput, FormatError> {
    format_code_with_context(
        source,
        kind,
        config,
        &mut crate::parse_context::ParseContext::new(config),
    )
}

pub(crate) fn format_code_with_context(
    source: &str,
    _kind: FileKind,
    config: &CompiledConfig,
    parsing: &mut crate::parse_context::ParseContext<'_>,
) -> Result<CodeOutput, FormatError> {
    if !is_active(source, config) {
        return Ok(CodeOutput {
            text: source.to_owned(),
            math_recheck: false,
        });
    }
    let expl3_active = config.config.code.expl3 && has_expl3_marker(source);
    let expl3_ranges = if expl3_active && config.config.protect.expl3 {
        Some(parsing.expl3_protected_ranges(source)?)
    } else {
        None
    };
    let parsed = parsing.snapshot(source, false, false)?;
    let mut protected = expl3_ranges.unwrap_or_else(|| parsed.protected.clone());
    if expl3_active {
        let exposed_model = (protected != parsed.protected).then(|| {
            parsed
                .metadata
                .exposed_model(source, &config.config, &protected)
        });
        let analysis = crate::source_analysis::SourceAnalysis::with_memo(
            source,
            &config.config,
            &parsed.protected,
            std::rc::Rc::clone(&parsed.metadata),
        );
        let model = exposed_model.as_deref().unwrap_or_else(|| analysis.model());
        let mut stored = model
            .stored_argument_ranges()
            .iter()
            .map(|range| {
                // Keep delimiters visible to the expl3 argument printer. Replacing
                // only their contents also preserves leading and trailing spaces.
                match source.as_bytes().get(range.start) {
                    Some(b'{' | b'[') if range.end > range.start + 1 => {
                        range.start + 1..range.end - 1
                    }
                    _ => range.clone(),
                }
            })
            .collect::<Vec<_>>();
        if config.config.protect.macro_bodies && !config.config.protect.format_macro_bodies {
            stored.extend(model.declarations().iter().filter_map(|declaration| {
                syntax::command_at(source, declaration.span.start)
                    .filter(|(name, _)| syntax::expl3_definition_variant(name).is_some())
                    .map(|_| declaration.span.clone())
            }));
        }
        // Keep module names opaque while exposing the key-definition command
        // to the code printer. Explicit whole-invocation protection stays intact.
        let prefixes = scanner::commands(source, &[])
            .into_iter()
            .filter(|(_, name, _)| *name == "keys_define:nn")
            .filter_map(|(_, _, end)| {
                let arguments = crate::layout::signature_argument_ranges(
                    source,
                    end..source.len(),
                    "mm".bytes(),
                );
                arguments
                    .first()
                    .map(|argument| (argument.end, argument.clone()))
            })
            .collect::<HashMap<_, _>>();
        for range in &mut protected {
            if let Some(argument) = prefixes
                .get(&range.end)
                .filter(|argument| range.start <= argument.start)
            {
                // Leave the argument delimiters available to the expl3
                // printer while preserving the module name byte for byte.
                range.start = argument.start + 1;
                range.end = argument.end - 1;
            }
        }
        protected.extend(stored.into_iter().filter(|range| !range.is_empty()));
        protected = merged(protected);
    }
    let declaration_protection = if expl3_active && !config.config.protect.expl3 {
        let mut scope_config = config.config.clone();
        scope_config.protect.expl3 = true;
        Cow::Owned(syntax::protected_ranges_with_syntax(
            source,
            &scope_config,
            &config.syntax,
        )?)
    } else {
        Cow::Borrowed(parsed.protected.as_slice())
    };
    let mut fragments = Vec::new();
    if config.config.code.algorithm2e {
        fragments.extend(
            scanner::environments(source, &protected)
                .into_iter()
                .filter(|(name, _)| algorithm_environment(name))
                .map(|(_, range)| (range, false)),
        );
        if !fragments.is_empty() {
            let analysis = if matches!(declaration_protection, Cow::Borrowed(_)) {
                crate::source_analysis::SourceAnalysis::with_memo(
                    source,
                    &config.config,
                    &declaration_protection,
                    std::rc::Rc::clone(&parsed.metadata),
                )
            } else {
                crate::source_analysis::SourceAnalysis::new(
                    source,
                    &config.config,
                    &declaration_protection,
                )
            };
            let declared = analysis.declared_commands()?;
            if [
                "For", "ForEach", "ForAll", "While", "If", "ElseIf", "Else", "eIf", "Repeat",
                "KwIn", "KwOut",
            ]
            .iter()
            .any(|name| declared.contains(*name))
            {
                fragments.clear();
            }
        }
    }
    if expl3_active {
        fragments.extend(
            expl3_regions(source, &protected)
                .into_iter()
                .map(|mut range| {
                    // Executed expl3 boundaries include local catcode restoration.
                    if let Some(scope) = declaration_protection
                        .get(
                            declaration_protection
                                .partition_point(|scope| scope.end <= range.start),
                        )
                        .filter(|scope| scope.start <= range.start)
                    {
                        range.end = range.end.min(scope.end);
                        if source.as_bytes().get(scope.start) == Some(&b'{')
                            && source.as_bytes().get(range.end.saturating_sub(1)) == Some(&b'}')
                        {
                            range.end -= 1;
                        }
                        // A scope terminator can have indentation after the
                        // fragment's last newline. Retain that newline so the
                        // printer does not attach the terminator to its last
                        // argument and change the next pass's boundary.
                        while range.end > range.start
                            && matches!(source.as_bytes()[range.end - 1], b' ' | b'\t')
                        {
                            range.end -= 1;
                        }
                    }
                    (range, true)
                }),
        );
    }
    fragments.sort_by_key(|(range, _)| (range.start, std::cmp::Reverse(range.end)));
    if fragments.is_empty() {
        return Ok(CodeOutput {
            text: source.to_owned(),
            math_recheck: false,
        });
    }
    if expl3_active {
        let mut nested_data = Vec::new();
        for (fragment, expl3) in &fragments {
            if !expl3 {
                continue;
            }
            let mut end_of_data = fragment.start;
            // Commands in a replacement body do not enter the top-level
            // declaration model. Keep their literal arguments opaque here
            // so enclosing indentation cannot alter the stored bytes either.
            let data = &source[fragment.clone()];
            let excluded = overlapping_ranges(&protected, fragment)
                .iter()
                .map(|range| {
                    range.start.saturating_sub(fragment.start)
                        ..range.end.min(fragment.end) - fragment.start
                })
                .collect::<Vec<_>>();
            for (start, name, end) in scanner::commands(data, &excluded) {
                if fragment.start + start < end_of_data {
                    continue;
                }
                if let Some(argument) = crate::source_model::expl3_stored_arguments(data, name, end)
                {
                    end_of_data = fragment.start + argument.end;
                    if data.as_bytes().get(argument.start) == Some(&b'{')
                        && argument.end > argument.start + 2
                    {
                        nested_data.push(
                            fragment.start + argument.start + 1..fragment.start + argument.end - 1,
                        );
                    }
                }
            }
        }
        protected.extend(nested_data);
        protected = merged(protected);
    }
    let mathematics = math_ranges(source, &protected);
    let recheck_ranges = if config.config.math.enabled {
        mathematics
            .iter()
            .filter(|range| {
                let body = &source[(*range).clone()];
                body.starts_with("\\[") || body.starts_with("$$") || body.starts_with("\\begin")
            })
            .cloned()
            .chain(
                scanner::commands(source, &protected)
                    .into_iter()
                    .filter(|(_, name, _)| {
                        matches!(
                            *name,
                            "mathbb"
                                | "mathcal"
                                | "mathfrak"
                                | "mathrm"
                                | "mathbf"
                                | "mathit"
                                | "mathsf"
                                | "mathtt"
                        )
                    })
                    .map(|(start, _, end)| start..end),
            )
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let mut math_recheck = false;
    let mut output = String::with_capacity(source.len());
    let mut cursor = 0;
    for (fragment, expl3) in fragments {
        let fragment_protection = overlapping_ranges(&protected, &fragment);
        if fragment.start < cursor
            || fragment_protection.iter().any(|range| {
                overlap(range, &fragment)
                    && !(fragment.start <= range.start && range.end <= fragment.end)
            })
            || fragment_protection
                .iter()
                .any(|range| range.start <= fragment.start && fragment.end <= range.end)
        {
            continue;
        }
        let original = &source[fragment.clone()];
        let mut opaque = fragment_protection
            .iter()
            .chain(overlapping_ranges(&mathematics, &fragment))
            .filter(|range| fragment.start <= range.start && range.end <= fragment.end)
            .map(|range| range.start - fragment.start..range.end - fragment.start)
            .collect::<Vec<_>>();
        // A conditional or catcode region is kept together with all of its bytes.
        opaque.retain(|range| !range.is_empty());
        let opaque = Opaque::new(original, opaque);
        let formatted = printer::format(&opaque, expl3, config.config.indent.width);
        let line_start = source[..fragment.start]
            .rfind('\n')
            .map_or(0, |position| position + 1);
        let prefix = &source[line_start..fragment.start];
        let prefix = if prefix
            .chars()
            .all(|character| matches!(character, ' ' | '\t'))
        {
            prefix
        } else {
            ""
        };
        let formatted = indent_fragment(&formatted, original, prefix);
        // Restore only after adding enclosing indentation so multiline literal
        // and mathematical regions retain their original internal bytes.
        let Some(formatted) = opaque.restore(&formatted) else {
            continue;
        };
        if formatted != original {
            math_recheck |= !overlapping_ranges(&recheck_ranges, &fragment).is_empty();
        }
        // A successfully printed enclosing fragment owns its nested fragments
        // even when its bytes already match the printer's output.
        output.push_str(&source[cursor..fragment.start]);
        output.push_str(&formatted);
        cursor = fragment.end;
    }
    output.push_str(&source[cursor..]);
    Ok(CodeOutput {
        text: output,
        math_recheck,
    })
}

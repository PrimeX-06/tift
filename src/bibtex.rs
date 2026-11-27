//! BibTeX layout with a dedicated value lexer.
//!
//! Defaults align assignments and collapse repeated spaces in ordinary literal
//! field values while preserving name case, field order and expression syntax.

use crate::syntax::{self, FormatDirectiveState};
use crate::{CompiledConfig, Diagnostic, FormatError, FormatOutput, Severity};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::ops::Range;

mod cleanup;
mod compact_strings;
mod normalization;
pub use cleanup::CleanupConfig;

/// BibTeX layout and sorting, with optional data edits in `cleanup`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct BibtexConfig {
    /// Format supported file types inside literal filecontents environments.
    pub format_filecontents: bool,
    /// Normalize assignment layout. Field alignment also requires `align_fields`.
    pub normalize: bool,
    /// `preserve`, `lower`, or `upper` for entry types and regular field names.
    pub name_case: String,
    /// Align regular field assignments while normalizing.
    pub align_fields: bool,
    pub separate_entries: bool,
    pub compact_strings: bool,
    /// Wrap literal author and editor lists at top-level name separators.
    pub name_list_columns: Option<usize>,
    /// Wrap long literal keyword lists after their outer commas.
    pub keyword_columns: Option<usize>,
    /// Width at which a literal list of related citation keys expands.
    pub related_columns: Option<usize>,
    /// Wrap long value expressions before each top-level concatenation operator.
    pub concat_columns: Option<usize>,
    /// `preserve`, `braces`, or `quotes`; applies only to safe literal value pieces.
    pub value_delimiters: String,
    /// Empty preserves order; `canonical`, `alphabetical`, or a list of field names.
    pub field_order: Vec<String>,
    /// Sort consecutive regular entries without crossing definitions or comments.
    pub sort: bool,
    /// Citation `key` or field names; a leading `-` reverses that criterion.
    pub sort_by: Vec<String>,
    /// Compare bibliography sort values case sensitively when sorting is enabled.
    pub case_sensitive: bool,
    /// Optional data transformations, disabled by default.
    pub cleanup: CleanupConfig,
}

impl Default for BibtexConfig {
    fn default() -> Self {
        Self {
            format_filecontents: true,
            normalize: true,
            name_case: "preserve".into(),
            align_fields: true,
            separate_entries: true,
            compact_strings: false,
            name_list_columns: None,
            keyword_columns: None,
            related_columns: None,
            concat_columns: None,
            value_delimiters: "preserve".into(),
            field_order: Vec::new(),
            sort: false,
            sort_by: vec!["key".into()],
            case_sensitive: false,
            cleanup: CleanupConfig::default(),
        }
    }
}

impl BibtexConfig {
    fn name<'a>(&self, name: &'a str) -> Cow<'a, str> {
        match self.name_case.as_str() {
            "lower" => Cow::Owned(name.to_ascii_lowercase()),
            "upper" => Cow::Owned(name.to_ascii_uppercase()),
            _ => Cow::Borrowed(name),
        }
    }
}

pub fn validate(config: &BibtexConfig) -> Result<(), String> {
    normalization::validate(config)?;
    cleanup::validate(&config.cleanup)
}

/// Resolve a supported payload type from the declared filename and locate its
/// contents, including when the environment itself is indented.
fn embedded_body(
    region: &str,
    name: &str,
    header: usize,
) -> Option<(crate::FileKind, Range<usize>)> {
    let mut open = crate::trivia::skip(region, header);
    if region.as_bytes().get(open) == Some(&b'[') {
        open = crate::trivia::skip(region, syntax::balanced_end(region, open, b'[', b']')?);
    }
    let end = syntax::balanced_end(region, open, b'{', b'}')?;
    let file = region[open + 1..end - 1].trim();
    let filename = file
        .strip_prefix(r"\jobname")
        .filter(|tail| !tail.as_bytes().first().is_some_and(u8::is_ascii_alphabetic))
        .map_or_else(|| file.to_owned(), |tail| format!("jobname{tail}"));
    if filename.contains(['\\', '{', '}', '%', '#', '~', '^']) {
        return None;
    }
    let kind = crate::FileKind::from_path(std::path::Path::new(&filename))?;
    let first = end + region[end..].find('\n')? + 1;
    let close = region.strip_suffix(&format!("\\end{{{name}}}"))?;
    let last = close.rfind('\n')? + 1;
    (first <= last && close[last..].trim_matches([' ', '\t']).is_empty())
        .then_some((kind, first..last))
}

pub(crate) fn format_embedded(
    source: &str,
    config: &CompiledConfig,
    parsing: &mut crate::parse_context::ParseContext<'_>,
    bom_bytes: usize,
    diagnostics: &mut Vec<Diagnostic>,
    history: Option<&mut crate::edit_plan::EditHistory>,
) -> Result<String, FormatError> {
    let parsed = parsing.snapshot(source, false, false)?;
    let analysis = parsed.analysis(&config.config);
    let declared = analysis.declared_environments();
    // The closing delimiter occupies its own physical line. Replacements over
    // the whole host already include this payload and must run only once.
    let local_config = if config.config.output.final_newline == crate::config::FinalNewline::Never
        || config
            .config
            .replacements
            .iter()
            .any(|rule| rule.enabled && rule.scope == crate::replacements::Scope::All)
    {
        let mut settings = config.config.clone();
        if settings.output.final_newline == crate::config::FinalNewline::Never {
            settings.output.final_newline = crate::config::FinalNewline::Preserve;
        }
        settings
            .replacements
            .retain(|rule| rule.scope != crate::replacements::Scope::All);
        Some(CompiledConfig::new(settings).map_err(|err| FormatError::Syntax(err.to_string()))?)
    } else {
        None
    };
    let payload_config = local_config.as_ref().unwrap_or(config);
    let mut edits = crate::edit_plan::EditPlan::new();
    for span in &parsed.protected {
        let region = &source[span.clone()];
        let Some(("begin", command_end)) = syntax::command_at(region, 0) else {
            continue;
        };
        let Some((name @ ("filecontents" | "filecontents*"), true, header)) =
            syntax::env_after_command(region, "begin", command_end)
        else {
            continue;
        };
        if declared.contains(name)
            || config
                .config
                .syntax
                .environment_arguments
                .contains_key(name)
        {
            continue;
        }
        let Some((kind, body)) = embedded_body(region, name, header) else {
            continue;
        };
        let body = span.start + body.start..span.start + body.end;
        let result = crate::format(&source[body.clone()], kind, payload_config)?;
        for mut diagnostic in result.diagnostics {
            diagnostic.span.start += body.start + bom_bytes;
            diagnostic.span.end += body.start + bom_bytes;
            diagnostics.push(diagnostic);
        }
        if result.text != source[body.clone()] {
            edits.replace(body, result.text);
        }
    }
    edits.apply_recording(source, history)
}

pub fn format_bibtex(source: &str, config: &CompiledConfig) -> Result<String, FormatError> {
    format_bibtex_inner(source, config).map(|output| output.text)
}

/// Format bibliography data and report source offsets for unsafe or malformed
/// constructs. Syntax errors preserve the complete original input.
pub fn format_bibtex_detailed(
    source: &str,
    config: &CompiledConfig,
) -> Result<FormatOutput, FormatError> {
    match format_bibtex_inner(source, config) {
        Ok(output) => Ok(output),
        Err(FormatError::Syntax(message)) => {
            // A failed transformed document must still refer to the caller's
            // original offsets. Recheck only this error path without cleanup.
            let message = if config.config.bibtex.cleanup.enabled {
                match format_bibtex_layout(source, config) {
                    Err(FormatError::Syntax(original)) => original,
                    _ => message,
                }
            } else {
                message
            };
            let (offset, description) = message
                .strip_prefix("BibTeX at byte ")
                .and_then(|rest| rest.split_once(": "))
                .and_then(|(offset, description)| {
                    offset
                        .parse::<usize>()
                        .ok()
                        .map(|offset| (offset, description))
                })
                .unwrap_or((0, message.as_str()));
            let mut start = offset.min(source.len());
            while !source.is_char_boundary(start) {
                start -= 1;
            }
            let end = source[start..]
                .chars()
                .next()
                .map_or(start, |character| start + character.len_utf8());
            Ok(FormatOutput {
                text: source.to_owned(),
                diagnostics: vec![Diagnostic {
                    severity: Severity::Error,
                    message: description.to_owned(),
                    span: start..end,
                }],
            })
        }
        Err(error) => Err(error),
    }
}

fn format_bibtex_inner(source: &str, config: &CompiledConfig) -> Result<FormatOutput, FormatError> {
    let prepared = cleanup::prepare(source, &config.config.bibtex.cleanup)?;
    let (entries, mut diagnostics, protected) = prepared.map_or_else(
        || (Vec::new(), Vec::new(), None),
        |prepared| {
            (
                prepared.entries,
                prepared.diagnostics,
                Some(prepared.protected),
            )
        },
    );
    let text = format_bibtex_prepared(source, config, entries, protected, &mut diagnostics)?;
    Ok(FormatOutput { text, diagnostics })
}

fn format_bibtex_layout(source: &str, config: &CompiledConfig) -> Result<String, FormatError> {
    format_bibtex_prepared(source, config, Vec::new(), None, &mut Vec::new())
}

fn format_bibtex_prepared(
    source: &str,
    config: &CompiledConfig,
    prepared: Vec<cleanup::Entry<'_>>,
    protected: Option<Vec<Range<usize>>>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<String, FormatError> {
    let mut prepared = prepared.into_iter().peekable();
    let protected = match protected {
        Some(ranges) => ranges,
        None => protected_ranges(source)?,
    };
    let mut output = String::with_capacity(source.len());
    let mut outside_blanks = 0;
    let mut previous_entry_end = None;
    let mut source_map = Vec::new();
    let mut strings = Vec::new();
    let maximum_blank_lines = config.config.whitespace.blank_lines.limit();
    let newline = if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let mut cursor = 0;
    let mut copy_start = 0;
    let bytes = source.as_bytes();
    while cursor < source.len() {
        let index = protected.partition_point(|range| range.end <= cursor);
        if let Some(range) = protected.get(index).filter(|range| range.start <= cursor) {
            previous_entry_end = None;
            cursor = range.end;
            continue;
        }
        if bytes[cursor] == b'%' {
            cursor = line_end(source, cursor, source.len());
            continue;
        }
        if bytes[cursor] != b'@' {
            cursor = memchr::memchr2(b'@', b'%', &bytes[cursor + 1..])
                .map_or(source.len(), |offset| cursor + 1 + offset);
            continue;
        }
        let entry_start = cursor;
        let Some((kind, type_end)) = entry_type(source, cursor) else {
            cursor += 1;
            continue;
        };
        let type_start = cursor + 1;
        cursor = type_end;
        let open = skip_whitespace(source, cursor, source.len());
        if !matches!(bytes.get(open), Some(b'{' | b'(')) {
            if kind.eq_ignore_ascii_case("comment") {
                previous_entry_end = None;
                cursor = line_end(source, cursor, source.len());
                write_outside_protected(
                    &mut output,
                    source,
                    copy_start..entry_start,
                    maximum_blank_lines,
                    &protected,
                    &mut outside_blanks,
                );
                output.push('@');
                output.push_str(&config.config.bibtex.name(kind));
                output.push_str(&source[type_start + kind.len()..cursor]);
                outside_blanks = 0;
                copy_start = cursor;
                continue;
            }
            return Err(syntax_error(
                open,
                "expected '{' or '(' after BibTeX entry type",
            ));
        }
        let prepared_entry = if prepared
            .peek()
            .is_some_and(|entry| entry.span.start == entry_start)
        {
            prepared.next()
        } else {
            None
        };
        let close = if let Some(entry) = &prepared_entry {
            entry.span.end - 1
        } else if kind.eq_ignore_ascii_case("comment") {
            find_comment_end(source, open)?
        } else {
            find_entry_end(source, open)?
        };
        cursor = close + 1;
        if kind.eq_ignore_ascii_case("comment") {
            previous_entry_end = None;
            write_outside_protected(
                &mut output,
                source,
                copy_start..entry_start,
                maximum_blank_lines,
                &protected,
                &mut outside_blanks,
            );
            output.push('@');
            output.push_str(&config.config.bibtex.name(kind));
            output.push_str(&source[type_start + kind.len()..cursor]);
            outside_blanks = 0;
            copy_start = cursor;
            continue;
        }
        write_outside_protected(
            &mut output,
            source,
            copy_start..entry_start,
            maximum_blank_lines,
            &protected,
            &mut outside_blanks,
        );
        if prepared_entry.as_ref().is_some_and(|entry| entry.removed) {
            copy_start = cursor;
            continue;
        }
        let regular =
            !kind.eq_ignore_ascii_case("string") && !kind.eq_ignore_ascii_case("preamble");
        if regular
            && config.config.bibtex.separate_entries
            && maximum_blank_lines != Some(0)
            && let Some(start) = previous_entry_end
        {
            separate_entries(&mut output, start, newline);
        }
        let base = entry_indent(source, entry_start);
        let indent = format!("{base}{}", config.config.indent.unit("bibtex", kind));
        let body_start = open + 1;
        let printed_kind = config.config.bibtex.name(kind);
        let opening = format!("@{printed_kind}{}", char::from(bytes[open]));
        let closing = &source[close..close + 1];
        let printed_start = output.len();
        let mut origins = Vec::new();
        if let Some(entry) = prepared_entry {
            output.push_str(&entry.format(
                source,
                &opening,
                closing,
                &base,
                &indent,
                newline,
                &config.config.bibtex,
                config.config.bibtex.sort.then_some(&mut origins),
            ));
        } else if kind.eq_ignore_ascii_case("preamble") {
            output.push_str(&format_preamble(
                source,
                body_start,
                close,
                &opening,
                closing,
                &base,
                &indent,
                newline,
                config.config.bibtex.concat_columns,
            )?);
        } else {
            output.push_str(&format_fields(
                source,
                body_start,
                close,
                &opening,
                closing,
                &base,
                &indent,
                newline,
                kind.eq_ignore_ascii_case("string"),
                &config.config.bibtex,
                config.config.bibtex.sort.then_some(&mut origins),
            )?);
        }
        if config.config.bibtex.sort {
            source_map.extend(origins.into_iter().map(|(printed, original)| {
                (
                    printed.start + printed_start..printed.end + printed_start,
                    original,
                )
            }));
        }
        if config.config.bibtex.compact_strings
            && kind.eq_ignore_ascii_case("string")
            && source[close + 1..]
                .split('\n')
                .next()
                .unwrap_or("")
                .trim()
                .is_empty()
            && source[physical_line_start(source, entry_start)..entry_start]
                .trim_matches([' ', '\t'])
                .is_empty()
            && let Some(candidate) = compact_strings::Candidate::parse(
                &output,
                printed_start..output.len(),
                opening.len(),
                &base,
                config,
            )
        {
            strings.push(candidate);
        }
        outside_blanks = 0;
        previous_entry_end = regular.then_some(output.len());
        copy_start = cursor;
    }
    write_outside_protected(
        &mut output,
        source,
        copy_start..source.len(),
        maximum_blank_lines,
        &protected,
        &mut outside_blanks,
    );
    let text = if config.config.bibtex.sort {
        let (text, mut warnings) = normalization::sort_entries(&output, &config.config.bibtex)?;
        for warning in &mut warnings {
            let entry =
                source_map.partition_point(|(printed, _)| printed.end <= warning.span.start);
            if let Some((_, original)) = source_map.get(entry).filter(|(printed, _)| {
                printed.start <= warning.span.start && warning.span.end <= printed.end
            }) {
                warning.span = original.clone();
            }
        }
        diagnostics.extend(warnings);
        text
    } else {
        output
    };
    // Sorting only permutes runs of regular entries; every string declaration
    // still occupies its measured range. Resolve diagnostics before shrinking it.
    compact_strings::render(text, &strings, config.config.wrap.columns)
}

fn separate_entries(output: &mut String, start: usize, newline: &str) {
    let gap = &output[start..];
    if !gap.lines().all(|line| {
        let line = line.trim_matches([' ', '\t', '\r']);
        line.is_empty() || line.starts_with('%')
    }) {
        return;
    }
    // A comment on the closing brace belongs to the preceding record.
    let first = gap.split('\n').next().unwrap_or("").trim_end_matches('\r');
    let prefix = if first.trim_start().starts_with('%') {
        first.len()
    } else {
        0
    };
    let rest = &gap[prefix..];
    let indent = rest.rsplit('\n').next().unwrap_or("");
    let leading = if let Some(percent) = rest.find('%') {
        rest[..percent].rfind('\n').map_or(0, |i| i + 1)
    } else {
        rest.len() - indent.len()
    };
    let split = start + prefix;
    let separator = if newline == "\r\n" {
        "\r\n\r\n"
    } else {
        "\n\n"
    };
    if &output[split..split + leading] != separator {
        output.replace_range(split..split + leading, separator);
    }
}

fn write_outside_protected(
    output: &mut String,
    source: &str,
    span: Range<usize>,
    maximum_blank_lines: Option<usize>,
    protected: &[Range<usize>],
    blanks: &mut usize,
) {
    let mut cursor = span.start;
    for range in protected[protected.partition_point(|range| range.end <= span.start)..]
        .iter()
        .take_while(|range| range.start < span.end)
    {
        let start = range.start.max(span.start);
        let end = range.end.min(span.end);
        write_outside(output, &source[cursor..start], maximum_blank_lines, blanks);
        output.push_str(&source[start..end]);
        *blanks = 0;
        cursor = end;
    }
    write_outside(
        output,
        &source[cursor..span.end],
        maximum_blank_lines,
        blanks,
    );
}

fn physical_line_start(source: &str, cursor: usize) -> usize {
    source[..cursor].rfind('\n').map_or(0, |n| n + 1)
}

fn merge_ranges(ranges: &mut Vec<Range<usize>>) {
    ranges.sort_by_key(|range| range.start);
    let mut write = 0;
    for read in 0..ranges.len() {
        if write > 0 && ranges[read].start <= ranges[write - 1].end {
            ranges[write - 1].end = ranges[write - 1].end.max(ranges[read].end);
        } else {
            if write != read {
                ranges[write] = ranges[read].clone();
            }
            write += 1;
        }
    }
    ranges.truncate(write);
}

/// Actual BibTeX comment directives, excluding braced/quoted values and
/// @comment bodies. A directive touching an entry preserves that whole entry.
pub(crate) fn protected_ranges(source: &str) -> Result<Vec<Range<usize>>, FormatError> {
    if !source.contains('%') || !(source.contains("tift") || source.contains("tex-fmt")) {
        return Ok(Vec::new());
    }
    let mut ranges = Vec::new();
    let mut entries = Vec::new();
    let mut directives = FormatDirectiveState::default();
    let mut cursor = 0;
    let bytes = source.as_bytes();
    while cursor < bytes.len() {
        if bytes[cursor] == b'%' {
            let finish = line_end(source, cursor, source.len());
            // A standalone skip suppresses instructions on its skipped row.
            if !ranges
                .last()
                .is_some_and(|range: &Range<usize>| range.start <= cursor && cursor < range.end)
            {
                directives.observe(source, cursor, finish, &mut ranges);
            }
            cursor = finish;
            continue;
        }
        if bytes[cursor] != b'@' {
            cursor += source[cursor..].chars().next().unwrap().len_utf8();
            continue;
        }
        let entry_start = cursor;
        let Some((kind, type_end)) = entry_type(source, cursor) else {
            cursor += 1;
            continue;
        };
        cursor = type_end;
        let open = skip_whitespace(source, cursor, source.len());
        if kind.eq_ignore_ascii_case("comment") {
            cursor = if matches!(bytes.get(open), Some(b'{' | b'(')) {
                find_comment_end(source, open).map_or(source.len(), |close| close + 1)
            } else {
                line_end(source, cursor, source.len())
            };
            entries.push(entry_start..cursor);
            continue;
        }
        if directives.is_disabled() {
            continue;
        }
        if !matches!(bytes.get(open), Some(b'{' | b'(')) {
            // Formatting validates active entries; range analysis can also run
            // before a replacement repairs an otherwise malformed entry.
            continue;
        }
        let initial_protection = ranges
            .iter()
            .any(|range| range.start <= entry_start && entry_start < range.end);
        let mut touched = initial_protection;
        let close = scan_entry_end(
            source,
            open,
            Some((&mut directives, &mut ranges, &mut touched)),
        );
        match close {
            Ok(close) => {
                let end = close + 1;
                if touched {
                    ranges.push(
                        physical_line_start(source, entry_start)
                            ..line_end(source, close, source.len()),
                    );
                }
                entries.push(entry_start..end);
                cursor = end;
            }
            Err(_) if touched || directives.is_disabled() => {
                ranges.push(physical_line_start(source, entry_start)..source.len());
                break;
            }
            Err(_) => break,
        }
    }
    directives.finish(source.len(), &mut ranges);
    merge_ranges(&mut ranges);
    // A trailing skip can be encountered after its entry was already scanned.
    let additions: Vec<_> = entries
        .into_iter()
        .filter(|entry| syntax::overlaps(entry, &ranges))
        .map(|entry| {
            physical_line_start(source, entry.start)
                ..line_end(source, entry.end.saturating_sub(1), source.len())
        })
        .collect();
    ranges.extend(additions);
    merge_ranges(&mut ranges);
    Ok(ranges)
}

fn write_outside(
    output: &mut String,
    text: &str,
    maximum_blank_lines: Option<usize>,
    blanks: &mut usize,
) {
    let Some(maximum) = maximum_blank_lines else {
        output.push_str(text);
        return;
    };
    let mut line_start = output.is_empty() || output.ends_with('\n');
    for line in text.split_inclusive('\n') {
        let body = line.trim_end_matches(['\r', '\n']);
        if line_start && line.ends_with('\n') && body.trim_matches([' ', '\t']).is_empty() {
            *blanks = blanks.saturating_add(1);
            if *blanks <= maximum {
                output.push_str(line);
            }
        } else {
            *blanks = 0;
            output.push_str(line);
        }
        line_start = line.ends_with('\n');
    }
}

fn syntax_error(offset: usize, message: &str) -> FormatError {
    FormatError::Syntax(format!("BibTeX at byte {offset}: {message}"))
}

fn entry_indent(source: &str, offset: usize) -> String {
    let start = source[..offset].rfind('\n').map_or(0, |n| n + 1);
    let prefix = &source[start..offset];
    if prefix.chars().all(|c| c == ' ' || c == '\t') {
        prefix.into()
    } else {
        String::new()
    }
}

fn skip_whitespace(source: &str, mut cursor: usize, end: usize) -> usize {
    while cursor < end && source.as_bytes()[cursor].is_ascii_whitespace() {
        cursor += 1;
    }
    cursor
}

fn line_end(source: &str, cursor: usize, end: usize) -> usize {
    source[cursor..end]
        .find('\n')
        .map_or(end, |n| cursor + n + 1)
}

fn skip_trivia(source: &str, mut cursor: usize, end: usize) -> (usize, Vec<&str>) {
    let mut comments = Vec::new();
    loop {
        cursor = skip_whitespace(source, cursor, end);
        if cursor < end && source.as_bytes()[cursor] == b'%' {
            let next = line_end(source, cursor, end);
            comments.push(source[cursor..next].trim_end_matches(['\r', '\n']));
            cursor = next;
        } else {
            return (cursor, comments);
        }
    }
}

/// Entry types use BibTeX identifiers, including package-defined names such
/// as ABNT-options. All passes must agree on where the type ends.
fn entry_type(source: &str, at: usize) -> Option<(&str, usize)> {
    let start = at + 1;
    let bytes = source.as_bytes();
    if !bytes
        .get(start)
        .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
    {
        return None;
    }
    let mut end = start + 1;
    while bytes.get(end).is_some_and(|byte| {
        byte.is_ascii_alphanumeric() || matches!(*byte, b'_' | b'-' | b'.' | b':')
    }) {
        end += 1;
    }
    Some((&source[start..end], end))
}

fn valid_citation_key(key: &str, brace_entry: bool) -> bool {
    // BibTeX reads the key up to its comma. Punctuation used by custom
    // bibliography styles (e.g. option=value keys) is part of the key.
    !key.chars()
        .any(|c| c.is_whitespace() || matches!(c, '{' | '}' | '"') || !brace_entry && c == ')')
}

fn find_entry_end(source: &str, open: usize) -> Result<usize, FormatError> {
    scan_entry_end(source, open, None)
}

fn scan_entry_end(
    source: &str,
    open: usize,
    mut directives: Option<(&mut FormatDirectiveState, &mut Vec<Range<usize>>, &mut bool)>,
) -> Result<usize, FormatError> {
    let bytes = source.as_bytes();
    let brace_entry = bytes[open] == b'{';
    let mut braces = usize::from(brace_entry);
    let outer_depth = braces;
    let mut quoted = false;
    let mut quote_braces = 0usize;
    let mut cursor = open + 1;
    while cursor < bytes.len() {
        if directives
            .as_ref()
            .is_some_and(|(state, _, _)| state.is_disabled())
            && bytes[cursor] != b'%'
        {
            if bytes[cursor] == b'@' {
                if let Some((kind, finish)) = entry_type(source, cursor)
                    && kind.eq_ignore_ascii_case("comment")
                {
                    let open = skip_whitespace(source, finish, source.len());
                    cursor = if matches!(bytes.get(open), Some(b'{' | b'(')) {
                        find_comment_end(source, open).map_or(source.len(), |close| close + 1)
                    } else {
                        line_end(source, finish, source.len())
                    };
                    continue;
                }
                cursor += 1;
            }
            cursor =
                memchr::memchr2(b'%', b'@', &bytes[cursor..]).map_or(bytes.len(), |n| cursor + n);
            continue;
        }
        if directives
            .as_ref()
            .is_some_and(|(state, _, _)| state.is_disabled())
            && bytes[cursor] == b'%'
        {
            let finish = line_end(source, cursor, bytes.len());
            let (state, ranges, touched) = directives.as_mut().unwrap();
            state.observe(source, cursor, finish, ranges);
            **touched = true;
            cursor = finish;
            continue;
        }
        match bytes[cursor] {
            b'\\' => {
                cursor += 1;
                if cursor < bytes.len() {
                    cursor += source[cursor..].chars().next().unwrap().len_utf8();
                }
            }
            b'"' if braces == outer_depth && quote_braces == 0 => {
                quoted = !quoted;
                cursor += 1;
            }
            b'{' if quoted => {
                quote_braces += 1;
                cursor += 1;
            }
            b'}' if quoted => {
                quote_braces = quote_braces.saturating_sub(1);
                cursor += 1;
            }
            b'{' => {
                braces += 1;
                cursor += 1;
            }
            b'}' => {
                if braces == 0 {
                    return Err(syntax_error(cursor, "unmatched '}'"));
                }
                braces -= 1;
                if brace_entry && braces == 0 {
                    return Ok(cursor);
                }
                cursor += 1;
            }
            b')' if !brace_entry && braces == 0 && !quoted => return Ok(cursor),
            b'%' if braces == outer_depth && !quoted => {
                let finish = line_end(source, cursor, bytes.len());
                if let Some((state, ranges, touched)) = directives.as_mut()
                    && !ranges
                        .last()
                        .is_some_and(|range| range.start <= cursor && cursor < range.end)
                {
                    let previous = ranges.len();
                    state.observe(source, cursor, finish, ranges);
                    **touched |= ranges.len() != previous || state.is_disabled();
                }
                cursor = finish;
            }
            _ => {
                let start = cursor + 1;
                let structural = memchr::memchr3(b'\\', b'{', b'}', &bytes[start..])
                    .map_or(bytes.len(), |offset| start + offset);
                cursor = memchr::memchr3(b'"', b'%', b')', &bytes[start..structural])
                    .map_or(structural, |offset| start + offset);
            }
        }
    }
    Err(syntax_error(open, "unterminated entry or quoted value"))
}

fn find_comment_end(source: &str, open: usize) -> Result<usize, FormatError> {
    let opening = source.as_bytes()[open];
    let closing = if opening == b'{' { b'}' } else { b')' };
    let mut depth = 1usize;
    for (index, byte) in source.as_bytes().iter().enumerate().skip(open + 1) {
        if *byte == opening {
            depth += 1;
        } else if *byte == closing {
            depth -= 1;
            if depth == 0 {
                return Ok(index);
            }
        }
    }
    Err(syntax_error(open, "unterminated comment entry"))
}

#[allow(clippy::too_many_arguments)]
fn format_fields(
    source: &str,
    start: usize,
    end: usize,
    opening: &str,
    closing: &str,
    base: &str,
    indent: &str,
    newline: &str,
    string_definition: bool,
    config: &BibtexConfig,
    mut origins: Option<&mut Vec<(Range<usize>, Range<usize>)>>,
) -> Result<String, FormatError> {
    if !string_definition
        && (config.normalize
            || config.name_list_columns.is_some()
            || config.keyword_columns.is_some()
            || config.related_columns.is_some()
            || config.name_case != "preserve"
            || !config.field_order.is_empty()
            || config.value_delimiters != "preserve")
    {
        return normalization::format_fields(
            source, start, end, opening, closing, base, indent, newline, config, origins,
        );
    }
    let mut output = opening.to_string();
    let mut cursor = start;
    let mut initial_comments = Vec::new();
    if !string_definition {
        let (next, comments) = skip_trivia(source, cursor, end);
        initial_comments = comments;
        cursor = next;
        let comma = source[cursor..end].find(',').map(|n| cursor + n);
        let Some(comma) = comma else {
            let key = source[cursor..end].trim();
            if key.is_empty() {
                output.push_str(closing);
                return Ok(output);
            }
            if !valid_citation_key(key, opening.ends_with('{')) {
                return Err(syntax_error(
                    cursor,
                    "invalid citation key or missing key comma",
                ));
            }
            output.push_str(key);
            output.push_str(closing);
            return Ok(output);
        };
        let key = source[cursor..comma].trim();
        if !valid_citation_key(key, opening.ends_with('{')) {
            return Err(syntax_error(cursor, "invalid citation key"));
        }
        output.push_str(key);
        output.push(',');
        cursor = comma + 1;
    }
    output.push_str(newline);
    write_comments(&mut output, &initial_comments, indent, newline);
    loop {
        let (next, comments) = skip_trivia(source, cursor, end);
        cursor = next;
        write_comments(&mut output, &comments, indent, newline);
        if cursor == end {
            break;
        }
        let name_start = cursor;
        while cursor < end
            && !source.as_bytes()[cursor].is_ascii_whitespace()
            && !matches!(
                source.as_bytes()[cursor],
                b'=' | b',' | b'{' | b'}' | b'"' | b'%' | b'(' | b')' | b'#'
            )
        {
            cursor += source[cursor..].chars().next().unwrap().len_utf8();
        }
        let name = &source[name_start..cursor];
        if name.is_empty() {
            return Err(syntax_error(name_start, "invalid field name"));
        }
        let (next, comments) = skip_trivia(source, cursor, end);
        cursor = next;
        write_comments(&mut output, &comments, indent, newline);
        if cursor >= end || source.as_bytes()[cursor] != b'=' {
            return Err(syntax_error(name_start, "expected field assignment"));
        }
        cursor += 1;
        let (next, comments) = skip_trivia(source, cursor, end);
        cursor = next;
        write_comments(&mut output, &comments, indent, newline);
        let (value, next) = read_expression(source, cursor, end)?;
        let (next, comments) = skip_trivia(source, next, end);
        output.push_str(indent);
        output.push_str(name);
        output.push_str(" = ");
        let printed_start = output.len();
        if string_definition {
            output.push_str(&source[value.clone()]);
        } else {
            normalization::write_value(&mut output, &source[value.clone()], name, config);
        }
        let comma = !string_definition || source.as_bytes().get(next) == Some(&b',');
        if let Some(columns) = config.concat_columns {
            normalization::wrap_concat(&mut output, printed_start, indent, newline, columns, comma);
        }
        if let Some(origins) = &mut origins {
            origins.push((printed_start..output.len(), value));
        }
        // BibTeX string commands accept no automatically added trailing comma.
        // Preserve an authored comma, including separators between assignments.
        if comma {
            output.push(',');
        }
        output.push_str(newline);
        cursor = next;
        write_comments(&mut output, &comments, indent, newline);
        if cursor < end {
            if source.as_bytes()[cursor] != b',' {
                return Err(syntax_error(cursor, "expected comma between fields"));
            }
            cursor += 1;
        }
    }
    output.push_str(base);
    output.push_str(closing);
    Ok(output)
}

fn write_comments(output: &mut String, comments: &[&str], indent: &str, newline: &str) {
    for comment in comments {
        output.push_str(indent);
        output.push_str(comment);
        output.push_str(newline);
    }
}

#[allow(clippy::too_many_arguments)]
fn format_preamble(
    source: &str,
    start: usize,
    end: usize,
    opening: &str,
    closing: &str,
    base: &str,
    indent: &str,
    newline: &str,
    concat_columns: Option<usize>,
) -> Result<String, FormatError> {
    let (cursor, comments) = skip_trivia(source, start, end);
    let (value, next) = read_expression(source, cursor, end)?;
    let (mut cursor, trailing_comments) = skip_trivia(source, next, end);
    let comma = source.as_bytes().get(cursor) == Some(&b',');
    let mut after_comma = Vec::new();
    if comma {
        cursor += 1;
        let (next, comments) = skip_trivia(source, cursor, end);
        cursor = next;
        after_comma = comments;
    }
    if cursor != end {
        return Err(syntax_error(cursor, "unexpected text after preamble"));
    }
    let mut output = opening.to_string();
    output.push_str(newline);
    write_comments(&mut output, &comments, indent, newline);
    output.push_str(indent);
    let printed_start = output.len();
    output.push_str(&source[value]);
    if let Some(columns) = concat_columns {
        normalization::wrap_concat(&mut output, printed_start, indent, newline, columns, comma);
    }
    if comma {
        output.push(',');
    }
    output.push_str(newline);
    write_comments(&mut output, &trailing_comments, indent, newline);
    write_comments(&mut output, &after_comma, indent, newline);
    output.push_str(base);
    output.push_str(closing);
    Ok(output)
}

fn read_expression(
    source: &str,
    start: usize,
    end: usize,
) -> Result<(std::ops::Range<usize>, usize), FormatError> {
    let mut cursor = start;
    let mut last_token_end;
    loop {
        cursor = skip_trivia(source, cursor, end).0;
        if cursor == end {
            return Err(syntax_error(cursor, "missing value"));
        }
        cursor = value_token_end(source, cursor, end)?;
        last_token_end = cursor;
        let next = skip_trivia(source, cursor, end).0;
        if next < end && source.as_bytes()[next] == b'#' {
            cursor = next + 1;
            continue;
        }
        return Ok((start..last_token_end, last_token_end));
    }
}

fn value_token_end(source: &str, start: usize, end: usize) -> Result<usize, FormatError> {
    match source.as_bytes()[start] {
        b'{' => braced_value_end(source, start, end),
        b'"' => quoted_value_end(source, start, end),
        b'#' | b',' | b'}' | b')' | b'=' | b'%' => Err(syntax_error(start, "expected value token")),
        _ => {
            let length = source.as_bytes()[start..end]
                .iter()
                .take_while(|&&byte| {
                    !byte.is_ascii_whitespace()
                        && !matches!(
                            byte,
                            b'#' | b',' | b'{' | b'}' | b'(' | b')' | b'"' | b'=' | b'%'
                        )
                })
                .count();
            if length == 0 {
                Err(syntax_error(start, "invalid value token"))
            } else {
                Ok(start + length)
            }
        }
    }
}

fn braced_value_end(source: &str, start: usize, end: usize) -> Result<usize, FormatError> {
    let mut depth = 1usize;
    let mut cursor = start + 1;
    while cursor < end {
        match source.as_bytes()[cursor] {
            b'\\' => {
                cursor += 1;
                if cursor < end {
                    cursor += source[cursor..].chars().next().unwrap().len_utf8();
                }
            }
            b'{' => {
                depth += 1;
                cursor += 1;
            }
            b'}' => {
                depth -= 1;
                cursor += 1;
                if depth == 0 {
                    return Ok(cursor);
                }
            }
            _ => {
                cursor = memchr::memchr3(b'\\', b'{', b'}', &source.as_bytes()[cursor + 1..end])
                    .map_or(end, |offset| cursor + 1 + offset)
            }
        }
    }
    Err(syntax_error(start, "unterminated braced value"))
}

fn quoted_value_end(source: &str, start: usize, end: usize) -> Result<usize, FormatError> {
    let mut braces = 0usize;
    let mut cursor = start + 1;
    while cursor < end {
        match source.as_bytes()[cursor] {
            b'\\' => {
                cursor += 1;
                if cursor < end {
                    cursor += source[cursor..].chars().next().unwrap().len_utf8();
                }
            }
            b'{' => {
                braces += 1;
                cursor += 1;
            }
            b'}' if braces > 0 => {
                braces -= 1;
                cursor += 1;
            }
            b'}' => return Err(syntax_error(cursor, "unmatched '}' in quoted value")),
            b'"' if braces == 0 => return Ok(cursor + 1),
            _ => {
                let start = cursor + 1;
                let structural = memchr::memchr3(b'\\', b'{', b'}', &source.as_bytes()[start..end])
                    .map_or(end, |offset| start + offset);
                cursor = memchr::memchr(b'"', &source.as_bytes()[start..structural])
                    .map_or(structural, |offset| start + offset);
            }
        }
    }
    Err(syntax_error(start, "unterminated quoted value"))
}

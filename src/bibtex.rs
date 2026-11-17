//! BibTeX layout with a dedicated value lexer.
//!
//! Defaults align assignments while preserving name case, field order and
//! the source of every value expression. Braced
//! and quoted values, concatenation, macros, string definitions, and preambles
//! are parsed without applying LaTeX whitespace transformations inside values.

use crate::syntax::{self, FormatDirectiveState};
use crate::{CompiledConfig, Diagnostic, FormatError, FormatOutput, Severity};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::ops::Range;

mod cleanup;
mod normalization;
pub use cleanup::CleanupConfig;

/// BibTeX layout and sorting, with optional data edits in `cleanup`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct BibtexConfig {
    /// Normalize assignment layout. Field alignment also requires `align_fields`.
    pub normalize: bool,
    /// `preserve`, `lower`, or `upper` for entry types and regular field names.
    pub name_case: String,
    /// Align regular field assignments while normalizing.
    pub align_fields: bool,
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
            normalize: true,
            name_case: "preserve".into(),
            align_fields: true,
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
    let mut source_map = Vec::new();
    let maximum_blank_lines = if config.config.whitespace.preserve_blank_lines {
        config.config.whitespace.max_blank_lines
    } else {
        Some(0)
    };
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
        cursor += 1;
        let type_start = cursor;
        while cursor < source.len()
            && (bytes[cursor].is_ascii_alphabetic() || bytes[cursor] == b'_')
        {
            cursor += 1;
        }
        if cursor == type_start {
            continue;
        }
        let kind = &source[type_start..cursor];
        let open = skip_whitespace(source, cursor, source.len());
        if !matches!(bytes.get(open), Some(b'{' | b'(')) {
            if kind.eq_ignore_ascii_case("comment") {
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
                source, body_start, close, &opening, closing, &base, &indent, newline,
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
        outside_blanks = 0;
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
    Ok(text)
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
        cursor += 1;
        let type_start = cursor;
        while bytes
            .get(cursor)
            .is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_')
        {
            cursor += 1;
        }
        if cursor == type_start {
            continue;
        }
        let kind = &source[type_start..cursor];
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
                let start = cursor + 1;
                let mut finish = start;
                while bytes
                    .get(finish)
                    .is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_')
                {
                    finish += 1;
                }
                if source[start..finish].eq_ignore_ascii_case("comment") {
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
            if key
                .chars()
                .any(|c| c.is_whitespace() || matches!(c, '{' | '}' | '(' | ')' | '"' | '='))
            {
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
        if key.is_empty()
            || key
                .chars()
                .any(|c| c.is_whitespace() || matches!(c, '{' | '}' | '(' | ')' | '"' | '='))
        {
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
        output.push_str(&source[value.clone()]);
        if let Some(origins) = &mut origins {
            origins.push((printed_start..output.len(), value));
        }
        // BibTeX string commands accept no automatically added trailing comma.
        // Preserve an authored comma, including separators between assignments.
        if !string_definition || source.as_bytes().get(next) == Some(&b',') {
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
    output.push_str(&source[value]);
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
        cursor = match source.as_bytes()[cursor] {
            b'{' => braced_value_end(source, cursor, end)?,
            b'"' => quoted_value_end(source, cursor, end)?,
            b'#' | b',' | b'}' | b')' | b'=' | b'%' => {
                return Err(syntax_error(cursor, "expected value token"));
            }
            _ => {
                let token_start = cursor;
                while cursor < end
                    && !source.as_bytes()[cursor].is_ascii_whitespace()
                    && !matches!(
                        source.as_bytes()[cursor],
                        b'#' | b',' | b'{' | b'}' | b'(' | b')' | b'"' | b'=' | b'%'
                    )
                {
                    cursor += source[cursor..].chars().next().unwrap().len_utf8();
                }
                if cursor == token_start {
                    return Err(syntax_error(cursor, "invalid value token"));
                }
                cursor
            }
        };
        last_token_end = cursor;
        let next = skip_trivia(source, cursor, end).0;
        if next < end && source.as_bytes()[next] == b'#' {
            cursor = next + 1;
            continue;
        }
        return Ok((start..last_token_end, last_token_end));
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

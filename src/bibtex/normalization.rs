//! Normalization and ordering reuse the value lexer to preserve expression syntax.

use super::{
    BibtexConfig, braced_value_end, entry_indent, entry_type, find_comment_end, find_entry_end,
    line_end, protected_ranges, quoted_value_end, read_expression, skip_trivia, skip_whitespace,
    syntax_error, valid_citation_key, value_token_end, write_comments,
};
use crate::{Diagnostic, FormatError, Severity};
use std::borrow::Cow;
use std::cmp::{Ordering, Reverse};
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::ops::Range;
use unicode_width::UnicodeWidthStr;

const CANONICAL_FIELDS: &[&str] = &[
    "title",
    "subtitle",
    "titleaddon",
    "shorttitle",
    "author",
    "shortauthor",
    "editor",
    "shorteditor",
    "editortype",
    "translator",
    "bookauthor",
    "journaltitle",
    "journal",
    "journalsubtitle",
    "shortjournal",
    "booktitle",
    "booksubtitle",
    "booktitleaddon",
    "maintitle",
    "mainsubtitle",
    "maintitleaddon",
    "eventtitle",
    "eventtitleaddon",
    "date",
    "year",
    "month",
    "day",
    "eventdate",
    "publisher",
    "institution",
    "school",
    "organization",
    "location",
    "address",
    "series",
    "volume",
    "volumes",
    "number",
    "issue",
    "edition",
    "version",
    "chapter",
    "part",
    "pages",
    "pagetotal",
    "eid",
    "doi",
    "eprint",
    "eprinttype",
    "archiveprefix",
    "eprintclass",
    "primaryclass",
    "isbn",
    "issn",
    "url",
    "urldate",
    "type",
    "howpublished",
    "pubstate",
    "language",
    "langid",
    "note",
    "addendum",
    "abstract",
    "annotation",
    "annote",
    "keywords",
    "crossref",
    "xref",
    "xdata",
    "related",
    "relatedtype",
    "relatedstring",
    "file",
];

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':'))
}

pub(super) fn validate(config: &BibtexConfig) -> Result<(), String> {
    if config
        .related_columns
        .is_some_and(|width| !(1..=1_000_000).contains(&width))
    {
        return Err("bibtex.related-columns must be between 1 and 1000000".into());
    }
    if config
        .keyword_columns
        .is_some_and(|width| !(1..=1_000_000).contains(&width))
    {
        return Err("bibtex.keyword-columns must be between 1 and 1000000".into());
    }
    if config
        .name_list_columns
        .is_some_and(|width| !(1..=1_000_000).contains(&width))
    {
        return Err("bibtex.name-list-columns must be between 1 and 1000000".into());
    }
    if config
        .concat_columns
        .is_some_and(|width| !(1..=1_000_000).contains(&width))
    {
        return Err("bibtex.concat-columns must be between 1 and 1000000".into());
    }
    if !matches!(config.name_case.as_str(), "preserve" | "lower" | "upper") {
        return Err("bibtex.name-case must be preserve, lower, or upper".into());
    }
    if !matches!(
        config.value_delimiters.as_str(),
        "preserve" | "braces" | "quotes"
    ) {
        return Err("bibtex.value-delimiters must be preserve, braces, or quotes".into());
    }
    if config.sort_by.is_empty() {
        return Err("bibtex.sort-by must contain at least one criterion".into());
    }
    if config
        .sort_by
        .iter()
        .any(|criterion| !valid_name(criterion.strip_prefix(['-', '+']).unwrap_or(criterion)))
    {
        return Err(
            "bibtex.sort-by requires field names or key, optionally prefixed with + or -".into(),
        );
    }
    if config.field_order.iter().any(|name| !valid_name(name)) {
        return Err("bibtex.field-order requires nonempty field names".into());
    }
    if config.field_order.len() > 1
        && config
            .field_order
            .iter()
            .any(|name| matches!(name.as_str(), "canonical" | "alphabetical"))
    {
        return Err("bibtex.field-order canonical and alphabetical must be used alone".into());
    }
    let mut unique = HashSet::new();
    if config
        .field_order
        .iter()
        .any(|name| !unique.insert(name.to_ascii_lowercase()))
    {
        return Err("bibtex.field-order contains repeated field names".into());
    }
    Ok(())
}

pub(super) struct Field<'a> {
    pub(super) name: &'a str,
    pub(super) value: Cow<'a, str>,
    pub(super) span: Range<usize>,
    pub(super) before: Vec<&'a str>,
    pub(super) after: Vec<&'a str>,
}

pub(super) struct Fields<'a> {
    pub(super) key: Cow<'a, str>,
    pub(super) fields: Vec<Field<'a>>,
    pub(super) trailing: Vec<&'a str>,
}

pub(super) fn parse_fields(
    source: &str,
    start: usize,
    end: usize,
) -> Result<Fields<'_>, FormatError> {
    let (mut cursor, mut before) = skip_trivia(source, start, end);
    let comma = source[cursor..end].find(',').map(|offset| cursor + offset);
    let key_end = comma.unwrap_or(end);
    let key = source[cursor..key_end].trim();
    if !valid_citation_key(key, source.as_bytes()[start - 1] == b'{') {
        return Err(syntax_error(cursor, "invalid citation key"));
    }
    let Some(comma) = comma else {
        return Ok(Fields {
            key: Cow::Borrowed(key),
            fields: Vec::new(),
            trailing: before,
        });
    };
    cursor = comma + 1;
    let mut fields = Vec::new();
    loop {
        let (next, comments) = skip_trivia(source, cursor, end);
        cursor = next;
        before.extend(comments);
        if cursor == end {
            return Ok(Fields {
                key: Cow::Borrowed(key),
                fields,
                trailing: before,
            });
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
        before.extend(comments);
        if source.as_bytes().get(cursor) != Some(&b'=') {
            return Err(syntax_error(name_start, "expected field assignment"));
        }
        let (next, comments) = skip_trivia(source, cursor + 1, end);
        cursor = next;
        before.extend(comments);
        let (value, next) = read_expression(source, cursor, end)?;
        let (next, mut after) = skip_trivia(source, next, end);
        cursor = next;
        if cursor < end {
            if source.as_bytes()[cursor] != b',' {
                return Err(syntax_error(cursor, "expected comma between fields"));
            }
            cursor += 1;
            // A comment immediately following the comma binds to this field.
            let horizontal = cursor;
            while cursor < end && matches!(source.as_bytes()[cursor], b' ' | b'\t') {
                cursor += 1;
            }
            if source.as_bytes().get(cursor) == Some(&b'%') {
                let finish = line_end(source, cursor, end);
                after.push(source[cursor..finish].trim_end_matches(['\r', '\n']));
                cursor = finish;
                loop {
                    let next = skip_whitespace(source, cursor, end);
                    if source[cursor..next].matches('\n').count() > 0
                        || source.as_bytes().get(next) != Some(&b'%')
                    {
                        break;
                    }
                    let finish = line_end(source, next, end);
                    after.push(source[next..finish].trim_end_matches(['\r', '\n']));
                    cursor = finish;
                }
            } else {
                cursor = horizontal;
            }
        }
        fields.push(Field {
            name,
            value: Cow::Borrowed(&source[value.clone()]),
            span: value,
            before,
            after,
        });
        before = Vec::new();
    }
}

fn field_order(fields: &mut [Field<'_>], order: &[String]) {
    if order.is_empty() {
        return;
    }
    if order[0] == "alphabetical" {
        fields.sort_by_cached_key(|field| field.name.to_ascii_lowercase());
        return;
    }
    let canonical = order[0] == "canonical";
    let ranks: HashMap<_, _> = if canonical {
        CANONICAL_FIELDS
            .iter()
            .enumerate()
            .map(|(rank, name)| ((*name).to_owned(), rank))
            .collect()
    } else {
        order
            .iter()
            .enumerate()
            .map(|(rank, name)| (name.to_ascii_lowercase(), rank))
            .collect()
    };
    fields.sort_by_cached_key(|field| {
        let name = field.name.to_ascii_lowercase();
        match ranks.get(&name) {
            Some(&rank) => (0, rank, String::new()),
            None => (1, 0, if canonical { name } else { String::new() }),
        }
    });
}

pub(super) fn write_value(output: &mut String, value: &str, field: &str, config: &BibtexConfig) {
    let mode = config.value_delimiters.as_str();
    let preserve_spaces = config.concat_columns.is_some()
        && value.contains('#')
        && !value.is_empty()
        && value_token_end(value, 0, value.len()).is_ok_and(|end| end < value.len());
    if (mode == "preserve" && (preserve_spaces || !value.contains("  ")))
        || matches!(
            field.to_ascii_lowercase().as_str(),
            "url" | "doi" | "eprint" | "file" | "pdf" | "verba" | "verbb" | "verbc" | "execute"
        )
    {
        output.push_str(value);
        return;
    }
    let mut cursor = 0;
    let mut copied = 0;
    while cursor < value.len() {
        cursor = crate::trivia::skip_until(value, cursor, value.len());
        if cursor == value.len() {
            break;
        }
        let opening = value.as_bytes()[cursor];
        if opening == b'#' {
            cursor += 1;
            continue;
        }
        // Cleanup can supply its own value. Leave an unrecognized remainder intact.
        let Ok(end) = value_token_end(value, cursor, value.len()) else {
            break;
        };
        if matches!(opening, b'{' | b'"') {
            let inner = &value[cursor + 1..end - 1];
            let (left, right) = if mode == "braces" || opening == b'{' && mode != "quotes" {
                ('{', '}')
            } else if opening == b'"' || quote_safe(inner) {
                ('"', '"')
            } else {
                ('{', '}')
            };
            output.push_str(&value[copied..cursor]);
            output.push(left);
            if preserve_spaces {
                output.push_str(inner);
            } else {
                write_literal(output, inner);
            }
            output.push(right);
            copied = end;
        }
        cursor = end;
    }
    output.push_str(&value[copied..]);
}

fn write_literal(output: &mut String, literal: &str) {
    let mut copied = 0;
    let mut offset = 0;
    for (index, line) in literal.split_inclusive('\n').enumerate() {
        let end = line
            .match_indices('%')
            .find(|&(at, _)| !crate::syntax::is_escaped(line, at))
            .map_or(line.len(), |(at, _)| at);
        let text = &line[..end];
        // Preserve physical line margins and any explicit TeX control space.
        let start = if index == 0 {
            0
        } else {
            text.len() - text.trim_start_matches([' ', '\t']).len()
        };
        let end = if line.ends_with('\n') && end == line.len() {
            text.trim_end_matches([' ', '\t', '\r', '\n']).len()
        } else {
            text.len()
        };
        if start < end {
            for spaces in crate::trivia::repeated_spaces(&text[start..end]) {
                let at = start + spaces.start;
                if !crate::syntax::is_escaped(line, at) {
                    output.push_str(&literal[copied..offset + at + 1]);
                    copied = offset + start + spaces.end;
                }
            }
        }
        offset += line.len();
    }
    output.push_str(&literal[copied..]);
}

fn quote_safe(inner: &str) -> bool {
    let mut braces = 0usize;
    let mut escaped = false;
    for character in inner.chars() {
        // BibTeX closes a quoted value at every outer-level quote. A
        // backslash before the quote belongs to TeX's accent command and does
        // not escape BibTeX's delimiter.
        if character == '"' && braces == 0 {
            return false;
        }
        if escaped {
            escaped = false;
        } else {
            match character {
                '\\' => escaped = true,
                '{' => braces += 1,
                '}' => braces = braces.saturating_sub(1),
                _ => (),
            }
        }
    }
    !escaped && braces == 0
}

fn wrap_related(
    output: &mut String,
    start: usize,
    base: &str,
    indent: &str,
    newline: &str,
    columns: usize,
) {
    let field = output.rsplit('\n').next().unwrap_or("");
    let value = &output[start..];
    if !value.starts_with(['{', '"'])
        || (!value.contains('\n') && UnicodeWidthStr::width(field) < columns)
        || value_token_end(value, 0, value.len()).ok() != Some(value.len())
    {
        return;
    }
    let inner = &value[1..value.len() - 1];
    if !inner.contains(',') {
        return;
    }
    let keys = inner
        .split(',')
        .map(|key| key.trim_matches(|c: char| c.is_ascii_whitespace()));
    if !keys.clone().all(|key| {
        !key.is_empty() && valid_citation_key(key, true) && !key.contains(['%', '#', '\\'])
    }) {
        return;
    }
    let mut formatted = String::from(&value[..1]);
    let unit = indent.strip_prefix(base).unwrap_or(indent);
    for (i, key) in keys.enumerate() {
        if i != 0 {
            formatted.push(',');
        }
        formatted.push_str(newline);
        formatted.push_str(indent);
        formatted.push_str(unit);
        formatted.push_str(key);
    }
    formatted.push_str(newline);
    formatted.push_str(indent);
    formatted.push_str(&value[value.len() - 1..]);
    output.replace_range(start.., &formatted);
}

fn wrap_literal_list(
    output: &mut String,
    start: usize,
    indent: &str,
    newline: &str,
    columns: usize,
    keywords: bool,
) {
    let value = &output[start..];
    if !value.starts_with(['{', '"'])
        || value.contains('%')
        || value_token_end(value, 0, value.len()).ok() != Some(value.len())
    {
        return;
    }
    let prefix = output[..start].rsplit('\n').next().unwrap_or("");
    if !value.contains('\n')
        && UnicodeWidthStr::width(prefix) + UnicodeWidthStr::width(value) < columns
    {
        return;
    }
    let inner = &value[1..value.len() - 1];
    let separators = if keywords {
        let mut ends = Vec::new();
        let mut pos = 0;
        while pos < inner.len() {
            match inner.as_bytes()[pos] {
                b'{' => {
                    let Some(end) = crate::syntax::balanced_end(inner, pos, b'{', b'}') else {
                        return;
                    };
                    pos = end;
                }
                b'\\' => {
                    let Some((_, end)) = crate::syntax::command_at(inner, pos) else {
                        return;
                    };
                    pos = end;
                }
                b',' => {
                    pos += 1;
                    ends.push(pos);
                }
                _ => pos += 1,
            }
        }
        ends
    } else {
        super::cleanup::name_separators(inner)
            .into_iter()
            .filter(|&at| !crate::syntax::is_escaped(value, at))
            .map(|at| at + "and".len())
            .collect()
    };
    if separators.is_empty() {
        return;
    }
    let padding = UnicodeWidthStr::width(prefix.strip_prefix(indent).unwrap_or(prefix)) + 1;
    let continuation = format!("{newline}{indent}{}", " ".repeat(padding));
    let value = output.split_off(start);
    let mut copied = 0;
    for end in separators {
        let start = 1 + end;
        let tail = &value[start..];
        // Preserve explicit TeX spaces and nonbreaking Unicode whitespace.
        let whitespace = tail.len()
            - tail
                .trim_start_matches(|c: char| c.is_ascii_whitespace())
                .len();
        if (!keywords && whitespace == 0) || (keywords && start + whitespace == value.len() - 1) {
            continue;
        }
        output.push_str(&value[copied..start]);
        output.push_str(&continuation);
        copied = start + whitespace;
    }
    output.push_str(&value[copied..]);
}

pub(super) fn wrap_concat(
    output: &mut String,
    start: usize,
    indent: &str,
    newline: &str,
    columns: usize,
    comma: bool,
) {
    let value = &output[start..];
    if !value.contains('#') {
        return;
    }
    let prefix = output[..start].rsplit('\n').next().unwrap_or("");
    let mut margin = UnicodeWidthStr::width(prefix);
    let exceeds = value.split_inclusive('\n').any(|line| {
        let width = margin + UnicodeWidthStr::width(line.trim_end_matches(['\r', '\n']));
        margin = 0;
        width + usize::from(comma && !line.ends_with('\n')) > columns
    });
    if !exceeds {
        return;
    }
    let mut gaps = Vec::new();
    let mut cursor = 0;
    while cursor < value.len() {
        let Ok(end) = value_token_end(value, cursor, value.len()) else {
            return;
        };
        if end == value.len() {
            break;
        }
        let operator = skip_whitespace(value, end, value.len());
        if value.as_bytes().get(operator) != Some(&b'#') {
            // Preserve comments between operands and their operators.
            return;
        }
        cursor = skip_whitespace(value, operator + 1, value.len());
        gaps.push(end..cursor);
    }
    if gaps.is_empty() {
        return;
    }
    let padding = UnicodeWidthStr::width(prefix.strip_prefix(indent).unwrap_or(prefix));
    let continuation = format!("{newline}{indent}{}# ", " ".repeat(padding));
    let value = output.split_off(start);
    let mut copied = 0;
    for gap in gaps {
        output.push_str(&value[copied..gap.start]);
        output.push_str(&continuation);
        copied = gap.end;
    }
    output.push_str(&value[copied..]);
}

#[allow(clippy::too_many_arguments)]
pub(super) fn format_fields(
    source: &str,
    start: usize,
    end: usize,
    opening: &str,
    closing: &str,
    base: &str,
    indent: &str,
    newline: &str,
    config: &BibtexConfig,
    origins: Option<&mut Vec<(Range<usize>, Range<usize>)>>,
) -> Result<String, FormatError> {
    let mut entry = parse_fields(source, start, end)?;
    Ok(format_parsed_fields(
        source,
        &mut entry,
        Some(start..end),
        opening,
        closing,
        base,
        indent,
        newline,
        config,
        origins,
    ))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn format_parsed_fields(
    source: &str,
    entry: &mut Fields<'_>,
    original_body: Option<Range<usize>>,
    opening: &str,
    closing: &str,
    base: &str,
    indent: &str,
    newline: &str,
    config: &BibtexConfig,
    mut origins: Option<&mut Vec<(Range<usize>, Range<usize>)>>,
) -> String {
    field_order(&mut entry.fields, &config.field_order);
    let mut output = opening.to_owned();
    output.push_str(&entry.key);
    if entry.fields.is_empty() {
        if !entry.trailing.is_empty() || entry.key.is_empty() {
            if let Some(body) = original_body {
                output.truncate(opening.len());
                output.push_str(&source[body]);
            } else {
                output.push(',');
                output.push_str(newline);
                write_comments(&mut output, &entry.trailing, indent, newline);
                output.push_str(base);
            }
        }
        output.push_str(closing);
        return output;
    }
    output.push(',');
    output.push_str(newline);
    let width = if config.normalize && config.align_fields {
        entry
            .fields
            .iter()
            .map(|field| UnicodeWidthStr::width(field.name))
            .max()
            .unwrap_or(0)
    } else {
        0
    };
    for field in &entry.fields {
        write_comments(&mut output, &field.before, indent, newline);
        output.push_str(indent);
        let name = config.name(field.name);
        output.push_str(&name);
        output.extend(std::iter::repeat_n(
            ' ',
            width.saturating_sub(UnicodeWidthStr::width(name.as_ref())),
        ));
        output.push_str(" = ");
        let printed_start = output.len();
        write_value(&mut output, &field.value, field.name, config);
        if let Some(columns) = config.name_list_columns
            && (field.name.eq_ignore_ascii_case("author")
                || field.name.eq_ignore_ascii_case("editor"))
        {
            wrap_literal_list(&mut output, printed_start, indent, newline, columns, false);
        }
        if field.name.eq_ignore_ascii_case("keywords")
            && let Some(columns) = config.keyword_columns
        {
            wrap_literal_list(&mut output, printed_start, indent, newline, columns, true);
        }
        if field.name.eq_ignore_ascii_case("related")
            && let Some(columns) = config.related_columns
        {
            wrap_related(&mut output, printed_start, base, indent, newline, columns);
        }
        if let Some(columns) = config.concat_columns {
            wrap_concat(&mut output, printed_start, indent, newline, columns, true);
        }
        if let Some(origins) = &mut origins {
            origins.push((printed_start..output.len(), field.span.clone()));
        }
        output.push(',');
        if let Some(comment) = field.after.first() {
            output.push(' ');
            output.push_str(comment);
        }
        output.push_str(newline);
        if field.after.len() > 1 {
            write_comments(&mut output, &field.after[1..], indent, newline);
        }
    }
    write_comments(&mut output, &entry.trailing, indent, newline);
    output.push_str(base);
    output.push_str(closing);
    output
}

struct Entry {
    range: Range<usize>,
    key: String,
    kind: String,
    values: HashMap<String, String>,
    indent: String,
    resolvable: bool,
}

fn resolve_value(
    source: &str,
    value: Range<usize>,
    macros: &HashMap<String, Option<String>>,
    remaining: &mut usize,
) -> Option<String> {
    let mut result = String::new();
    let mut cursor = value.start;
    while cursor < value.end {
        cursor = skip_trivia(source, cursor, value.end).0;
        if cursor == value.end {
            break;
        }
        match source.as_bytes()[cursor] {
            b'{' | b'"' => {
                let end = if source.as_bytes()[cursor] == b'{' {
                    braced_value_end(source, cursor, value.end)
                } else {
                    quoted_value_end(source, cursor, value.end)
                };
                let Ok(end) = end else {
                    return None;
                };
                let piece = &source[cursor + 1..end - 1];
                *remaining = remaining.checked_sub(piece.len())?;
                result.push_str(piece);
                cursor = end;
            }
            b'#' => cursor += 1,
            _ => {
                let start = cursor;
                while cursor < value.end
                    && !source.as_bytes()[cursor].is_ascii_whitespace()
                    && !matches!(source.as_bytes()[cursor], b'#' | b'%')
                {
                    cursor += source[cursor..].chars().next().unwrap().len_utf8();
                }
                if start == cursor {
                    cursor += 1;
                    continue;
                }
                let token = &source[start..cursor];
                let piece = match macros.get(&token.to_ascii_lowercase()) {
                    Some(Some(value)) => value.as_str(),
                    Some(None) => return None,
                    None => token,
                };
                *remaining = remaining.checked_sub(piece.len())?;
                result.push_str(piece);
            }
        }
    }
    Some(result)
}

fn read_string_definitions(
    source: &str,
    mut cursor: usize,
    end: usize,
    macros: &mut HashMap<String, Option<String>>,
    remaining: &mut usize,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<(), FormatError> {
    while cursor < end {
        cursor = skip_trivia(source, cursor, end).0;
        if cursor == end {
            break;
        }
        let start = cursor;
        while cursor < end
            && !source.as_bytes()[cursor].is_ascii_whitespace()
            && source.as_bytes()[cursor] != b'='
        {
            cursor += source[cursor..].chars().next().unwrap().len_utf8();
        }
        let name = source[start..cursor].to_ascii_lowercase();
        cursor = skip_trivia(source, cursor, end).0;
        if source.as_bytes().get(cursor) != Some(&b'=') {
            return Err(syntax_error(cursor, "expected string assignment"));
        }
        cursor = skip_trivia(source, cursor + 1, end).0;
        let (value, next) = read_expression(source, cursor, end)?;
        let resolved = resolve_value(source, value.clone(), macros, remaining);
        if resolved.is_none() {
            expansion_warning(diagnostics, value, &name);
        }
        macros.insert(name, resolved);
        cursor = skip_trivia(source, next, end).0;
        if source.as_bytes().get(cursor) == Some(&b',') {
            cursor += 1;
        }
    }
    Ok(())
}

pub(super) fn sort_entries(
    source: &str,
    config: &BibtexConfig,
) -> Result<(String, Vec<Diagnostic>), FormatError> {
    let protected = protected_ranges(source)?;
    let mut entries = Vec::new();
    let mut macros = HashMap::new();
    // Bound all materialized sort values together by the input size. Source
    // order and raw expressions remain available when expansion cannot finish.
    let mut remaining = source.len().saturating_mul(4);
    let mut diagnostics = Vec::new();
    let mut needed: HashSet<_> = config
        .sort_by
        .iter()
        .map(|criterion| {
            criterion
                .strip_prefix(['-', '+'])
                .unwrap_or(criterion)
                .to_ascii_lowercase()
        })
        .chain(["crossref".into(), "xref".into(), "xdata".into()])
        .collect();
    if needed.contains("year") {
        needed.insert("date".into());
    }
    let mut cursor = 0;
    while cursor < source.len() {
        let index = protected.partition_point(|range| range.end <= cursor);
        if let Some(range) = protected.get(index).filter(|range| range.start <= cursor) {
            cursor = range.end;
            continue;
        }
        if source.as_bytes()[cursor] == b'%' {
            cursor = line_end(source, cursor, source.len());
            continue;
        }
        if source.as_bytes()[cursor] != b'@' {
            cursor = memchr::memchr2(b'@', b'%', &source.as_bytes()[cursor + 1..])
                .map_or(source.len(), |offset| cursor + 1 + offset);
            continue;
        }
        let start = cursor;
        let Some((kind, type_end)) = entry_type(source, cursor) else {
            cursor += 1;
            continue;
        };
        let kind = kind.to_ascii_lowercase();
        cursor = type_end;
        let open = skip_whitespace(source, cursor, source.len());
        if !matches!(source.as_bytes().get(open), Some(b'{' | b'(')) {
            cursor = line_end(source, cursor, source.len());
            continue;
        }
        let close = if kind == "comment" {
            find_comment_end(source, open)?
        } else {
            find_entry_end(source, open)?
        };
        cursor = close + 1;
        if kind == "string" {
            read_string_definitions(
                source,
                open + 1,
                close,
                &mut macros,
                &mut remaining,
                &mut diagnostics,
            )?;
            continue;
        }
        if matches!(kind.as_str(), "comment" | "preamble") {
            continue;
        }
        let fields = parse_fields(source, open + 1, close)?;
        // Without a citation key this record is a boundary, even if it is
        // the only anonymous record in the bibliography.
        if fields.key.is_empty() {
            continue;
        }
        let mut values = HashMap::new();
        let mut resolvable = true;
        for field in fields.fields {
            let name = field.name.to_ascii_lowercase();
            if needed.contains(&name) {
                values.entry(name).or_insert_with(|| {
                    resolve_value(source, field.span.clone(), &macros, &mut remaining)
                        .unwrap_or_else(|| {
                            resolvable = false;
                            expansion_warning(&mut diagnostics, field.span.clone(), field.name);
                            field.value.into_owned()
                        })
                });
            }
        }
        // A standalone source comment immediately above a record, or a note
        // beside its closing delimiter, stays with that record during sorting.
        let line_start = source[..start].rfind('\n').map_or(0, |i| i + 1);
        let attached_comment = config.separate_entries
            && (line_start > 0
                && !crate::syntax::is_protected(line_start - 1, &protected)
                && source[..line_start]
                    .lines()
                    .next_back()
                    .is_some_and(|line| line.trim_start().starts_with('%'))
                || source[cursor..]
                    .trim_start_matches([' ', '\t'])
                    .starts_with('%'));
        entries.push(Entry {
            range: start..cursor,
            key: fields.key.into(),
            kind,
            values,
            indent: entry_indent(source, start),
            resolvable: resolvable && !attached_comment,
        });
    }
    let mut counts = HashMap::new();
    for entry in &entries {
        *counts.entry(entry.key.as_str()).or_insert(0usize) += 1;
    }
    let mut output = String::with_capacity(source.len());
    let mut copied = 0;
    let mut begin = 0;
    while begin < entries.len() {
        let mut end = begin + 1;
        if counts[entries[begin].key.as_str()] == 1 && entries[begin].resolvable {
            while end < entries.len()
                && counts[entries[end].key.as_str()] == 1
                && entries[end].resolvable
                && source[entries[end - 1].range.end..entries[end].range.start]
                    .trim()
                    .is_empty()
                && entries[end].indent == entries[begin].indent
            {
                end += 1;
            }
        }
        output.push_str(&source[copied..entries[begin].range.start]);
        let run = &entries[begin..end];
        let order = ordered_run(run, config);
        for (slot, &entry_index) in order.iter().enumerate() {
            if slot > 0 {
                output.push_str(&source[run[slot - 1].range.end..run[slot].range.start]);
            }
            output.push_str(&source[run[entry_index].range.clone()]);
        }
        copied = entries[end - 1].range.end;
        begin = end;
    }
    output.push_str(&source[copied..]);
    Ok((output, diagnostics))
}

fn expansion_warning(diagnostics: &mut Vec<Diagnostic>, span: Range<usize>, name: &str) {
    diagnostics.push(Diagnostic {
        severity: Severity::Warning,
        message: format!("BibTeX sort preserved {name} expression: string expansion exceeds the input-sized allocation budget or depends on an unresolved string"),
        span,
    });
}

fn plain(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut whitespace = false;
    for character in text.chars() {
        if matches!(character, '{' | '}') {
            continue;
        }
        if character.is_whitespace() {
            whitespace = !output.is_empty();
        } else {
            if whitespace {
                output.push(' ');
                whitespace = false;
            }
            output.push(character);
        }
    }
    output
}

fn top_level_split<'a>(text: &'a str, delimiter: &str) -> Vec<&'a str> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut cursor = 0;
    let mut braces = 0usize;
    while cursor < text.len() {
        match text.as_bytes()[cursor] {
            b'\\' => {
                cursor += 1;
                if cursor < text.len() {
                    cursor += text[cursor..].chars().next().unwrap().len_utf8();
                }
            }
            b'{' => {
                braces += 1;
                cursor += 1;
            }
            b'}' => {
                braces = braces.saturating_sub(1);
                cursor += 1;
            }
            _ if braces == 0
                && text[cursor..]
                    .get(..delimiter.len())
                    .is_some_and(|part| part.eq_ignore_ascii_case(delimiter)) =>
            {
                result.push(&text[start..cursor]);
                cursor += delimiter.len();
                start = cursor;
            }
            _ => cursor += text[cursor..].chars().next().unwrap().len_utf8(),
        }
    }
    result.push(&text[start..]);
    result
}

fn author_key(value: &str) -> String {
    let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
    top_level_split(&value, " and ")
        .into_iter()
        .map(|author| {
            let author = author.trim();
            if author.starts_with('{')
                && braced_value_end(author, 0, author.len()).is_ok_and(|end| end == author.len())
            {
                return plain(author);
            }
            let comma = top_level_split(author, ",");
            if comma.len() > 1 {
                return format!(
                    "{}\0{}",
                    plain(comma[0]),
                    comma[1..]
                        .iter()
                        .map(|part| plain(part))
                        .collect::<Vec<_>>()
                        .join("\0")
                );
            }
            let words: Vec<_> = author.split_whitespace().collect();
            if words.len() < 2 {
                return plain(author);
            }
            let family = words
                .iter()
                .take(words.len() - 1)
                .position(|word| {
                    word.trim_start_matches('{')
                        .chars()
                        .find(|character| character.is_alphabetic())
                        .is_some_and(char::is_lowercase)
                })
                .unwrap_or(words.len() - 1);
            format!(
                "{}\0{}",
                plain(&words[family..].join(" ")),
                plain(&words[..family].join(" "))
            )
        })
        .collect::<Vec<_>>()
        .join("\u{1}")
}

fn sort_value(entry: &Entry, criterion: &str, case_sensitive: bool) -> String {
    let value = match criterion {
        "key" | "entry-key" => entry.key.clone(),
        "entry-type" => entry.kind.clone(),
        "author" | "editor" | "translator" | "sortname" => entry
            .values
            .get(criterion)
            .map_or_else(String::new, |value| author_key(value)),
        "year" => {
            let value = entry
                .values
                .get("year")
                .or_else(|| entry.values.get("date"))
                .map_or("", String::as_str);
            let value = plain(value);
            if !entry.values.contains_key("year")
                && value
                    .as_bytes()
                    .get(..4)
                    .is_some_and(|bytes| bytes.iter().all(u8::is_ascii_digit))
            {
                value[..4].into()
            } else {
                value
            }
        }
        _ => entry
            .values
            .get(criterion)
            .map_or_else(String::new, |value| plain(value)),
    };
    if case_sensitive {
        value
    } else {
        value.to_lowercase()
    }
}

fn compare_value(left: &str, right: &str, numeric: bool, descending: bool) -> Ordering {
    match (left.is_empty(), right.is_empty()) {
        (true, false) => return Ordering::Greater,
        (false, true) => return Ordering::Less,
        _ => (),
    }
    let order = if numeric {
        match (left.parse::<i64>(), right.parse::<i64>()) {
            (Ok(left), Ok(right)) => left.cmp(&right),
            (Ok(_), Err(_)) => Ordering::Less,
            (Err(_), Ok(_)) => Ordering::Greater,
            _ => left.cmp(right),
        }
    } else {
        left.cmp(right)
    };
    if descending { order.reverse() } else { order }
}

fn ordered_run(entries: &[Entry], config: &BibtexConfig) -> Vec<usize> {
    let criteria: Vec<_> = config
        .sort_by
        .iter()
        .map(|criterion| {
            let descending = criterion.starts_with('-');
            (
                criterion
                    .strip_prefix(['-', '+'])
                    .unwrap_or(criterion)
                    .to_ascii_lowercase(),
                descending,
            )
        })
        .collect();
    let values: Vec<Vec<_>> = entries
        .iter()
        .map(|entry| {
            criteria
                .iter()
                .map(|(name, _)| sort_value(entry, name, config.case_sensitive))
                .collect()
        })
        .collect();
    let mut priority: Vec<_> = (0..entries.len()).collect();
    priority.sort_by(|&left, &right| {
        for (index, (criterion, descending)) in criteria.iter().enumerate() {
            let order = compare_value(
                &values[left][index],
                &values[right][index],
                matches!(criterion.as_str(), "year" | "volume" | "number"),
                *descending,
            );
            if order != Ordering::Equal {
                return order;
            }
        }
        Ordering::Equal
    });
    // Preserve every existing dependency's source-relative order. Independent
    // entries can still take their requested positions around that constraint.
    let keys: HashMap<_, _> = entries
        .iter()
        .enumerate()
        .map(|(index, entry)| (entry.key.as_str(), index))
        .collect();
    let mut edges: Vec<HashSet<usize>> = vec![HashSet::new(); entries.len()];
    let mut indegree = vec![0usize; entries.len()];
    for (index, entry) in entries.iter().enumerate() {
        for name in ["crossref", "xref", "xdata"] {
            if let Some(value) = entry.values.get(name) {
                for key in top_level_split(value, ",") {
                    if let Some(&target) = keys.get(plain(key).trim())
                        && index != target
                    {
                        let (from, to) = if index < target {
                            (index, target)
                        } else {
                            (target, index)
                        };
                        if edges[from].insert(to) {
                            indegree[to] += 1;
                        }
                    }
                }
            }
        }
    }
    let mut ranks = vec![0; entries.len()];
    for (rank, &index) in priority.iter().enumerate() {
        ranks[index] = rank;
    }
    let mut ready: BinaryHeap<_> = indegree
        .iter()
        .enumerate()
        .filter(|(_, degree)| **degree == 0)
        .map(|(index, _)| Reverse((ranks[index], index)))
        .collect();
    let mut order = Vec::with_capacity(entries.len());
    while let Some(Reverse((_, next))) = ready.pop() {
        order.push(next);
        for &target in &edges[next] {
            indegree[target] -= 1;
            if indegree[target] == 0 {
                ready.push(Reverse((ranks[target], target)));
            }
        }
    }
    if order.len() == entries.len() {
        order
    } else {
        (0..entries.len()).collect()
    }
}

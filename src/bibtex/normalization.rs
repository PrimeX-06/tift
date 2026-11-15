//! Normalization and ordering reuse the value lexer without rewriting expressions.

use super::{
    BibtexConfig, braced_value_end, entry_indent, find_comment_end, find_entry_end, line_end,
    protected_ranges, quoted_value_end, read_expression, skip_trivia, skip_whitespace,
    syntax_error, write_comments,
};
use crate::FormatError;
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

struct Field<'a> {
    name: &'a str,
    value: Range<usize>,
    before: Vec<&'a str>,
    after: Vec<&'a str>,
}

struct Fields<'a> {
    key: &'a str,
    fields: Vec<Field<'a>>,
    trailing: Vec<&'a str>,
}

fn parse_fields(source: &str, start: usize, end: usize) -> Result<Fields<'_>, FormatError> {
    let (mut cursor, mut before) = skip_trivia(source, start, end);
    let comma = source[cursor..end].find(',').map(|offset| cursor + offset);
    let key_end = comma.unwrap_or(end);
    let key = source[cursor..key_end].trim();
    if key.chars().any(|character| {
        character.is_whitespace() || matches!(character, '{' | '}' | '(' | ')' | '"' | '=')
    }) || comma.is_some() && key.is_empty()
    {
        return Err(syntax_error(cursor, "invalid citation key"));
    }
    let Some(comma) = comma else {
        return Ok(Fields {
            key,
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
                key,
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
            value,
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

fn normalize_value(source: &str, range: Range<usize>, field: &str, mode: &str) -> String {
    let value = &source[range.clone()];
    if mode == "preserve"
        || matches!(
            field.to_ascii_lowercase().as_str(),
            "url" | "doi" | "eprint" | "file" | "pdf"
        )
    {
        return value.into();
    }
    let mut cursor = range.start;
    let mut copied = cursor;
    let mut output = String::with_capacity(value.len());
    while cursor < range.end {
        let (next, _) = skip_trivia(source, cursor, range.end);
        cursor = next;
        if cursor == range.end {
            break;
        }
        let opening = source.as_bytes()[cursor];
        let end = match opening {
            b'{' => braced_value_end(source, cursor, range.end).ok(),
            b'"' => quoted_value_end(source, cursor, range.end).ok(),
            _ => None,
        };
        if let Some(end) = end {
            let inner = &source[cursor + 1..end - 1];
            let convert = mode == "braces" && opening == b'"'
                || mode == "quotes" && opening == b'{' && quote_safe(inner);
            if convert {
                output.push_str(&source[copied..cursor]);
                output.push(if mode == "braces" { '{' } else { '"' });
                output.push_str(inner);
                output.push(if mode == "braces" { '}' } else { '"' });
                copied = end;
            }
            cursor = end;
        } else {
            cursor += source[cursor..].chars().next().unwrap().len_utf8();
        }
    }
    output.push_str(&source[copied..range.end]);
    output
}

fn quote_safe(inner: &str) -> bool {
    let mut braces = 0usize;
    let mut escaped = false;
    for character in inner.chars() {
        if escaped {
            escaped = false;
        } else {
            match character {
                '\\' => escaped = true,
                '{' => braces += 1,
                '}' => braces = braces.saturating_sub(1),
                '"' if braces == 0 => return false,
                _ => (),
            }
        }
    }
    !escaped && braces == 0
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
) -> Result<String, FormatError> {
    let mut entry = parse_fields(source, start, end)?;
    field_order(&mut entry.fields, &config.field_order);
    let mut output = opening.to_owned();
    output.push_str(entry.key);
    if entry.fields.is_empty() {
        if !entry.trailing.is_empty() {
            output.truncate(opening.len());
            output.push_str(&source[start..end]);
        }
        output.push_str(closing);
        return Ok(output);
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
    for field in entry.fields {
        write_comments(&mut output, &field.before, indent, newline);
        output.push_str(indent);
        let name = if config.normalize {
            field.name.to_ascii_lowercase()
        } else {
            field.name.into()
        };
        output.push_str(&name);
        output.extend(std::iter::repeat_n(
            ' ',
            width.saturating_sub(UnicodeWidthStr::width(name.as_str())),
        ));
        output.push_str(" = ");
        output.push_str(&normalize_value(
            source,
            field.value,
            field.name,
            &config.value_delimiters,
        ));
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
    Ok(output)
}

struct Entry {
    range: Range<usize>,
    key: String,
    kind: String,
    values: HashMap<String, String>,
    indent: String,
}

fn resolve_value(source: &str, value: Range<usize>, macros: &HashMap<String, String>) -> String {
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
                    return source[value].into();
                };
                result.push_str(&source[cursor + 1..end - 1]);
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
                result.push_str(
                    macros
                        .get(&token.to_ascii_lowercase())
                        .map_or(token, String::as_str),
                );
            }
        }
    }
    result
}

fn read_string_definitions(
    source: &str,
    mut cursor: usize,
    end: usize,
    macros: &mut HashMap<String, String>,
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
        macros.insert(name, resolve_value(source, value, macros));
        cursor = skip_trivia(source, next, end).0;
        if source.as_bytes().get(cursor) == Some(&b',') {
            cursor += 1;
        }
    }
    Ok(())
}

pub(super) fn sort_entries(source: &str, config: &BibtexConfig) -> Result<String, FormatError> {
    let protected = protected_ranges(source)?;
    let mut entries = Vec::new();
    let mut macros = HashMap::new();
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
            cursor += source[cursor..].chars().next().unwrap().len_utf8();
            continue;
        }
        let start = cursor;
        cursor += 1;
        let type_start = cursor;
        while cursor < source.len()
            && (source.as_bytes()[cursor].is_ascii_alphabetic()
                || source.as_bytes()[cursor] == b'_')
        {
            cursor += 1;
        }
        if cursor == type_start {
            continue;
        }
        let kind = source[type_start..cursor].to_ascii_lowercase();
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
            read_string_definitions(source, open + 1, close, &mut macros)?;
            continue;
        }
        if matches!(kind.as_str(), "comment" | "preamble") {
            continue;
        }
        let fields = parse_fields(source, open + 1, close)?;
        let mut values = HashMap::new();
        for field in fields.fields {
            let name = field.name.to_ascii_lowercase();
            if needed.contains(&name) {
                values
                    .entry(name)
                    .or_insert_with(|| resolve_value(source, field.value, &macros));
            }
        }
        entries.push(Entry {
            range: start..cursor,
            key: fields.key.into(),
            kind,
            values,
            indent: entry_indent(source, start),
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
        if counts[entries[begin].key.as_str()] == 1 {
            while end < entries.len()
                && counts[entries[end].key.as_str()] == 1
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
    Ok(output)
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

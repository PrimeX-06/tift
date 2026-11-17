//! Optional bibliography data edits over the shared BibTeX value parser.

use super::{
    braced_value_end, find_comment_end, find_entry_end, line_end, normalization, protected_ranges,
    quoted_value_end, skip_whitespace,
};
use crate::{Diagnostic, FormatError, Severity};
use normalization::{Field, Fields};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::hash_map::RandomState;
use std::collections::{HashMap, HashSet};
use std::hash::BuildHasher;
use std::ops::Range;

/// Data cleanup is opt-in at both the group and individual operation levels.
/// Existing citation keys, macro expressions and conflicting data survive.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct CleanupConfig {
    /// Master switch. Every cleanup operation remains opt-in when this is false.
    pub enabled: bool,
    /// Convert literal month names and numbers to BibTeX month macros.
    pub months: bool,
    /// Repair all-uppercase literal title fields while respecting TeX braces.
    pub sentence_case_titles: bool,
    /// Remove repeated fields only when their values are identical.
    pub remove_duplicate_fields: bool,
    /// Remove literal fields whose value contains only whitespace.
    pub remove_empty_fields: bool,
    /// Merge compatible records with the same citation key.
    pub merge_duplicate_entries: bool,
    /// Normalize safe literal URL and DOI values.
    pub normalize_urls: bool,
    /// Escape raw TeX special characters in descriptive literal fields.
    pub escape_special_characters: bool,
    /// Generate a citation key only when the source key is empty.
    pub generate_missing_keys: bool,
    /// Retain this many authors followed by BibTeX's `and others` marker.
    pub max_authors: Option<usize>,
}

impl CleanupConfig {
    /// Return whether at least one cleanup operation can inspect or edit data.
    /// Keeping this gate next to the fields prevents callers from maintaining
    /// a second list when new cleanup operations are added.
    pub fn is_active(&self) -> bool {
        self.enabled
            && (self.months
                || self.sentence_case_titles
                || self.remove_duplicate_fields
                || self.remove_empty_fields
                || self.merge_duplicate_entries
                || self.normalize_urls
                || self.escape_special_characters
                || self.generate_missing_keys
                || self.max_authors.is_some())
    }
}

pub(super) fn validate(config: &CleanupConfig) -> Result<(), String> {
    if config.max_authors == Some(0) {
        return Err("bibtex.cleanup.max-authors must be positive".into());
    }
    Ok(())
}

pub(super) struct Entry<'a> {
    pub(super) span: Range<usize>,
    kind: &'a str,
    key: Cow<'a, str>,
    fields: Vec<Field<'a>>,
    trailing: Vec<&'a str>,
    body: Range<usize>,
    run: usize,
    changed: bool,
    pub(super) removed: bool,
}

impl Entry<'_> {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn format(
        self,
        source: &str,
        opening: &str,
        closing: &str,
        base: &str,
        indent: &str,
        newline: &str,
        config: &super::BibtexConfig,
        origins: Option<&mut Vec<(Range<usize>, Range<usize>)>>,
    ) -> String {
        let original_body = (!self.changed).then_some(self.body);
        let mut fields = Fields {
            key: self.key,
            fields: self.fields,
            trailing: self.trailing,
        };
        normalization::format_parsed_fields(
            source,
            &mut fields,
            original_body,
            opening,
            closing,
            base,
            indent,
            newline,
            config,
            origins,
        )
    }
}

pub(super) struct Prepared<'a> {
    pub(super) entries: Vec<Entry<'a>>,
    pub(super) diagnostics: Vec<Diagnostic>,
    pub(super) protected: Vec<Range<usize>>,
}

/// Single literal pieces can be edited without resolving string definitions.
/// Concatenations and bare macro references remain expressions.
fn literal(value: &str) -> Option<(&str, char, char)> {
    let value = value.trim();
    let opening = *value.as_bytes().first()?;
    let end = match opening {
        b'{' => braced_value_end(value, 0, value.len()).ok()?,
        b'"' => quoted_value_end(value, 0, value.len()).ok()?,
        _ => return None,
    };
    (end == value.len()).then(|| {
        (
            &value[1..end - 1],
            char::from(opening),
            if opening == b'{' { '}' } else { '"' },
        )
    })
}

fn entries(
    source: &str,
    allow_empty: bool,
) -> Result<(Vec<Entry<'_>>, Vec<Range<usize>>), FormatError> {
    let protected = protected_ranges(source)?;
    let mut output = Vec::new();
    let mut cursor = 0;
    let mut run = 0;
    let mut previous_end = 0;
    while cursor < source.len() {
        let index = protected.partition_point(|range| range.end <= cursor);
        if let Some(range) = protected.get(index).filter(|range| range.start <= cursor) {
            cursor = range.end;
            run += 1;
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
        cursor += 1;
        let type_start = cursor;
        while source
            .as_bytes()
            .get(cursor)
            .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
        {
            cursor += 1;
        }
        if type_start == cursor {
            continue;
        }
        let kind = &source[type_start..cursor];
        let open = skip_whitespace(source, cursor, source.len());
        if !matches!(source.as_bytes().get(open), Some(b'{' | b'(')) {
            cursor = line_end(source, cursor, source.len());
            run += 1;
            continue;
        }
        let close = if kind.eq_ignore_ascii_case("comment") {
            find_comment_end(source, open)?
        } else {
            find_entry_end(source, open)?
        };
        cursor = close + 1;
        if matches!(
            kind.to_ascii_lowercase().as_str(),
            "comment" | "string" | "preamble"
        ) {
            run += 1;
            previous_end = cursor;
            continue;
        }
        if !source[previous_end..start].trim().is_empty() {
            run += 1;
        }
        previous_end = cursor;
        let parsed = normalization::parse_fields_allow_empty(source, open + 1, close, allow_empty)?;
        output.push(Entry {
            span: start..cursor,
            kind,
            key: parsed.key,
            fields: parsed.fields,
            trailing: parsed.trailing,
            body: open + 1..close,
            run,
            changed: false,
            removed: false,
        });
    }
    Ok((output, protected))
}

pub(super) fn prepare<'a>(
    source: &'a str,
    config: &CleanupConfig,
) -> Result<Option<Prepared<'a>>, FormatError> {
    if !config.is_active() {
        return Ok(None);
    }
    let (mut entries, protected) = entries(source, config.generate_missing_keys)?;
    let mut diagnostics = Vec::new();
    let mut used_keys: HashSet<String> = if config.generate_missing_keys {
        entries.iter().map(|entry| entry.key.to_string()).collect()
    } else {
        HashSet::new()
    };
    let mut next_suffix: HashMap<String, usize> = HashMap::new();
    for entry in &mut entries {
        for field in &mut entry.fields {
            let name = field.name.to_ascii_lowercase();
            if !needs_literal(&name, config) {
                continue;
            }
            let piece = literal(&field.value);
            let month = (name == "month" && config.months)
                .then(|| month(&field.value, piece))
                .flatten();
            if piece.is_none() && needs_literal(&name, config) && month.is_none() {
                diagnostics.push(Diagnostic {
                    severity: Severity::Warning,
                    message: format!("BibTeX cleanup preserved {} expression: a single literal value is required", field.name),
                    span: field.span.clone(),
                });
            }
            if config.normalize_urls
                && matches!(name.as_str(), "url" | "doi")
                && piece.is_some_and(|(inner, _, _)| clean_url(inner).is_none())
            {
                diagnostics.push(Diagnostic {
                    severity: Severity::Warning,
                    message: format!(
                        "BibTeX cleanup preserved {} value containing an unresolved TeX command",
                        field.name
                    ),
                    span: field.span.clone(),
                });
            }
            let value = clean_value(&name, &field.value, piece, month, config);
            entry.changed |= value != field.value;
            if let Cow::Owned(value) = value {
                field.value = Cow::Owned(value);
            }
        }
        clean_fields(entry, config, &mut diagnostics);
        if config.generate_missing_keys && entry.key.is_empty() {
            let stem = key_stem(entry);
            let mut key = stem.clone();
            let suffix = next_suffix.entry(stem.clone()).or_insert(2usize);
            while used_keys.contains(&key) {
                key = format!("{stem}{suffix}");
                *suffix += 1;
            }
            used_keys.insert(key.clone());
            entry.key = Cow::Owned(key);
            entry.changed = true;
        }
    }
    if config.merge_duplicate_entries {
        merge_entries(&mut entries, &mut diagnostics);
    }
    Ok(Some(Prepared {
        entries,
        diagnostics,
        protected,
    }))
}

fn needs_literal(name: &str, config: &CleanupConfig) -> bool {
    match name {
        "month" => config.months,
        "title" | "booktitle" | "journaltitle" => {
            config.sentence_case_titles || config.escape_special_characters
        }
        "url" | "doi" => config.normalize_urls,
        "author" => config.max_authors.is_some() || config.escape_special_characters,
        _ => config.escape_special_characters && text_field(name),
    }
}

fn text_field(name: &str) -> bool {
    matches!(
        name,
        "title"
            | "subtitle"
            | "booktitle"
            | "booksubtitle"
            | "journaltitle"
            | "journal"
            | "note"
            | "abstract"
            | "publisher"
            | "organization"
            | "institution"
            | "author"
            | "editor"
            | "address"
            | "location"
    )
}

fn clean_fields(entry: &mut Entry<'_>, config: &CleanupConfig, diagnostics: &mut Vec<Diagnostic>) {
    if !config.remove_empty_fields && !config.remove_duplicate_fields {
        return;
    }
    let mut fields: Vec<Field<'_>> = Vec::with_capacity(entry.fields.len());
    let mut indices: HashMap<String, HashMap<u64, Vec<usize>>> = HashMap::new();
    let hash = RandomState::new();
    for field in std::mem::take(&mut entry.fields) {
        if config.remove_empty_fields
            && literal(&field.value).is_some_and(|(text, _, _)| text.trim().is_empty())
        {
            entry.trailing.extend(field.before);
            entry.trailing.extend(field.after);
            entry.changed = true;
        } else {
            if config.remove_duplicate_fields {
                let values = indices.entry(field.name.to_ascii_lowercase()).or_default();
                let fingerprint = hash.hash_one(field.value.as_ref());
                let previous = values.get(&fingerprint).and_then(|bucket| {
                    bucket
                        .iter()
                        .copied()
                        .find(|&index| fields[index].value == field.value)
                });
                if let Some(previous) = previous {
                    fields[previous].after.extend(field.before);
                    fields[previous].after.extend(field.after);
                    entry.changed = true;
                    continue;
                }
                if !values.is_empty() {
                    diagnostics.push(Diagnostic {
                        severity: Severity::Warning,
                        message: format!(
                            "conflicting duplicate BibTeX field {} preserved",
                            field.name
                        ),
                        span: field.span.clone(),
                    });
                }
                values.entry(fingerprint).or_default().push(fields.len());
            }
            fields.push(field);
        }
    }
    entry.fields = fields;
}

type FieldLookup = HashMap<String, HashMap<u64, Vec<usize>>>;

fn field_lookup(entry: &Entry<'_>, hash: &RandomState) -> FieldLookup {
    let mut fields: FieldLookup = HashMap::new();
    for (index, field) in entry.fields.iter().enumerate() {
        fields
            .entry(field.name.to_ascii_lowercase())
            .or_default()
            .entry(hash.hash_one(field.value.as_ref()))
            .or_default()
            .push(index);
    }
    fields
}

#[derive(Hash, PartialEq, Eq)]
struct EntrySignature {
    kind: String,
    fields: Vec<(String, u64)>,
}

fn signature(entry: &Entry<'_>, hash: &RandomState) -> EntrySignature {
    let mut fields: Vec<_> = entry
        .fields
        .iter()
        .map(|field| {
            (
                field.name.to_ascii_lowercase(),
                hash.hash_one(field.value.as_ref()),
            )
        })
        .collect();
    // Stable ordering retains the meaning of conflicting repeated assignments.
    fields.sort_by(|left, right| left.0.cmp(&right.0));
    EntrySignature {
        kind: entry.kind.to_ascii_lowercase(),
        fields,
    }
}

fn same_fields(left: &Entry<'_>, right: &Entry<'_>) -> bool {
    let sorted = |entry: &Entry<'_>| {
        let mut indices: Vec<_> = (0..entry.fields.len()).collect();
        indices.sort_by_cached_key(|&index| entry.fields[index].name.to_ascii_lowercase());
        indices
    };
    left.fields.len() == right.fields.len()
        && left.kind.eq_ignore_ascii_case(right.kind)
        && sorted(left)
            .into_iter()
            .zip(sorted(right))
            .all(|(left_index, right_index)| {
                left.fields[left_index]
                    .name
                    .eq_ignore_ascii_case(right.fields[right_index].name)
                    && left.fields[left_index].value == right.fields[right_index].value
            })
}

struct MergeGroup {
    primary: usize,
    fields: FieldLookup,
    // Conflicting records make the target of later complementary records
    // ambiguous. Exact duplicates can still be resolved by their signatures.
    exact: Option<HashMap<EntrySignature, Vec<usize>>>,
}

fn merge_entries(entries: &mut [Entry<'_>], diagnostics: &mut Vec<Diagnostic>) {
    let hash = RandomState::new();
    let mut seen: HashMap<(usize, String), MergeGroup> = HashMap::new();
    for index in 0..entries.len() {
        let key = (entries[index].run, entries[index].key.to_string());
        let Some(group) = seen.get_mut(&key) else {
            seen.insert(
                key,
                MergeGroup {
                    primary: index,
                    fields: field_lookup(&entries[index], &hash),
                    exact: None,
                },
            );
            continue;
        };
        if let Some(exact) = &mut group.exact {
            let fingerprint = signature(&entries[index], &hash);
            let target = exact.get(&fingerprint).and_then(|candidates| {
                candidates
                    .iter()
                    .copied()
                    .find(|&candidate| same_fields(&entries[candidate], &entries[index]))
            });
            if let Some(target) = target {
                let mut lookup = field_lookup(&entries[target], &hash);
                merge_record(entries, target, index, &mut lookup, &hash);
                continue;
            }
            exact.entry(fingerprint).or_default().push(index);
        } else {
            let primary = &entries[group.primary];
            let compatible = primary.kind.eq_ignore_ascii_case(entries[index].kind)
                && entries[index].fields.iter().all(|field| {
                    group
                        .fields
                        .get(&field.name.to_ascii_lowercase())
                        .is_none_or(|values| {
                            values.len() == 1
                                && values
                                    .get(&hash.hash_one(field.value.as_ref()))
                                    .is_some_and(|bucket| {
                                        bucket
                                            .iter()
                                            .all(|&at| primary.fields[at].value == field.value)
                                    })
                        })
                });
            if compatible {
                merge_record(entries, group.primary, index, &mut group.fields, &hash);
                continue;
            }
            let mut exact = HashMap::new();
            exact.insert(signature(primary, &hash), vec![group.primary]);
            exact
                .entry(signature(&entries[index], &hash))
                .or_insert_with(Vec::new)
                .push(index);
            group.exact = Some(exact);
            group.fields.clear();
        }
        diagnostics.push(Diagnostic {
            severity: Severity::Warning,
            message: format!("duplicate BibTeX key {} preserved: conflicting records make complementary merging ambiguous", entries[index].key),
            span: entries[index].span.clone(),
        });
    }
}

fn merge_record(
    entries: &mut [Entry<'_>],
    target: usize,
    removed: usize,
    lookup: &mut FieldLookup,
    hash: &RandomState,
) {
    let (prefix, suffix) = entries.split_at_mut(removed);
    let target = &mut prefix[target];
    let removed = &mut suffix[0];
    for field in std::mem::take(&mut removed.fields) {
        let values = lookup.entry(field.name.to_ascii_lowercase()).or_default();
        let fingerprint = hash.hash_one(field.value.as_ref());
        let existing = values.get(&fingerprint).and_then(|bucket| {
            bucket
                .iter()
                .copied()
                .find(|&at| target.fields[at].value == field.value)
        });
        if let Some(existing) = existing {
            target.fields[existing].after.extend(field.before);
            target.fields[existing].after.extend(field.after);
        } else {
            values
                .entry(fingerprint)
                .or_default()
                .push(target.fields.len());
            target.fields.push(field);
        }
    }
    target.trailing.append(&mut removed.trailing);
    target.changed = true;
    removed.removed = true;
}

fn clean_value<'a>(
    name: &str,
    value: &'a str,
    piece: Option<(&'a str, char, char)>,
    month: Option<&'static str>,
    config: &CleanupConfig,
) -> Cow<'a, str> {
    if let Some(month) = month {
        return if value == month {
            Cow::Borrowed(value)
        } else {
            Cow::Owned(month.to_owned())
        };
    }
    let Some((inner, opening, closing)) = piece else {
        return Cow::Borrowed(value);
    };
    let mut text = Cow::Borrowed(inner);
    if config.normalize_urls
        && matches!(name, "url" | "doi")
        && let Some(cleaned) = clean_url(&text)
        && cleaned != text
    {
        text = Cow::Owned(cleaned.into_owned());
    }
    if config.sentence_case_titles
        && matches!(name, "title" | "booktitle" | "journaltitle")
        && let Cow::Owned(cleaned) = sentence_case(&text)
    {
        text = Cow::Owned(cleaned);
    }
    if name == "author"
        && let Some(maximum) = config.max_authors
    {
        let authors = split_authors(&text);
        let count =
            authors.len() - usize::from(authors.last().is_some_and(|name| *name == "others"));
        if count > maximum {
            text = Cow::Owned(authors[..maximum].join(" and ") + " and others");
        }
    }
    if config.escape_special_characters
        && text_field(name)
        && let Cow::Owned(cleaned) = escape_special(&text)
    {
        text = Cow::Owned(cleaned);
    }
    if text == inner {
        Cow::Borrowed(value)
    } else {
        Cow::Owned(format!("{opening}{text}{closing}"))
    }
}

fn month(value: &str, piece: Option<(&str, char, char)>) -> Option<&'static str> {
    const MONTHS: &[(&str, &str)] = &[
        ("jan", "january"),
        ("feb", "february"),
        ("mar", "march"),
        ("apr", "april"),
        ("may", "may"),
        ("jun", "june"),
        ("jul", "july"),
        ("aug", "august"),
        ("sep", "september"),
        ("oct", "october"),
        ("nov", "november"),
        ("dec", "december"),
    ];
    let text = if let Some((text, _, _)) = piece {
        text.trim()
    } else {
        if let Some((short, _)) = MONTHS.iter().find(|(short, _)| value == *short) {
            return Some(short);
        }
        if !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        value
    };
    let text = text.trim_end_matches('.').to_ascii_lowercase();
    if let Ok(number) = text.parse::<usize>() {
        return MONTHS.get(number.checked_sub(1)?).map(|pair| pair.0);
    }
    MONTHS
        .iter()
        .find(|(short, long)| text == *short || text == *long || text == "sept" && *short == "sep")
        .map(|pair| pair.0)
}

fn sentence_case(text: &str) -> Cow<'_, str> {
    if text.contains('$') || text.contains("\\(") || text.contains("\\[") {
        return Cow::Borrowed(text);
    }
    let mut depth = 0usize;
    let mut command = false;
    let mut escaped = false;
    let mut letters = Vec::new();
    for (at, character) in text.char_indices() {
        if escaped {
            escaped = false;
            command = character.is_alphabetic();
            continue;
        }
        if command && character.is_alphabetic() {
            continue;
        }
        command = false;
        match character {
            '\\' => escaped = true,
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            _ if depth == 0 && (character.is_uppercase() || character.is_lowercase()) => {
                if character.is_lowercase() {
                    return Cow::Borrowed(text);
                }
                letters.push((at, character))
            }
            _ => (),
        }
    }
    if letters.len() < 2
        || letters
            .iter()
            .any(|(_, character)| character.is_lowercase())
    {
        return Cow::Borrowed(text);
    }
    let mut output = String::with_capacity(text.len());
    let mut cursor = 0;
    // The first unprotected letter starts the sentence; protected acronyms survive.
    for &(at, character) in letters.iter().skip(1) {
        output.push_str(&text[cursor..at]);
        output.extend(character.to_lowercase());
        cursor = at + character.len_utf8();
    }
    output.push_str(&text[cursor..]);
    Cow::Owned(output)
}

fn clean_url(text: &str) -> Option<Cow<'_, str>> {
    let mut text = text.trim();
    if let Some(inner) = text
        .strip_prefix("\\url{")
        .and_then(|inner| inner.strip_suffix('}'))
        && braced_value_end(text, 4, text.len()).ok() == Some(text.len())
    {
        text = inner.trim();
    }
    if !text.contains('\\') {
        return Some(Cow::Borrowed(text));
    }
    let mut output = String::with_capacity(text.len());
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '\\' && characters.peek().is_some_and(|next| "%&#_".contains(*next)) {
            output.push(characters.next().unwrap());
        } else if character == '\\' {
            return None;
        } else {
            output.push(character);
        }
    }
    Some(Cow::Owned(output))
}

fn escape_special(text: &str) -> Cow<'_, str> {
    if text.contains('$') || text.contains("\\(") || text.contains("\\[") {
        return Cow::Borrowed(text);
    }
    if !text
        .bytes()
        .any(|byte| matches!(byte, b'%' | b'&' | b'#' | b'_' | b'~' | b'^'))
    {
        return Cow::Borrowed(text);
    }
    let mut output = String::with_capacity(text.len());
    let mut cursor = 0;
    while cursor < text.len() {
        let character = text[cursor..].chars().next().unwrap();
        match character {
            '\\' => {
                let start = cursor;
                cursor += 1;
                if let Some(next) = text[cursor..].chars().next() {
                    cursor += next.len_utf8();
                    if next.is_ascii_alphabetic() {
                        while text
                            .as_bytes()
                            .get(cursor)
                            .is_some_and(u8::is_ascii_alphabetic)
                        {
                            cursor += 1;
                        }
                        let command = &text[start + 1..cursor];
                        if matches!(command, "verb" | "lstinline") {
                            output.push_str(&text[start..]);
                            return if output == text {
                                Cow::Borrowed(text)
                            } else {
                                Cow::Owned(output)
                            };
                        }
                        // Command operands can contain URLs or other raw syntax.
                        loop {
                            let argument = skip_whitespace(text, cursor, text.len());
                            if text.as_bytes().get(argument) != Some(&b'{') {
                                break;
                            }
                            let Some(end) = braced_value_end(text, argument, text.len()).ok()
                            else {
                                break;
                            };
                            cursor = end;
                        }
                    }
                }
                output.push_str(&text[start..cursor]);
                continue;
            }
            '%' | '&' | '#' | '_' => {
                output.push('\\');
                output.push(character);
            }
            '~' => output.push_str("\\textasciitilde{}"),
            '^' => output.push_str("\\textasciicircum{}"),
            _ => output.push(character),
        }
        cursor += character.len_utf8();
    }
    if output == text {
        Cow::Borrowed(text)
    } else {
        Cow::Owned(output)
    }
}

/// Split only top-level BibTeX `and` separators, preserving corporate authors.
fn split_authors(text: &str) -> Vec<&str> {
    let mut depth = 0usize;
    let mut escaped = false;
    let mut starts = Vec::new();
    for (at, character) in text.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match character {
            '\\' => escaped = true,
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            'a' | 'A'
                if depth == 0
                    && text[at..]
                        .get(..3)
                        .is_some_and(|word| word.eq_ignore_ascii_case("and"))
                    && text[..at]
                        .chars()
                        .next_back()
                        .is_some_and(char::is_whitespace)
                    && text[at + 3..]
                        .chars()
                        .next()
                        .is_some_and(char::is_whitespace) =>
            {
                starts.push(at)
            }
            _ => (),
        }
    }
    let mut authors = Vec::new();
    let mut cursor = 0;
    for at in starts {
        authors.push(text[cursor..at].trim());
        cursor = at + 3;
    }
    authors.push(text[cursor..].trim());
    authors
}

fn key_stem(entry: &Entry<'_>) -> String {
    let field = |name: &str| {
        entry
            .fields
            .iter()
            .find(|field| field.name.eq_ignore_ascii_case(name))
            .and_then(|field| {
                literal(&field.value)
                    .map(|(text, _, _)| text.to_owned())
                    .or_else(|| {
                        field
                            .value
                            .chars()
                            .all(|character| character.is_ascii_digit())
                            .then(|| field.value.to_string())
                    })
            })
            .unwrap_or_default()
    };
    let authors = field("author");
    let first = split_authors(&authors).first().copied().unwrap_or("");
    let surname = first.split_once(',').map_or_else(
        || first.split_whitespace().last().unwrap_or(""),
        |pair| pair.0,
    );
    let mut stem: String = surname
        .chars()
        .filter(|character| character.is_alphanumeric())
        .take(48)
        .collect();
    stem.extend(
        field("year")
            .chars()
            .filter(|character| character.is_ascii_digit())
            .take(4),
    );
    stem.extend(
        field("title")
            .split_whitespace()
            .next()
            .unwrap_or("")
            .chars()
            .filter(|character| character.is_alphanumeric())
            .take(24),
    );
    if stem.is_empty() {
        "entry".to_owned()
    } else {
        stem
    }
}

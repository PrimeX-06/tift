//! Width-selected layouts for explicitly selected command argument groups.

use std::collections::BTreeMap;
use std::ops::Range;

use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::{CompiledConfig, FormatError, syntax};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct ArgumentLayoutConfig {
    pub mode: String,
    pub line_width: Option<usize>,
    pub commands: Vec<String>,
    pub split_commas: bool,
    pub split_equals: bool,
    pub rules: BTreeMap<String, ArgumentLayoutRule>,
}

impl Default for ArgumentLayoutConfig {
    fn default() -> Self {
        Self {
            mode: "auto".into(),
            line_width: None,
            commands: [
                "pgfkeys",
                "tikzset",
                "pgfplotsset",
                "tcbset",
                "hypersetup",
                "geometry",
                "newgeometry",
                "setlist",
                "KOMAoptions",
                "SetupKeyvalOptions",
                "DeclareKeys",
                "ExecuteBibliographyOptions",
                "setkeys",
                "keys_define:nn",
                "keys_set:nn",
            ]
            .map(String::from)
            .into(),
            split_commas: true,
            split_equals: false,
            rules: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct ArgumentLayoutRule {
    pub mode: Option<String>,
    pub line_width: Option<usize>,
    /// Zero-based mandatory brace argument selected for this command.
    pub argument: usize,
    /// Select a zero-based optional bracket argument instead of `argument`.
    /// A known signature or explicit optional-argument syntax is required.
    pub optional_argument: Option<usize>,
    pub split_commas: Option<bool>,
    pub split_equals: Option<bool>,
}

pub fn validate(config: &ArgumentLayoutConfig) -> Result<(), String> {
    let mode = |value: &str| {
        matches!(value, "auto" | "preserve" | "expand")
            .then_some(())
            .ok_or_else(|| format!("arguments.mode must be auto, preserve or expand: {value:?}"))
    };
    let width = |value: Option<usize>| -> Result<(), String> {
        if value == Some(0) {
            Err("arguments.line-width must be positive".into())
        } else {
            Ok(())
        }
    };
    mode(&config.mode)?;
    width(config.line_width)?;
    for (name, rule) in &config.rules {
        if name.is_empty() || rule.argument > 255 || rule.optional_argument.is_some_and(|i| i > 255)
        {
            return Err(
                "argument rules require a command name and an argument index below 256".into(),
            );
        }
        if let Some(value) = &rule.mode {
            mode(value)?;
        }
        width(rule.line_width)?;
    }
    if config.commands.iter().any(String::is_empty) {
        return Err("arguments.commands must contain nonempty command names".into());
    }
    Ok(())
}

struct Settings<'a> {
    mode: &'a str,
    width: Option<usize>,
    argument: usize,
    optional_argument: Option<usize>,
    commas: bool,
    equals: bool,
}

fn settings<'a>(name: &str, config: &'a CompiledConfig) -> Option<Settings<'a>> {
    let base = &config.config.arguments;
    let unstarred = name.trim_end_matches('*');
    let rule = base.rules.get(name).or_else(|| base.rules.get(unstarred));
    let default_optional = rule.is_none()
        && matches!(config.config.wrap.mode.as_str(), "wrap" | "rewrap")
        && unstarred == "includegraphics";
    if rule.is_none()
        && !default_optional
        && !base
            .commands
            .iter()
            .any(|command| command == name || command == unstarred)
    {
        return None;
    }
    let mode = rule.and_then(|r| r.mode.as_deref()).unwrap_or(&base.mode);
    let width = rule
        .and_then(|r| r.line_width)
        .or(base.line_width)
        .or_else(|| {
            (matches!(config.config.wrap.mode.as_str(), "wrap" | "rewrap"))
                .then_some(config.config.wrap.columns)
        });
    (mode != "preserve" && (mode == "expand" || width.is_some())).then_some(Settings {
        mode,
        width,
        argument: rule.map_or_else(
            || {
                usize::from(matches!(
                    unstarred,
                    "setkeys" | "keys_define:nn" | "keys_set:nn"
                ))
            },
            |r| r.argument,
        ),
        optional_argument: rule
            .and_then(|r| r.optional_argument)
            .or(default_optional.then_some(0)),
        commas: rule
            .and_then(|r| r.split_commas)
            .unwrap_or(base.split_commas),
        equals: rule
            .and_then(|r| r.split_equals)
            .unwrap_or(base.split_equals),
    })
}

fn protected_at(cursor: usize, protected: &[Range<usize>]) -> Option<&Range<usize>> {
    protected
        .get(protected.partition_point(|range| range.end <= cursor))
        .filter(|range| range.start <= cursor)
}

fn comment_end(source: &str, cursor: usize, limit: usize) -> usize {
    source[cursor..limit]
        .find('\n')
        .map_or(limit, |end| cursor + end + 1)
}

fn group_end(source: &str, start: usize, protected: &[Range<usize>]) -> Option<usize> {
    let open = *source.as_bytes().get(start)?;
    let close = match open {
        b'{' => b'}',
        b'[' => b']',
        b'<' => b'>',
        b'(' => b')',
        _ => return None,
    };
    if let Some(range) = protected_at(start, protected) {
        return (range.start == start
            && source.as_bytes().get(range.end.checked_sub(1)?) == Some(&close))
        .then_some(range.end);
    }
    let mut cursor = start + 1;
    let mut depth = 1usize;
    while cursor < source.len() {
        if let Some(range) = protected_at(cursor, protected) {
            cursor = range.end;
            continue;
        }
        match source.as_bytes()[cursor] {
            b'\\' => cursor = syntax::command_at(source, cursor)?.1,
            b'%' => cursor = comment_end(source, cursor, source.len()),
            b'{' if open != b'{' => cursor = group_end(source, cursor, protected)?,
            byte if byte == open => {
                depth += 1;
                cursor += 1;
            }
            byte if byte == close => {
                depth -= 1;
                cursor += 1;
                if depth == 0 {
                    return Some(cursor);
                }
            }
            _ => cursor += source[cursor..].chars().next()?.len_utf8(),
        }
    }
    None
}

fn argument_group(
    source: &str,
    mut cursor: usize,
    index: usize,
    optional: bool,
    protected: &[Range<usize>],
) -> Option<Range<usize>> {
    let mut argument = 0;
    loop {
        cursor = crate::trivia::skip(source, cursor);
        if protected_at(cursor, protected).is_some() {
            return None;
        }
        match source.as_bytes().get(cursor)? {
            b'[' if optional => cursor = group_end(source, cursor, protected)?,
            b'{' => {
                let end = group_end(source, cursor, protected)?;
                if argument == index {
                    return Some(cursor..end);
                }
                argument += 1;
                cursor = end;
            }
            _ => return None,
        }
    }
}

fn optional_group(
    source: &str,
    name: &str,
    cursor: usize,
    index: usize,
    config: &CompiledConfig,
    declared: &std::collections::HashSet<String>,
    protected: &[Range<usize>],
) -> Option<(Range<usize>, usize)> {
    let base = name.trim_end_matches('*');
    // Local declarations can change both the bracket's role and its grammar.
    // Keep these invocations intact rather than applying a package signature.
    if declared.contains(base) {
        return None;
    }
    if let Some(rule) = config.config.syntax.commands.get(base) {
        if !rule.optional_arguments || rule.verbatim || rule.preserve_body {
            return None;
        }
    } else if let Some(signature) = crate::packages::command_signature(source, name, cursor) {
        let ranges = signature_groups(source, cursor, signature, protected)?;
        let selected = ranges
            .iter()
            .filter(|range| source.as_bytes()[range.start] == b'[')
            .nth(index)?
            .clone();
        return Some((selected, ranges.last()?.end));
    } else if !crate::package_commands::builtin_command_rule(base)
        .is_some_and(|rule| rule.optional_arguments && !rule.verbatim && !rule.preserve_body)
    {
        // An explicit layout selection does not imply an unknown macro reads
        // brackets. Require explicit syntax for custom commands.
        return None;
    }
    let mut cursor = cursor;
    for optional in 0..=index {
        cursor = crate::trivia::skip(source, cursor);
        if protected_at(cursor, protected).is_some() || source.as_bytes().get(cursor) != Some(&b'[')
        {
            return None;
        }
        let end = group_end(source, cursor, protected)?;
        if optional == index {
            let rule = config
                .config
                .syntax
                .commands
                .get(base)
                .or_else(|| crate::package_commands::builtin_command_rule(base));
            let mut finish = end;
            // Explicit generic syntax describes leading optional arguments
            // followed by a fixed number of mandatory argument tokens.
            while source.as_bytes().get(crate::trivia::skip(source, finish)) == Some(&b'[') {
                finish = group_end(source, crate::trivia::skip(source, finish), protected)?;
            }
            if let Some(count) = rule.and_then(|rule| rule.arguments) {
                for _ in 0..count {
                    finish = mandatory_end(source, crate::trivia::skip(source, finish), protected)?;
                }
            }
            return Some((cursor..end, finish));
        }
        cursor = end;
    }
    None
}

fn mandatory_end(source: &str, start: usize, protected: &[Range<usize>]) -> Option<usize> {
    match source.as_bytes().get(start)? {
        b'{' => group_end(source, start, protected),
        b'\\' => syntax::command_at(source, start)
            .map(|(name, end)| end - usize::from(name.len() > 1 && name.ends_with('*'))),
        b'}' | b']' => None,
        _ => source[start..].chars().next().map(|c| start + c.len_utf8()),
    }
}

fn signature_groups(
    source: &str,
    mut cursor: usize,
    signature: &str,
    protected: &[Range<usize>],
) -> Option<Vec<Range<usize>>> {
    let mut ranges = Vec::new();
    for slot in signature.bytes() {
        let start = crate::trivia::skip(source, cursor);
        if protected_at(start, protected).is_some_and(|range| range.start < start) {
            return None;
        }
        let end = match slot {
            b'o' if source.as_bytes().get(start) == Some(&b'[') => {
                group_end(source, start, protected)?
            }
            b'd' if source.as_bytes().get(start) == Some(&b'<') => {
                group_end(source, start, protected)?
            }
            b'p' if source.as_bytes().get(start) == Some(&b'(') => {
                group_end(source, start, protected)?
            }
            b'o' | b'd' | b'p' => continue,
            b'm' => mandatory_end(source, start, protected)?,
            _ => return None,
        };
        ranges.push(start..end);
        cursor = end;
    }
    Some(ranges)
}

pub(crate) fn literal_argument_ranges(
    source: &str,
    config: &CompiledConfig,
    protected: &[Range<usize>],
    declared: &std::collections::HashSet<String>,
) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut cursor = 0;
    while cursor < source.len() {
        if let Some(range) = protected_at(cursor, &ranges) {
            cursor = range.end;
            continue;
        }
        if let Some(range) = protected_at(cursor, protected) {
            cursor = range.end;
            continue;
        }
        if source.as_bytes()[cursor] == b'%' {
            cursor = comment_end(source, cursor, source.len());
            continue;
        }
        if let Some((name, end)) = syntax::command_at(source, cursor) {
            let base = name.trim_end_matches('*');
            if !declared.contains(base) && !config.config.syntax.commands.contains_key(base) {
                let signature = match base {
                    "url" | "nolinkurl" | "path" | "hyperbaseurl" | "input" | "include"
                    | "includeonly" | "bibliography" => Some("m"),
                    "href" | "hyperimage" => Some("omm"),
                    "includegraphics" => crate::document_environments::command_signature(base),
                    "addbibresource" | "addglobalbib" => Some("om"),
                    _ => None,
                };
                if let Some(signature) = signature
                    && let Some(groups) = signature_groups(source, end, signature, protected)
                    && let Some(group) = groups
                        .into_iter()
                        .find(|range| source.as_bytes()[range.start] == b'{')
                {
                    let index = ranges
                        .binary_search_by_key(&group.start, |range| range.start)
                        .unwrap_or_else(|index| index);
                    ranges.insert(index, group);
                    // Metadata before the literal argument can contain
                    // other literal commands; visit it before skipping
                    // the recorded data span.
                    cursor = end;
                    continue;
                }
            }
            cursor = end;
        } else {
            cursor += source[cursor..]
                .chars()
                .next()
                .expect("source character")
                .len_utf8();
        }
    }
    ranges
}

fn merge_ranges(ranges: &mut Vec<Range<usize>>, additions: Vec<Range<usize>>) {
    ranges.extend(additions);
    ranges.sort_unstable_by_key(|range| range.start);
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
    for range in ranges.drain(..) {
        if let Some(previous) = merged
            .last_mut()
            .filter(|previous| previous.end >= range.start)
        {
            previous.end = previous.end.max(range.end);
        } else {
            merged.push(range);
        }
    }
    *ranges = merged;
}

fn math_end(source: &str, start: usize, limit: usize, protected: &[Range<usize>]) -> Option<usize> {
    let (opening, closing) = if source[start..].starts_with("$$") {
        ("$$", "$$")
    } else if source.as_bytes()[start] == b'$' {
        ("$", "$")
    } else if source[start..].starts_with("\\(") {
        ("\\(", "\\)")
    } else if source[start..].starts_with("\\[") {
        ("\\[", "\\]")
    } else {
        return None;
    };
    let mut cursor = start + opening.len();
    while cursor < limit {
        if let Some(range) = protected_at(cursor, protected) {
            cursor = range.end.min(limit);
            continue;
        }
        if source[cursor..limit].starts_with(closing) && !syntax::is_escaped(source, cursor) {
            return Some(cursor + closing.len());
        }
        if source.as_bytes()[cursor] == b'%' && !syntax::is_escaped(source, cursor) {
            cursor = comment_end(source, cursor, limit);
        } else {
            cursor += source[cursor..].chars().next()?.len_utf8();
        }
    }
    None
}

/// Return punctuation owned by this brace body, excluding nested syntax.
struct Punctuation {
    points: Vec<(usize, u8)>,
    empty_comments: Vec<Range<usize>>,
}

fn punctuation(
    source: &str,
    body: Range<usize>,
    protected: &[Range<usize>],
) -> Option<Punctuation> {
    let mut result = Vec::new();
    let mut empty_comments = Vec::new();
    let mut cursor = body.start;
    while cursor < body.end {
        if let Some(range) = protected_at(cursor, protected) {
            if range.end > body.end {
                return None;
            }
            cursor = range.end;
            continue;
        }
        match source.as_bytes()[cursor] {
            b'%' => {
                let end = comment_end(source, cursor, body.end);
                // Empty comments suppress the newline and the following
                // indentation. Preserve comments containing actual text.
                if !source[cursor + 1..end]
                    .trim_matches([' ', '\t', '\r', '\n'])
                    .is_empty()
                {
                    return None;
                }
                let mut removed_end = end;
                while removed_end < body.end
                    && matches!(source.as_bytes()[removed_end], b' ' | b'\t')
                {
                    removed_end += 1;
                }
                if source.as_bytes().get(removed_end) == Some(&b'\n')
                    || source[removed_end..body.end].starts_with("\r\n")
                {
                    return None;
                }
                // Removing a comment between a control word and a letter
                // would merge them into a different command token.
                let token_boundary =
                    empty_comments
                        .iter()
                        .rev()
                        .fold(cursor, |end, removed: &Range<usize>| {
                            if removed.end == end {
                                removed.start
                            } else {
                                end
                            }
                        });
                if source
                    .as_bytes()
                    .get(removed_end)
                    .is_some_and(|byte| byte.is_ascii_alphabetic() || b"@_:".contains(byte))
                    && source[body.start..cursor]
                        .rfind('\\')
                        .is_some_and(|offset| {
                            syntax::command_at(source, body.start + offset).is_some_and(
                                |(name, end)| {
                                    end == token_boundary
                                        && name.bytes().next().is_some_and(|byte| {
                                            byte.is_ascii_alphabetic() || b"@_:".contains(&byte)
                                        })
                                },
                            )
                        })
                {
                    return None;
                }
                empty_comments.push(cursor..removed_end);
                cursor = removed_end;
            }
            b' ' | b'\t' | b'\r' | b'\n' => {
                let start = cursor;
                while cursor < body.end
                    && matches!(source.as_bytes()[cursor], b' ' | b'\t' | b'\r' | b'\n')
                {
                    cursor += 1;
                }
                // A blank physical line produces a TeX paragraph token.
                if source[start..cursor]
                    .bytes()
                    .filter(|byte| *byte == b'\n')
                    .count()
                    > 1
                {
                    return None;
                }
            }
            b'{' | b'[' => {
                cursor = group_end(source, cursor, protected).filter(|end| *end <= body.end)?
            }
            b'$' => cursor = math_end(source, cursor, body.end, protected)?,
            b'\\' => {
                cursor =
                    if source[cursor..].starts_with("\\(") || source[cursor..].starts_with("\\[") {
                        math_end(source, cursor, body.end, protected)?
                    } else {
                        syntax::command_at(source, cursor)?.1
                    };
            }
            byte @ (b',' | b'=') => {
                result.push((cursor, byte));
                cursor += 1;
            }
            _ => cursor += source[cursor..].chars().next()?.len_utf8(),
        }
    }
    Some(Punctuation {
        points: result,
        empty_comments,
    })
}

fn column(text: &str, tab_width: usize) -> usize {
    if text.bytes().all(|byte| (32..127).contains(&byte)) {
        return text.len();
    }
    let mut column = 0;
    for grapheme in text.graphemes(true) {
        if grapheme == "\t" {
            column += tab_width - column % tab_width;
        } else {
            column += UnicodeWidthStr::width(grapheme);
        }
    }
    column
}

fn trim_region(source: &str, range: Range<usize>) -> Range<usize> {
    let text = &source[range.clone()];
    let start = range.start + text.len() - text.trim_start_matches([' ', '\t', '\r', '\n']).len();
    let mut end = range.start + text.trim_end_matches([' ', '\t', '\r', '\n']).len();
    if end < range.end && syntax::is_escaped(source, end) {
        end += source[end..]
            .chars()
            .next()
            .expect("trailing whitespace")
            .len_utf8();
    }
    start.min(end)..end
}

fn boundary(output: &mut String, had_space: bool, newline: &str, indent: &str) {
    if !had_space {
        output.push('%');
    }
    output.push_str(newline);
    output.push_str(indent);
}

struct Replacement<'a> {
    group: Range<usize>,
    text: &'a str,
}

/// Copy an original span with its already-rendered direct children. Every
/// punctuation boundary belongs to the current group, so children never cross
/// the span boundaries used by expansion.
fn copy_children(
    output: &mut String,
    source: &str,
    range: Range<usize>,
    children: &[Replacement<'_>],
) {
    let mut copied = range.start;
    let first = children.partition_point(|child| child.group.end <= range.start);
    for child in children[first..]
        .iter()
        .take_while(|child| child.group.start < range.end)
    {
        if child.group.start >= range.start && child.group.end <= range.end {
            output.push_str(&source[copied..child.group.start]);
            output.push_str(child.text);
            copied = child.group.end;
        }
    }
    output.push_str(&source[copied..range.end]);
}

fn format_group(
    source: &str,
    group: Range<usize>,
    settings: &Settings<'_>,
    config: &CompiledConfig,
    protected: &[Range<usize>],
    children: &[Replacement<'_>],
    newline: &str,
) -> Option<String> {
    let tab = config.config.indent.width;
    if settings.mode == "auto" {
        let width = settings.width?;
        let line_start = source[..group.start].rfind('\n').map_or(0, |pos| pos + 1);
        let after = source[group.end..]
            .split('\n')
            .next()
            .unwrap_or("")
            .trim_end_matches('\r');
        let original = &source[line_start..group.end + after.len()];
        if original.lines().all(|line| column(line, tab) <= width) {
            return None;
        }
    }
    expand_group(
        source, group, settings, config, protected, newline, children,
    )
}

fn expand_group(
    source: &str,
    group: Range<usize>,
    settings: &Settings<'_>,
    config: &CompiledConfig,
    protected: &[Range<usize>],
    newline: &str,
    children: &[Replacement<'_>],
) -> Option<String> {
    let body = group.start + 1..group.end - 1;
    let parsed = punctuation(source, body.clone(), protected)?;
    if !parsed.empty_comments.is_empty() {
        // Work on this source line and group only. This also lets a narrower
        // width relayout an argument formatted during a previous pass.
        let prefix_start = source[..group.start].rfind('\n').map_or(0, |pos| pos + 1);
        let suffix_end = source[group.end..]
            .find('\n')
            .map_or(source.len(), |pos| group.end + pos);
        let mut normalized = String::new();
        let mut copied = prefix_start;
        let mut removed_lengths = Vec::with_capacity(parsed.empty_comments.len() + 1);
        removed_lengths.push(0usize);
        for removed in &parsed.empty_comments {
            normalized.push_str(&source[copied..removed.start]);
            copied = removed.end;
            removed_lengths.push(removed_lengths.last().copied().unwrap() + removed.len());
        }
        normalized.push_str(&source[copied..suffix_end]);
        let remap = |offset: usize| {
            let removed = parsed
                .empty_comments
                .partition_point(|removed| removed.end <= offset);
            offset - prefix_start - removed_lengths[removed]
        };
        let local_protected: Vec<_> = protected
            .iter()
            .filter(|range| range.start >= group.start && range.end <= group.end)
            .map(|range| remap(range.start)..remap(range.end))
            .collect();
        let local_children: Vec<_> = children
            .iter()
            .map(|child| Replacement {
                group: remap(child.group.start)..remap(child.group.end),
                text: child.text,
            })
            .collect();
        return expand_group(
            &normalized,
            remap(group.start)..remap(group.end),
            settings,
            config,
            &local_protected,
            newline,
            &local_children,
        );
    }
    let points = parsed.points;
    let tab = config.config.indent.width;
    let before = source[..group.start].rsplit('\n').next().unwrap_or("");
    let commas: Vec<_> = points
        .iter()
        .filter(|(_, byte)| *byte == b',')
        .map(|(pos, _)| *pos)
        .collect();
    if (!settings.commas || commas.is_empty())
        && (!settings.equals || !points.iter().any(|(_, byte)| *byte == b'='))
    {
        return None;
    }
    let base: String = before
        .chars()
        .take_while(|character| matches!(character, ' ' | '\t'))
        .collect();
    let bracket = source.as_bytes()[group.start] == b'[';
    let unit = config.config.indent.unit(
        if bracket {
            "optional-argument"
        } else {
            "mandatory-argument"
        },
        "argument",
    );
    let inner = format!("{base}{unit}");
    // Both key and value remain in the same mandatory-argument scope. Use
    // that structural indentation so a later indent pass retains the layout.
    let value_indent = &inner;
    let mut pieces = Vec::new();
    let mut start = body.start;
    if settings.commas {
        for comma in commas {
            pieces.push(start..comma + 1);
            start = comma + 1;
        }
    }
    pieces.push(start..body.end);
    let mut output = String::from(if bracket { "[" } else { "{" });
    let mut previous_end = body.start;
    let mut point_index = 0;
    for piece in pieces {
        let trimmed = trim_region(source, piece);
        if trimmed.is_empty() {
            continue;
        }
        let whitespace = &source[previous_end..trimmed.start];
        boundary(&mut output, !whitespace.is_empty(), newline, &inner);
        let text = &source[trimmed.clone()];
        while point_index < points.len() && points[point_index].0 < trimmed.start {
            point_index += 1;
        }
        let equal = settings
            .equals
            .then(|| {
                points[point_index..]
                    .iter()
                    .take_while(|(pos, _)| *pos < trimmed.end)
                    .find(|(_, byte)| *byte == b'=')
            })
            .flatten();
        let expand_equal = equal.is_some()
            && (settings.mode == "expand"
                || settings
                    .width
                    .is_some_and(|width| column(&format!("{inner}{text}"), tab) > width));
        if let Some((equal, _)) = equal.filter(|_| expand_equal) {
            copy_children(&mut output, source, trimmed.start..equal + 1, children);
            let value = trim_region(source, equal + 1..trimmed.end);
            if !value.is_empty() {
                boundary(&mut output, value.start > equal + 1, newline, value_indent);
                copy_children(&mut output, source, value, children);
            }
        } else {
            copy_children(&mut output, source, trimmed.clone(), children);
        }
        previous_end = trimmed.end;
    }
    boundary(&mut output, previous_end < body.end, newline, &base);
    output.push(if bracket { ']' } else { '}' });
    Some(output)
}

pub(crate) fn is_active(config: &CompiledConfig) -> bool {
    settings("includegraphics", config).is_some()
        || config
            .config
            .arguments
            .commands
            .iter()
            .chain(config.config.arguments.rules.keys())
            .any(|name| settings(name, config).is_some())
}

pub fn format_arguments(source: &str, config: &CompiledConfig) -> Result<String, FormatError> {
    format_arguments_for_kind(source, config, crate::FileKind::Tex)
}

struct Candidate<'a> {
    group: Range<usize>,
    settings: Settings<'a>,
    children: Vec<usize>,
    resume: usize,
}

pub(crate) fn format_arguments_for_kind(
    source: &str,
    config: &CompiledConfig,
    kind: crate::FileKind,
) -> Result<String, FormatError> {
    format_arguments_with_context(
        source,
        config,
        kind,
        &mut crate::parse_context::ParseContext::new(config),
    )
}

pub(crate) fn format_arguments_with_context(
    source: &str,
    config: &CompiledConfig,
    kind: crate::FileKind,
    parsing: &mut crate::parse_context::ParseContext<'_>,
) -> Result<String, FormatError> {
    if !is_active(config) {
        return Ok(source.into());
    }
    let parsed = parsing.snapshot(source, false, false)?;
    let analysis = crate::source_analysis::SourceAnalysis::with_memo(
        source,
        &config.config,
        &parsed.protected,
        std::rc::Rc::clone(&parsed.metadata),
    );
    let declared = analysis.declared_commands()?;
    let mut protected = parsed.protected.clone();
    let literal = literal_argument_ranges(source, config, &protected, &declared);
    merge_ranges(&mut protected, literal);
    let mut candidates: Vec<Candidate<'_>> = Vec::new();
    let mut roots = Vec::new();
    let mut parents: Vec<usize> = Vec::new();
    let mut cursor = 0;
    while cursor < source.len() {
        while parents
            .last()
            .is_some_and(|&i| cursor >= candidates[i].group.end - 1)
        {
            let parent = parents.pop().expect("enclosing argument");
            cursor = cursor.max(candidates[parent].resume);
        }
        if cursor >= source.len() {
            break;
        }
        if let Some(range) = protected_at(cursor, &protected) {
            cursor = range.end;
            continue;
        }
        if source.as_bytes()[cursor] == b'%' {
            cursor = comment_end(source, cursor, source.len());
            continue;
        }
        if (source.as_bytes()[cursor] == b'$'
            || source[cursor..].starts_with("\\(")
            || source[cursor..].starts_with("\\["))
            && let Some(end) = math_end(source, cursor, source.len(), &protected)
        {
            cursor = end;
            continue;
        }
        if let Some((name, end)) = syntax::command_at(source, cursor) {
            if let Some(settings) = settings(name, config) {
                let base = name.trim_end_matches('*');
                let syntax_rule = config.config.syntax.commands.get(base).or_else(|| {
                    crate::package_commands::builtin_command_rule(base)
                        .filter(|_| !declared.contains(base))
                });
                if settings.optional_argument.is_none()
                    && syntax_rule
                        .and_then(|rule| rule.arguments)
                        .is_some_and(|count| settings.argument >= count)
                {
                    cursor = end;
                    continue;
                }
                let optional = syntax_rule.is_none_or(|rule| rule.optional_arguments);
                let selected = if let Some(index) = settings.optional_argument {
                    optional_group(source, name, end, index, config, &declared, &protected)
                } else {
                    argument_group(source, end, settings.argument, optional, &protected).map(
                        |group| {
                            let resume = group.end;
                            (group, resume)
                        },
                    )
                };
                if let Some((group, resume)) = selected {
                    if parents
                        .last()
                        .is_some_and(|&parent| resume > candidates[parent].group.end - 1)
                    {
                        cursor = end;
                        continue;
                    }
                    // The tree is traversed iteratively. Extremely deep
                    // selected invocations retain their deeper body instead
                    // of building unbounded intermediate formatted strings.
                    if parents.len() >= 128 {
                        cursor = group.end;
                        continue;
                    }
                    let index = candidates.len();
                    if let Some(&parent) = parents.last() {
                        candidates[parent].children.push(index);
                    } else {
                        roots.push(index);
                    }
                    cursor = group.start + 1;
                    candidates.push(Candidate {
                        group,
                        settings,
                        children: Vec::new(),
                        resume,
                    });
                    parents.push(index);
                    continue;
                }
            }
            cursor = end;
        } else {
            cursor += source[cursor..]
                .chars()
                .next()
                .expect("source character")
                .len_utf8();
        }
    }
    let mut rendered: Vec<Option<String>> = (0..candidates.len()).map(|_| None).collect();
    let newline = if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    for index in (0..candidates.len()).rev() {
        let candidate = &candidates[index];
        let children: Vec<_> = candidate
            .children
            .iter()
            .filter_map(|&child| {
                rendered[child].as_deref().map(|text| Replacement {
                    group: candidates[child].group.clone(),
                    text,
                })
            })
            .collect();
        let mut result = format_group(
            source,
            candidate.group.clone(),
            &candidate.settings,
            config,
            &protected,
            &children,
            newline,
        );
        if result.is_none() && !children.is_empty() {
            let mut text = String::new();
            copy_children(&mut text, source, candidate.group.clone(), &children);
            result = Some(text);
        }
        rendered[index] = result.filter(|text| text != &source[candidate.group.clone()]);
    }
    let mut output = String::with_capacity(source.len());
    let mut copied = 0;
    let mut changed = false;
    let mut changed_groups = Vec::new();
    for index in roots {
        if let Some(text) = &rendered[index] {
            let group = &candidates[index].group;
            output.push_str(&source[copied..group.start]);
            let start = output.len();
            output.push_str(text);
            changed_groups.push(start..output.len());
            copied = group.end;
            changed = true;
        }
    }
    output.push_str(&source[copied..]);
    if changed {
        crate::layout::indent_after_arguments(&output, config, kind, parsing, &changed_groups)
    } else {
        Ok(output)
    }
}

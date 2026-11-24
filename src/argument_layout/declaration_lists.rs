use super::{ForeachListMode, column};
use crate::{CompiledConfig, FormatError, parse_context::ParseContext, syntax};
use std::ops::Range;

pub(super) fn is_active(source: &str, config: &CompiledConfig) -> bool {
    let arguments = &config.config.arguments;
    arguments.mode != "preserve"
        && (source.contains(r"\newglossaryentry")
            || source.contains(r"\hyphenation")
            || source.contains(r"\DeclareFontShape")
            || arguments.graphics_paths != ForeachListMode::Preserve
                && source.contains(r"\graphicspath")
            || arguments.listings_literate
                && source.contains("literate")
                && source.contains(r"\lst"))
}

pub(super) fn format(
    source: &str,
    config: &CompiledConfig,
    parsing: &mut ParseContext<'_>,
) -> Result<String, FormatError> {
    let parsed = parsing.snapshot(source, true, false)?;
    let analysis = crate::source_analysis::SourceAnalysis::with_memo(
        source,
        &config.config,
        &parsed.protected,
        std::rc::Rc::clone(&parsed.metadata),
    );
    let declared = analysis.declared_commands()?;
    let options = &config.config.arguments;
    let mut edits = Vec::new();
    let mut consumed = 0;
    for token in parsed.tokens.as_deref().expect("parsed tokens") {
        if token.kind != syntax::TokenKind::Command || token.span.start < consumed {
            continue;
        }
        let name = &source[token.span.start + 1..token.span.end];
        let signature: &[u8] = match name {
            "newglossaryentry" => b"mm",
            "hyphenation" => b"m",
            "DeclareFontShape" => b"mmmmmm",
            "graphicspath" if options.graphics_paths != ForeachListMode::Preserve => b"m",
            "lstset" if options.listings_literate => b"m",
            "lstdefinestyle" if options.listings_literate => b"omom",
            _ => continue,
        };
        if declared.contains(name)
            || config.config.syntax.commands.contains_key(name)
            || config
                .config
                .protect
                .commands
                .iter()
                .any(|command| command == name)
            || options.rules.contains_key(name)
        {
            continue;
        }
        let base = source[..token.span.start].rsplit('\n').next().unwrap_or("");
        if !base.trim_matches([' ', '\t']).is_empty() {
            continue;
        }
        let groups = crate::layout::signature_argument_ranges(
            source,
            token.span.end..source.len(),
            signature.iter().copied(),
        );
        let Some(group) = groups.last() else {
            continue;
        };
        let command = token.span.start..group.end;
        if !source[group.clone()].starts_with('{') || group.end - group.start < 2 {
            continue;
        }
        if syntax::overlaps(&command, &parsed.protected) {
            // Glossaries marks both braced arguments as stored data. Only the
            // outer key separators are editable. Respect wider protected spans.
            if name != "newglossaryentry"
                || !super::declaration_records::glossary_data(&groups, &parsed.protected, &command)
            {
                continue;
            }
        }
        let tail = source[group.end..].split('\n').next().unwrap_or("").trim();
        if !tail.is_empty() && !tail.starts_with('%') {
            continue;
        }
        consumed = group.end;
        let unit = config.config.indent.unit("mandatory-argument", "argument");
        let ending = source[token.span.start..]
            .split_once('\n')
            .map_or_else(|| source.contains("\r\n"), |(line, _)| line.ends_with('\r'));
        let newline = if ending { "\r\n" } else { "\n" };
        if matches!(
            name,
            "newglossaryentry" | "hyphenation" | "DeclareFontShape"
        ) {
            if groups.len() == signature.len()
                && groups.iter().all(|g| source.as_bytes()[g.start] == b'{')
            {
                let target = if name == "DeclareFontShape" {
                    &groups[4]
                } else {
                    group
                };
                let width = options.line_width.unwrap_or(config.config.wrap.columns);
                let expand = options.mode == "expand"
                    || source[target.clone()].contains('\n')
                    || source[token.span.start - base.len()..group.end]
                        .lines()
                        .any(|line| column(line, config.config.indent.width) > width);
                let body = target.start + 1..target.end - 1;
                if let Some(text) = super::declaration_records::format(
                    name,
                    &source[body.clone()],
                    base,
                    &unit,
                    newline,
                    expand,
                    width.saturating_sub(
                        column(base, config.config.indent.width)
                            + column(&unit, config.config.indent.width),
                    ),
                ) {
                    edits.push((body, text));
                }
            }
            continue;
        }
        let body = group.start + 1..group.end - 1;
        if name == "graphicspath" {
            if let Some(rendered) = graphics_paths(
                &source[body.clone()],
                base,
                &unit,
                newline,
                options.graphics_paths,
                column(
                    &source[token.span.start - base.len()..group.end],
                    config.config.indent.width,
                ),
                options.line_width.unwrap_or(config.config.wrap.columns),
            ) {
                edits.push((body, rendered));
            }
        } else {
            for entry in crate::packages::keys::definition_entries(&source[body.clone()]) {
                listings_entry(
                    source,
                    body.start + entry.start..body.start + entry.end,
                    body.end,
                    base,
                    &unit,
                    newline,
                    &mut edits,
                );
            }
        }
    }
    crate::layout::apply_edits(source, edits)
}

fn listings_entry(
    source: &str,
    entry: Range<usize>,
    body_end: usize,
    base: &str,
    unit: &str,
    newline: &str,
    edits: &mut Vec<(Range<usize>, String)>,
) {
    let at = entry.start;
    let entry = &source[entry];
    let Some((key, value)) = entry.trim_end_matches(',').split_once('=') else {
        return;
    };
    if key.trim() != "literate" {
        return;
    }
    let start = at + key.len() + 1;
    let value = value.trim_end_matches(|c: char| c.is_ascii_whitespace());
    let key_at = at + key.len() - key.trim_start().len();
    let line = source[..key_at].rsplit('\n').next().unwrap_or("");
    let inline = !line.trim_matches([' ', '\t']).is_empty();
    let key_indent = if inline {
        format!("{base}{unit}")
    } else {
        line.to_owned()
    };
    let Some(mut rendered) = literate(value, &format!("{newline}{key_indent}{unit}")) else {
        return;
    };
    let mut end = start + value.len();
    if inline {
        let lead = format!("{newline}{key_indent}");
        let previous_break = edits
            .last()
            .is_some_and(|(r, text)| r.end == key_at && text.ends_with(&lead));
        rendered.insert_str(0, &source[key_at..start]);
        if !previous_break {
            rendered.insert_str(0, &lead);
        }
    }
    let after = &source[end..body_end];
    let spaces = after.len() - after.trim_start_matches([' ', '\t']).len();
    if after[spaces..].starts_with(',') {
        let comma = end + spaces + 1;
        let tail = &source[comma..body_end];
        let next = comma + tail.len() - tail.trim_start_matches([' ', '\t']).len();
        rendered.push(',');
        end = if source[next..].starts_with('%') {
            comma
        } else {
            next
        };
        if next < body_end && !source[next..].starts_with(['\r', '\n', '%']) {
            rendered.push_str(newline);
            rendered.push_str(&key_indent);
        }
    }
    if end == body_end {
        rendered.push_str(newline);
        rendered.push_str(base);
    }
    edits.push((if inline { key_at } else { start }..end, rendered));
}

fn graphics_paths(
    body: &str,
    base: &str,
    unit: &str,
    newline: &str,
    mode: ForeachListMode,
    width: usize,
    limit: usize,
) -> Option<String> {
    let mut cursor = 0;
    let mut paths = Vec::new();
    while cursor < body.len() {
        match body.as_bytes()[cursor] {
            b'{' => {
                let end = syntax::balanced_end(body, cursor, b'{', b'}')?;
                let path = &body[cursor + 1..end - 1];
                if path.is_empty() || path.contains(['{', '}', '\\', '%', '\n', '\r', '#']) {
                    return None;
                }
                paths.push(&body[cursor..end]);
                cursor = end;
            }
            b'%' => {
                let (comment, _) = body[cursor + 1..].split_once('\n')?;
                if !comment.trim().is_empty() {
                    return None;
                }
                cursor += comment.len() + 2;
            }
            c if c.is_ascii_whitespace() => cursor += 1,
            _ => return None,
        }
    }
    if paths.len() < 2
        || mode == ForeachListMode::Auto
            && paths.len() < 4
            && !body.contains('\n')
            && width <= limit
    {
        return None;
    }
    let mut result = String::new();
    for path in paths {
        result.push('%');
        result.push_str(newline);
        result.push_str(base);
        result.push_str(unit);
        result.push_str(path);
    }
    result.push('%');
    result.push_str(newline);
    result.push_str(base);
    Some(result)
}

fn literate(value: &str, continuation: &str) -> Option<String> {
    if value.contains('%') {
        return None;
    }
    let whitespace = |s: &str| {
        s.len()
            - s.trim_start_matches(|c: char| c.is_ascii_whitespace())
                .len()
    };
    let mut pos = whitespace(value);
    let starred = value.as_bytes().get(pos) == Some(&b'*');
    pos += usize::from(starred);
    let mut rules: Vec<Range<usize>> = Vec::new();
    loop {
        pos += whitespace(&value[pos..]);
        if pos == value.len() {
            break;
        }
        let start = pos;
        for _ in 0..2 {
            pos = syntax::balanced_end(value, pos, b'{', b'}')?;
            pos += whitespace(&value[pos..]);
        }
        if value.as_bytes().get(pos) == Some(&b'{') {
            let end = syntax::balanced_end(value, pos, b'{', b'}')?;
            let count = &value[pos + 1..end - 1];
            if count.is_empty() || !count.bytes().all(|c| c.is_ascii_digit()) {
                return None;
            }
            pos = end;
        } else if value.as_bytes().get(pos)?.is_ascii_digit() {
            pos += 1;
        } else {
            return None;
        }
        rules.push(start..pos);
    }
    if rules.len() < 2 {
        return None;
    }
    let mut result = if starred {
        String::from("*")
    } else {
        String::new()
    };
    for rule in rules {
        result.push_str(continuation);
        result.push_str(&value[rule]);
    }
    Some(result)
}

//! l3keys, kvoptions and expression-condition command boundaries.

use std::{collections::BTreeMap, ops::Range};

use crate::{config::CommandRule, syntax, trivia};

pub(crate) fn command_signature(name: &str) -> Option<&'static str> {
    if matches!(name, "ProcessKeyvalOptions*" | "ProcessLocalKeyvalOptions*") {
        return Some("");
    }
    Some(match name.trim_end_matches('*') {
        "keys_define:nn" | "keys_set:nn" | "keys_set:nV" | "keys_set:ne" | "keys_set:no" => "mm",
        "keys_set_known:nnN" | "keys_set_filter:nnn" | "keys_set_groups:nnn" => "mmm",
        "SetupKeyvalOptions" => "m",
        "ProcessKeyvalOptions" | "ProcessLocalKeyvalOptions" => "o",
        "DeclareBoolOption" | "DeclareStringOption" => "om",
        "DeclareComplementaryOption" | "DeclareVoidOption" => "mm",
        "define@key" => "mmom",
        "setkeys" => "mm",
        "ifthenelse" | "ifboolexpr" => "mmm",
        "ifboolKV" => "ommm",
        "newtest" => "mom",
        "RequirePDFTeX" | "RequireXeTeX" | "RequireLuaTeX" | "RequirepTeX" | "RequireupTeX"
        | "RequirepTeXng" | "RequireVTeX" | "RequireAleph" | "RequireLuaHBTeX" | "RequireHINT"
        | "RequireeTeX" | "RequireTUTeX" => "",
        _ => return None,
    })
}

pub(crate) fn setting_command(name: &str) -> bool {
    matches!(
        name,
        "keys_set:nn"
            | "keys_set:nV"
            | "keys_set:ne"
            | "keys_set:no"
            | "keys_set_known:nnN"
            | "keys_set_filter:nnn"
            | "keys_set_groups:nnn"
    )
}

pub(super) fn add_command_rules(commands: &mut BTreeMap<String, CommandRule>) {
    for name in [
        "keys_define:nn",
        "keys_set:nn",
        "keys_set:nV",
        "keys_set:ne",
        "keys_set:no",
        "keys_set_known:nnN",
        "keys_set_filter:nnn",
        "keys_set_groups:nnn",
        "SetupKeyvalOptions",
        "ProcessKeyvalOptions",
        "ProcessLocalKeyvalOptions",
        "DeclareBoolOption",
        "DeclareStringOption",
        "DeclareComplementaryOption",
        "DeclareVoidOption",
        "define@key",
        "setkeys",
        "ifthenelse",
        "ifboolexpr",
        "ifboolKV",
        "newtest",
        "RequirePDFTeX",
        "RequireXeTeX",
        "RequireLuaTeX",
        "RequirepTeX",
        "RequireupTeX",
        "RequirepTeXng",
        "RequireVTeX",
        "RequireAleph",
        "RequireLuaHBTeX",
        "RequireHINT",
        "RequireeTeX",
        "RequireTUTeX",
    ] {
        let signature = command_signature(name).expect("registered key command");
        commands.entry(name.into()).or_insert(CommandRule {
            arguments: Some(signature.bytes().filter(|byte| *byte == b'm').count()),
            optional_arguments: signature.contains('o'),
            ..Default::default()
        });
    }
}

pub(crate) fn is_metadata_command(name: &str) -> bool {
    command_signature(name).is_some()
}

fn arguments(source: &str, end: usize, shape: &str) -> Option<Vec<Range<usize>>> {
    let ranges = crate::layout::signature_argument_ranges(source, end..source.len(), shape.bytes());
    (ranges
        .iter()
        .filter(|range| source.as_bytes()[range.start] != b'[')
        .count()
        == shape.bytes().filter(|byte| *byte == b'm').count())
    .then_some(ranges)
}

/// Settings, defaults and expression tests are data; executable branches stay visible.
pub(crate) fn command_header_end(source: &str, name: &str, end: usize) -> Option<usize> {
    let base = name.trim_end_matches('*');
    let ranges = arguments(source, end, command_signature(name)?)?;
    match base {
        "keys_define:nn" | "ifthenelse" | "ifboolexpr" => ranges.first().map(|range| range.end),
        "ifboolKV" => ranges
            .get(usize::from(
                source.as_bytes()[ranges.first()?.start] == b'[',
            ))
            .map(|range| range.end),
        "setkeys" => None,
        "keys_set_known:nnN" => ranges.get(1).map(|range| range.end),
        "ProcessKeyvalOptions" if name.ends_with('*') => None,
        "RequirePDFTeX" | "RequireXeTeX" | "RequireLuaTeX" | "RequirepTeX" | "RequireupTeX"
        | "RequirepTeXng" | "RequireVTeX" | "RequireAleph" | "RequireLuaHBTeX" | "RequireHINT"
        | "RequireeTeX" | "RequireTUTeX" => None,
        _ => ranges.last().map(|range| range.end),
    }
}

/// Split only top-level commas. Nested groups, control symbols and comments
/// retain their exact bytes and do not become key boundaries.
pub(crate) fn definition_entries(source: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut start = 0;
    let mut cursor = 0;
    while cursor < source.len() {
        match source.as_bytes()[cursor] {
            b'%' => {
                cursor = source[cursor..]
                    .find('\n')
                    .map_or(source.len(), |offset| cursor + offset + 1)
            }
            b'\\' => cursor = syntax::command_at(source, cursor).map_or(cursor + 1, |(_, end)| end),
            b'{' => {
                cursor = syntax::balanced_end(source, cursor, b'{', b'}').unwrap_or(source.len())
            }
            b',' => {
                ranges.push(start..cursor + 1);
                cursor += 1;
                start = cursor;
            }
            _ => {
                cursor += source[cursor..]
                    .chars()
                    .next()
                    .expect("source character")
                    .len_utf8()
            }
        }
    }
    if start < source.len() {
        ranges.push(start..source.len());
    }
    ranges
}

/// Literal property values are delayed definition data. Their braces are
/// included so arbitrary TeX bodies cannot alter the surrounding parser scope.
pub(crate) fn delayed_ranges(source: &str, name: &str, end: usize) -> Vec<Range<usize>> {
    // PGF stores the function body for later expansion. Its group can be
    // closed by a helper when the function runs. TikZ graph activations
    // likewise register literal tokens, including raw math shifts.
    if matches!(
        name.trim_end_matches('*'),
        "pgfmathdeclarefunction" | "tikz@lib@prepare@active"
    ) {
        return arguments(source, end, "mmm").unwrap_or_default();
    }
    if matches!(
        name,
        "tikzset" | "pgfkeys" | "pgfqkeys" | "tcbset" | "forestset" | "gtrset"
    ) {
        let shape = if name == "pgfqkeys" { "mm" } else { "m" };
        let Some(args) = arguments(source, end, shape) else {
            return Vec::new();
        };
        let Some(body) = args
            .last()
            .filter(|range| source.as_bytes()[range.start] == b'{')
        else {
            return Vec::new();
        };
        let body = body.start + 1..body.end - 1;
        let mut ranges = Vec::new();
        for entry in definition_entries(&source[body.clone()]) {
            let text = &source[body.start + entry.start..body.start + entry.end];
            let Some(equal) = text.find('=') else {
                continue;
            };
            let key = text[..equal].trim();
            if key.contains("/.style")
                || key.contains("/.code")
                || key.contains("/.append code")
                || key.contains("/.prepend code")
            {
                let start = trivia::skip(source, body.start + entry.start + equal + 1);
                let finish = body.start + entry.end;
                // Read every group in handlers such as .style args={#1}{...}.
                let mut cursor = start;
                while source.as_bytes().get(cursor) == Some(&b'{') {
                    let Some(end) = syntax::balanced_end(source, cursor, b'{', b'}') else {
                        break;
                    };
                    if end > finish {
                        break;
                    }
                    ranges.push(cursor..end);
                    cursor = trivia::skip(source, end);
                }
            }
        }
        return ranges;
    }
    if name != "keys_define:nn" {
        return Vec::new();
    }
    let Some(args) = arguments(source, end, "mm") else {
        return Vec::new();
    };
    if args
        .iter()
        .any(|range| source.as_bytes()[range.start] != b'{')
    {
        return Vec::new();
    }
    let body = args[1].start + 1..args[1].end - 1;
    let mut ranges = Vec::new();
    for entry in definition_entries(&source[body.clone()]) {
        let mut cursor = body.start + entry.start;
        let limit = body.start + entry.end;
        while cursor < limit {
            if source.as_bytes()[cursor] == b'%' {
                cursor = trivia::skip_until(source, cursor, limit);
            } else if source.as_bytes()[cursor] == b'=' {
                let start = trivia::skip_until(source, cursor + 1, limit);
                if source.as_bytes().get(start) == Some(&b'{')
                    && let Some(end) = syntax::balanced_end(source, start, b'{', b'}')
                    && end <= limit
                {
                    ranges.push(start..end);
                }
                break;
            } else if let Some((_, end)) = syntax::command_at(source, cursor) {
                cursor = end;
            } else {
                cursor += source[cursor..]
                    .chars()
                    .next()
                    .expect("source character")
                    .len_utf8();
            }
        }
    }
    ranges
}

//! Fixed public command interfaces for tkz-tab and the tikzmark library.

use crate::config::CommandRule;
use std::collections::BTreeMap;

pub(super) fn add_command_rules(commands: &mut BTreeMap<String, CommandRule>) {
    super::extend_commands(
        commands,
        &[
            // tkz-tab.sty:129,197,264,479,776,814,841,856,888,905,922,930.
            ("tkzTabColors", 0, true),
            ("tkzTabSetup", 0, true),
            ("tkzTabInit", 2, true),
            ("tkzTabLine", 1, true),
            ("tkzTabVar", 1, true),
            ("tkzTabVal", 5, true),
            ("tkzTabIma", 4, true),
            ("tkzTabImaFrom", 4, true),
            ("tkzTabTan", 4, true),
            ("tkzTabTanFrom", 4, true),
            ("tkzTabSlope", 1, false),
            ("tkzTab", 4, true),
            // tikzlibrarytikzmark.code.tex:223,315,349,459,529,1063.
            ("tikzmarkalias", 2, false),
            ("pgfmark", 1, false),
            ("iftikzmark", 3, false),
            ("subnode", 2, true),
            ("tikzmarknode", 2, true),
            ("SaveNode", 1, true),
            ("tikzmark", 1, true),
        ],
    );
    // The exists/onpage variants open TeX conditionals after their arguments.
}

fn marker_coordinate(value: &str) -> bool {
    let value = value.trim_start();
    value.starts_with('(')
        || value.starts_with("+(")
        || value.starts_with("++(")
        || value.starts_with("\\pgfpoint")
        || value.starts_with("\\pgfqpoint")
}

fn coordinate_end(source: &str, cursor: usize) -> Option<usize> {
    let start = crate::trivia::skip(source, cursor);
    if source.as_bytes().get(start) == Some(&b'{') && marker_coordinate(&source[start + 1..]) {
        crate::syntax::balanced_end(source, start, b'{', b'}')
    } else {
        None
    }
}

pub(crate) fn command_signature(source: &str, name: &str, cursor: usize) -> Option<&'static str> {
    match name.trim_end_matches('*') {
        "tikzmark" => {
            let end = header_end(source, cursor, "dom")?;
            Some(if coordinate_end(source, end).is_some() {
                "domm"
            } else {
                "dom"
            })
        }
        "pgfmark" => Some("dm"),
        "tikzmarkalias" => Some("mm"),
        "iftikzmark" => Some("mmm"),
        "subnode" | "tikzmarknode" => Some("omm"),
        "SaveNode" => Some("om"),
        _ => None,
    }
}

pub(crate) fn is_metadata_command(name: &str) -> bool {
    matches!(
        name.trim_end_matches('*'),
        "tikzmark"
            | "pgfmark"
            | "tikzmarkalias"
            | "iftikzmark"
            | "subnode"
            | "tikzmarknode"
            | "SaveNode"
    )
}

fn mandatory_end(source: &str, start: usize) -> Option<usize> {
    match source.as_bytes().get(start)? {
        b'{' => crate::syntax::balanced_end(source, start, b'{', b'}'),
        b'\\' => crate::syntax::command_at(source, start)
            .map(|(name, end)| end - usize::from(name.len() > 1 && name.ends_with('*'))),
        b'}' | b']' => None,
        _ => source[start..].chars().next().map(|c| start + c.len_utf8()),
    }
}

fn header_end(source: &str, cursor: usize, signature: &str) -> Option<usize> {
    let mut end = cursor;
    for slot in signature.bytes() {
        let start = crate::trivia::skip(source, end);
        match slot {
            b'o' | b'd' => {
                let (open, close) = if slot == b'o' {
                    (b'[', b']')
                } else {
                    (b'<', b'>')
                };
                if source.as_bytes().get(start) == Some(&open) {
                    end = crate::syntax::balanced_end(source, start, open, close)?;
                }
            }
            b'm' => end = mandatory_end(source, start)?,
            _ => return None,
        }
    }
    Some(end)
}

pub(crate) fn command_header_end(source: &str, name: &str, cursor: usize) -> Option<usize> {
    if name.trim_end_matches('*') == "tikzmark" {
        let end = header_end(source, cursor, "dom")?;
        return Some(coordinate_end(source, end).unwrap_or(end));
    }
    let signature = match name.trim_end_matches('*') {
        // Options and node names are delayed data; the visible text remains
        // available to ordinary nested formatting.
        "subnode" | "tikzmarknode" => "om",
        "iftikzmark" => "m",
        _ => command_signature(source, name, cursor)?,
    };
    let end = header_end(source, cursor, signature)?;
    (end > cursor).then_some(end)
}

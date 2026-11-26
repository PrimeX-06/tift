//! Dependency matrices, qtree's space-delimited tree DSL and pie-chart data.
//! Package readers retain their token boundaries; explicit syntax and source
//! bindings are checked by the caller before using these literal defaults.

use std::collections::BTreeMap;
use std::ops::Range;

use crate::config::CommandRule;
use crate::syntax;
use crate::trivia::skip;

pub(crate) const ALIGNMENT_ENVIRONMENTS: &[&str] = &["deptext"];

pub(crate) fn header_signature(name: &str) -> Option<&'static str> {
    match name {
        // tikz-dependency.sty:142,150; the collected matrix body follows O{}.
        "dependency" | "deptext" => Some("o"),
        _ => None,
    }
}

pub(crate) fn default_alignment_delimiter(name: &str) -> Option<&'static str> {
    (name == "deptext").then_some(r"\\&")
}

/// Styles can replace the matrix's ampersand command. Restrict automatic
/// alignment to literal spacing keys; explicit alignment rules remain useful
/// for custom styles and delimiters.
pub(crate) fn default_separator_safe(header: &str) -> bool {
    let header = header.trim();
    let header = header
        .strip_prefix('[')
        .and_then(|header| header.strip_suffix(']'))
        .unwrap_or(header);
    header.split(',').all(|option| {
        if skip(option, 0) == option.len() {
            return true;
        }
        let Some((key, value)) = option.split_once('=') else {
            return false;
        };
        matches!(key.trim(), "column sep" | "row sep") && literal_dimension(value)
    })
}

fn literal_dimension(value: &str) -> bool {
    let start = skip(value, 0);
    let value = if value.as_bytes().get(start) == Some(&b'{') {
        let Some(end) = syntax::balanced_end(value, start, b'{', b'}') else {
            return false;
        };
        if skip(value, end) != value.len() {
            return false;
        }
        &value[start + 1..end - 1]
    } else {
        value
    };
    let mut cursor = skip(value, 0);
    if matches!(value.as_bytes().get(cursor), Some(b'+' | b'-')) {
        cursor += 1;
    }
    let mut digits = 0;
    while value.as_bytes().get(cursor).is_some_and(u8::is_ascii_digit) {
        digits += 1;
        cursor += 1;
    }
    if value.as_bytes().get(cursor) == Some(&b'.') {
        cursor += 1;
        while value.as_bytes().get(cursor).is_some_and(u8::is_ascii_digit) {
            digits += 1;
            cursor += 1;
        }
    }
    cursor = skip(value, cursor);
    digits > 0
        && matches!(
            value.get(cursor..cursor + 2),
            Some("pt" | "pc" | "in" | "bp" | "cm" | "mm" | "dd" | "cc" | "sp" | "ex" | "em")
        )
        && skip(value, cursor + 2) == value.len()
}

/// Only picture constructors run TikZ's command-shortcut installer. Plain
/// scopes retain the bindings of their enclosing picture.
pub(crate) fn reinstall_picture_commands(name: &str) -> bool {
    matches!(
        name,
        "tikzpicture" | "dependency" | "circuitikz" | "ganttchart"
    )
}

pub(crate) fn is_tikz_environment(name: &str) -> bool {
    name == "dependency"
}

pub(crate) fn is_box(name: &str) -> bool {
    header_signature(name).is_some()
}

pub(crate) fn is_non_prose(name: &str) -> bool {
    header_signature(name).is_some()
}

pub(crate) fn preserves_tight_whitespace(name: &str) -> bool {
    header_signature(name).is_some()
}

fn matrix_command(name: &str) -> bool {
    matches!(
        name,
        "xymatrix"
            | "xymatrix@"
            | "xymatrix@C"
            | "xymatrix@R"
            | "xymatrix@M"
            | "xymatrix@W"
            | "xymatrix@H"
            | "xymatrix@L"
    )
}

pub(crate) fn is_data_command(name: &str) -> bool {
    matrix_command(name)
        || matches!(
            name,
            "depstyle"
                | "depkeys"
                | "deproot"
                | "depedge"
                | "wordgroup"
                | "groupedge"
                | "Tree"
                | "pie"
                | "pgfpie@pie"
        )
}

pub(super) fn add_command_rules(commands: &mut BTreeMap<String, CommandRule>) {
    super::extend_commands(
        commands,
        &[
            // tikz-dependency.sty:139--251 (v1.1, installed TeX Live 2026).
            ("depstyle", 2, false),
            ("depkeys", 1, false),
            ("settgtlayer", 0, false),
            ("deproot", 2, true),
            ("depedge", 3, true),
            ("wordref", 2, false),
            ("rootref", 0, false),
            ("matrixref", 0, false),
            ("storelabelnode", 1, false),
            ("storefirstcorner", 1, false),
            ("storesecondcorner", 1, false),
            ("wordgroup", 4, true),
            ("groupedge", 4, true),
            // tikz-qtree.tex:41--76: the bracket is a required DSL delimiter.
            ("Tree", 0, true),
            // tikzlibrarypie.code.tex:220--226. The short `pie` alias exists
            // inside TikZ only; avoid adding that name to the global registry.
            ("pgfpie@pie", 1, true),
        ],
    );
}

fn mandatory_end(source: &str, start: usize) -> Option<usize> {
    match source.as_bytes().get(start)? {
        b'{' => syntax::balanced_end(source, start, b'{', b'}'),
        b'\\' => syntax::command_at(source, start)
            .map(|(name, end)| end - usize::from(name.len() > 1 && name.ends_with('*'))),
        b'}' => None,
        _ => source[start..]
            .chars()
            .next()
            .map(|ch| start + ch.len_utf8()),
    }
}

fn option_end(source: &str, cursor: usize) -> Option<usize> {
    let start = skip(source, cursor);
    if source.as_bytes().get(start) == Some(&b'[') {
        syntax::balanced_end(source, start, b'[', b']')
    } else {
        Some(cursor)
    }
}

/// qtree's final root label ends at a TeX space token, or at the semicolon of
/// a literal node. Braced label text shields both delimiters. Control-word
/// trailing whitespace and comment newlines do not produce TeX space tokens.
fn tree_label_end(source: &str, start: usize) -> Option<usize> {
    let mut cursor = skip(source, start);
    let node = syntax::command_at(source, cursor).is_some_and(|(name, _)| name == "node");
    while cursor < source.len() {
        match source.as_bytes()[cursor] {
            b'{' => cursor = syntax::balanced_end(source, cursor, b'{', b'}')?,
            b'\\' => {
                let (name, end) = syntax::command_at(source, cursor)?;
                cursor = end;
                if !node
                    && name
                        .bytes()
                        .all(|byte| byte.is_ascii_alphabetic() || byte == b'@')
                {
                    cursor = skip(source, cursor);
                }
            }
            b'%' => {
                cursor = source[cursor..]
                    .find('\n')
                    .map_or(source.len(), |offset| cursor + offset + 1);
            }
            b';' if node => return Some(cursor + 1),
            byte if !node && byte.is_ascii_whitespace() => return Some(cursor + 1),
            _ => cursor += source[cursor..].chars().next()?.len_utf8(),
        }
    }
    None
}

/// One borrowed range covers data read by each package. qtree's bracketed
/// subtree and pie's value/label list contain delayed drawing tokens. Keep
/// their physical whitespace intact. Dependency link labels remain available
/// for normal formatting after the protected options and numeric indices.
pub(crate) fn literal_data_range(
    source: &str,
    command_end: usize,
    name: &str,
    in_tikz: bool,
) -> Option<Range<usize>> {
    // Xy-pic reads spacing switches and the following alignment as one matrix.
    if matrix_command(name) {
        let start = skip(source, command_end);
        let brace = if name == "xymatrix" && source.as_bytes().get(start) != Some(&b'@') {
            start
        } else {
            start
                + source[start..].find(|c: char| {
                    !(c.is_ascii_alphanumeric() || "@=.+-!<>^".contains(c) || c == ' ' || c == '\t')
                })?
        };
        let end = syntax::balanced_end(source, brace, b'{', b'}')?;
        return Some(command_end..end);
    }
    if name == "Tree" {
        let start = skip(source, command_end);
        let mut end = syntax::balanced_end(source, start, b'[', b']')?;
        let after = skip(source, end);
        if source.as_bytes().get(after) == Some(&b'.') {
            end = tree_label_end(source, after + 1)?;
        }
        return Some(command_end..end);
    }
    if name == "pie" && !in_tikz {
        return None;
    }
    let (count, optional, content) = match name {
        "pie" | "pgfpie@pie" | "depkeys" => (1, name != "depkeys", false),
        "depstyle" => (2, false, false),
        "deproot" => (1, true, true),
        "depedge" => (2, true, true),
        "wordgroup" | "groupedge" => (4, true, false),
        _ => return None,
    };
    let mut end = if optional {
        option_end(source, command_end)?
    } else {
        command_end
    };
    for _ in 0..count {
        end = mandatory_end(source, skip(source, end))?;
    }
    if content {
        mandatory_end(source, skip(source, end))?;
    }
    Some(command_end..end)
}

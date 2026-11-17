//! Environment signatures shared by theorem, presentation and book sources.
//! Metadata is selected by the environment invocation, without class loading.

use std::ops::Range;

pub(crate) const LIST_ENVIRONMENTS: &[&str] = &["introduction", "problemset", "change"];

const THEOREM_ENVIRONMENTS: &[&str] = &[
    "theorem",
    "definition",
    "postulate",
    "axiom",
    "corollary",
    "lemma",
    "proposition",
    "conjecture",
    "example",
    "remark",
    "note",
    "case",
    "proof",
    "exercise",
    "problem",
];

fn theorem_environment(name: &str) -> bool {
    THEOREM_ENVIRONMENTS.contains(&name.strip_suffix('*').unwrap_or(name))
}

pub(crate) fn header_signature(name: &str) -> Option<&'static str> {
    if theorem_environment(name) {
        return Some("o");
    }
    Some(match name {
        "introduction" | "problemset" => "o",
        "custom" => "m",
        "assumption" | "conclusion" | "property" | "change" => "",
        "abstract" | "abstract*" | "titlepage" | "theindex" => "",
        "thebibliography" => "m",
        "adjustwidth" | "adjustwidth*" => "mm",
        "alertblock" | "exampleblock" => "dm",
        "overlayarea" => "mm",
        "overprint" => "o",
        "onlyenv" | "uncoverenv" | "visibleenv" | "invisibleenv" => "d",
        "altenv" => "dmmmm",
        "semiverbatim" => "",
        // Shared by presentation columns with widths and paracol's immediate
        // column body. The required width is recognized per invocation.
        "column" => "do",
        "column*" => "o",
        _ => return None,
    })
}

// Header lookahead may skip comments and a normal line ending, while an empty
// physical line ends the header and retains its paragraph token in the body.
fn header_trivia_end(source: &str, mut cursor: usize, limit: usize) -> Option<usize> {
    let mut at_line_start = false;
    while cursor < limit {
        match source.as_bytes()[cursor] {
            b' ' | b'\t' | b'\r' => cursor += 1,
            b'\n' => {
                if at_line_start {
                    return None;
                }
                at_line_start = true;
                cursor += 1;
            }
            b'%' => {
                cursor = source[cursor..limit]
                    .find('\n')
                    .map_or(limit, |offset| cursor + offset + 1);
                at_line_start = true;
            }
            _ => break,
        }
    }
    Some(cursor)
}

fn argument_end(source: &str, start: usize, limit: usize, open: u8, close: u8) -> Option<usize> {
    crate::syntax::balanced_end(source, start, open, close).filter(|end| *end <= limit)
}

/// amsthm consumes `[title]`; ElegantBook also accepts a braced title and a
/// braced label. A leading brace group alone cannot distinguish a title from
/// grouped body text. Those groups remain in the body unless configured.
/// A literal `\label{key}` immediately after the optional title is unambiguous
/// metadata in either grammar and can stay attached to the opening token.
pub(crate) fn invocation_signature(
    source: &str,
    body: Range<usize>,
    name: &str,
) -> Option<&'static str> {
    let default = header_signature(name)?;
    if name == "column" {
        source.get(body.clone())?;
        return Some(column_signature(source, body));
    }
    if !theorem_environment(name) {
        return None;
    }
    source.get(body.clone())?;
    let Some(mut cursor) = header_trivia_end(source, body.start, body.end) else {
        return Some(default);
    };
    let overlay = if let Some(end) = argument_end(source, cursor, body.end, b'<', b'>') {
        cursor = end;
        true
    } else {
        false
    };
    let fallback = if overlay { "do" } else { default };
    let Some(start) = header_trivia_end(source, cursor, body.end) else {
        return Some(fallback);
    };
    cursor = argument_end(source, start, body.end, b'[', b']').unwrap_or(start);
    let Some(start) = header_trivia_end(source, cursor, body.end) else {
        return Some(fallback);
    };
    let Some(("label", end)) = crate::syntax::command_at(source, start) else {
        return Some(fallback);
    };
    let Some(start) = header_trivia_end(source, end, body.end) else {
        return Some(fallback);
    };
    if argument_end(source, start, body.end, b'{', b'}').is_none() {
        return Some(fallback);
    }
    Some(if overlay { "dolm" } else { "olm" })
}

fn column_signature(source: &str, body: Range<usize>) -> &'static str {
    let Some(mut cursor) = header_trivia_end(source, body.start, body.end) else {
        return "do";
    };
    for (open, close) in [(b'<', b'>'), (b'[', b']')] {
        if let Some(end) = argument_end(source, cursor, body.end, open, close) {
            let Some(next) = header_trivia_end(source, end, body.end) else {
                return "do";
            };
            cursor = next;
        }
    }
    let Some(end) = argument_end(source, cursor, body.end, b'{', b'}') else {
        return "do";
    };
    if column_width(&source[cursor + 1..end - 1]) {
        "dom"
    } else {
        "do"
    }
}

fn column_width(value: &str) -> bool {
    let mut cursor = crate::trivia::skip(value, 0);
    if value[cursor..].starts_with("\\dimexpr") {
        return crate::syntax::command_at(value, cursor).is_some_and(|(name, _)| name == "dimexpr");
    }
    if matches!(value.as_bytes().get(cursor), Some(b'+' | b'-')) {
        cursor += 1;
    }
    let number_start = cursor;
    while value.as_bytes().get(cursor).is_some_and(u8::is_ascii_digit) {
        cursor += 1;
    }
    if matches!(value.as_bytes().get(cursor), Some(b'.' | b',')) {
        cursor += 1;
        while value.as_bytes().get(cursor).is_some_and(u8::is_ascii_digit) {
            cursor += 1;
        }
    }
    let has_number = value.as_bytes()[number_start..cursor]
        .iter()
        .any(u8::is_ascii_digit);
    if cursor > number_start && !has_number {
        return false;
    }
    cursor = crate::trivia::skip(value, cursor);
    let end = if let Some((name, end)) = crate::syntax::command_at(value, cursor) {
        if !matches!(
            name,
            "textwidth" | "columnwidth" | "linewidth" | "paperwidth" | "hsize"
        ) {
            return false;
        }
        end
    } else if has_number
        && matches!(
            value.get(cursor..cursor + 2),
            Some("pt" | "pc" | "in" | "bp" | "cm" | "mm" | "dd" | "cc" | "sp" | "ex" | "em")
        )
    {
        cursor + 2
    } else {
        return false;
    };
    let end = crate::trivia::skip(value, end);
    end == value.len() || value.get(end..) == Some("\\relax")
}

pub(crate) fn preserves_tight_whitespace(name: &str) -> bool {
    theorem_environment(name)
        || matches!(
            name,
            "custom" | "solution" | "assumption" | "conclusion" | "property" | "column" | "column*"
        )
}

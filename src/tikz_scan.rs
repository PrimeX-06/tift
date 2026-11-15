//! Shared literal TikZ/pgffor boundaries. Scans borrow their enclosing range.

use std::ops::Range;

use crate::syntax;
use crate::trivia::skip_until;

pub(crate) fn path_command(name: &str) -> bool {
    matches!(
        name,
        "draw"
            | "path"
            | "fill"
            | "filldraw"
            | "shade"
            | "shadedraw"
            | "clip"
            | "useasboundingbox"
            | "node"
            | "coordinate"
            | "matrix"
            | "pic"
            // pgfplots: a digit 3 and/or + can follow this control word.
            | "addplot"
    )
}

pub(crate) fn environment(name: &str) -> bool {
    if crate::packages::tikz_diagrams::is_tikz_environment(name) {
        return true;
    }
    crate::packages::tikz_derived::is_tikz_environment(name)
        || matches!(
            name,
            "tikzpicture" | "scope" | "axis" | "groupplot" | "circuitikz" | "feynman"
        )
}

pub(crate) fn comment_end(source: &str, start: usize, limit: usize) -> usize {
    memchr::memchr(b'\n', &source.as_bytes()[start..limit])
        .map_or(limit, |offset| start + offset + 1)
}

/// Match braces, options or coordinates. Braces protect other delimiters.
/// Counters replace recursive scans, without allocating a nesting stack.
pub(crate) fn group_end(
    source: &str,
    start: usize,
    limit: usize,
    protected: &[Range<usize>],
) -> Option<usize> {
    let source = source.get(..limit)?;
    let open = *source.as_bytes().get(start)?;
    let close = match open {
        b'{' => b'}',
        b'[' => b']',
        b'(' => b')',
        _ => return None,
    };
    let mut braces = usize::from(open == b'{');
    let mut delimiters = usize::from(open != b'{');
    let mut cursor = start + 1;
    while cursor < limit {
        if let Some(range) = protected
            .get(protected.partition_point(|range| range.end <= cursor))
            .filter(|range| range.start <= cursor)
        {
            if range.end > limit {
                return None;
            }
            cursor = range.end;
            continue;
        }
        match source.as_bytes()[cursor] {
            b'%' => cursor = comment_end(source, cursor, limit),
            b'\\' => cursor = syntax::command_at(source, cursor)?.1,
            b'{' => {
                braces += 1;
                cursor += 1;
            }
            b'}' if braces > 0 => {
                braces -= 1;
                cursor += 1;
                if open == b'{' && braces == 0 {
                    return Some(cursor);
                }
            }
            byte if braces == 0 && byte == open => {
                delimiters += 1;
                cursor += 1;
            }
            byte if braces == 0 && byte == close => {
                delimiters -= 1;
                cursor += 1;
                if delimiters == 0 {
                    return Some(cursor);
                }
            }
            _ => cursor += 1,
        }
    }
    None
}

/// Node/matrix text follows literal options, names and `at (coordinate)`.
pub(crate) fn node_body(
    source: &str,
    mut cursor: usize,
    limit: usize,
    protected: &[Range<usize>],
) -> Option<Range<usize>> {
    let source = source.get(..limit)?;
    loop {
        cursor = skip_until(source, cursor, limit);
        match source.as_bytes().get(cursor).copied()? {
            b'[' | b'(' => cursor = group_end(source, cursor, limit, protected)?,
            b'{' => {
                let end = group_end(source, cursor, limit, protected)?;
                return Some(cursor + 1..end - 1);
            }
            b'a' if source[cursor..].starts_with("at")
                && source.as_bytes().get(cursor + 2).is_some_and(|byte| {
                    byte.is_ascii_whitespace() || matches!(byte, b'(' | b'%')
                }) =>
            {
                cursor = skip_until(source, cursor + 2, limit);
                if source.as_bytes().get(cursor) != Some(&b'(') {
                    return None;
                }
                cursor = group_end(source, cursor, limit, protected)?;
            }
            _ => return None,
        }
    }
}

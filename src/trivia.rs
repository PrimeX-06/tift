//! ASCII whitespace and comment lookahead for literal package arguments.

use std::ops::Range;

/// Locate runs of two or more ASCII spaces.
pub(crate) fn repeated_spaces(source: &str) -> impl Iterator<Item = Range<usize>> + '_ {
    let mut cursor = 0;
    let finder = memchr::memmem::Finder::new(b"  ");
    std::iter::from_fn(move || {
        let start = cursor + finder.find(&source.as_bytes()[cursor..])?;
        cursor = start + 2;
        while source.as_bytes().get(cursor) == Some(&b' ') {
            cursor += 1;
        }
        Some(start..cursor)
    })
}

/// Skip ordinary separators without crossing the enclosing source range.
/// Callers with paragraph-sensitive or token-adjacent lookahead keep their
/// own rules; Unicode whitespace remains argument content here.
pub(crate) fn skip_until(source: &str, mut cursor: usize, limit: usize) -> usize {
    loop {
        while cursor < limit && source.as_bytes()[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor >= limit || source.as_bytes()[cursor] != b'%' {
            return cursor;
        }
        cursor = source[cursor..limit]
            .find('\n')
            .map_or(limit, |offset| cursor + offset + 1);
    }
}

pub(crate) fn skip(source: &str, cursor: usize) -> usize {
    skip_until(source, cursor, source.len())
}

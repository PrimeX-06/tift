//! ASCII whitespace and comment lookahead for literal package arguments.

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

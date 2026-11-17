//! Shared argument boundaries for built-in compact and xparse signatures.

use super::{balanced_end, command_at, xparse};

pub(crate) fn signature_end(source: &str, cursor: usize, signature: &str) -> Option<usize> {
    if xparse::extended(signature) {
        return xparse::Signature::parse(signature)?.end(source, cursor..source.len());
    }
    let mut end = cursor;
    for slot in signature.bytes() {
        let start = crate::trivia::skip(source, end);
        end = match (slot, source.as_bytes().get(start)) {
            (b'o' | b'O', Some(b'[')) => balanced_end(source, start, b'[', b']')?,
            (b'p', Some(b'(')) => balanced_end(source, start, b'(', b')')?,
            (b'o' | b'O' | b'p', _) => continue,
            (b'm', Some(b'{')) => balanced_end(source, start, b'{', b'}')?,
            (b'm', Some(b'\\')) => command_at(source, start)
                .map(|(name, end)| end - usize::from(name.len() > 1 && name.ends_with('*')))?,
            (b'm', Some(b'}')) => return None,
            (b'm', Some(_)) => start + source[start..].chars().next()?.len_utf8(),
            _ => return None,
        };
    }
    Some(end)
}

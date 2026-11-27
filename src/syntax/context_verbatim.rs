//! ConTeXt verbatim input uses literal delimiters and line-oriented buffers.
use super::{command_at, skip_trivia};

pub(super) fn document(source: &str) -> bool {
    let start = skip_trivia(source, 0);
    command_at(source, start).is_some_and(|(name, _)| {
        matches!(
            name,
            "startcomponent"
                | "startdocument"
                | "starttext"
                | "startproject"
                | "startproduct"
                | "startenvironment"
                | "usemodule"
                | "useMPlibrary"
                | "setupbodyfont"
        ) || name == "environment"
            && (source.contains(r"\startcomponent") || source.contains(r"\startdocument"))
    })
}

pub(super) fn reader_end(source: &str, name: &str, end: usize) -> Option<usize> {
    if name == "discretionary" {
        let class = source[skip_trivia(source, end)..].strip_prefix("class")?;
        let number = class.trim_start();
        let digits = number.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            return None;
        }
        let mut end = source.len() - number.len() + digits;
        for _ in 0..3 {
            end = super::balanced_end(source, skip_trivia(source, end), b'{', b'}')?;
        }
        return Some(end);
    }
    literal_reader_end(source, name, end)
}

pub(super) fn literal_reader_end(source: &str, name: &str, end: usize) -> Option<usize> {
    match name {
        "type" | "typ" => {
            let mut start = skip_trivia(source, end);
            if source[start..].starts_with('[') {
                start = skip_trivia(source, super::balanced_end(source, start, b'[', b']')?);
            }
            let delimiter = source[start..].chars().next()?;
            let body = start + delimiter.len_utf8();
            let close = match delimiter {
                '{' => return crate::code_package_syntax::raw_group_end(source, start),
                '<' if source[body..].starts_with('<') => '>',
                '<' => return None,
                c if c.is_alphanumeric() || c.is_whitespace() => return None,
                c => return source[body..].find(c).map(|at| body + at + c.len_utf8()),
            };
            let mut depth = 1;
            for (at, ch) in source[body..].char_indices() {
                if ch == delimiter {
                    depth += 1;
                } else if ch == close {
                    depth -= 1;
                    if depth == 0 {
                        return Some(body + at + 1);
                    }
                }
            }
            None
        }
        "startbuffer" | "starttyping" | "startluacode" | "startMPcode" | "startMPdefinitions"
        | "startMPinclusions" => {
            let suffix = &name["start".len()..];
            let mut depth = 1;
            for (offset, _) in source[end..].match_indices('\\') {
                let at = end + offset;
                let Some((marker, finish)) = command_at(source, at) else {
                    continue;
                };
                if marker == name {
                    depth += 1;
                } else if marker.strip_prefix("stop") == Some(suffix) {
                    depth -= 1;
                    if depth == 0 {
                        return Some(finish);
                    }
                }
            }
            None
        }
        _ => None,
    }
}

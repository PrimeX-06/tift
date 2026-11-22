//! Structural parser for expl3 function names and their argument signatures.

use std::ops::Range;

use crate::{syntax, trivia};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArgumentKind {
    Token,
    Value,
    General,
    Branch,
    ParameterText,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Argument {
    pub(crate) kind: ArgumentKind,
    pub(crate) range: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Call<'a> {
    pub(crate) name: &'a str,
    pub(crate) base: &'a str,
    pub(crate) signature: &'a str,
    pub(crate) arguments: Vec<Argument>,
    pub(crate) end: usize,
}

pub(super) fn is_expl3_name(name: &str) -> bool {
    let Some((base, signature)) = name.rsplit_once(':') else {
        return false;
    };
    !base.is_empty()
        && base
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'@'))
        && signature
            .bytes()
            .all(|byte| b"NncVvofxeTFpwD".contains(&byte))
}

fn tex_argument_end(source: &str, start: usize) -> Option<usize> {
    match source.as_bytes().get(start)? {
        b'{' => syntax::balanced_end(source, start, b'{', b'}'),
        b'}' | b'%' => None,
        b'\\' => syntax::command_at(source, start)
            .map(|(name, end)| end - usize::from(name.len() > 1 && name.ends_with('*'))),
        b'#' => {
            let digits = start
                + source.as_bytes()[start..]
                    .iter()
                    .take_while(|&&byte| byte == b'#')
                    .count();
            Some(digits + usize::from(matches!(source.as_bytes().get(digits), Some(b'1'..=b'9'))))
        }
        _ => Some(start + source[start..].chars().next()?.len_utf8()),
    }
}

pub(crate) fn parse_call<'a>(source: &str, name: &'a str, command_end: usize) -> Option<Call<'a>> {
    if !is_expl3_name(name) {
        return None;
    }
    let (base, signature) = name.rsplit_once(':')?;
    if signature.bytes().any(|kind| matches!(kind, b'w' | b'D')) {
        return None;
    }

    let mut arguments = Vec::with_capacity(signature.len());
    let mut cursor = command_end;
    for kind in signature.bytes() {
        let start = trivia::skip(source, cursor);
        let (argument_kind, end) = if kind == b'p' {
            (
                ArgumentKind::ParameterText,
                syntax::parameter_text_end(source, start)?,
            )
        } else {
            let end = tex_argument_end(source, start)?;
            let argument_kind = match kind {
                b'N' => ArgumentKind::Token,
                b'V' => ArgumentKind::Value,
                b'T' | b'F' => ArgumentKind::Branch,
                _ => ArgumentKind::General,
            };
            (argument_kind, end)
        };
        arguments.push(Argument {
            kind: argument_kind,
            range: start..end,
        });
        cursor = end;
    }

    Some(Call {
        name,
        base,
        signature,
        arguments,
        end: cursor,
    })
}

pub(crate) fn is_code_argument(call: &Call, slot: usize, kind: u8) -> bool {
    if slot == 1
        && matches!(
            call.base,
            "str_case" | "str_case_e" | "tl_case" | "int_case" | "dim_case"
        )
    {
        return true;
    }
    if call.base == "exp_args" && slot == 0 && kind == b'N' {
        return true;
    }
    if call.base == "reverse_if" && slot == 0 && kind == b'N' {
        return true;
    }
    if slot + 1 == call.arguments.len()
        && matches!(
            (call.base, call.signature),
            ("hook_gput_code", "nnn")
                | ("hook_gput_next_code", "nn")
                | ("hook_gput_next_code", "ne")
        )
    {
        return true;
    }
    if matches!(kind, b'T' | b'F') {
        return true;
    }
    if crate::syntax::expl3_definition_variant(call.name).is_some() {
        return slot + 1 == call.arguments.len();
    }

    let name = call.base;
    let last = slot + 1 == call.arguments.len();
    let inline_callback = name.split('_').any(|part| part == "inline")
        || ["_do_while", "_while_do", "_do_until", "_until_do"]
            .iter()
            .any(|pattern| name.contains(pattern));
    last && matches!(kind, b'm' | b'n') && inline_callback
}

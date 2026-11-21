//! Shared argument boundaries for built-in compact and xparse signatures.

use super::{balanced_end, command_at, xparse};
use std::ops::Range;

/// A literal pgffor loop binds each slash-separated control sequence until the
/// end of its braced body. Unknown list/body syntax supplies no inferred scope.
pub(crate) struct ForeachBinding<'a> {
    pub names: Vec<&'a str>,
    pub end: usize,
}

pub(crate) fn foreach_binding(
    source: &str,
    end: usize,
    group_end: impl FnOnce(usize) -> Option<usize>,
) -> Option<ForeachBinding<'_>> {
    let mut cursor = crate::trivia::skip(source, end);
    let mut names = Vec::new();
    loop {
        let (name, finish) = command_at(source, cursor)?;
        if name.ends_with('*') || name.is_empty() {
            return None;
        }
        names.push(name);
        cursor = crate::trivia::skip(source, finish);
        if source.as_bytes().get(cursor) != Some(&b'/') {
            break;
        }
        cursor = crate::trivia::skip(source, cursor + 1);
    }
    if source.as_bytes().get(cursor) == Some(&b'[') {
        cursor = crate::trivia::skip(source, balanced_end(source, cursor, b'[', b']')?);
    }
    if !source[cursor..].starts_with("in")
        || source
            .as_bytes()
            .get(cursor + 2)
            .is_some_and(u8::is_ascii_alphabetic)
    {
        return None;
    }
    cursor = crate::trivia::skip(source, cursor + 2);
    cursor = crate::trivia::skip(source, balanced_end(source, cursor, b'{', b'}')?);
    if source.as_bytes().get(cursor) != Some(&b'{') {
        return None;
    }
    let end = group_end(cursor)?;
    Some(ForeachBinding { names, end })
}

/// Literal argument grammars at the scanner's current TeX scope. An unknown
/// declaration shadows an earlier grammar until its defining scope closes.
pub(super) struct SignatureBindings {
    scopes: Vec<[std::collections::HashSet<String>; 2]>,
    values: [std::collections::HashMap<String, Vec<ScopedSignature>>; 2],
}

struct ScopedSignature {
    depth: usize,
    binding: Option<BoundSignature>,
    inline_code: Option<crate::code_package_syntax::GeneratedInlineKind>,
}

struct BoundSignature {
    syntax: xparse::Signature,
    math: Option<crate::packages::math::Signature>,
    literal: bool,
    literal_slots: u16,
}

impl Default for SignatureBindings {
    fn default() -> Self {
        Self {
            scopes: vec![Default::default()],
            values: Default::default(),
        }
    }
}

impl SignatureBindings {
    pub(super) fn enter(&mut self) {
        self.scopes.push(Default::default());
    }

    pub(super) fn leave(&mut self) {
        if self.scopes.len() > 1 {
            let depth = self.scopes.len() - 1;
            let names = self.scopes.pop().expect("local argument scope");
            for (index, names) in names.into_iter().enumerate() {
                for name in names {
                    if let Some(values) = self.values[index].get_mut(&name) {
                        if values.last().is_some_and(|value| value.depth == depth) {
                            values.pop();
                        }
                        if values.is_empty() {
                            self.values[index].remove(&name);
                        }
                    }
                }
            }
        }
    }

    pub(super) fn get(&self, environment: bool, name: &str) -> Option<&xparse::Signature> {
        self.values[usize::from(environment)]
            .get(name)?
            .last()?
            .binding
            .as_ref()
            .map(|binding| &binding.syntax)
    }

    pub(super) fn math_signature(&self, name: &str) -> Option<crate::packages::math::Signature> {
        self.values[0].get(name)?.last()?.binding.as_ref()?.math
    }

    pub(super) fn literal(&self, name: &str) -> bool {
        self.values[0]
            .get(name)
            .and_then(|values| values.last())
            .and_then(|value| value.binding.as_ref())
            .is_some_and(|binding| binding.literal)
    }

    pub(super) fn mark_literal(&mut self, name: &str) {
        if let Some(binding) = self.values[0]
            .get_mut(name)
            .and_then(|values| values.last_mut())
            .and_then(|value| value.binding.as_mut())
        {
            binding.literal = true;
        }
    }

    pub(super) fn set_literal(&mut self, name: &str, literal: bool) {
        if let Some(binding) = self.values[0]
            .get_mut(name)
            .and_then(|values| values.last_mut())
            .and_then(|value| value.binding.as_mut())
        {
            binding.literal = literal;
        }
    }

    pub(super) fn mark_literal_slots(&mut self, name: &str, slots: u16) {
        if let Some(binding) = self.values[0]
            .get_mut(name)
            .and_then(|values| values.last_mut())
            .and_then(|value| value.binding.as_mut())
        {
            binding.literal_slots = slots;
        }
    }

    pub(super) fn literal_slots(&self, name: &str) -> u16 {
        self.values[0]
            .get(name)
            .and_then(|values| values.last())
            .and_then(|value| value.binding.as_ref())
            .map_or(0, |binding| binding.literal_slots)
    }

    pub(super) fn import(&mut self, name: &str, command: &crate::config::ImportedCommand) {
        let signature = command.arguments.as_ref().and_then(|arguments| {
            let signature = arguments.signature.clone().unwrap_or_else(|| {
                format!(
                    "{}{}",
                    if arguments.optional_arguments {
                        "o"
                    } else {
                        ""
                    },
                    "m".repeat(arguments.arguments)
                )
            });
            xparse::Signature::parse(&signature)
        });
        self.assign(false, name.to_owned(), signature, None, command.global);
        if let Some(binding) = self.values[0]
            .get_mut(name)
            .and_then(|values| values.last_mut())
            .and_then(|value| value.binding.as_mut())
        {
            binding.literal = command.literal_arguments;
        }
    }

    pub(super) fn contains(&self, environment: bool, name: &str) -> bool {
        self.values[usize::from(environment)].contains_key(name)
    }

    pub(super) fn inline_code(
        &self,
        name: &str,
    ) -> Option<crate::code_package_syntax::GeneratedInlineKind> {
        self.values[0].get(name)?.last()?.inline_code
    }

    pub(super) fn assign_inline_code(
        &mut self,
        name: String,
        kind: crate::code_package_syntax::GeneratedInlineKind,
        global: bool,
    ) {
        self.assign(false, name.clone(), None, None, global);
        self.values[0]
            .get_mut(&name)
            .and_then(|values| values.last_mut())
            .expect("fresh inline declaration")
            .inline_code = Some(kind);
    }

    pub(super) fn assign(
        &mut self,
        environment: bool,
        name: String,
        signature: Option<xparse::Signature>,
        math: Option<crate::packages::math::Signature>,
        global: bool,
    ) {
        let index = usize::from(environment);
        let depth = if global { 0 } else { self.scopes.len() - 1 };
        let values = self.values[index].entry(name.clone()).or_default();
        let binding = signature.map(|syntax| BoundSignature {
            syntax,
            math,
            literal: false,
            literal_slots: 0,
        });
        if global {
            values.clear();
        }
        if let Some(value) = values.last_mut().filter(|value| value.depth == depth) {
            value.binding = binding;
            value.inline_code = None;
        } else {
            values.push(ScopedSignature {
                depth,
                binding,
                inline_code: None,
            });
        }
        self.scopes[depth][index].insert(name);
    }
}

/// Accept only an uninterrupted sequence of undelimited TeX parameters.
/// Literal suffixes and spaces after a parameter are delimiters.
pub(crate) fn undelimited_signature(parameter_text: &str) -> Option<String> {
    let text = parameter_text.trim_start();
    if text.len() > 18 || !text.len().is_multiple_of(2) {
        return None;
    }
    text.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .enumerate()
        .all(|(index, chunk)| *chunk == [b'#', b'1' + index as u8])
        .then(|| "m".repeat(text.len() / 2))
}

/// Literal primitive delimiters share the xparse boundary representation.
/// A one-token prefix can open the first slot; following slots end at their
/// declared token sequence. Catcode-dependent and whitespace delimiters stay
/// unknown so the reader cannot change their meaning.
pub(crate) fn parameter_signature(parameter_text: &str) -> Option<String> {
    let uncommented;
    let parameter_text = if parameter_text.contains('%') {
        let mut output = String::new();
        let mut cursor = 0;
        while cursor < parameter_text.len() {
            if parameter_text.as_bytes()[cursor] == b'%' {
                cursor += parameter_text[cursor..]
                    .find('\n')
                    .map_or(parameter_text.len() - cursor, |at| at + 1);
            } else if let Some((_, end)) = command_at(parameter_text, cursor) {
                output.push_str(&parameter_text[cursor..end]);
                cursor = end;
            } else {
                let character = parameter_text[cursor..].chars().next()?;
                output.push(character);
                cursor += character.len_utf8();
            }
        }
        uncommented = output;
        &uncommented
    } else {
        parameter_text
    };
    if let Some(signature) = undelimited_signature(parameter_text) {
        return Some(signature);
    }
    let text = parameter_text.trim_start();
    let parameters: Vec<_> = text.match_indices('#').map(|(at, _)| at).collect();
    if parameters.is_empty() || parameters.len() > 9 {
        return None;
    }
    for (index, &at) in parameters.iter().enumerate() {
        if text.as_bytes().get(at + 1) != Some(&(b'1' + index as u8)) {
            return None;
        }
    }
    let literal = |text: &str| {
        !text.is_empty()
            && !text
                .chars()
                .any(|character| character.is_whitespace() || "{}%#".contains(character))
    };
    let prefix = &text[..parameters[0]];
    if !prefix.is_empty()
        && (!literal(prefix)
            || command_at(prefix, 0).map_or_else(
                || prefix.chars().count() != 1,
                |(_, end)| end != prefix.len(),
            ))
    {
        return None;
    }
    let mut signature = String::new();
    for (index, &at) in parameters.iter().enumerate() {
        let delimiter = &text[at + 2..parameters.get(index + 1).copied().unwrap_or(text.len())];
        if index == 0 && !prefix.is_empty() {
            if !literal(delimiter)
                || command_at(delimiter, 0).map_or_else(
                    || delimiter.chars().count() != 1,
                    |(_, end)| end != delimiter.len(),
                )
            {
                return None;
            }
            signature.push_str(&format!("r{prefix} {delimiter} "));
        } else if delimiter.is_empty() {
            signature.push_str("m ");
        } else if literal(delimiter) {
            signature.push_str(&format!("u{{{delimiter}}} "));
        } else {
            return None;
        }
    }
    xparse::Signature::parse(&signature)?;
    Some(signature.trim_end().to_owned())
}

pub(super) fn configured_signature(rule: &crate::config::CommandRule) -> Option<xparse::Signature> {
    if let Some(signature) = &rule.signature {
        return xparse::Signature::parse(signature);
    }
    let count = rule.arguments.filter(|&count| count <= 9)?;
    xparse::Signature::parse(&format!(
        "{}{}",
        if rule.optional_arguments { "o" } else { "" },
        "m".repeat(count)
    ))
}

pub(crate) fn invocation_cursor(source: &str, end: usize, shape: &str) -> usize {
    if source.as_bytes().get(end.wrapping_sub(1)) != Some(&b'*') {
        return end;
    }
    xparse::Signature::parse(shape)
        .map_or(end, |signature| signature.invocation_cursor(source, end))
}

/// Data domains use logical argument slots, including absent optionals and
/// individual embellishments. All providers share the xparse boundary parser.
pub(crate) fn domain_ranges(
    source: &str,
    cursor: usize,
    shape: &str,
    domains: &str,
) -> Option<Vec<Range<usize>>> {
    let signature = xparse::Signature::parse(shape)?;
    let cursor = signature.invocation_cursor(source, cursor);
    let slots = signature.slot_ranges(source, cursor..source.len())?;
    let mut ranges = Vec::new();
    for argument in slots {
        if *domains.as_bytes().get(argument.slot)? == b'U' {
            ranges.push(argument.range);
        }
    }
    Some(ranges)
}

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

/// Diagnose an argument whose opening delimiter is present but never closes.
/// Absent optional arguments and missing token operands remain distinct from
/// this certain lexical failure.
pub(crate) fn unclosed_argument(
    source: &str,
    mut cursor: usize,
    signature: &str,
) -> Option<Range<usize>> {
    if xparse::extended(signature) {
        return None;
    }
    for slot in signature.bytes() {
        let start = crate::trivia::skip(source, cursor);
        let open = match slot {
            b'o' | b'O' => b'[',
            b'p' => b'(',
            b'd' => b'<',
            b'm' | b'g' => b'{',
            _ => return None,
        };
        if source.as_bytes().get(start) == Some(&open) {
            let close = match open {
                b'[' => b']',
                b'(' => b')',
                b'<' => b'>',
                _ => b'}',
            };
            let Some(end) = balanced_end(source, start, open, close) else {
                return Some(start..source.len());
            };
            cursor = end;
        } else if slot == b'm' {
            cursor = if source.as_bytes().get(start) == Some(&b'\\') {
                let (name, end) = command_at(source, start)?;
                end - usize::from(name.len() > 1 && name.ends_with('*'))
            } else {
                start + source.get(start..)?.chars().next()?.len_utf8()
            };
        }
    }
    None
}

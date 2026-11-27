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
    delimited: Option<DelimitedReader>,
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

    pub(super) fn copied_verb(&self, name: &str) -> bool {
        self.values[0]
            .get(name)
            .and_then(|values| values.last())
            .map_or(name == "verb", |value| {
                matches!(value.delimited, Some(DelimitedReader::CopiedVerb))
            })
    }

    pub(super) fn mark_copied_verb(&mut self, name: &str) {
        if let Some(value) = self.values[0]
            .get_mut(name)
            .and_then(|values| values.last_mut())
        {
            value.delimited = Some(DelimitedReader::CopiedVerb);
        }
    }

    pub(super) fn observe_reader(&mut self, name: &str, body: &str) {
        let Some(binding) = self.values[0].get_mut(name).and_then(|v| v.last_mut()) else {
            return;
        };
        if !body.contains("##1#")
            && !body.contains(r"\begingroup")
            && !body.contains(r"\expandafter")
            && !body.contains(r"\tokencycle")
            && !body.trim_start().starts_with(r"\verb")
        {
            return;
        }
        let Some(shape) = binding.binding.as_ref().and_then(|b| b.syntax.compact()) else {
            return;
        };
        binding.delimited = DelimitedReader::from_body(body, &shape);
    }

    pub(super) fn delimited_end<'a>(
        &'a self,
        source: &str,
        mut at: usize,
        mut name: &'a str,
        tokcycle_loaded: impl Fn() -> bool,
    ) -> Option<usize> {
        let mut raw = false;
        let mut remaining = self.values[0].len();
        while remaining > 0 {
            let reader = self.values[0].get(name)?.last()?.delimited.as_ref()?;
            match reader {
                DelimitedReader::CopiedVerb => return super::inline_end(source, "verb", at),
                DelimitedReader::PercentGroupForward(target) => {
                    at = super::skip_trivia(source, at);
                    if source.as_bytes().get(at) == Some(&b'[') {
                        at = balanced_end(source, at, b'[', b']')?;
                    }
                    name = target;
                    raw = true;
                }
                DelimitedReader::TokenCycleGroup => {
                    if !raw || self.contains(false, "tokencycle") || !tokcycle_loaded() {
                        return None;
                    }
                    at = super::skip_trivia(source, at);
                    if source.as_bytes().get(at) == Some(&b'[') {
                        return None;
                    }
                    return super::literal_group_end(source, at);
                }
                DelimitedReader::Verbatim(delimiter) => {
                    if self.contains(false, "verb") {
                        return None;
                    }
                    return source[at..super::line_end(source, at)]
                        .find(*delimiter)
                        .map(|offset| at + offset + delimiter.len_utf8());
                }
                DelimitedReader::Forward {
                    target,
                    changes_catcodes,
                } => {
                    raw |= changes_catcodes;
                    name = target;
                }
                DelimitedReader::Capture {
                    optional,
                    changes_catcodes,
                } => {
                    if !raw && !changes_catcodes {
                        return None;
                    }
                    at = super::skip_trivia(source, at);
                    if *optional && source.as_bytes().get(at) == Some(&b'[') {
                        at = super::skip_trivia(source, balanced_end(source, at, b'[', b']')?);
                    }
                    let delimiter = *source.as_bytes().get(at)?;
                    if !delimiter.is_ascii_punctuation() || b"\\{}%#".contains(&delimiter) {
                        return None;
                    }
                    return source[at + 1..]
                        .find(char::from(delimiter))
                        .map(|i| at + i + 2);
                }
            }
            remaining -= 1;
        }
        None
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
            value.delimited = None;
        } else {
            values.push(ScopedSignature {
                depth,
                binding,
                inline_code: None,
                delimited: None,
            });
        }
        self.scopes[depth][index].insert(name);
    }
}

/// An outer parameter supplies the final delimiter of a nested TeX reader.
/// Forwarders are resolved at invocation so that later definitions and scopes apply.
enum DelimitedReader {
    CopiedVerb,
    PercentGroupForward(String),
    TokenCycleGroup,
    Verbatim(char),
    Forward {
        target: String,
        changes_catcodes: bool,
    },
    Capture {
        optional: bool,
        changes_catcodes: bool,
    },
}

impl DelimitedReader {
    fn from_body(body: &str, shape: &str) -> Option<Self> {
        // An optional setup argument selects local printing options. Require
        // both branches to consist solely of definitions before the common
        // catcode assignment and tail call.
        if shape == "o" && body.contains(r"\catcode") {
            let mut rest = body
                .trim()
                .strip_prefix(r"\begingroup")?
                .trim_start()
                .strip_prefix(r"\ifx\relax#1\relax")?;
            for terminator in [r"\else", r"\fi"] {
                rest = rest.trim_start();
                while let Some(("def", end)) = command_at(rest, 0) {
                    let (_, after) = command_at(rest, super::skip_trivia(rest, end))?;
                    let finish = balanced_end(rest, super::skip_trivia(rest, after), b'{', b'}')?;
                    rest = rest[finish..].trim_start();
                }
                rest = rest.strip_prefix(terminator)?;
            }
            let ("catcode", end) = command_at(rest, super::skip_trivia(rest, 0))? else {
                return None;
            };
            let (37, 12, end) = super::literal_catcode_assignment(rest, end, false)? else {
                return None;
            };
            let (target, end) = command_at(rest, super::skip_trivia(rest, end))?;
            return (super::skip_trivia(rest, end) == rest.len())
                .then(|| Self::PercentGroupForward(target.to_owned()));
        }
        if shape == "om" && body.contains(r"\tokencycle") {
            let mut at = super::skip_trivia(body, 0);
            if let Some(("ttfamily", end)) = command_at(body, at) {
                at = super::skip_trivia(body, end);
            }
            let ("tokencycle", mut at) = command_at(body, at)? else {
                return None;
            };
            for _ in 0..4 {
                at = balanced_end(body, super::skip_trivia(body, at), b'{', b'}')?;
            }
            return (body[super::skip_trivia(body, at)..].trim_end()
                == r"#2\endtokencycle\endgroup")
                .then_some(Self::TokenCycleGroup);
        }
        if shape.is_empty() {
            let body = body.trim_start();
            if let Some(("verb" | "verb*", end)) = command_at(body, 0) {
                let mut rest = body[end..].chars();
                if let Some(delimiter) = rest.next()
                    && delimiter.is_ascii_punctuation()
                    && !matches!(delimiter, '\\' | '{' | '}' | '%' | '#')
                    && rest.as_str().is_empty()
                {
                    return Some(Self::Verbatim(delimiter));
                }
            }
        }
        // Find the terminal control and ignore comments between definition and
        // invocation. Only recognize a scanner actually called in tail position.
        let params = match shape {
            "" => None,
            "m" => Some("##1#1"),
            "om" => Some("##1#2"),
            _ => return None,
        };
        let mut tail = None;
        let mut at = 0;
        let mut grouped = false;
        let mut changes_catcodes = false;
        while at < body.len() {
            at = super::skip_trivia(body, at);
            if at == body.len() {
                break;
            }
            if body.as_bytes()[at] == b'{' {
                at = balanced_end(body, at, b'{', b'}')?;
                tail = None;
                continue;
            }
            let Some((name, end)) = command_at(body, at) else {
                at += body[at..].chars().next()?.len_utf8();
                tail = None;
                continue;
            };
            if name.starts_with("if") || matches!(name, "else" | "fi" | "expandafter" if at != 0) {
                return None;
            }
            if let Some(params) = params
                && name == "def"
                && let Some((reader, after)) = command_at(body, super::skip_trivia(body, end))
                && body[after..].starts_with(params)
                && let Some(group_end) = balanced_end(body, after + params.len(), b'{', b'}')
                && let Some((invoked, finish)) =
                    command_at(body, super::skip_trivia(body, group_end))
                && invoked == reader
                && super::skip_trivia(body, finish) == body.len()
            {
                return Some(Self::Capture {
                    optional: shape == "om",
                    changes_catcodes,
                });
            }
            grouped |= name == "begingroup";
            changes_catcodes |= name == "dospecials";
            tail = Some((name, at));
            at = end;
        }
        let (last, start) = tail?;
        if !shape.is_empty() {
            return None;
        }
        if last == "string" {
            let first = super::skip_trivia(body, 0);
            let (op, end) = command_at(body, first)?;
            let (target, end) = command_at(body, super::skip_trivia(body, end))?;
            return (op == "expandafter" && super::skip_trivia(body, end) == start).then(|| {
                Self::Forward {
                    target: target.to_owned(),
                    changes_catcodes: false,
                }
            });
        }
        grouped.then(|| Self::Forward {
            target: last.to_owned(),
            changes_catcodes,
        })
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
    kernel_item: bool,
    protected_group_end: impl Fn(usize) -> Option<usize>,
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
            let end = if open == b'[' && kernel_item {
                // \@item[#1] takes the first closing bracket outside a TeX group.
                xparse::until_end(source, start + 1, source.len(), "]")
            } else {
                balanced_end(source, start, open, close)
                    .or_else(|| (open == b'{').then(|| protected_group_end(start)).flatten())
            };
            let Some(end) = end else {
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

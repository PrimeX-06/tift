//! Literal xparse argument specifications and invocation boundaries.

use std::{borrow::Cow, ops::Range};

use super::{balanced_end, command_at};
use crate::trivia;

#[derive(Clone, Debug)]
enum Kind {
    Mandatory,
    Optional {
        open: Cow<'static, str>,
        close: Cow<'static, str>,
        required: bool,
    },
    Token(Cow<'static, str>),
    Embellishments(Vec<String>),
    Until(String),
    Verbatim,
    Body,
}

#[derive(Clone, Debug)]
struct Slot {
    kind: Kind,
    skip_space: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct Signature {
    slots: Cow<'static, [Slot]>,
}

const MANDATORY: Slot = Slot {
    kind: Kind::Mandatory,
    skip_space: true,
};
const OPTIONAL: Slot = Slot {
    kind: Kind::Optional {
        open: Cow::Borrowed("["),
        close: Cow::Borrowed("]"),
        required: false,
    },
    skip_space: true,
};
const STAR: Slot = Slot {
    kind: Kind::Token(Cow::Borrowed("*")),
    skip_space: true,
};
static MANDATORY_SLOTS: [Slot; 9] = [MANDATORY; 9];
static OPTIONAL_SLOTS: [Slot; 9] = [
    OPTIONAL, MANDATORY, MANDATORY, MANDATORY, MANDATORY, MANDATORY, MANDATORY, MANDATORY,
    MANDATORY,
];
static STAR_SLOTS: [Slot; 9] = [
    STAR, MANDATORY, MANDATORY, MANDATORY, MANDATORY, MANDATORY, MANDATORY, MANDATORY, MANDATORY,
];
static STAR_OPTIONAL_SLOTS: [Slot; 9] = [
    STAR, OPTIONAL, MANDATORY, MANDATORY, MANDATORY, MANDATORY, MANDATORY, MANDATORY, MANDATORY,
];

#[derive(Clone, Debug)]
pub(crate) struct ArgumentSlot {
    pub range: Range<usize>,
    pub body: Range<usize>,
    pub slot: usize,
}

pub(crate) fn extended(signature: &str) -> bool {
    !signature.bytes().all(|byte| b"moOpgld".contains(&byte))
}

fn token(source: &str, cursor: &mut usize) -> Option<String> {
    *cursor = trivia::skip(source, *cursor);
    let start = *cursor;
    if let Some((name, end)) = command_at(source, start) {
        *cursor = end - usize::from(name.len() > 1 && name.ends_with('*'));
    } else {
        *cursor += source[start..].chars().next()?.len_utf8();
    }
    Some(source[start..*cursor].to_owned())
}

fn group(source: &str, cursor: &mut usize) -> Option<Range<usize>> {
    let start = trivia::skip(source, *cursor);
    *cursor = balanced_end(source, start, b'{', b'}')?;
    Some(start + 1..*cursor - 1)
}

impl Signature {
    pub(crate) fn boolean_slots(&self) -> u16 {
        self.slots
            .iter()
            .enumerate()
            .filter(|(index, slot)| *index < 9 && matches!(slot.kind, Kind::Token(_)))
            .fold(0, |mask, (index, _)| mask | (1 << index))
    }
    /// The command lexer includes an adjacent star. Revisit it only when a
    /// leading argument slot can consume that star as an xparse switch.
    pub(crate) fn invocation_cursor(&self, source: &str, end: usize) -> usize {
        if source.as_bytes().get(end.wrapping_sub(1)) != Some(&b'*') {
            return end;
        }
        for slot in self.slots.iter() {
            match &slot.kind {
                Kind::Token(token) if token == "*" => return end - 1,
                Kind::Optional {
                    required: false, ..
                }
                | Kind::Token(_) => (),
                _ => break,
            }
        }
        end
    }

    pub(crate) fn parse(source: &str) -> Option<Self> {
        // Built-in and compact declaration grammars share immutable slots.
        // Extended grammars keep the full parser and its validation below.
        if source.len() <= 9 {
            let (slots, tail) = if let Some(tail) = source.strip_prefix("so") {
                (&STAR_OPTIONAL_SLOTS, tail)
            } else if let Some(tail) = source.strip_prefix('s') {
                (&STAR_SLOTS, tail)
            } else if let Some(tail) = source.strip_prefix('o') {
                (&OPTIONAL_SLOTS, tail)
            } else {
                (&MANDATORY_SLOTS, source)
            };
            if tail.bytes().all(|byte| byte == b'm') {
                return Some(Self {
                    slots: Cow::Borrowed(&slots[..source.len()]),
                });
            }
        }
        let mut cursor = 0;
        let mut slots = Vec::new();
        let mut arguments = 0;
        while trivia::skip(source, cursor) < source.len() {
            cursor = trivia::skip(source, cursor);
            let mut skip_space = true;
            let mut modifiers = false;
            loop {
                match source.as_bytes().get(cursor) {
                    Some(b'+' | b'!') => {
                        if source.as_bytes()[cursor] == b'!' {
                            skip_space = false;
                        }
                        cursor = trivia::skip(source, cursor + 1);
                        modifiers = true;
                    }
                    Some(b'>') => {
                        cursor += 1;
                        group(source, &mut cursor)?;
                        cursor = trivia::skip(source, cursor);
                        modifiers = true;
                    }
                    _ => break,
                }
            }
            let kind = *source.as_bytes().get(cursor)?;
            cursor += 1;
            let mut count = 1;
            let kind = match kind {
                b'm' => Kind::Mandatory,
                b'o' | b'O' => {
                    if kind == b'O' {
                        group(source, &mut cursor)?;
                    }
                    Kind::Optional {
                        open: Cow::Borrowed("["),
                        close: Cow::Borrowed("]"),
                        required: false,
                    }
                }
                b'g' | b'G' => {
                    if kind == b'G' {
                        group(source, &mut cursor)?;
                    }
                    Kind::Optional {
                        open: Cow::Borrowed("{"),
                        close: Cow::Borrowed("}"),
                        required: false,
                    }
                }
                b'd' | b'D' | b'r' | b'R' => {
                    let open = token(source, &mut cursor)?;
                    let close = token(source, &mut cursor)?;
                    if matches!(kind, b'D' | b'R') {
                        group(source, &mut cursor)?;
                    }
                    Kind::Optional {
                        open: open.into(),
                        close: close.into(),
                        required: matches!(kind, b'r' | b'R'),
                    }
                }
                b's' => Kind::Token(Cow::Borrowed("*")),
                b't' => {
                    let start = trivia::skip(source, cursor);
                    if source.as_bytes().get(start) == Some(&b'{') {
                        let range = group(source, &mut cursor)?;
                        let mut position = range.start;
                        let value = token(source, &mut position)?;
                        if trivia::skip(source, position) != range.end {
                            return None;
                        }
                        Kind::Token(value.into())
                    } else {
                        Kind::Token(token(source, &mut cursor)?.into())
                    }
                }
                b'e' | b'E' => {
                    let range = group(source, &mut cursor)?;
                    let mut position = range.start;
                    let mut tokens = Vec::new();
                    while trivia::skip(source, position) < range.end {
                        let value = token(source, &mut position)?;
                        if position > range.end || tokens.contains(&value) {
                            return None;
                        }
                        tokens.push(value);
                    }
                    if tokens.is_empty() {
                        return None;
                    }
                    count = tokens.len();
                    if kind == b'E' {
                        let defaults = group(source, &mut cursor)?;
                        let mut position = defaults.start;
                        let mut default_count = 0;
                        while trivia::skip(source, position) < defaults.end {
                            let begin = trivia::skip(source, position);
                            if source.as_bytes()[begin] == b'{' {
                                group(source, &mut position)?;
                            } else {
                                token(source, &mut position)?;
                            }
                            default_count += 1;
                        }
                        if default_count != tokens.len() {
                            return None;
                        }
                    }
                    Kind::Embellishments(tokens)
                }
                b'u' => Kind::Until(source[group(source, &mut cursor)?].to_owned()),
                b'v' => Kind::Verbatim,
                b'b' => Kind::Body,
                _ => return None,
            };
            if modifiers && matches!(kind, Kind::Body) && !skip_space {
                return None;
            }
            arguments += count;
            if arguments > 9 {
                return None;
            }
            slots.push(Slot { kind, skip_space });
        }
        Some(Self {
            slots: Cow::Owned(slots),
        })
    }

    /// Retain compact signatures for existing m/o/O declaration consumers.
    pub(crate) fn compact(&self) -> Option<String> {
        self.slots
            .iter()
            .map(|slot| match &slot.kind {
                Kind::Mandatory => Some('m'),
                Kind::Optional {
                    open,
                    close,
                    required: false,
                } if open == "[" && close == "]" && slot.skip_space => Some('o'),
                _ => None,
            })
            .collect()
    }

    pub(crate) fn ranges(&self, source: &str, body: Range<usize>) -> Option<Vec<Range<usize>>> {
        let mut ranges = Vec::with_capacity(self.slots.len());
        self.scan(source, body, |argument, _| ranges.push(argument.range))?;
        Some(ranges)
    }

    pub(crate) fn slot_ranges(
        &self,
        source: &str,
        body: Range<usize>,
    ) -> Option<Vec<ArgumentSlot>> {
        let mut ranges = Vec::with_capacity(self.slots.len());
        self.scan(source, body, |argument, _| ranges.push(argument))?;
        Some(ranges)
    }

    pub(crate) fn literal_ranges(
        &self,
        source: &str,
        body: Range<usize>,
    ) -> Option<Vec<Range<usize>>> {
        let mut ranges = Vec::new();
        self.scan(source, body, |argument, literal| {
            if literal {
                ranges.push(argument.range);
            }
        })?;
        Some(ranges)
    }

    pub(crate) fn end(&self, source: &str, body: Range<usize>) -> Option<usize> {
        self.scan(source, body, |_, _| {})
    }

    fn scan(
        &self,
        source: &str,
        body: Range<usize>,
        mut visit: impl FnMut(ArgumentSlot, bool),
    ) -> Option<usize> {
        let mut cursor = body.start;
        let mut argument_slot = 0;
        for slot in self.slots.iter() {
            let start = if slot.skip_space {
                trivia::skip_until(source, cursor, body.end)
            } else {
                cursor
            };
            match &slot.kind {
                Kind::Mandatory => {
                    let end = argument_end(source, start, body.end)?;
                    let body = argument_body(source, start..end);
                    visit(
                        ArgumentSlot {
                            range: start..end,
                            body,
                            slot: argument_slot,
                        },
                        source.as_bytes()[start] == b'\\',
                    );
                    cursor = end;
                }
                Kind::Optional {
                    open,
                    close,
                    required,
                } => {
                    if source.get(start..body.end)?.starts_with(open.as_ref()) {
                        let end = delimited_end(source, start, body.end, open, close)?;
                        visit(
                            ArgumentSlot {
                                range: start..end,
                                body: start + open.len()..end - close.len(),
                                slot: argument_slot,
                            },
                            open != "[" && open != "{",
                        );
                        cursor = end;
                    } else if *required {
                        return None;
                    }
                }
                Kind::Token(token) => {
                    if matches_token(source, start, body.end, token) {
                        cursor = start + token.len();
                        visit(
                            ArgumentSlot {
                                range: start..cursor,
                                body: start..cursor,
                                slot: argument_slot,
                            },
                            token.starts_with('\\'),
                        );
                    }
                }
                Kind::Embellishments(tokens) => {
                    let mut seen = [false; 9];
                    let mut next = start;
                    loop {
                        let token_start = if slot.skip_space {
                            trivia::skip_until(source, next, body.end)
                        } else {
                            next
                        };
                        let Some(index) = tokens.iter().enumerate().position(|(i, token)| {
                            !seen[i] && matches_token(source, token_start, body.end, token)
                        }) else {
                            break;
                        };
                        let token_end = token_start + tokens[index].len();
                        let argument = trivia::skip_until(source, token_end, body.end);
                        let end = argument_end(source, argument, body.end)?;
                        visit(
                            ArgumentSlot {
                                range: token_start..end,
                                body: argument_body(source, argument..end),
                                slot: argument_slot + index,
                            },
                            true,
                        );
                        seen[index] = true;
                        next = end;
                        cursor = end;
                    }
                }
                Kind::Until(delimiter) => {
                    let end = until_end(source, start, body.end, delimiter)?;
                    visit(
                        ArgumentSlot {
                            range: start..end,
                            body: start..end - delimiter.len(),
                            slot: argument_slot,
                        },
                        true,
                    );
                    cursor = end;
                }
                Kind::Verbatim => {
                    let end = verbatim_end(source, start, body.end)?;
                    let width = source[start..].chars().next()?.len_utf8();
                    visit(
                        ArgumentSlot {
                            range: start..end,
                            body: start + width..end - width,
                            slot: argument_slot,
                        },
                        true,
                    );
                    cursor = end;
                }
                Kind::Body => (),
            }
            argument_slot += match &slot.kind {
                Kind::Embellishments(tokens) => tokens.len(),
                _ => 1,
            };
        }
        Some(cursor)
    }
}

fn argument_body(source: &str, range: Range<usize>) -> Range<usize> {
    if source.as_bytes().get(range.start) == Some(&b'{') {
        range.start + 1..range.end - 1
    } else {
        range
    }
}

fn matches_token(source: &str, start: usize, limit: usize, value: &str) -> bool {
    source
        .get(start..limit)
        .is_some_and(|text| text.starts_with(value))
        && (!value.starts_with('\\')
            || command_at(source, start).is_some_and(|(name, end)| {
                end - usize::from(name.len() > 1 && name.ends_with('*')) == start + value.len()
            }))
}

fn argument_end(source: &str, start: usize, limit: usize) -> Option<usize> {
    if start >= limit || source.as_bytes()[start] == b'}' {
        return None;
    }
    let end = if source.as_bytes()[start] == b'{' {
        balanced_end(source, start, b'{', b'}')?
    } else if let Some((name, end)) = command_at(source, start) {
        end - usize::from(name.len() > 1 && name.ends_with('*'))
    } else {
        start + source[start..].chars().next()?.len_utf8()
    };
    (end <= limit).then_some(end)
}

fn delimited_end(
    source: &str,
    start: usize,
    limit: usize,
    open: &str,
    close: &str,
) -> Option<usize> {
    let mut cursor = start + open.len();
    let mut depth = 1;
    while cursor < limit {
        if source.as_bytes()[cursor] == b'%' {
            cursor = trivia::skip_until(source, cursor, limit);
            continue;
        }
        if matches_token(source, cursor, limit, close) {
            depth -= 1;
            cursor += close.len();
            if depth == 0 {
                return Some(cursor);
            }
        } else if open != close && matches_token(source, cursor, limit, open) {
            depth += 1;
            cursor += open.len();
        } else if source.as_bytes()[cursor] == b'}' {
            // An unmatched outer group cannot belong to this argument. In
            // particular, expansion-dependent delimiters must not search past
            // their enclosing declaration into unrelated source.
            return None;
        } else {
            cursor = argument_end(source, cursor, limit)?;
        }
    }
    None
}

fn until_end(source: &str, start: usize, limit: usize, delimiter: &str) -> Option<usize> {
    if delimiter.is_empty() {
        return Some(start);
    }
    let mut cursor = start;
    while cursor < limit {
        if source[cursor..limit].starts_with(delimiter) {
            return Some(cursor + delimiter.len());
        }
        if source.as_bytes()[cursor] == b'}' {
            return None;
        }
        if source.as_bytes()[cursor] == b'%' {
            cursor = trivia::skip_until(source, cursor, limit);
        } else {
            cursor = argument_end(source, cursor, limit)?;
        }
    }
    None
}

fn verbatim_end(source: &str, start: usize, limit: usize) -> Option<usize> {
    let delimiter = source[start..limit].chars().next()?;
    let close = if delimiter == '{' { '}' } else { delimiter };
    if delimiter.is_ascii_whitespace() || delimiter == '\\' {
        return None;
    }
    let begin = start + delimiter.len_utf8();
    let end = source[begin..limit].find(close)? + begin;
    (!source[begin..end].contains(['\r', '\n'])).then_some(end + close.len_utf8())
}

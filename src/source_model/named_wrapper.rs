use super::{Binding, Config, EnvironmentArguments, HashMap, syntax};
use crate::packages::{self, context::InvocationContext};

const ENVIRONMENTS: &[&str] = &[
    "itemize",
    "enumerate",
    "description",
    "quote",
    "quotation",
    "center",
    "flushleft",
    "flushright",
];
const BODY_LIMIT: usize = 4096;
const CALL_LIMIT: usize = 512;
const DEPTH_LIMIT: usize = 16;

#[derive(Clone, Debug)]
pub(crate) struct NamedWrapper {
    arity: usize,
    literals: u16,
    uses_item: bool,
}

fn environment_bit(name: &str) -> Option<u16> {
    ENVIRONMENTS
        .iter()
        .position(|&known| known == name)
        .map(|index| 1 << index)
}

pub(super) fn dependency(name: &str) -> bool {
    matches!(name, "begin" | "end" | "item" | "par")
        || ENVIRONMENTS.contains(&name.strip_prefix("end").unwrap_or(name))
}

fn plain(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b" \t\r\n.,;:!?'-()".contains(&byte)
}

fn trivia(source: &str, mut cursor: usize) -> Option<usize> {
    let mut new_line = false;
    while let Some(byte) = source.as_bytes().get(cursor) {
        match byte {
            b' ' | b'\t' => cursor += 1,
            b'%' => {
                cursor += source[cursor..]
                    .find(['\r', '\n'])
                    .unwrap_or(source.len() - cursor);
                new_line = false;
            }
            b'\r' | b'\n' if !new_line => {
                cursor += if source[cursor..].starts_with("\r\n") {
                    2
                } else {
                    1
                };
                new_line = true;
            }
            b'\r' | b'\n' => return None,
            _ => break,
        }
    }
    Some(cursor)
}

fn name_group(source: &str, cursor: usize) -> Option<(&str, usize)> {
    let open = trivia(source, cursor)?;
    let (name, _) = source[open..].strip_prefix('{')?.split_once('}')?;
    Some((name, open + 2 + name.len()))
}

impl NamedWrapper {
    pub(super) fn classify(
        command: &str,
        body: &str,
        signature: Option<&EnvironmentArguments>,
    ) -> Option<Self> {
        if body.len() > BODY_LIMIT
            || !body.contains('#')
            || body.contains("^^")
            || !matches!(
                command,
                "def" | "gdef" | "newcommand" | "renewcommand" | "providecommand"
            )
        {
            return None;
        }
        let shape = signature?.signature.as_deref()?;
        if !(1..=9).contains(&shape.len()) || !shape.bytes().all(|slot| slot == b'm') {
            return None;
        }
        let mut proof = Self {
            arity: shape.len(),
            literals: 0,
            uses_item: false,
        };
        let mut slots = 0u16;
        let mut stack = Vec::new();
        let mut cursor = trivia(body, 0)?;
        while cursor < body.len() {
            if let Some((control, end)) = syntax::command_at(body, cursor) {
                match control {
                    "begin" | "end" => {
                        let (name, finish) = name_group(body, end)?;
                        if let [b'#', digit @ b'1'..=b'9'] = name.as_bytes() {
                            slots |= 1 << (digit - b'1');
                        } else {
                            proof.literals |= environment_bit(name)?;
                        }
                        if control == "begin" {
                            if stack.len() == DEPTH_LIMIT {
                                return None;
                            }
                            stack.push(name);
                        } else if stack.pop() != Some(name) {
                            return None;
                        }
                        cursor = finish;
                    }
                    "item" if !stack.is_empty() => {
                        proof.uses_item = true;
                        cursor = trivia(body, end)?;
                        if let Some(label) = body[cursor..].strip_prefix('[') {
                            let (label, _) = label.split_once(']')?;
                            if !label.bytes().all(plain) {
                                return None;
                            }
                            cursor += label.len() + 2;
                        }
                    }
                    _ => return None,
                }
            } else {
                if !plain(body.as_bytes()[cursor]) {
                    return None;
                }
                cursor += 1;
            }
            cursor = trivia(body, cursor)?;
        }
        (stack.is_empty() && slots == (1 << proof.arity) - 1).then_some(proof)
    }

    pub(super) fn accepts(
        &self,
        source: &str,
        end: usize,
        config: &Config,
        bindings: &HashMap<String, Binding>,
        context: &InvocationContext<'_>,
    ) -> bool {
        let tail = &source[end..];
        let tail = &tail[..tail.floor_char_boundary(CALL_LIMIT.min(tail.len()))];
        let mut cursor = 0;
        let mut used = self.literals;
        for _ in 0..self.arity {
            let Some((name, finish)) = name_group(tail, cursor) else {
                return false;
            };
            let Some(bit) = environment_bit(name) else {
                return false;
            };
            used |= bit;
            cursor = finish;
        }
        let unshadowed = |name: &str| {
            !bindings.contains_key(name)
                && !bindings.contains_key(&format!("environment:{name}"))
                && !config.syntax.commands.contains_key(name)
                && !config.protect.commands.iter().any(|entry| entry == name)
        };
        !tail[..cursor].contains("^^")
            && ["begin", "end", "par"].into_iter().all(unshadowed)
            && (!self.uses_item || unshadowed("item"))
            && ENVIRONMENTS.iter().enumerate().all(|(index, &name)| {
                used & (1 << index) == 0
                    || unshadowed(name)
                        && unshadowed(&format!("end{name}"))
                        && !config.syntax.environment_arguments.contains_key(name)
                        && packages::context_ambiguous(name, context).is_none()
                        && packages::context_header_signature(name, context).is_none()
                        && !config
                            .protect
                            .environments
                            .iter()
                            .chain(&config.protect.file_contents)
                            .any(|entry| entry == name)
            })
    }
}

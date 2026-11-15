//! Bounded expansion of literal, zero-argument TeX replacements.
//!
//! The index records source-order bindings. It never executes replacement text;
//! parameters, active tokens and executable primitives remain unresolved.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::Arc;

use crate::{Config, syntax};

const MAX_DEPTH: usize = 32;
const MAX_OUTPUT: usize = 4096;
const MAX_STEPS: usize = 16_384;

#[derive(Clone, Debug)]
enum Token {
    Text(String),
    Macro(String),
    CsName(Vec<Token>),
}

#[derive(Clone, Debug)]
enum Replacement {
    Tokens(Arc<[Token]>),
    Parameterized {
        arguments: usize,
        optional: bool,
        effectful: bool,
    },
    Unknown {
        effectful: bool,
    },
}

#[derive(Clone, Debug)]
struct Binding {
    replacement: Replacement,
    epoch: usize,
}

#[derive(Clone, Debug)]
struct Event {
    at: usize,
    binding: Option<Binding>,
}

/// An owned binding history reusable for queries at different source offsets.
#[derive(Default, Debug)]
pub struct LiteralExpansionIndex {
    events: HashMap<String, Vec<Event>>,
    current: HashMap<String, Binding>,
    scopes: Vec<HashMap<String, Option<Binding>>>,
    barriers: Vec<usize>,
    constructed: HashMap<usize, (String, usize)>,
    custom_conditions: HashSet<String>,
    conditions: usize,
    loops: Vec<usize>,
    global_assignment: bool,
    source_len: usize,
    disabled_at: Option<usize>,
    continuation_safe: bool,
}

impl LiteralExpansionIndex {
    /// Build once for an immutable source and its validated protection ranges.
    pub fn build(source: &str, config: &Config, protected: &[Range<usize>]) -> Self {
        let mut index = Self {
            scopes: vec![HashMap::new()],
            continuation_safe: true,
            ..Self::default()
        };
        index.append_segment(source, config, protected, 0);
        index
    }

    /// Append a complete source segment, with protection offsets relative to it.
    ///
    /// Returns false when a token or declaration crosses the segment boundary.
    /// Such an index must be rebuilt from the complete next source snapshot.
    pub fn append_segment(
        &mut self,
        source: &str,
        config: &Config,
        protected: &[Range<usize>],
        base_offset: usize,
    ) -> bool {
        if base_offset != self.source_len || !self.continuation_safe {
            return false;
        }
        self.source_len += source.len();
        if self.disabled_at.is_some() {
            self.continuation_safe = false;
            return false;
        }
        let mut pos = 0;
        let mut protected_index = 0;
        while pos < source.len() {
            while protected
                .get(protected_index)
                .is_some_and(|range| range.end <= pos)
            {
                protected_index += 1;
            }
            if let Some(range) = protected
                .get(protected_index)
                .filter(|range| range.start <= pos)
            {
                if range.start == pos
                    && syntax::command_at(source, pos).is_some_and(|(name, _)| {
                        matches!(
                            name,
                            "catcode" | "catcodetable" | "ExplSyntaxOn" | "ExplSyntaxOff"
                        )
                    })
                {
                    self.disabled_at = Some(base_offset + pos);
                    self.continuation_safe = false;
                    return false;
                }
                pos = range.end.min(source.len());
                continue;
            }
            if source.as_bytes()[pos] == b'%' {
                pos = comment_end(source, pos);
                continue;
            }
            if let Some((command, end)) = syntax::command_at(source, pos) {
                let name = command.trim_end_matches('*');
                let at = base_offset + pos;
                let overridden = self.current.contains_key(name);
                if matches!(
                    name,
                    "catcode" | "catcodetable" | "ExplSyntaxOn" | "ExplSyntaxOff"
                ) {
                    self.disabled_at = Some(at);
                    self.continuation_safe = false;
                    return false;
                }
                if name == "global" && !overridden {
                    self.global_assignment = true;
                    pos = end;
                    continue;
                }
                if matches!(name, "long" | "outer" | "protected") && !overridden {
                    pos = end;
                    continue;
                }
                if definition_command(name) && !overridden {
                    let Some(definition) = definition(source, name, end) else {
                        self.disabled_at = Some(at);
                        self.continuation_safe = false;
                        return false;
                    };
                    let global = self.global_assignment || matches!(name, "gdef" | "xdef");
                    let existing = self.current.contains_key(&definition.name)
                        || builtin_name(&definition.name, config);
                    let install = match name {
                        "providecommand"
                        | "ProvideDocumentCommand"
                        | "newcommand"
                        | "NewDocumentCommand" => !existing,
                        _ => true,
                    };
                    if install {
                        let text = &source[definition.body.clone()];
                        let mut replacement = if definition.zero_arguments {
                            tokenize(text).map_or_else(
                                || Replacement::Unknown {
                                    effectful: text.contains('\\'),
                                },
                                |tokens| Replacement::Tokens(tokens.into()),
                            )
                        } else {
                            definition.arguments.map_or(
                                Replacement::Unknown { effectful: true },
                                |arguments| Replacement::Parameterized {
                                    arguments,
                                    optional: definition.optional,
                                    effectful: text.contains(['\\', '#']),
                                },
                            )
                        };
                        if self.conditions > 0 {
                            replacement = Replacement::Unknown { effectful: true };
                        } else if matches!(name, "edef" | "xdef") {
                            replacement = match self.expand(text, at) {
                                Some(value) => Replacement::Tokens(vec![Token::Text(value)].into()),
                                None => Replacement::Unknown {
                                    effectful: text.contains('\\'),
                                },
                            };
                        }
                        self.assign(
                            &definition.name,
                            replacement,
                            global,
                            base_offset + definition.end,
                        );
                    }
                    self.global_assignment = false;
                    pos = definition.end;
                    continue;
                }
                if (name == "let" || name == "futurelet") && !overridden {
                    let target_start = trivia_end(source, end);
                    if let Some((target, target_end)) = control_token(source, target_start) {
                        let mut rhs = trivia_end(source, target_end);
                        if source.as_bytes().get(rhs) == Some(&b'=') {
                            rhs = trivia_end(source, rhs + 1);
                        }
                        let (replacement, finish) = if name == "futurelet" {
                            (Replacement::Unknown { effectful: true }, target_end)
                        } else if let Some((alias, finish)) = control_token(source, rhs) {
                            let replacement = self
                                .binding(&alias, at)
                                .map_or(Replacement::Unknown { effectful: false }, |binding| {
                                    binding.replacement.clone()
                                });
                            (replacement, finish)
                        } else if let Some(character) = source[rhs..].chars().next() {
                            (
                                if literal_character(character) {
                                    Replacement::Tokens(
                                        vec![Token::Text(character.to_string())].into(),
                                    )
                                } else {
                                    Replacement::Unknown { effectful: false }
                                },
                                rhs + character.len_utf8(),
                            )
                        } else {
                            self.disabled_at = Some(at);
                            self.continuation_safe = false;
                            return false;
                        };
                        self.assign(
                            &target,
                            if self.conditions == 0 {
                                replacement
                            } else {
                                Replacement::Unknown { effectful: true }
                            },
                            self.global_assignment,
                            base_offset + finish,
                        );
                        self.global_assignment = false;
                        pos = finish;
                        continue;
                    }
                }
                self.global_assignment = false;
                if let Some(finish) = (!overridden)
                    .then(|| syntax::primitive_data_end(source, command, end))
                    .flatten()
                {
                    pos = finish;
                    continue;
                }
                if name == "newif" && !overridden {
                    let target = trivia_end(source, end);
                    if let Some((condition, finish)) = control_token(source, target) {
                        self.custom_conditions.insert(condition.clone());
                        self.assign(
                            &condition,
                            Replacement::Unknown { effectful: false },
                            false,
                            base_offset + finish,
                        );
                        pos = finish;
                        continue;
                    }
                }
                if (name == "csname" || name == "ifcsname")
                    && !overridden
                    && let Some((tokens, finish)) = csname_tokens(source, end, 0)
                {
                    let Some(value) = self.expand_tokens(&tokens, at) else {
                        self.barriers.push(at);
                        if name == "ifcsname" {
                            self.conditions += 1;
                        }
                        pos = finish;
                        continue;
                    };
                    let value: String = value.chars().filter(|c| !c.is_whitespace()).collect();
                    if name == "csname" {
                        self.constructed
                            .insert(at, (value.clone(), base_offset + finish));
                        self.condition_command(&value, config);
                        if definition_command(&value)
                            || matches!(
                                value.as_str(),
                                "let"
                                    | "futurelet"
                                    | "input"
                                    | "include"
                                    | "usepackage"
                                    | "RequirePackage"
                                    | "documentclass"
                                    | "LoadClass"
                            )
                        {
                            self.barriers.push(at);
                        }
                        if !self.current.contains_key(&value) && !builtin_name(&value, config) {
                            self.assign(
                                &value,
                                Replacement::Unknown { effectful: false },
                                false,
                                base_offset + finish,
                            );
                        }
                    } else {
                        self.conditions += 1;
                    }
                    pos = finish;
                    continue;
                }
                if let Some((_, begin, finish)) =
                    (!overridden).then(|| environment(source, pos)).flatten()
                {
                    if begin {
                        self.scopes.push(HashMap::new());
                    } else {
                        self.pop_scope(base_offset + finish);
                    }
                    pos = finish;
                    continue;
                }
                if matches!(name, "begingroup" | "bgroup") && !overridden {
                    self.scopes.push(HashMap::new());
                } else if matches!(name, "endgroup" | "egroup") && !overridden {
                    self.pop_scope(base_offset + end);
                }
                self.condition_command(name, config);
                if matches!(
                    name,
                    "expandafter"
                        | "csdef"
                        | "csgdef"
                        | "csedef"
                        | "csxdef"
                        | "cslet"
                        | "undef"
                        | "csundef"
                ) {
                    self.barriers.push(at);
                } else if let Some(binding) = self.binding(name, at) {
                    if let Replacement::Parameterized {
                        arguments,
                        optional,
                        effectful,
                    } = binding.replacement
                    {
                        let mut finish = end;
                        let mut count = arguments;
                        if optional {
                            finish = trivia_end(source, finish);
                            if source.as_bytes().get(finish) == Some(&b'[') {
                                finish = syntax::balanced_end(source, finish, b'[', b']')
                                    .unwrap_or(source.len());
                            }
                            count = count.saturating_sub(1);
                        }
                        for _ in 0..count {
                            finish = argument_end(source, finish).unwrap_or(source.len());
                        }
                        if effectful {
                            self.barriers.push(at);
                        }
                        pos = finish;
                        continue;
                    }
                    let effectful = matches!(
                        binding.replacement,
                        Replacement::Unknown { effectful: true }
                    ) || matches!(binding.replacement, Replacement::Tokens(_))
                        && self.expand(&format!("\\{name}"), at).is_none();
                    if effectful {
                        self.barriers.push(at);
                    }
                }
                pos = condition_operand_end(source, name, end).unwrap_or(end);
                continue;
            }
            match source.as_bytes()[pos] {
                b'{' => self.scopes.push(HashMap::new()),
                b'}' => self.pop_scope(base_offset + pos + 1),
                _ => (),
            }
            pos += source[pos..]
                .chars()
                .next()
                .expect("source character")
                .len_utf8();
        }
        self.continuation_safe = source.is_empty() || source.ends_with('\n');
        self.continuation_safe
    }

    /// Resolve an expression using bindings visible immediately before `at`.
    pub fn expand(&self, expression: &str, at: usize) -> Option<String> {
        if at > self.source_len || self.disabled_at.is_some_and(|start| start <= at) {
            return None;
        }
        self.expand_tokens(&tokenize(expression)?, at)
    }

    /// Resolved csname controls and their original source end offsets.
    pub fn constructed_commands(&self) -> &HashMap<usize, (String, usize)> {
        &self.constructed
    }

    /// Consume the index when only its resolved control names are needed.
    pub fn into_constructed_commands(self) -> HashMap<usize, (String, usize)> {
        self.constructed
    }

    fn expand_tokens(&self, tokens: &[Token], at: usize) -> Option<String> {
        let mut output = String::new();
        let mut steps = MAX_STEPS;
        self.write_tokens(tokens, at, &mut output, &mut Vec::new(), &mut steps)?;
        Some(output)
    }

    fn write_tokens(
        &self,
        tokens: &[Token],
        at: usize,
        output: &mut String,
        stack: &mut Vec<String>,
        steps: &mut usize,
    ) -> Option<()> {
        if stack.len() >= MAX_DEPTH {
            return None;
        }
        for token in tokens {
            *steps = steps.checked_sub(1)?;
            match token {
                Token::Text(text) => {
                    if output.len().checked_add(text.len())? > MAX_OUTPUT {
                        return None;
                    }
                    output.push_str(text);
                }
                Token::Macro(name) => self.write_macro(name, at, output, stack, steps)?,
                Token::CsName(parts) => {
                    let mut name = String::new();
                    self.write_tokens(parts, at, &mut name, stack, steps)?;
                    let name: String = name.chars().filter(|c| !c.is_whitespace()).collect();
                    self.write_macro(&name, at, output, stack, steps)?;
                }
            }
        }
        Some(())
    }

    fn write_macro(
        &self,
        name: &str,
        at: usize,
        output: &mut String,
        stack: &mut Vec<String>,
        steps: &mut usize,
    ) -> Option<()> {
        if stack.iter().any(|pending| pending == name) {
            return None;
        }
        let Replacement::Tokens(tokens) = &self.binding(name, at)?.replacement else {
            return None;
        };
        stack.push(name.into());
        let result = self.write_tokens(tokens, at, output, stack, steps);
        stack.pop();
        result
    }

    fn binding(&self, name: &str, at: usize) -> Option<&Binding> {
        let history = self.events.get(name)?;
        let end = history.partition_point(|event| event.at <= at);
        let binding = history.get(end.checked_sub(1)?)?.binding.as_ref()?;
        let epoch = self.barriers.partition_point(|barrier| *barrier <= at);
        (binding.epoch == epoch).then_some(binding)
    }

    fn assign(&mut self, name: &str, replacement: Replacement, global: bool, at: usize) {
        if global {
            for scope in &mut self.scopes {
                scope.remove(name);
            }
        } else {
            let previous = self.current.get(name).cloned();
            self.scopes
                .last_mut()
                .expect("root scope")
                .entry(name.into())
                .or_insert(previous);
        }
        let binding = Binding {
            replacement,
            epoch: self.barriers.len(),
        };
        self.current.insert(name.into(), binding.clone());
        self.record(name, Some(binding), at);
    }

    fn record(&mut self, name: &str, binding: Option<Binding>, at: usize) {
        self.events
            .entry(name.into())
            .or_default()
            .push(Event { at, binding });
    }

    fn pop_scope(&mut self, at: usize) {
        if self.scopes.len() <= 1 {
            return;
        }
        for (name, previous) in self.scopes.pop().expect("local scope") {
            if let Some(binding) = &previous {
                self.current.insert(name.clone(), binding.clone());
            } else {
                self.current.remove(&name);
            }
            self.record(&name, previous, at);
        }
    }

    fn condition_command(&mut self, name: &str, config: &Config) {
        if self.current.contains_key(name) && !self.custom_conditions.contains(name) {
            return;
        }
        if name == "loop" {
            self.loops.push(self.conditions);
        } else if name == "repeat" {
            if let Some(before) = self.loops.pop()
                && self.conditions == before + 1
            {
                self.conditions = before;
            }
        } else if name == "fi" {
            self.conditions = self.conditions.saturating_sub(1);
        } else if syntax::is_condition_command(name, config)
            || self.custom_conditions.contains(name)
        {
            self.conditions += 1;
        }
    }
}

struct Definition {
    name: String,
    body: Range<usize>,
    end: usize,
    zero_arguments: bool,
    arguments: Option<usize>,
    optional: bool,
}

fn definition_command(name: &str) -> bool {
    matches!(
        name,
        "def"
            | "gdef"
            | "edef"
            | "xdef"
            | "newcommand"
            | "renewcommand"
            | "providecommand"
            | "NewDocumentCommand"
            | "RenewDocumentCommand"
            | "ProvideDocumentCommand"
            | "DeclareDocumentCommand"
            | "DeclareRobustCommand"
            | "DeclareMathOperator"
    )
}

fn definition(source: &str, command: &str, end: usize) -> Option<Definition> {
    let primitive = matches!(command, "def" | "gdef" | "edef" | "xdef");
    let mut cursor = trivia_end(source, end);
    let (name, target_end) = if !primitive && source.as_bytes().get(cursor) == Some(&b'{') {
        let finish = syntax::balanced_end(source, cursor, b'{', b'}')?;
        let (name, target_end) = control_token(source, trivia_end(source, cursor + 1))?;
        (trivia_end(source, target_end) == finish - 1).then_some((name, finish))?
    } else {
        control_token(source, cursor)?
    };
    cursor = trivia_end(source, target_end);
    let mut zero_arguments = true;
    let mut arguments = Some(0);
    let mut optional = false;
    if primitive {
        let header_end = source[cursor..].find('{')? + cursor;
        zero_arguments = source[cursor..header_end].trim().is_empty();
        let header = &source[cursor..header_end];
        arguments = if zero_arguments {
            Some(0)
        } else {
            let count = header.len() / 2;
            (count <= 9
                && header.len() == count * 2
                && header.bytes().enumerate().all(|(at, byte)| {
                    if at % 2 == 0 {
                        byte == b'#'
                    } else {
                        byte == b'1' + (at / 2) as u8
                    }
                }))
            .then_some(count)
        };
        cursor = header_end;
    } else if command.ends_with("DocumentCommand") {
        let finish = syntax::balanced_end(source, cursor, b'{', b'}')?;
        zero_arguments = source[cursor + 1..finish - 1].trim().is_empty();
        let signature = source[cursor + 1..finish - 1].trim();
        arguments = signature
            .bytes()
            .all(|byte| byte == b'm')
            .then_some(signature.len());
        cursor = trivia_end(source, finish);
    } else {
        if source.as_bytes().get(cursor) == Some(&b'[') {
            let finish = syntax::balanced_end(source, cursor, b'[', b']')?;
            zero_arguments = source[cursor + 1..finish - 1].trim() == "0";
            arguments = source[cursor + 1..finish - 1]
                .trim()
                .parse::<usize>()
                .ok()
                .filter(|count| *count <= 9);
            cursor = trivia_end(source, finish);
            if source.as_bytes().get(cursor) == Some(&b'[') {
                zero_arguments = false;
                optional = true;
                cursor = trivia_end(source, syntax::balanced_end(source, cursor, b'[', b']')?);
            }
        }
        zero_arguments &= !matches!(command, "DeclareRobustCommand" | "DeclareMathOperator");
        if matches!(command, "DeclareRobustCommand" | "DeclareMathOperator") {
            arguments = None;
        }
    }
    let finish = syntax::balanced_end(source, cursor, b'{', b'}')?;
    Some(Definition {
        name,
        body: cursor + 1..finish - 1,
        end: finish,
        zero_arguments,
        arguments,
        optional,
    })
}

fn tokenize(source: &str) -> Option<Vec<Token>> {
    if source.len() > MAX_STEPS {
        return None;
    }
    let (tokens, end) = tokens_until(source, 0, false, 0)?;
    (end == source.len()).then_some(tokens)
}

fn csname_tokens(source: &str, start: usize, depth: usize) -> Option<(Vec<Token>, usize)> {
    tokens_until(source, start, true, depth + 1)
}

fn tokens_until(
    source: &str,
    mut pos: usize,
    csname: bool,
    depth: usize,
) -> Option<(Vec<Token>, usize)> {
    if depth >= MAX_DEPTH {
        return None;
    }
    let mut tokens = Vec::new();
    let mut text = String::new();
    let mut steps = 0;
    while pos < source.len() {
        steps += 1;
        if steps > MAX_STEPS {
            return None;
        }
        if source.as_bytes()[pos] == b'%' {
            pos = comment_end(source, pos);
            continue;
        }
        if let Some((name, end)) = control_token(source, pos) {
            if csname && name == "endcsname" {
                if !text.is_empty() {
                    tokens.push(Token::Text(text));
                }
                return Some((tokens, end));
            }
            if !text.is_empty() {
                tokens.push(Token::Text(std::mem::take(&mut text)));
            }
            if name == "csname" {
                let (parts, finish) = csname_tokens(source, end, depth)?;
                tokens.push(Token::CsName(parts));
                pos = finish;
            } else {
                tokens.push(Token::Macro(name.clone()));
                pos = end;
                if name
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic() || "@_:".contains(c))
                {
                    pos = trivia_end(source, pos);
                }
            }
            continue;
        }
        let character = source[pos..].chars().next()?;
        if !literal_character(character) {
            return None;
        }
        if character.is_whitespace() {
            let start = pos;
            while source
                .as_bytes()
                .get(pos)
                .is_some_and(u8::is_ascii_whitespace)
            {
                pos += 1;
            }
            if pos == start {
                pos += character.len_utf8();
            }
            if source[start..pos]
                .bytes()
                .filter(|byte| *byte == b'\n')
                .count()
                > 1
            {
                return None;
            }
            if !text.ends_with(' ') {
                text.push(' ');
            }
        } else {
            text.push(character);
            pos += character.len_utf8();
        }
        if text.len() > MAX_OUTPUT {
            return None;
        }
    }
    if csname {
        return None;
    }
    if !text.is_empty() {
        tokens.push(Token::Text(text));
    }
    Some((tokens, pos))
}

fn literal_character(character: char) -> bool {
    !"\\{}#$&^~".contains(character) && (!character.is_control() || character.is_ascii_whitespace())
}

fn control_token(source: &str, pos: usize) -> Option<(String, usize)> {
    if source.as_bytes().get(pos) != Some(&b'\\') {
        return None;
    }
    let first = source[pos + 1..].chars().next()?;
    let mut end = pos + 1 + first.len_utf8();
    if first.is_ascii_alphabetic() || "@_:".contains(first) {
        while let Some(character) = source[end..].chars().next() {
            if character.is_ascii_alphabetic() || "@_:".contains(character) {
                end += character.len_utf8();
            } else {
                break;
            }
        }
    }
    Some((source[pos + 1..end].into(), end))
}

fn comment_end(source: &str, pos: usize) -> usize {
    source[pos..]
        .find('\n')
        .map_or(source.len(), |end| pos + end + 1)
}

fn trivia_end(source: &str, mut pos: usize) -> usize {
    loop {
        while source
            .as_bytes()
            .get(pos)
            .is_some_and(u8::is_ascii_whitespace)
        {
            pos += 1;
        }
        if source.as_bytes().get(pos) == Some(&b'%') {
            pos = comment_end(source, pos);
        } else {
            return pos;
        }
    }
}

fn environment(source: &str, pos: usize) -> Option<(String, bool, usize)> {
    let (command, end) = syntax::command_at(source, pos)?;
    if !matches!(command, "begin" | "end") {
        return None;
    }
    let start = trivia_end(source, end);
    let finish = syntax::balanced_end(source, start, b'{', b'}')?;
    Some((
        source[start + 1..finish - 1].trim().into(),
        command == "begin",
        finish,
    ))
}

fn condition_operand_end(source: &str, name: &str, end: usize) -> Option<usize> {
    let count = match name {
        "ifx" => 2,
        "ifdefined" | "ifprimitive" => 1,
        _ => return None,
    };
    let mut pos = end;
    for _ in 0..count {
        pos = trivia_end(source, pos);
        pos = control_token(source, pos).map_or_else(
            || source[pos..].chars().next().map(|c| pos + c.len_utf8()),
            |(_, end)| Some(end),
        )?;
    }
    Some(pos)
}

fn argument_end(source: &str, pos: usize) -> Option<usize> {
    let pos = trivia_end(source, pos);
    if source.as_bytes().get(pos) == Some(&b'{') {
        syntax::balanced_end(source, pos, b'{', b'}')
    } else {
        control_token(source, pos).map_or_else(
            || source[pos..].chars().next().map(|c| pos + c.len_utf8()),
            |(_, end)| Some(end),
        )
    }
}

fn builtin_name(name: &str, config: &Config) -> bool {
    crate::package_commands::builtin_command_rule(name).is_some()
        || syntax::is_condition_command(name, config)
        || definition_command(name)
        || matches!(
            name,
            "input"
                | "include"
                | "begin"
                | "end"
                | "let"
                | "futurelet"
                | "csname"
                | "endcsname"
                | "relax"
                | "global"
                | "long"
                | "outer"
                | "protected"
                | "string"
                | "noexpand"
                | "par"
        )
}

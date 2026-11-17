//! Scoped conditional meanings, numeric operands and inactive-branch boundaries.

use std::collections::HashMap;
use std::ops::Range;

use super::{
    CompiledSyntax, TokenKind, command_at, env_at, is_condition_command, latex_macro_definition,
    lex_with_ranges, line_end, skip_trivia, tex_token_end,
};
use crate::Config;

/// Engine and output switches supplied by iftex and its compatibility packages.
/// Their truth values depend on the TeX run and remain unknown to source layout.
pub(super) fn package_condition(name: &str) -> bool {
    matches!(
        name,
        "ifpdf"
            | "ifxetex"
            | "ifluatex"
            | "ifpdftex"
            | "ifptex"
            | "ifuptex"
            | "ifptexng"
            | "ifvtex"
            | "ifvtexdvi"
            | "ifvtexpdf"
            | "ifvtexps"
            | "ifaleph"
            | "iftex"
            | "ifXeTeX"
            | "ifLuaTeX"
            | "ifPDFTeX"
            | "ifpTeX"
            | "ifupTeX"
            | "ifpTeXng"
            | "ifVTeX"
            | "ifAleph"
            | "ifluahbtex"
            | "ifLuaHBTeX"
            | "ifhint"
            | "ifHINT"
            | "ifetex"
            | "ifeTeX"
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ConditionalMeaning {
    True,
    False,
    Case,
    Other,
    Else,
    Or,
    Fi,
    Unless,
    Ordinary,
}

#[derive(Default)]
pub(super) struct ConditionalBindings {
    scopes: Vec<ConditionalBindingScope>,
    depth: usize,
    global: bool,
}

struct ConditionalBindingScope {
    depth: usize,
    values: HashMap<String, ConditionalMeaning>,
}

impl ConditionalBindings {
    pub(super) fn has_global_prefix(&self) -> bool {
        self.global
    }

    pub(super) fn meaning(&self, name: &str, config: &Config) -> ConditionalMeaning {
        if let Some(meaning) = self
            .scopes
            .iter()
            .rev()
            .find_map(|scope| scope.values.get(name))
        {
            return *meaning;
        }
        match name {
            "iftrue" => ConditionalMeaning::True,
            "iffalse" => ConditionalMeaning::False,
            "ifcase" => ConditionalMeaning::Case,
            "else" => ConditionalMeaning::Else,
            "or" => ConditionalMeaning::Or,
            "fi" => ConditionalMeaning::Fi,
            "unless" => ConditionalMeaning::Unless,
            _ if is_condition_command(name, config) => ConditionalMeaning::Other,
            _ => ConditionalMeaning::Ordinary,
        }
    }

    pub(super) fn assigned(&self, name: &str) -> bool {
        self.scopes
            .iter()
            .rev()
            .any(|scope| scope.values.contains_key(name))
    }

    pub(super) fn enter(&mut self) {
        self.depth += 1;
    }

    pub(super) fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
        while self
            .scopes
            .last()
            .is_some_and(|scope| scope.depth > self.depth)
        {
            self.scopes.pop();
        }
    }

    pub(super) fn assign(&mut self, name: &str, meaning: ConditionalMeaning, global: bool) {
        let depth = if global { 0 } else { self.depth };
        let scope = if global {
            for scope in &mut self.scopes {
                if scope.depth > 0 {
                    scope.values.remove(name);
                }
            }
            if self.scopes.first().is_none_or(|scope| scope.depth != 0) {
                self.scopes.insert(
                    0,
                    ConditionalBindingScope {
                        depth,
                        values: HashMap::new(),
                    },
                );
            }
            &mut self.scopes[0].values
        } else {
            if self.scopes.last().is_none_or(|scope| scope.depth != depth) {
                self.scopes.push(ConditionalBindingScope {
                    depth,
                    values: HashMap::new(),
                });
            }
            &mut self.scopes.last_mut().expect("binding scope").values
        };
        if let Some(value) = scope.get_mut(name) {
            *value = meaning;
        } else {
            scope.insert(name.into(), meaning);
        }
    }

    pub(super) fn observe(&mut self, source: &str, name: &str, end: usize, config: &Config) {
        self.observe_with_uncertainty(source, name, end, config, false);
    }

    pub(super) fn observe_with_uncertainty(
        &mut self,
        source: &str,
        name: &str,
        end: usize,
        config: &Config,
        uncertain: bool,
    ) {
        let base = name.trim_end_matches('*');
        if base == "global" {
            self.global = true;
            return;
        }
        if matches!(base, "long" | "outer" | "protected") {
            return;
        }
        let primitive = matches!(base, "def" | "gdef" | "edef" | "xdef" | "let" | "futurelet");
        if primitive
            || latex_macro_definition(base)
                && !base.ends_with("environment")
                && !base.ends_with("Environment")
        {
            let mut target = skip_trivia(source, end);
            if source.as_bytes().get(target) == Some(&b'{') {
                target = skip_trivia(source, target + 1);
            }
            if let Some((target_name, target_end)) = command_at(source, target) {
                let mut meaning = ConditionalMeaning::Ordinary;
                if matches!(base, "let" | "futurelet") {
                    let mut operand = skip_trivia(source, target_end);
                    if base == "futurelet" {
                        operand = tex_token_end(source, operand)
                            .map_or(source.len(), |finish| skip_trivia(source, finish));
                    } else if source.as_bytes().get(operand) == Some(&b'=') {
                        operand = skip_trivia(source, operand + 1);
                    }
                    if let Some((value, _)) = command_at(source, operand) {
                        meaning = self.meaning(value.trim_end_matches('*'), config);
                    }
                }
                if base != "providecommand"
                    || !self.assigned(target_name)
                        && self.meaning(target_name, config) == ConditionalMeaning::Ordinary
                {
                    if uncertain && meaning != self.meaning(target_name, config) {
                        meaning = ConditionalMeaning::Other;
                    }
                    self.assign(
                        target_name,
                        meaning,
                        self.global || matches!(base, "gdef" | "xdef"),
                    );
                }
            }
        }
        self.global = false;
    }
}

pub(super) struct ActiveCondition {
    pub(super) meaning: ConditionalMeaning,
    pub(super) selected: Option<i32>,
    pub(super) branch: i32,
    pub(super) had_else: bool,
}

// Read operands whose token boundaries are fixed without macro expansion.
// A comment or an expandable continuation keeps the test dynamic.
pub(super) fn literal_integer(source: &str, start: usize) -> Option<(i64, usize)> {
    let mut cursor = skip_trivia(source, start);
    let mut sign = 1i64;
    while matches!(source.as_bytes().get(cursor), Some(b'+' | b'-')) {
        if source.as_bytes()[cursor] == b'-' {
            sign = -sign;
        }
        cursor = skip_trivia(source, cursor + 1);
    }
    if source.as_bytes().get(cursor) == Some(&b'`') {
        cursor += 1;
        let value = if source.as_bytes().get(cursor) == Some(&b'\\') {
            let (name, end) = command_at(source, cursor)?;
            if name.chars().count() != 1 {
                return None;
            }
            cursor = end;
            name.chars().next()? as i64
        } else {
            let value = source[cursor..].chars().next()?;
            cursor += value.len_utf8();
            value as i64
        };
        return Some((sign * value, cursor));
    }
    let radix = match source.as_bytes().get(cursor) {
        Some(b'\'') => {
            cursor += 1;
            8
        }
        Some(b'"') => {
            cursor += 1;
            16
        }
        _ => 10,
    };
    let digits = cursor;
    while source[cursor..]
        .chars()
        .next()
        .is_some_and(|c| c.is_digit(radix))
    {
        cursor += 1;
    }
    if cursor == digits || source.as_bytes().get(cursor) == Some(&b'%') {
        return None;
    }
    Some((
        sign * i64::from_str_radix(&source[digits..cursor], radix).ok()?,
        cursor,
    ))
}

fn literal_dimension(source: &str, start: usize) -> Option<(i64, usize)> {
    let mut cursor = skip_trivia(source, start);
    let mut sign = 1i64;
    while matches!(source.as_bytes().get(cursor), Some(b'+' | b'-')) {
        if source.as_bytes()[cursor] == b'-' {
            sign = -sign;
        }
        cursor = skip_trivia(source, cursor + 1);
    }
    let begin = cursor;
    while source
        .as_bytes()
        .get(cursor)
        .is_some_and(u8::is_ascii_digit)
    {
        cursor += 1;
    }
    let mut integer = if begin == cursor {
        0
    } else {
        source[begin..cursor].parse::<i64>().ok()?
    };
    let mut fraction = 0i64;
    let mut digits = cursor != begin;
    if matches!(source.as_bytes().get(cursor), Some(b'.' | b',')) {
        cursor += 1;
        let decimal = cursor;
        while source
            .as_bytes()
            .get(cursor)
            .is_some_and(u8::is_ascii_digit)
        {
            cursor += 1;
        }
        digits |= cursor > decimal;
        // TeX's round_decimals reads at most seventeen fractional digits.
        for digit in source.as_bytes()[decimal..cursor].iter().take(17).rev() {
            fraction = (fraction + i64::from(*digit - b'0') * 131_072) / 10;
        }
        fraction = (fraction + 1) / 2;
    }
    if !digits || source.as_bytes().get(cursor) == Some(&b'%') {
        return None;
    }
    cursor = skip_trivia(source, cursor);
    let unit = source.get(cursor..cursor + 2)?.to_ascii_lowercase();
    if unit == "sp" {
        return Some((sign * integer, cursor + 2));
    }
    let (numerator, denominator) = match unit.as_str() {
        "pt" => (1, 1),
        "pc" => (12, 1),
        "in" => (7227, 100),
        "bp" => (7227, 7200),
        "cm" => (7227, 254),
        "mm" => (7227, 2540),
        "dd" => (1238, 1157),
        "cc" => (14856, 1157),
        _ => return None,
    };
    // Use TeX's rational unit conversion and truncation in scaled points.
    let converted = integer.checked_mul(numerator)?;
    integer = converted / denominator;
    fraction = (numerator * fraction + 65_536 * (converted % denominator)) / denominator;
    let scaled = integer.checked_mul(65_536)?.checked_add(fraction)?;
    if scaled > 1_073_741_823 {
        return None;
    }
    Some((sign * scaled, cursor + 2))
}

fn numeric_operand_end(source: &str, mut start: usize, dimension: bool) -> Option<usize> {
    loop {
        start = skip_trivia(source, start);
        let literal = if dimension {
            literal_dimension(source, start)
        } else {
            literal_integer(source, start)
        };
        if let Some((_, end)) = literal {
            return Some(end);
        }
        while matches!(source.as_bytes().get(start), Some(b'+' | b'-')) {
            start = skip_trivia(source, start + 1);
        }
        let (name, end) = command_at(source, start)?;
        return match name {
            "the" | "number" => {
                start = end;
                continue;
            }
            "catcode" | "lccode" | "uccode" | "sfcode" | "mathcode" | "delcode" | "count"
            | "dimen" | "skip" => literal_integer(source, end).map(|(_, finish)| finish),
            "numexpr" | "dimexpr" => {
                let mut cursor = end;
                while cursor < source.len() {
                    if source.as_bytes()[cursor] == b'%' {
                        cursor = line_end(source, cursor);
                    } else if let Some((name, finish)) = command_at(source, cursor) {
                        if name == "relax" {
                            return Some(finish);
                        }
                        cursor = finish;
                    } else {
                        cursor += source[cursor..].chars().next()?.len_utf8();
                    }
                }
                None
            }
            _ => Some(end),
        };
    }
}

pub(super) fn numeric_test_end(source: &str, name: &str, start: usize) -> Option<usize> {
    let dimension = name.contains("dim");
    let left = numeric_operand_end(source, start, dimension)?;
    if name == "ifodd" {
        return Some(left);
    }
    let relation = skip_trivia(source, left);
    if !matches!(source.as_bytes().get(relation), Some(b'<' | b'=' | b'>')) {
        return None;
    }
    numeric_operand_end(source, relation + 1, dimension)
}

pub(super) fn literal_numeric_condition(
    source: &str,
    name: &str,
    end: usize,
    bindings: &ConditionalBindings,
) -> Option<(ConditionalMeaning, usize)> {
    if bindings.assigned(name) {
        return None;
    }
    let dimensions = matches!(name, "ifdim" | "ifabsdim" | "ifpdfabsdim");
    if !dimensions && !matches!(name, "ifnum" | "ifabsnum" | "ifpdfabsnum" | "ifodd") {
        return None;
    }
    let read = |start| {
        if dimensions {
            literal_dimension(source, start)
        } else {
            literal_integer(source, start)
        }
    };
    let (mut left, mut finish) = read(end)?;
    let result = if name == "ifodd" {
        left % 2 != 0
    } else {
        let relation = skip_trivia(source, finish);
        let operator = *source.as_bytes().get(relation)?;
        if !matches!(operator, b'<' | b'=' | b'>') {
            return None;
        }
        let (mut right, next) = read(relation + 1)?;
        finish = next;
        if name.contains("abs") {
            left = left.checked_abs()?;
            right = right.checked_abs()?;
        }
        match operator {
            b'<' => left < right,
            b'>' => left > right,
            _ => left == right,
        }
    };
    if source.as_bytes().get(finish) == Some(&b'%') {
        return None;
    }
    if source.as_bytes().get(finish) == Some(&b'\\') {
        let (continuation, _) = command_at(source, finish)?;
        if continuation != "relax" || bindings.assigned(continuation) {
            return None;
        }
    }
    if source
        .as_bytes()
        .get(finish)
        .is_some_and(u8::is_ascii_whitespace)
    {
        finish += 1;
    }
    Some((
        if result {
            ConditionalMeaning::True
        } else {
            ConditionalMeaning::False
        },
        finish,
    ))
}

pub(super) fn literal_case_number(
    source: &str,
    end: usize,
    bindings: &ConditionalBindings,
) -> Option<(i32, usize)> {
    let mut cursor = skip_trivia(source, end);
    let start = cursor;
    if matches!(source.as_bytes().get(cursor), Some(b'+' | b'-')) {
        cursor += 1;
    }
    let digits = cursor;
    while source
        .as_bytes()
        .get(cursor)
        .is_some_and(u8::is_ascii_digit)
    {
        cursor += 1;
    }
    if digits == cursor {
        return None;
    }
    let value = source[start..cursor].parse().ok()?;
    match source.as_bytes().get(cursor) {
        Some(b'%') => return None,
        Some(b'\\') => {
            let (name, _) = command_at(source, cursor)?;
            if name != "relax" || bindings.assigned(name) {
                return None;
            }
        }
        _ => (),
    }
    if source
        .as_bytes()
        .get(cursor)
        .is_some_and(u8::is_ascii_whitespace)
    {
        cursor += 1;
    }
    Some((value, cursor))
}

pub(super) fn inactive_branch_marker(
    source: &str,
    start: usize,
    config: &Config,
    bindings: &ConditionalBindings,
    stop_alternative: bool,
    case: bool,
) -> Option<(ConditionalMeaning, Range<usize>)> {
    let mut pos = start;
    let mut depth = 1usize;
    while pos < source.len() {
        match source.as_bytes()[pos] {
            b'%' => {
                pos = line_end(source, pos);
                continue;
            }
            b'\\' => {
                let (name, end) = command_at(source, pos)?;
                let meaning = bindings.meaning(name.trim_end_matches('*'), config);
                if matches!(
                    meaning,
                    ConditionalMeaning::True
                        | ConditionalMeaning::False
                        | ConditionalMeaning::Case
                        | ConditionalMeaning::Other
                ) {
                    depth += 1;
                } else if meaning == ConditionalMeaning::Fi {
                    depth -= 1;
                    if depth == 0 {
                        return Some((meaning, pos..end));
                    }
                } else if depth == 1
                    && stop_alternative
                    && (meaning == ConditionalMeaning::Else
                        || case && meaning == ConditionalMeaning::Or)
                {
                    return Some((meaning, pos..end));
                }
                pos = end;
                continue;
            }
            _ => pos += source[pos..].chars().next()?.len_utf8(),
        }
    }
    None
}

pub(super) fn dead_branch_needs_protection(
    source: &str,
    range: Range<usize>,
    config: &Config,
    compiled: &CompiledSyntax,
    bindings: &ConditionalBindings,
) -> bool {
    let text = &source[range];
    let Ok(tokens) = lex_with_ranges(text, config, compiled, &[]) else {
        return true;
    };
    #[derive(PartialEq, Eq)]
    enum Scope<'a> {
        Group,
        Environment(&'a str),
    }
    let mut stack = Vec::new();
    let mut conditions = 0usize;
    for token in &tokens {
        match token.kind {
            TokenKind::Math
            | TokenKind::Protected
            | TokenKind::OpenBracket
            | TokenKind::CloseBracket => return true,
            TokenKind::OpenBrace => stack.push(Scope::Group),
            TokenKind::CloseBrace => {
                if stack.pop() != Some(Scope::Group) {
                    return true;
                }
            }
            TokenKind::BeginEnvironment => {
                let (name, _, _) = env_at(text, token.span.start).expect("environment token");
                stack.push(Scope::Environment(name));
            }
            TokenKind::EndEnvironment => {
                let (name, _, _) = env_at(text, token.span.start).expect("environment token");
                if stack.pop() != Some(Scope::Environment(name)) {
                    return true;
                }
            }
            TokenKind::Command => {
                let name = command_at(text, token.span.start)
                    .expect("command token")
                    .0
                    .trim_end_matches('*');
                match bindings.meaning(name, config) {
                    ConditionalMeaning::True
                    | ConditionalMeaning::False
                    | ConditionalMeaning::Case
                    | ConditionalMeaning::Other => conditions += 1,
                    ConditionalMeaning::Fi => {
                        if conditions == 0 {
                            return true;
                        }
                        conditions -= 1;
                    }
                    ConditionalMeaning::Else | ConditionalMeaning::Or => (),
                    _ => return true,
                }
            }
            TokenKind::SpecialBegin(_) | TokenKind::SpecialMiddle(_) | TokenKind::SpecialEnd(_) => {
                return true;
            }
            _ => (),
        }
    }
    !stack.is_empty() || conditions != 0
}

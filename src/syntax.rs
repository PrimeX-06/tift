pub(crate) mod arguments;
mod declarations;
pub(crate) mod xparse;
pub use declarations::{
    DeclarationIndex, infer_declaration_config, infer_declaration_config_indexed,
    infer_declaration_config_with_continuation, infer_declaration_config_with_reuse,
};
mod conditions;
use conditions::{
    ActiveCondition, ConditionalBindings, ConditionalMeaning, dead_branch_needs_protection,
    inactive_branch_marker, literal_case_number, literal_integer, literal_numeric_condition,
    numeric_test_end,
};

pub(crate) use crate::block_storage::SharedBlock;
use crate::block_storage::{BlockName, NamePool, NameStore, OwnedNames, StructuralBlock};
use crate::config::SyntaxConfig;
use crate::{Config, FormatError};
use std::collections::HashMap;
use std::collections::HashSet;
use std::ops::Range;
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Debug)]
pub struct CompiledSyntax {
    pub command: Option<Arc<fancy_regex::Regex>>,
    pub environment: Option<Arc<fancy_regex::Regex>>,
    pub condition: Option<Arc<fancy_regex::Regex>>,
    pub item: Option<Arc<fancy_regex::Regex>>,
    pub between: Option<Arc<fancy_regex::Regex>>,
    specials: Vec<(Matcher, Matcher, Vec<Matcher>)>,
}

#[derive(Debug)]
enum Matcher {
    Literal(String),
    Regex(Arc<fancy_regex::Regex>),
}

static REGEX_CACHE: OnceLock<Mutex<HashMap<String, Arc<fancy_regex::Regex>>>> = OnceLock::new();

fn cached_regex(pattern: &str, enabled: bool) -> Result<Arc<fancy_regex::Regex>, String> {
    if !enabled {
        return fancy_regex::RegexBuilder::new(pattern)
            .backtrack_limit(1_000_000)
            .build()
            .map(Arc::new)
            .map_err(|e| e.to_string());
    }
    let cache = REGEX_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut cache = cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(regex) = cache.get(pattern) {
        return Ok(Arc::clone(regex));
    }
    let regex = Arc::new(
        fancy_regex::RegexBuilder::new(pattern)
            .backtrack_limit(1_000_000)
            .build()
            .map_err(|e| e.to_string())?,
    );
    if cache.len() >= 256 {
        cache.clear();
    }
    cache.insert(pattern.into(), Arc::clone(&regex));
    Ok(regex)
}

pub fn compile(config: &SyntaxConfig) -> Result<CompiledSyntax, String> {
    compile_with_cache(config, true)
}

pub fn compile_with_cache(config: &SyntaxConfig, enabled: bool) -> Result<CompiledSyntax, String> {
    let optional = |s: &Option<String>| {
        s.as_deref()
            .map(|pattern| cached_regex(pattern, enabled))
            .transpose()
    };
    let mut specials = Vec::new();
    for rule in &config.special {
        if rule.name.is_empty() || rule.begin.is_empty() || rule.end.is_empty() {
            return Err("special rules require nonempty name, begin and end".into());
        }
        let make = |pattern: &str| -> Result<Matcher, String> {
            if rule.regex {
                let regex = cached_regex(pattern, enabled)?;
                if regex.find("").map_err(|e| e.to_string())?.is_some() {
                    return Err("special delimiters must consume characters".into());
                }
                Ok(Matcher::Regex(regex))
            } else {
                Ok(Matcher::Literal(pattern.into()))
            }
        };
        if rule
            .middle
            .iter()
            .any(|pattern| pattern.is_empty() || pattern == &rule.begin || pattern == &rule.end)
        {
            return Err(
                "special middle delimiters must be nonempty and differ from begin and end".into(),
            );
        }
        let mut middle = Vec::new();
        for pattern in &rule.middle {
            middle.push(make(pattern)?);
        }
        specials.push((make(&rule.begin)?, make(&rule.end)?, middle));
    }
    Ok(CompiledSyntax {
        command: optional(&config.command_pattern)?,
        environment: optional(&config.environment_pattern)?,
        condition: optional(&config.condition_pattern)?,
        item: optional(&config.item_pattern)?,
        between: optional(&config.argument_between)?,
        specials,
    })
}

impl Matcher {
    fn matches(&self, source: &str) -> Result<Vec<Range<usize>>, FormatError> {
        match self {
            Self::Literal(s) => Ok(source
                .match_indices(s)
                .map(|(start, text)| start..start + text.len())
                .collect()),
            Self::Regex(re) => re
                .find_iter(source)
                .map(|matched| {
                    let matched = matched.map_err(|e| FormatError::Syntax(e.to_string()))?;
                    if matched.start() == matched.end() {
                        return Err(FormatError::Syntax("zero-width special delimiter".into()));
                    }
                    Ok(matched.start()..matched.end())
                })
                .collect(),
        }
    }
}

#[derive(Default)]
struct SpecialMatches {
    begin: Vec<Range<usize>>,
    end: Vec<Range<usize>>,
    middle: Vec<Range<usize>>,
}
fn at_match(ranges: &[Range<usize>], pos: usize) -> Option<usize> {
    ranges
        .binary_search_by_key(&pos, |r| r.start)
        .ok()
        .map(|i| ranges[i].end)
}
fn next_match(ranges: &[Range<usize>], pos: usize) -> Option<&Range<usize>> {
    ranges.get(ranges.partition_point(|r| r.start < pos))
}
fn special_matches(
    source: &str,
    config: &Config,
    compiled: &CompiledSyntax,
    protected_only: bool,
) -> Result<Vec<SpecialMatches>, FormatError> {
    compiled
        .specials
        .iter()
        .enumerate()
        .map(|(i, (begin, end, middle))| {
            if config.syntax.special[i].verbatim != protected_only {
                return Ok(SpecialMatches::default());
            }
            let mut middle_ranges = Vec::new();
            for matcher in middle {
                middle_ranges.extend(matcher.matches(source)?);
            }
            middle_ranges.sort_by_key(|range| (range.start, std::cmp::Reverse(range.end)));
            middle_ranges.dedup_by_key(|range| range.start);
            Ok(SpecialMatches {
                begin: begin.matches(source)?,
                end: end.matches(source)?,
                middle: middle_ranges,
            })
        })
        .collect()
}

pub fn is_protected(offset: usize, ranges: &[Range<usize>]) -> bool {
    let index = ranges.partition_point(|range| range.end <= offset);
    ranges.get(index).is_some_and(|range| range.start <= offset)
}

pub fn overlaps(range: &Range<usize>, ranges: &[Range<usize>]) -> bool {
    let index = ranges.partition_point(|r| r.end <= range.start);
    ranges.get(index).is_some_and(|r| r.start < range.end)
}

pub fn is_escaped(source: &str, offset: usize) -> bool {
    let bytes = source.as_bytes();
    let mut count = 0;
    let mut pos = offset;
    while pos > 0 && bytes[pos - 1] == b'\\' {
        count += 1;
        pos -= 1;
    }
    count % 2 == 1
}

pub fn command_at(source: &str, start: usize) -> Option<(&str, usize)> {
    let bytes = source.as_bytes();
    if bytes.get(start) != Some(&b'\\') {
        return None;
    }
    let name_start = start + 1;
    let next = *bytes.get(name_start)?;
    let control_word = |byte: u8| byte.is_ascii_alphabetic() || matches!(byte, b'@' | b'_' | b':');
    let mut end = name_start + 1;
    if control_word(next) {
        while bytes.get(end).is_some_and(|&byte| control_word(byte)) {
            end += 1;
        }
        if bytes.get(end) == Some(&b'*') {
            end += 1;
        }
    } else if !next.is_ascii() {
        // A non-ASCII control symbol is exactly one Unicode scalar, as before.
        end = name_start + source[name_start..].chars().next()?.len_utf8();
    }
    Some((&source[name_start..end], end))
}

fn skip_space(source: &str, mut pos: usize) -> usize {
    while let Some(&byte) = source.as_bytes().get(pos) {
        if byte.is_ascii_whitespace() || byte == b'\x0b' {
            pos += 1;
        } else if !byte.is_ascii() {
            let character = source[pos..].chars().next().expect("UTF-8 boundary");
            if !character.is_whitespace() {
                break;
            }
            pos += character.len_utf8();
        } else {
            break;
        }
    }
    pos
}

fn skip_trivia(source: &str, mut pos: usize) -> usize {
    loop {
        pos = skip_space(source, pos);
        if source.as_bytes().get(pos) != Some(&b'%') {
            return pos;
        }
        pos = line_end(source, pos);
    }
}

// These kernel commands take a test token before their two branch arguments.
// A bracket, math shift or conditional control sequence in that position is
// data, so it must not open a structural block in either scanner.
fn next_character_test_end(source: &str, name: &str, end: usize) -> Option<usize> {
    if !matches!(
        name.trim_end_matches('*'),
        "@ifnextchar" | "kernel@ifnextchar"
    ) {
        return None;
    }
    // command_at includes a trailing star; here that star already is the test.
    if name.ends_with('*') {
        return Some(end);
    }
    let start = skip_trivia(source, end);
    match source.as_bytes().get(start)? {
        b'{' => balanced_end(source, start, b'{', b'}'),
        b'\\' => {
            let (token, mut finish) = command_at(source, start)?;
            // A star after a control word is a separate TeX token.
            if token.len() > 1 && token.ends_with('*') {
                finish -= 1;
            }
            Some(finish)
        }
        _ => Some(start + source[start..].chars().next()?.len_utf8()),
    }
}

// These primitives read one TeX token as data. In particular, \string{ does
// not start a group. A trailing star belongs to that token, not the primitive.
pub(crate) fn primitive_data_end(source: &str, name: &str, end: usize) -> Option<usize> {
    let base = name.trim_end_matches('*');
    if !matches!(base, "string" | "meaning" | "show" | "noexpand") {
        return None;
    }
    let start = if name.ends_with('*') {
        end - 1
    } else {
        skip_trivia(source, end)
    };
    if source.as_bytes().get(start) == Some(&b'\\') {
        let (token, finish) = command_at(source, start)?;
        Some(if token.len() > 1 && token.ends_with('*') {
            finish - 1
        } else {
            finish
        })
    } else if base != "noexpand" {
        Some(start + source[start..].chars().next()?.len_utf8())
    } else {
        None
    }
}

fn tex_token_end(source: &str, start: usize) -> Option<usize> {
    let start = skip_trivia(source, start);
    if source.as_bytes().get(start) == Some(&b'\\') {
        let (name, end) = command_at(source, start)?;
        Some(if name.len() > 1 && name.ends_with('*') {
            end - 1
        } else {
            end
        })
    } else {
        Some(start + source[start..].chars().next()?.len_utf8())
    }
}

fn primitive_test_end(source: &str, name: &str, end: usize) -> Option<usize> {
    let base = name.trim_end_matches('*');
    let start = if name.ends_with('*') { end - 1 } else { end };
    match base {
        "ifx" => tex_token_end(source, tex_token_end(source, start)?),
        "ifdefined" | "ifprimitive" => tex_token_end(source, start),
        "if" | "ifcat" => {
            // Expanded operands require macro execution to determine. Direct
            // character operands can be read precisely without expansion.
            let first = skip_trivia(source, start);
            if source.as_bytes().get(first) == Some(&b'\\') {
                return None;
            }
            let next = tex_token_end(source, first)?;
            let second = skip_trivia(source, next);
            (source.as_bytes().get(second) != Some(&b'\\'))
                .then(|| tex_token_end(source, second))
                .flatten()
        }
        "ifcsname" => {
            constructed_csname(source, start - "ifcsname".len() - 1).map(|(_, finish)| finish)
        }
        "ifnum" | "ifdim" | "ifabsnum" | "ifabsdim" | "ifpdfabsnum" | "ifpdfabsdim" | "ifodd" => {
            numeric_test_end(source, base, start)
        }
        _ => None,
    }
}

pub(crate) fn let_declaration_end(source: &str, name: &str, end: usize) -> Option<usize> {
    if !matches!(name, "let" | "futurelet") {
        return None;
    }
    let target = tex_token_end(source, end)?;
    let mut operand = skip_trivia(source, target);
    if name == "futurelet" {
        // Its two lookahead tokens return to the input stream for execution.
        return Some(target);
    }
    if source.as_bytes().get(operand) == Some(&b'=') {
        operand += 1;
        // TeX skips one optional space after the optional equals sign.
        if source.as_bytes().get(operand) == Some(&b' ') {
            operand += 1;
        }
    }
    tex_token_end(source, operand)
}

fn primitive_literal_argument_end(source: &str, name: &str, end: usize) -> Option<usize> {
    if !matches!(name, "detokenize" | "showtokens") {
        return None;
    }
    balanced_end(source, skip_trivia(source, end), b'{', b'}')
}

fn definition_header_end(source: &str, name: &str, end: usize) -> Option<usize> {
    (latex_macro_definition(name) || matches!(name, "def" | "gdef" | "edef" | "xdef"))
        .then(|| macro_body(source, name, end).map(|body| body.start))
        .flatten()
}

fn literal_csname(source: &str, start: usize) -> Option<(String, usize)> {
    if !source.as_bytes().get(start..)?.starts_with(b"\\csname") {
        return None;
    }
    if command_at(source, start)?.0 != "csname" {
        return None;
    }
    constructed_csname(source, start)
}

fn constructed_csname(source: &str, start: usize) -> Option<(String, usize)> {
    let (name, mut pos) = command_at(source, start)?;
    if name != "csname" && name != "ifcsname" {
        return None;
    }
    let mut constructed = String::new();
    while pos < source.len() {
        match source.as_bytes()[pos] {
            b'\\' => {
                let (name, end) = command_at(source, pos)?;
                return (name == "endcsname").then_some((constructed, end));
            }
            b'%' => pos = line_end(source, pos),
            _ => {
                let character = source[pos..].chars().next()?;
                if !character.is_whitespace() {
                    constructed.push(character);
                }
                pos += character.len_utf8();
            }
        }
    }
    None
}

// Resolve only zero-argument literal replacements, with ordinary TeX scope and
// source-order semantics. Parameterized, conditional and executable replacement
// text remains unknown; formatting never runs that text to discover a name.
pub(crate) fn constant_constructed_commands(
    source: &str,
    config: &Config,
    protected: &[Range<usize>],
) -> HashMap<usize, (String, usize)> {
    if !source.match_indices("\\csname").any(|(start, _)| {
        !is_protected(start, protected)
            && command_at(source, start).is_some_and(|(name, _)| name == "csname")
            && literal_csname(source, start).is_none()
    }) {
        return HashMap::new();
    }
    crate::literal_expansion::LiteralExpansionIndex::build(source, config, protected)
        .into_constructed_commands()
}

pub fn balanced_end(source: &str, start: usize, open: u8, close: u8) -> Option<usize> {
    if source.as_bytes().get(start) != Some(&open) {
        return None;
    }
    let mut depth = 1usize;
    let mut braces = 0usize;
    let mut pos = start + 1;
    while pos < source.len() {
        match source.as_bytes()[pos] {
            b'\\' => {
                pos = command_at(source, pos)?.1;
                continue;
            }
            b'%' => {
                pos = source[pos..]
                    .find('\n')
                    .map_or(source.len(), |i| pos + i + 1);
                continue;
            }
            b'{' if open != b'{' => braces += 1,
            b'}' if open != b'{' => braces = braces.saturating_sub(1),
            c if c == open && braces == 0 => depth += 1,
            c if c == close && braces == 0 => {
                depth -= 1;
                if depth == 0 {
                    return Some(pos + 1);
                }
            }
            _ => (),
        }
        pos += source[pos..].chars().next()?.len_utf8();
    }
    None
}

fn env_at(source: &str, start: usize) -> Option<(&str, bool, usize)> {
    let (command, end) = command_at(source, start)?;
    env_after_command(source, command, end)
}

fn env_after_command<'a>(
    source: &'a str,
    command: &str,
    end: usize,
) -> Option<(&'a str, bool, usize)> {
    if command != "begin" && command != "end" {
        return None;
    }
    let brace = skip_trivia(source, end);
    let close = balanced_end(source, brace, b'{', b'}')?;
    Some((
        source[brace + 1..close - 1].trim(),
        command == "begin",
        close,
    ))
}

fn line_start(source: &str, pos: usize) -> usize {
    source[..pos].rfind('\n').map_or(0, |i| i + 1)
}
fn line_end(source: &str, pos: usize) -> usize {
    source[pos..]
        .find('\n')
        .map_or(source.len(), |i| pos + i + 1)
}

/// Shared physical-line controls for TeX and BibTeX comment scanners.
/// Call only for actual comments outside literal bodies and verbatim content.
#[derive(Default)]
pub(crate) struct FormatDirectiveState {
    off: Option<(usize, usize)>,
}

impl FormatDirectiveState {
    pub(crate) fn is_disabled(&self) -> bool {
        self.off.is_some()
    }

    pub(crate) fn observe(
        &mut self,
        source: &str,
        comment_start: usize,
        finish: usize,
        ranges: &mut Vec<Range<usize>>,
    ) -> usize {
        let comment = &source[comment_start + 1..finish];
        if !comment.starts_with(' ') {
            return finish;
        }
        let Some((namespace, action)) = comment.trim_start_matches(' ').split_once(':') else {
            return finish;
        };
        if !matches!(namespace, "tift" | "tex-fmt") || !action.starts_with(' ') {
            return finish;
        }
        match action
            .trim_start_matches(' ')
            .trim_end_matches([' ', '\t', '\r', '\n'])
        {
            "off" => {
                if let Some((_, depth)) = &mut self.off {
                    *depth += 1;
                } else {
                    self.off = Some((line_start(source, comment_start), 1));
                }
            }
            "on" => {
                if let Some((start, depth)) = &mut self.off {
                    *depth -= 1;
                    if *depth == 0 {
                        ranges.push(*start..finish);
                        self.off = None;
                    }
                }
            }
            "skip" if self.off.is_none() => {
                let start = line_start(source, comment_start);
                let standalone = source[start..comment_start]
                    .trim_matches([' ', '\t'])
                    .is_empty();
                let end = if namespace == "tift" && standalone && finish < source.len() {
                    line_end(source, finish)
                } else {
                    finish
                };
                ranges.push(start..end);
                return end;
            }
            _ => (),
        }
        finish
    }

    pub(crate) fn finish(self, source_len: usize, ranges: &mut Vec<Range<usize>>) {
        if let Some((start, _)) = self.off {
            ranges.push(start..source_len);
        }
    }
}

fn inline_end(source: &str, name: &str, mut pos: usize) -> Option<usize> {
    if crate::code_package_syntax::INLINE_CODE_COMMANDS.contains(&name.trim_end_matches('*')) {
        return crate::code_package_syntax::inline_code_end(source, name, pos);
    }
    if name.trim_end_matches('*') == "verb"
        || crate::code_environments::INLINE_VERBATIM_COMMANDS.contains(&name.trim_end_matches('*'))
    {
        let delimiter = source[pos..].chars().next()?;
        if delimiter.is_whitespace() {
            return None;
        }
        let body = pos + delimiter.len_utf8();
        return source[body..line_end(source, body)]
            .find(delimiter)
            .map(|i| body + i + delimiter.len_utf8());
    }
    pos = skip_space(source, pos);
    if source.as_bytes().get(pos) == Some(&b'[') {
        pos = balanced_end(source, pos, b'[', b']')?;
        pos = skip_space(source, pos);
    }
    if matches!(name.trim_end_matches('*'), "mintinline" | "mint") {
        pos = balanced_end(source, pos, b'{', b'}')?;
        pos = skip_space(source, pos);
    }
    if source.as_bytes().get(pos) == Some(&b'{') {
        return balanced_end(source, pos, b'{', b'}');
    }
    let delimiter = source[pos..].chars().next()?;
    if delimiter.is_whitespace() {
        return None;
    }
    let body = pos + delimiter.len_utf8();
    let suffix = &source[body..line_end(source, body)];
    suffix
        .find(delimiter)
        .map(|i| body + i + delimiter.len_utf8())
}

fn macro_body(source: &str, name: &str, end: usize) -> Option<Range<usize>> {
    let name = name.trim_end_matches('*');
    let mut pos = skip_trivia(source, end);
    if matches!(name, "def" | "gdef" | "edef" | "xdef") {
        if source.as_bytes().get(pos) == Some(&b'\\') {
            pos = command_at(source, pos)?.1;
        }
        while pos < source.len() && source.as_bytes()[pos] != b'{' {
            pos += source[pos..].chars().next()?.len_utf8();
        }
    } else {
        if source.as_bytes().get(pos) == Some(&b'{') {
            pos = balanced_end(source, pos, b'{', b'}')?;
        } else if source.as_bytes().get(pos) == Some(&b'\\') {
            pos = command_at(source, pos)?.1;
        } else {
            return None;
        }
        pos = skip_trivia(source, pos);
        while source.as_bytes().get(pos) == Some(&b'[') {
            pos = skip_trivia(source, balanced_end(source, pos, b'[', b']')?);
        }
        if name.contains("Document") {
            pos = skip_trivia(source, balanced_end(source, pos, b'{', b'}')?);
        }
    }
    let mut finish = balanced_end(source, pos, b'{', b'}')?;
    if name.to_ascii_lowercase().contains("environment") {
        finish = balanced_end(source, skip_trivia(source, finish), b'{', b'}')?;
    }
    Some(pos..finish)
}

fn raw_command_body(source: &str, end: usize, count: Option<usize>) -> Option<Range<usize>> {
    let mut pos = skip_trivia(source, end);
    while source.as_bytes().get(pos) == Some(&b'[') {
        pos = skip_trivia(source, balanced_end(source, pos, b'[', b']')?);
    }
    let mut body = None;
    for _ in 0..count.unwrap_or(1) {
        let finish = balanced_end(source, pos, b'{', b'}')?;
        body = Some(pos..finish);
        pos = skip_trivia(source, finish);
    }
    body
}

/// Lua chunks use TeX-balanced arguments, but their whitespace belongs to Lua.
fn embedded_lua_end(source: &str, name: &str, end: usize) -> Option<usize> {
    let native = matches!(name, "directlua" | "latelua");
    if !native && !crate::luadraw_syntax::LUA_CODE_COMMANDS.contains(&name) {
        return None;
    }
    let mut start = skip_trivia(source, end);
    if native && source.as_bytes().get(start) != Some(&b'{') {
        start = skip_trivia(source, literal_integer(source, start)?.1);
    }
    balanced_end(source, start, b'{', b'}')
}

/// Scan complete raw lines once, avoiding repeated prefix scans for fake ends.
fn line_delimited_environment_end(
    source: &str,
    begin_end: usize,
    terminator: &str,
) -> Option<usize> {
    let mut offset = line_end(source, begin_end);
    for line in source[offset..].split_inclusive('\n') {
        let content = line.trim_start_matches([' ', '\t', '\r']);
        if content
            .strip_prefix(terminator)
            .is_some_and(|tail| tail.trim_matches([' ', '\t', '\r', '\n']).is_empty())
        {
            return Some(offset + line.len() - content.len() + terminator.len());
        }
        offset += line.len();
    }
    None
}

fn primitive_condition(name: &str) -> bool {
    matches!(
        name,
        "if" | "ifcat"
            | "ifnum"
            | "ifdim"
            | "ifodd"
            | "ifvmode"
            | "ifhmode"
            | "ifmmode"
            | "ifinner"
            | "ifvoid"
            | "ifhbox"
            | "ifvbox"
            | "ifx"
            | "ifeof"
            | "iftrue"
            | "iffalse"
            | "ifcase"
            | "ifdefined"
            | "ifcsname"
            | "ifincsname"
            | "iffontchar"
            | "ifprimitive"
            | "ifabsnum"
            | "ifabsdim"
            | "ifpdfabsnum"
            | "ifpdfabsdim"
    )
}

/// Whether a command opens a primitive, known package, or configured condition.
pub fn is_condition_command(name: &str, config: &Config) -> bool {
    primitive_condition(name)
        || conditions::package_condition(name)
        || crate::code_package_syntax::is_condition(name)
        || config
            .syntax
            .conditions
            .iter()
            .any(|condition| condition == name)
}

fn math_environment(name: &str) -> bool {
    crate::packages::math::is_math_environment(name)
        || crate::packages::visual_science::is_display_math(name)
        || crate::package_environments::is_math(name)
        || matches!(
            name,
            "xalignat" | "xalignat*" | "xxalignat" | "algomathdisplay" | "tikzcd"
        )
        || matches!(
            name.trim_end_matches('*'),
            "equation"
                | "align"
                | "alignat"
                | "flalign"
                | "gather"
                | "multline"
                | "displaymath"
                | "math"
                | "eqnarray"
                | "array"
                | "matrix"
                | "pmatrix"
                | "bmatrix"
                | "Bmatrix"
                | "vmatrix"
                | "Vmatrix"
                | "smallmatrix"
                | "cases"
                | "aligned"
                | "alignedat"
                | "gathered"
                | "split"
        )
}

fn math_delimiter_command(name: &str) -> bool {
    matches!(
        name.trim_end_matches('*'),
        "left"
            | "right"
            | "middle"
            | "big"
            | "Big"
            | "bigg"
            | "Bigg"
            | "bigl"
            | "bigr"
            | "Bigl"
            | "Bigr"
            | "biggl"
            | "biggr"
            | "Biggl"
            | "Biggr"
            | "in"
            | "notin"
            | "ni"
            | "notni"
            | "subset"
            | "supset"
            | "subseteq"
            | "supseteq"
            | "cup"
            | "cap"
            | "setminus"
            | "smallsetminus"
            | "backslash"
    )
}

// In a formula, a literal interval can follow a control word. An optional
// argument must close before the enclosing formula's own terminator.
fn math_optional_argument_end(source: &str, start: usize) -> Option<usize> {
    let mut pos = start + 1;
    let mut brackets = 1usize;
    let mut braces = 0usize;
    while pos < source.len() {
        match source.as_bytes()[pos] {
            b'\\' => {
                let (name, end) = command_at(source, pos)?;
                if braces == 0 && matches!(name, "]" | ")") {
                    return None;
                }
                if braces == 0
                    && name == "end"
                    && env_at(source, pos).is_some_and(|(name, _, _)| math_environment(name))
                {
                    return None;
                }
                pos = end;
                continue;
            }
            b'%' => {
                pos = line_end(source, pos);
                continue;
            }
            b'{' => braces += 1,
            b'}' => {
                if braces == 0 {
                    return None;
                }
                braces -= 1;
            }
            b'$' if braces == 0 => return None,
            b'[' if braces == 0 => brackets += 1,
            b']' if braces == 0 => {
                brackets -= 1;
                if brackets == 0 {
                    return Some(pos + 1);
                }
            }
            _ => (),
        }
        pos += source[pos..].chars().next()?.len_utf8();
    }
    None
}

fn column_specification(source: &str, name: &str, end: usize) -> Option<Range<usize>> {
    if matches!(name, "tabu" | "longtabu") {
        return crate::package_tables::tabu_column_spec(source, end..source.len());
    }
    let signature = crate::packages::header_signature(name)?;
    let target = crate::packages::column_spec_slot(name)?;
    let mut cursor = end;
    for (index, slot) in signature.bytes().enumerate() {
        let start = skip_trivia(source, cursor);
        let (open, close) = if slot == b'm' {
            (b'{', b'}')
        } else {
            (b'[', b']')
        };
        if source.as_bytes().get(start) != Some(&open) {
            if slot == b'm' {
                return None;
            }
            continue;
        }
        cursor = balanced_end(source, start, open, close)?;
        if index == target {
            return Some(start..cursor);
        }
    }
    None
}

fn opaque_suffix<'a>(
    source: &str,
    protected: &'a [Range<usize>],
    config: &Config,
) -> Option<&'a Range<usize>> {
    protected.last().filter(|range| {
        range.end == source.len()
            && (command_at(source, range.start).is_some_and(|(name, end)| {
                matches!(name, "catcode" | "obeyspaces" | "obeylines" | "scantokens")
                    || name == "ExplSyntaxOn" && !source[end..].contains("\\ExplSyntaxOff")
            }) || env_at(source, range.start).is_some_and(|(name, begin, _)| {
                begin
                    && inferred_protected_environments(source, config, protected)
                        .iter()
                        .any(|environment| environment == name)
            }))
    })
}

#[derive(Debug, Clone)]
pub(crate) struct MacroDefinition {
    name: String,
    start: usize,
    end: usize,
    bodies: Vec<Range<usize>>,
    environment: Option<String>,
    command: Option<String>,
}

fn latex_macro_definition(name: &str) -> bool {
    matches!(
        name.trim_end_matches('*'),
        "newcommand"
            | "renewcommand"
            | "providecommand"
            | "DeclareRobustCommand"
            | "DeclareMathOperator"
            | "newenvironment"
            | "renewenvironment"
            | "NewDocumentCommand"
            | "RenewDocumentCommand"
            | "ProvideDocumentCommand"
            | "DeclareDocumentCommand"
            | "NewDocumentEnvironment"
            | "RenewDocumentEnvironment"
            | "ProvideDocumentEnvironment"
            | "DeclareDocumentEnvironment"
    )
}

fn macro_definitions(source: &str, config: &Config) -> Result<Vec<MacroDefinition>, FormatError> {
    let protected = protected_ranges(source, config)?;
    Ok(macro_definitions_with_ranges(source, &protected))
}

pub(crate) fn macro_definitions_with_ranges(
    source: &str,
    protected: &[Range<usize>],
) -> Vec<MacroDefinition> {
    let mut output = Vec::new();
    let mut pos = 0usize;
    while pos < source.len() {
        let index = protected.partition_point(|range| range.end <= pos);
        if let Some(range) = protected.get(index).filter(|range| range.start <= pos) {
            pos = range.end;
            continue;
        }
        if source.as_bytes()[pos] == b'%' {
            pos = line_end(source, pos);
            continue;
        }
        if source.as_bytes()[pos] == b'\\' {
            let Some((name, end)) = command_at(source, pos) else {
                break;
            };
            if name.trim_end_matches('*') == "newtheorem"
                && let Some(declaration) = context_theorem_declaration(source, pos, name, end)
            {
                output.push(MacroDefinition {
                    name: name.into(),
                    start: pos,
                    end: declaration.span.end,
                    bodies: Vec::new(),
                    environment: Some(declaration.name),
                    command: None,
                });
                pos = declaration.span.end;
                continue;
            }
            if (latex_macro_definition(name) || matches!(name, "def" | "gdef" | "edef" | "xdef"))
                && let Some(range) = macro_body(source, name, end)
            {
                let first_end =
                    balanced_end(source, range.start, b'{', b'}').expect("macro body was balanced");
                let mut bodies: Vec<Range<usize>> =
                    std::iter::once(range.start + 1..first_end - 1).collect();
                let environment = if name.to_ascii_lowercase().contains("environment") {
                    let next = skip_trivia(source, first_end);
                    bodies.push(next + 1..range.end - 1);
                    let name_start = skip_trivia(source, end);
                    balanced_end(source, name_start, b'{', b'}')
                        .map(|finish| source[name_start + 1..finish - 1].trim().to_owned())
                } else {
                    None
                };
                let command = if environment.is_none() {
                    let mut start = skip_trivia(source, end);
                    if source.as_bytes().get(start) == Some(&b'{') {
                        start = skip_trivia(source, start + 1);
                    }
                    command_at(source, start).map(|(name, _)| name.to_owned())
                } else {
                    None
                };
                output.push(MacroDefinition {
                    name: name.into(),
                    start: pos,
                    end: range.end,
                    bodies,
                    environment,
                    command,
                });
                pos = range.end;
                continue;
            }
            pos = end;
            continue;
        }
        let limit = protected
            .get(index)
            .map_or(source.len(), |range| range.start);
        pos = next_declaration_token(source, pos, limit);
    }
    output
}

fn next_declaration_token(source: &str, start: usize, limit: usize) -> usize {
    memchr::memchr2(b'\\', b'%', &source.as_bytes()[start..limit])
        .map_or(limit, |offset| start + offset)
}

#[derive(Clone)]
struct ContextDeclaration {
    name: String,
    environment: bool,
    span: Range<usize>,
    arguments: Option<crate::config::EnvironmentArguments>,
    raw: bool,
}

fn context_literal_name(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()
        && !value
            .chars()
            .any(|c| c.is_whitespace() || "\\{}[]%#,".contains(c)))
    .then(|| value.to_owned())
}

fn context_argument(source: &str, cursor: &mut usize, open: u8, close: u8) -> Option<Range<usize>> {
    let start = skip_trivia(source, *cursor);
    let finish = balanced_end(source, start, open, close)?;
    *cursor = finish;
    Some(start + 1..finish - 1)
}

fn context_signature(source: &str, specification: Range<usize>) -> Option<String> {
    let text = &source[specification];
    let signature = xparse::Signature::parse(text)?;
    Some(signature.compact().unwrap_or_else(|| text.to_owned()))
}

fn context_macro_declaration(
    source: &str,
    start: usize,
    command: &str,
    end: usize,
) -> Option<ContextDeclaration> {
    context_macro_declaration_with_config(source, start, command, end, None)
}

fn context_macro_declaration_with_config(
    source: &str,
    start: usize,
    command: &str,
    end: usize,
    config: Option<&Config>,
) -> Option<ContextDeclaration> {
    let body = macro_body(source, command, end)?;
    let mut cursor = end;
    let environment = command.to_ascii_lowercase().contains("environment");
    let name = if environment {
        context_literal_name(&source[context_argument(source, &mut cursor, b'{', b'}')?])?
    } else {
        cursor = skip_trivia(source, cursor);
        if source.as_bytes().get(cursor) == Some(&b'{') {
            let name_range = context_argument(source, &mut cursor, b'{', b'}')?;
            let name_start = skip_trivia(source, name_range.start);
            let (name, finish) = command_at(source, name_start)?;
            if skip_trivia(source, finish) != name_range.end {
                return None;
            }
            name.to_owned()
        } else {
            let (name, finish) = command_at(source, cursor)?;
            cursor = finish;
            name.to_owned()
        }
    };
    let configured_signature = config.is_some_and(|config| {
        if environment {
            config.syntax.environment_arguments.contains_key(&name)
        } else {
            config.syntax.commands.contains_key(&name)
        }
    });
    // The balanced body establishes completion even when explicit syntax makes
    // interpreting this declaration's argument specification unnecessary.
    let specification = if configured_signature {
        None
    } else if command.contains("Document") {
        context_argument(source, &mut cursor, b'{', b'}')
            .and_then(|range| context_signature(source, range))
    } else {
        let count = context_argument(source, &mut cursor, b'[', b']');
        let total = count.as_ref().map_or(Some(0), |range| {
            source[range.clone()].trim().parse::<usize>().ok()
        });
        let optional =
            count.is_some() && context_argument(source, &mut cursor, b'[', b']').is_some();
        total.filter(|total| *total <= 9).and_then(|total| {
            let required = total.checked_sub(usize::from(optional))?;
            Some(format!(
                "{}{}",
                if optional { "o" } else { "" },
                "m".repeat(required)
            ))
        })
    };
    let arguments = specification
        .filter(|_| skip_trivia(source, cursor) == body.start)
        .map(|signature| crate::config::EnvironmentArguments {
            signature: Some(signature),
            ..Default::default()
        });
    Some(ContextDeclaration {
        name,
        environment,
        span: start..body.end,
        arguments,
        raw: false,
    })
}

fn context_theorem_declaration(
    source: &str,
    start: usize,
    command: &str,
    end: usize,
) -> Option<ContextDeclaration> {
    let mut cursor = skip_trivia(source, end);
    let starred = command.ends_with('*') || source.as_bytes().get(cursor) == Some(&b'*');
    if !command.ends_with('*') && starred {
        cursor += 1;
    }
    let name = context_literal_name(&source[context_argument(source, &mut cursor, b'{', b'}')?])?;
    let shared = !starred && context_argument(source, &mut cursor, b'[', b']').is_some();
    context_argument(source, &mut cursor, b'{', b'}')?;
    if !starred && !shared {
        context_argument(source, &mut cursor, b'[', b']');
    }
    Some(ContextDeclaration {
        name,
        environment: true,
        span: start..cursor,
        arguments: Some(crate::config::EnvironmentArguments {
            signature: Some("o".into()),
            ..Default::default()
        }),
        raw: false,
    })
}

/// Reuse parsed definitions to collect ordinary environment header signatures.
/// The surrounding scan tracks scopes without reparsing definition bodies.
pub(crate) fn environment_argument_map_with_definitions(
    source: &str,
    config: &Config,
    protected: &[Range<usize>],
    definitions: &[MacroDefinition],
) -> HashMap<String, crate::config::EnvironmentArguments> {
    if !definitions
        .iter()
        .any(|definition| definition.environment.is_some())
    {
        return HashMap::new();
    }
    let mut selected = HashMap::new();
    let mut cursor = 0;
    let mut definition_index = 0;
    let mut depth = 0usize;
    let mut conditions = 0usize;
    let mut environments = 0usize;
    let mut known_conditions = HashSet::new();
    while cursor < source.len() {
        let index = protected.partition_point(|range| range.end <= cursor);
        if let Some(range) = protected.get(index).filter(|range| range.start <= cursor) {
            cursor = range.end;
            continue;
        }
        while definitions
            .get(definition_index)
            .is_some_and(|definition| definition.start < cursor)
        {
            definition_index += 1;
        }
        if let Some(definition) = definitions
            .get(definition_index)
            .filter(|definition| definition.start == cursor)
        {
            let finish = definition.end;
            if let Some(name) = &definition.environment {
                if depth == 0 && environments == 0 {
                    let base = definition.name.trim_end_matches('*');
                    if conditions > 0 {
                        selected.insert(name.clone(), None);
                    } else if !base.starts_with("Provide")
                        || !selected.contains_key(name)
                            && !config.syntax.environment_arguments.contains_key(name)
                            && crate::layout::environment_signature(name).is_none()
                    {
                        let (_, end) =
                            command_at(source, cursor).expect("parsed definition command");
                        let declaration = if base == "newtheorem" {
                            context_theorem_declaration(source, cursor, &definition.name, end)
                        } else {
                            context_macro_declaration(source, cursor, base, end)
                        };
                        let arguments = declaration.and_then(|declaration| declaration.arguments);
                        selected.insert(name.clone(), arguments);
                    }
                }
            } else if depth == 0
                && environments == 0
                && let Some(name) = &definition.command
            {
                for target in [Some(name.as_str()), name.strip_prefix("end")]
                    .into_iter()
                    .flatten()
                {
                    if selected.contains_key(target) {
                        selected.insert(target.to_owned(), None);
                    }
                }
            }
            cursor = finish;
            definition_index += 1;
            continue;
        }
        if source.as_bytes()[cursor] == b'%' {
            cursor = line_end(source, cursor);
            continue;
        }
        if let Some((command, end)) = command_at(source, cursor) {
            let base = command.trim_end_matches('*');
            if base == "newif"
                && let Some((name, finish)) = command_at(source, skip_trivia(source, end))
            {
                known_conditions.insert(name.to_owned());
                cursor = finish;
                continue;
            }
            if matches!(base, "let" | "futurelet")
                && let Some(declaration) = context_primitive_declaration(source, cursor, base, end)
            {
                if depth == 0 && environments == 0 {
                    for target in [
                        Some(declaration.name.as_str()),
                        declaration.name.strip_prefix("end"),
                    ]
                    .into_iter()
                    .flatten()
                    {
                        if selected.contains_key(target) {
                            selected.insert(target.to_owned(), None);
                        }
                    }
                }
                cursor = let_declaration_end(source, base, end).unwrap_or(declaration.span.end);
                continue;
            }
            if let Some(finish) = primitive_data_end(source, command, end) {
                cursor = finish;
                continue;
            }
            if let Some((name, begin, finish)) = env_at(source, cursor) {
                if name != "document" {
                    if begin {
                        environments += 1;
                    } else {
                        environments = environments.saturating_sub(1);
                    }
                }
                cursor = finish;
                continue;
            }
            match base {
                "begingroup" | "bgroup" => depth += 1,
                "endgroup" | "egroup" => depth = depth.saturating_sub(1),
                "fi" => conditions = conditions.saturating_sub(1),
                _ if is_condition_command(base, config) || known_conditions.contains(base) => {
                    conditions += 1
                }
                _ => (),
            }
            cursor = end;
            continue;
        }
        match source.as_bytes()[cursor] {
            b'{' => depth += 1,
            b'}' => depth = depth.saturating_sub(1),
            _ => (),
        }
        cursor += source[cursor..]
            .chars()
            .next()
            .expect("source character")
            .len_utf8();
    }
    selected
        .into_iter()
        .filter_map(|(name, arguments)| arguments.map(|arguments| (name, arguments)))
        .collect()
}

pub(crate) fn command_argument_map_with_definitions(
    source: &str,
    config: &Config,
    protected: &[Range<usize>],
    definitions: &[MacroDefinition],
) -> HashMap<String, crate::config::EnvironmentArguments> {
    if !definitions
        .iter()
        .any(|definition| definition.command.is_some())
    {
        return HashMap::new();
    }
    let mut selected = HashMap::new();
    let mut cursor = 0;
    let mut definition_index = 0;
    let mut depth = 0usize;
    let mut conditions = 0usize;
    let mut environments = 0usize;
    let mut known_conditions = HashSet::new();
    while cursor < source.len() {
        let index = protected.partition_point(|range| range.end <= cursor);
        if let Some(range) = protected.get(index).filter(|range| range.start <= cursor) {
            cursor = range.end;
            continue;
        }
        while definitions
            .get(definition_index)
            .is_some_and(|definition| definition.start < cursor)
        {
            definition_index += 1;
        }
        if let Some(definition) = definitions
            .get(definition_index)
            .filter(|definition| definition.start == cursor)
        {
            let finish = definition.end;
            if let Some(name) = &definition.command {
                if depth == 0 && environments == 0 {
                    let base = definition.name.trim_end_matches('*');
                    if conditions > 0 {
                        selected.insert(name.clone(), None);
                    } else if !base.starts_with("Provide")
                        || !selected.contains_key(name)
                            && !config.syntax.commands.contains_key(name)
                            && !context_builtin_command(name)
                    {
                        let (_, end) =
                            command_at(source, cursor).expect("parsed definition command");
                        let declaration = if base == "newtheorem" {
                            context_theorem_declaration(source, cursor, &definition.name, end)
                        } else {
                            context_macro_declaration(source, cursor, base, end)
                        };
                        let arguments = declaration.and_then(|declaration| declaration.arguments);
                        selected.insert(name.clone(), arguments);
                    }
                }
            } else if depth == 0
                && environments == 0
                && let Some(name) = &definition.environment
            {
                for target in [Some(name.as_str()), name.strip_prefix("end")]
                    .into_iter()
                    .flatten()
                {
                    if selected.contains_key(target) {
                        selected.insert(target.to_owned(), None);
                    }
                }
            }
            cursor = finish;
            definition_index += 1;
            continue;
        }
        if source.as_bytes()[cursor] == b'%' {
            cursor = line_end(source, cursor);
            continue;
        }
        if let Some((command, end)) = command_at(source, cursor) {
            let base = command.trim_end_matches('*');
            if base == "newif"
                && let Some((name, finish)) = command_at(source, skip_trivia(source, end))
            {
                known_conditions.insert(name.to_owned());
                cursor = finish;
                continue;
            }
            if matches!(base, "let" | "futurelet")
                && let Some(declaration) = context_primitive_declaration(source, cursor, base, end)
            {
                if depth == 0 && environments == 0 {
                    for target in [
                        Some(declaration.name.as_str()),
                        declaration.name.strip_prefix("end"),
                    ]
                    .into_iter()
                    .flatten()
                    {
                        if selected.contains_key(target) {
                            selected.insert(target.to_owned(), None);
                        }
                    }
                }
                cursor = let_declaration_end(source, base, end).unwrap_or(declaration.span.end);
                continue;
            }
            if let Some(finish) = primitive_data_end(source, command, end) {
                cursor = finish;
                continue;
            }
            if let Some((name, begin, finish)) = env_at(source, cursor) {
                if name != "document" {
                    if begin {
                        environments += 1;
                    } else {
                        environments = environments.saturating_sub(1);
                    }
                }
                cursor = finish;
                continue;
            }
            match base {
                "begingroup" | "bgroup" => depth += 1,
                "endgroup" | "egroup" => depth = depth.saturating_sub(1),
                "fi" => conditions = conditions.saturating_sub(1),
                _ if is_condition_command(base, config) || known_conditions.contains(base) => {
                    conditions += 1
                }
                _ => (),
            }
            cursor = end;
            continue;
        }
        match source.as_bytes()[cursor] {
            b'{' => depth += 1,
            b'}' => depth = depth.saturating_sub(1),
            _ => (),
        }
        cursor += source[cursor..]
            .chars()
            .next()
            .expect("source character")
            .len_utf8();
    }
    selected
        .into_iter()
        .filter_map(|(name, arguments)| arguments.map(|arguments| (name, arguments)))
        .collect()
}

fn context_factory_range(source: &str, range: &Range<usize>) -> bool {
    let mut cursor = range.start;
    while cursor < range.end {
        let Some((command, end)) = command_at(source, cursor) else {
            return false;
        };
        let finish = crate::package_environments::literal_box_declaration(source, command, end)
            .map(|declaration| declaration.end)
            .or_else(|| {
                crate::package_environments::verbatim_declaration(source, command, end)
                    .map(|declaration| declaration.end)
            });
        let Some(finish) = finish.filter(|finish| *finish <= range.end) else {
            return false;
        };
        cursor = finish;
    }
    cursor == range.end
}

pub(crate) fn token_data_range(source: &str, range: &Range<usize>) -> bool {
    let mut cursor = range.start;
    while cursor < range.end {
        let Some((name, end)) = command_at(source, cursor) else {
            return false;
        };
        let finish = primitive_data_end(source, name, end);
        let Some(finish) = finish.filter(|finish| *finish <= range.end) else {
            return false;
        };
        cursor = finish;
    }
    cursor == range.end
}

fn context_box_declaration(
    source: &str,
    start: usize,
    command: &str,
    end: usize,
) -> Option<ContextDeclaration> {
    if let Some(declaration) =
        crate::package_environments::literal_box_declaration(source, command, end)
    {
        return Some(ContextDeclaration {
            name: declaration.name,
            environment: true,
            span: start..declaration.end,
            arguments: Some(declaration.arguments),
            raw: false,
        });
    }
    if !matches!(
        command,
        "newtcolorbox"
            | "renewtcolorbox"
            | "NewTColorBox"
            | "RenewTColorBox"
            | "DeclareTColorBox"
            | "ProvideTColorBox"
    ) {
        return None;
    }
    let mut cursor = end;
    context_argument(source, &mut cursor, b'[', b']');
    let name = context_literal_name(&source[context_argument(source, &mut cursor, b'{', b'}')?])?;
    if matches!(command, "newtcolorbox" | "renewtcolorbox") {
        if context_argument(source, &mut cursor, b'[', b']').is_some() {
            context_argument(source, &mut cursor, b'[', b']');
        }
    } else {
        context_argument(source, &mut cursor, b'{', b'}')?;
    }
    context_argument(source, &mut cursor, b'{', b'}')?;
    Some(ContextDeclaration {
        name,
        environment: true,
        span: start..cursor,
        arguments: None,
        raw: false,
    })
}

fn context_primitive_declaration(
    source: &str,
    start: usize,
    command: &str,
    end: usize,
) -> Option<ContextDeclaration> {
    let target_start = skip_trivia(source, end);
    let (name, target_end) = command_at(source, target_start)?;
    let finish = if matches!(command, "def" | "gdef" | "edef" | "xdef") {
        macro_body(source, command, end)?.end
    } else if command == "futurelet" {
        target_end
    } else {
        let mut after = skip_trivia(source, target_end);
        if source.as_bytes().get(after) == Some(&b'=') {
            after = skip_trivia(source, after + 1);
        }
        command_at(source, after).map_or_else(
            || after + source[after..].chars().next().map_or(0, char::len_utf8),
            |(_, end)| end,
        )
    };
    Some(ContextDeclaration {
        name: name.to_owned(),
        environment: false,
        span: start..finish,
        arguments: None,
        raw: false,
    })
}

fn context_builtin_command(name: &str) -> bool {
    latex_macro_definition(name)
        || crate::packages::scoped_environment_for_command(name).is_some()
        || primitive_condition(name)
        || crate::package_commands::builtin_command_rule(name).is_some()
        || crate::packages::math::command(name).is_some()
        || matches!(
            name,
            "input"
                | "include"
                | "begin"
                | "end"
                | "string"
                | "noexpand"
                | "def"
                | "gdef"
                | "edef"
                | "xdef"
                | "let"
                | "newtheorem"
                | "newlist"
                | "newtcolorbox"
                | "renewtcolorbox"
                | "NewTColorBox"
                | "RenewTColorBox"
                | "DeclareTColorBox"
                | "ProvideTColorBox"
                | "newminted"
                | "lstnewenvironment"
                | "newtcblisting"
                | "renewtcblisting"
                | "NewTCBListing"
                | "RenewTCBListing"
                | "DeclareTCBListing"
                | "ProvideTCBListing"
                | "excludecomment"
                | "includecomment"
                | "specialcomment"
                | "processcomment"
        )
}

fn next_inference_delimiter(bytes: &[u8], mut cursor: usize, limit: usize) -> usize {
    while cursor < limit {
        // Neither search can repeatedly inspect the complete remaining text
        // when only the other delimiter family is nearby. Byte chunks may
        // cross Unicode scalars; an ASCII delimiter or the caller's validated
        // limit always returns a UTF-8 boundary.
        let end = cursor.saturating_add(128).min(limit);
        let chunk = &bytes[cursor..end];
        let command = memchr::memchr2(b'\\', b'%', chunk).unwrap_or(chunk.len());
        let group = memchr::memchr2(b'{', b'}', chunk).unwrap_or(chunk.len());
        let next = command.min(group);
        if next < chunk.len() {
            return cursor + next;
        }
        cursor = end;
    }
    limit
}

fn declaration_reader_complete(source: &str, name: &str, start: usize, end: usize) -> bool {
    let base = name.trim_end_matches('*');
    match base {
        "string" | "meaning" | "show" | "noexpand" => {
            tex_token_end(source, if name.ends_with('*') { end - 1 } else { end }).is_some()
        }
        "let" => let_declaration_end(source, base, end).is_some(),
        "futurelet" => tex_token_end(source, end)
            .and_then(|target| tex_token_end(source, target))
            .and_then(|first| tex_token_end(source, first))
            .is_some(),
        "expandafter" => tex_token_end(source, end)
            .and_then(|first| tex_token_end(source, first))
            .is_some(),
        "detokenize" | "showtokens" => primitive_literal_argument_end(source, base, end).is_some(),
        "newcolumntype" => macro_body(source, base, end).is_some(),
        "@ifnextchar" | "kernel@ifnextchar" => next_character_test_end(source, name, end).is_some(),
        "begin" | "end" => env_at(source, start).is_some(),
        "newif" => command_at(source, skip_trivia(source, end)).is_some(),
        "unless" => command_at(source, skip_trivia(source, end))
            .is_some_and(|(next, _)| primitive_condition(next)),
        "csname" => constructed_csname(source, start).is_some(),
        "ifx" | "ifdefined" | "ifprimitive" | "if" | "ifcat" | "ifcsname" | "ifnum" | "ifdim"
        | "ifabsnum" | "ifabsdim" | "ifpdfabsnum" | "ifpdfabsdim" | "ifodd" => {
            primitive_test_end(source, name, end).is_some()
        }
        "ifcase" | "ifvoid" | "ifhbox" | "ifvbox" | "ifeof" => {
            literal_integer(source, end).is_some()
        }
        _ => true,
    }
}

fn declaration_protection_validation_needed(source: &str, config: &Config) -> bool {
    config.syntax.special.iter().any(|rule| rule.verbatim)
        || source.match_indices('\\').any(|(start, _)| {
            command_at(source, start).is_some_and(|(name, _)| {
                let base = name.trim_end_matches('*');
                matches!(base, "begin" | "else" | "or")
                    || config
                        .protect
                        .commands
                        .iter()
                        .any(|command| command == base || command == name)
                    || config
                        .syntax
                        .commands
                        .get(base)
                        .is_some_and(|rule| rule.verbatim)
                    || config.protect.dynamic_catcodes
                        && matches!(base, "catcode" | "obeyspaces" | "obeylines" | "scantokens")
                    || config.protect.expl3 && base == "ExplSyntaxOn"
            })
        })
}

fn context_declaration_command(command: &str) -> bool {
    let command = command.trim_end_matches('*');
    latex_macro_definition(command)
        || matches!(
            command,
            "def"
                | "gdef"
                | "edef"
                | "xdef"
                | "let"
                | "futurelet"
                | "newtheorem"
                | "newlist"
                | "newtcolorbox"
                | "renewtcolorbox"
                | "NewTColorBox"
                | "RenewTColorBox"
                | "DeclareTColorBox"
                | "ProvideTColorBox"
                | "newminted"
                | "DefineVerbatimEnvironment"
                | "CustomVerbatimEnvironment"
                | "RecustomVerbatimEnvironment"
                | "lstnewenvironment"
                | "newtcblisting"
                | "renewtcblisting"
                | "NewTCBListing"
                | "RenewTCBListing"
                | "DeclareTCBListing"
                | "ProvideTCBListing"
                | "excludecomment"
                | "includecomment"
                | "specialcomment"
                | "processcomment"
        )
}

fn has_context_declaration(source: &str) -> bool {
    source.match_indices('\\').any(|(start, _)| {
        command_at(source, start).is_some_and(|(command, _)| context_declaration_command(command))
    })
}

pub(crate) fn has_analysis_declaration_tokens(source: &str) -> bool {
    source.match_indices('\\').any(|(start, _)| {
        command_at(source, start).is_some_and(|(command, _)| {
            context_declaration_command(command)
                || matches!(
                    command.trim_end_matches('*'),
                    "newcolumntype"
                        | "renewcolumntype"
                        | "NewEnviron"
                        | "RenewEnviron"
                        | "ProvideEnviron"
                        | "DeclareEnviron"
                )
        })
    })
}

pub(crate) fn declared_environment_names(
    source: &str,
    config: &Config,
) -> Result<HashSet<String>, FormatError> {
    if !["environment", "DocumentEnvironment", "newtheorem"]
        .iter()
        .any(|name| source.contains(name))
    {
        return Ok(HashSet::new());
    }
    Ok(declared_environment_names_with_definitions(
        &macro_definitions(source, config)?,
    ))
}

pub(crate) fn declared_environment_names_with_definitions(
    definitions: &[MacroDefinition],
) -> HashSet<String> {
    definitions
        .iter()
        .filter(|definition| {
            !(definition.name == "ProvideDocumentEnvironment"
                && definition
                    .environment
                    .as_deref()
                    .is_some_and(|name| crate::packages::scoped_header_signature(name).is_some()))
        })
        .filter_map(|definition| definition.environment.clone())
        .collect()
}

pub(crate) fn declared_command_names(
    source: &str,
    config: &Config,
) -> Result<HashSet<String>, FormatError> {
    let definitions = macro_definitions(source, config)?;
    let protected = protected_ranges(source, config)?;
    Ok(declared_command_names_with_ranges(
        source,
        &protected,
        &definitions,
    ))
}

pub(crate) fn declared_command_names_with_ranges(
    source: &str,
    protected: &[Range<usize>],
    definitions: &[MacroDefinition],
) -> HashSet<String> {
    let mut names: HashSet<_> = definitions
        .iter()
        .filter(|definition| {
            !(matches!(
                definition.name.as_str(),
                "providecommand" | "ProvideDocumentCommand"
            ) && definition.command.as_deref().is_some_and(|name| {
                crate::packages::scoped_environment_for_command(name).is_some()
            }))
        })
        .filter_map(|definition| definition.command.clone())
        .collect();
    // A replacement body may redefine a command when its wrapper is called.
    // Conservatively record literal targets without executing the wrapper.
    for body in definitions.iter().flat_map(|definition| &definition.bodies) {
        let mut cursor = body.start;
        while cursor < body.end {
            if source.as_bytes()[cursor] == b'%' {
                cursor = line_end(source, cursor).min(body.end);
                continue;
            }
            if let Some((name, end)) = command_at(source, cursor) {
                if matches!(name, "def" | "gdef" | "edef" | "xdef" | "let" | "futurelet")
                    || latex_macro_definition(name)
                        && !name.contains("environment")
                        && !name.contains("Environment")
                {
                    let mut target = skip_trivia(source, end);
                    if source.as_bytes().get(target) == Some(&b'{') {
                        target = skip_trivia(source, target + 1);
                    }
                    if target < body.end
                        && let Some((target, finish)) = command_at(source, target)
                        && finish <= body.end
                    {
                        names.insert(target.into());
                    }
                }
                cursor = end;
            } else {
                let next = cursor
                    + source[cursor..]
                        .chars()
                        .next()
                        .expect("valid position")
                        .len_utf8();
                cursor = next_declaration_token(source, next, body.end);
            }
        }
    }
    let mut pos = 0;
    while pos < source.len() {
        let index = protected.partition_point(|range| range.end <= pos);
        let range = protected.get(index).filter(|range| range.start <= pos);
        // Literal primitive declarations may themselves be protected. Read
        // their target at the range start, then skip their replacement body.
        if range.is_none_or(|range| range.start == pos)
            && let Some((name, end)) = command_at(source, pos)
        {
            if matches!(name, "def" | "gdef" | "edef" | "xdef" | "let" | "futurelet")
                && let Some((target, _)) = command_at(source, skip_trivia(source, end))
            {
                names.insert(target.into());
            }
            if let Some(range) = range {
                pos = range.end;
            } else {
                pos = end;
            }
            continue;
        }
        if let Some(range) = range {
            pos = range.end;
        } else if source.as_bytes()[pos] == b'%' {
            pos = line_end(source, pos);
        } else {
            let next = pos
                + source[pos..]
                    .chars()
                    .next()
                    .expect("valid position")
                    .len_utf8();
            let limit = protected
                .get(index)
                .map_or(source.len(), |range| range.start);
            pos = next_declaration_token(source, next, limit);
        }
    }
    names
}

/// Infer literal list wrappers, including transitive wrappers and enumitem lists.
/// The returned names exclude those already present in `known`.
pub fn infer_list_environments(source: &str, known: &[String]) -> Vec<String> {
    infer_list_environments_with_config(source, known, &Config::default())
}

pub fn infer_list_environments_with_config(
    source: &str,
    known: &[String],
    config: &Config,
) -> Vec<String> {
    if ![
        "newenvironment",
        "renewenvironment",
        "DocumentEnvironment",
        "newlist",
    ]
    .iter()
    .any(|name| source.contains(name))
    {
        return Vec::new();
    }
    let Ok(definitions) = macro_definitions(source, config) else {
        return Vec::new();
    };
    let Ok(compiled) = compile(&config.syntax) else {
        return Vec::new();
    };
    infer_list_environments_with_definitions(source, known, config, &definitions, &compiled)
}

pub(crate) fn infer_list_environments_with_definitions(
    source: &str,
    known: &[String],
    config: &Config,
    definitions: &[MacroDefinition],
    compiled: &CompiledSyntax,
) -> Vec<String> {
    let mut declarations: Vec<(usize, String, Vec<String>)> = Vec::new();
    for definition in definitions {
        let Some(name) = definition.environment.clone() else {
            continue;
        };
        if definition.bodies.len() < 2 {
            continue;
        }
        let first = &source[definition.bodies[0].clone()];
        let body = format!("{first}\n{}", &source[definition.bodies[1].clone()]);
        let boundary = first.len() + 1;
        let Ok(tokens) = lex(&body, config, compiled) else {
            continue;
        };
        let mut bases = Vec::new();
        let mut depth = 0usize;
        let mut environments: Vec<(String, usize, usize)> = Vec::new();
        let mut direct: Vec<(String, usize)> = Vec::new();
        let mut valid = true;
        for token in tokens {
            match token.kind {
                TokenKind::OpenBrace | TokenKind::OpenBracket => depth += 1,
                TokenKind::CloseBrace | TokenKind::CloseBracket => depth = depth.saturating_sub(1),
                TokenKind::BeginEnvironment => {
                    if let Some((base, _, _)) = env_at(&body, token.span.start) {
                        environments.push((base.to_owned(), token.span.start, depth));
                    }
                }
                TokenKind::EndEnvironment => {
                    let Some((base, _, _)) = env_at(&body, token.span.start) else {
                        continue;
                    };
                    let Some((open, start, group)) = environments.pop() else {
                        valid = false;
                        break;
                    };
                    if open != base {
                        valid = false;
                        break;
                    }
                    if group == 0 && start < boundary && token.span.start >= boundary {
                        bases.push(open);
                    }
                }
                TokenKind::Command if depth == 0 => {
                    if let Some((base, _)) = command_at(&body, token.span.start) {
                        if matches!(base, "list" | "trivlist") {
                            direct.push((base.into(), token.span.start));
                        } else if matches!(base, "endlist" | "endtrivlist")
                            && let Some((open, start)) = direct.pop()
                            && base.strip_prefix("end") == Some(open.as_str())
                            && start < boundary
                            && token.span.start >= boundary
                        {
                            bases.push(open);
                        }
                    }
                }
                _ => (),
            }
        }
        if !valid || !environments.is_empty() || !direct.is_empty() {
            bases.clear();
        }
        declarations.push((definition.start, name, bases));
    }
    // Read literal enumitem declarations outside comments and protected regions.
    if source.contains("\\newlist")
        && let Ok(tokens) = lex(source, config, compiled)
    {
        for token in tokens {
            if token.kind != TokenKind::Command {
                continue;
            }
            let Some(("newlist", end)) = command_at(source, token.span.start) else {
                continue;
            };
            let name_start = skip_trivia(source, end);
            let Some(name_end) = balanced_end(source, name_start, b'{', b'}') else {
                continue;
            };
            let base_start = skip_trivia(source, name_end);
            let Some(base_end) = balanced_end(source, base_start, b'{', b'}') else {
                continue;
            };
            declarations.push((
                token.span.start,
                source[name_start + 1..name_end - 1].trim().to_owned(),
                vec![source[base_start + 1..base_end - 1].trim().to_owned()],
            ));
        }
    }
    declarations.sort_by_key(|(start, _, _)| *start);
    let mut dependencies = std::collections::BTreeMap::new();
    for (_, name, bases) in declarations {
        dependencies.insert(name, bases);
    }
    let original: HashSet<String> = known.iter().cloned().collect();
    let mut inferred = original.clone();
    inferred.insert("trivlist".into());
    loop {
        let mut changed = false;
        for (name, bases) in &dependencies {
            if !name.is_empty() && bases.iter().any(|base| inferred.contains(base)) {
                changed |= inferred.insert(name.clone());
            }
        }
        if !changed {
            break;
        }
    }
    let mut names: Vec<_> = dependencies
        .into_keys()
        .filter(|name| inferred.contains(name) && !original.contains(name))
        .collect();
    names.sort();
    names.dedup();
    names
}

/// Adjust leading indentation in structurally safe LaTeX definitions. Their
/// bodies remain protected from the ordinary alignment, wrapping and replacement
/// pipeline; ambiguous definitions retain their original source.
pub(crate) fn macro_layout_active(source: &str, config: &crate::CompiledConfig) -> bool {
    config.config.protect.macro_bodies
        && config.config.protect.format_macro_bodies
        && [
            "\\def",
            "\\gdef",
            "\\edef",
            "\\xdef",
            "\\newcommand",
            "\\renewcommand",
            "\\providecommand",
            "\\newenvironment",
            "\\renewenvironment",
            "\\NewDocumentCommand",
            "\\RenewDocumentCommand",
            "\\ProvideDocumentCommand",
            "\\DeclareDocumentCommand",
            "\\NewDocumentEnvironment",
            "\\RenewDocumentEnvironment",
            "\\ProvideDocumentEnvironment",
            "\\DeclareDocumentEnvironment",
            "\\DeclareRobustCommand",
            "\\DeclareMathOperator",
        ]
        .iter()
        .any(|name| {
            source.match_indices(name).any(|(start, _)| {
                command_at(source, start)
                    .is_some_and(|(command, _)| command.trim_end_matches('*') == &name[1..])
            })
        })
}

pub fn format_macro_definitions(
    source: &str,
    config: &crate::CompiledConfig,
) -> Result<String, FormatError> {
    if !macro_layout_active(source, config) {
        return Ok(source.into());
    }
    let definitions = macro_definitions(source, &config.config)?;
    if definitions.is_empty() {
        return Ok(source.into());
    }
    let mut options = config.config.clone();
    options.protect.format_macro_bodies = false;
    options.syntax.infer_list_environments = false;
    options.replacements.clear();
    options.wrap.mode = "preserve".into();
    options.sentences.enabled = false;
    options.alignment.enabled = false;
    options.breaks.enabled = false;
    options.whitespace.remove_trailing = false;
    options.whitespace.before_processing = false;
    options.whitespace.preserve_blank_lines = true;
    options.whitespace.max_blank_lines = None;
    options.whitespace.line_ending = crate::config::LineEnding::Preserve;
    options.whitespace.final_newline = crate::config::FinalNewline::Preserve;
    options.indent.preamble = true;
    let inner = crate::CompiledConfig::new(options)
        .map_err(|error| FormatError::Syntax(error.to_string()))?;
    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    for definition in definitions {
        if !definition
            .bodies
            .iter()
            .any(|body| source[body.clone()].contains('\n'))
        {
            continue;
        }
        let wrapper = if matches!(definition.name.as_str(), "def" | "gdef" | "edef" | "xdef") {
            "tiftdefinition"
        } else {
            &definition.name
        };
        let mut synthetic = format!("\\{wrapper}{{\n");
        let mut markers = Vec::new();
        for (index, body) in definition.bodies.iter().enumerate() {
            let begin = format!("%\u{1}tift-macro-body-{index}-begin");
            let end = format!("%\u{1}tift-macro-body-{index}-end");
            synthetic.push_str(&begin);
            synthetic.push('\n');
            let start = synthetic.len();
            synthetic.push_str(&source[body.clone()]);
            let finish = synthetic.len();
            synthetic.push('\n');
            synthetic.push_str(&end);
            synthetic.push('\n');
            markers.push((begin, end, start..finish));
        }
        synthetic.push_str("}\n");
        let Ok(protected) = protected_ranges(&synthetic, &inner.config) else {
            continue;
        };
        let Ok((tokens, blocks)) = shared_blocks_with_ranges(&synthetic, &inner, &protected) else {
            continue;
        };
        let risky = tokens
            .iter()
            .filter(|token| token.kind == TokenKind::Command)
            .any(|token| {
                command_at(&synthetic, token.span.start).is_some_and(|(name, _)| {
                    matches!(
                        name,
                        "def"
                            | "gdef"
                            | "edef"
                            | "xdef"
                            | "let"
                            | "futurelet"
                            | "expandafter"
                            | "afterassignment"
                            | "aftergroup"
                            | "csname"
                            | "endcsname"
                            | "newif"
                            | "bgroup"
                            | "egroup"
                            | "begingroup"
                            | "endgroup"
                            | "newcolumntype"
                    ) || latex_macro_definition(name) && token.span.start != 0
                })
            })
            || protected.iter().any(|range| {
                command_at(&synthetic, range.start).is_some_and(|(name, _)| {
                    matches!(
                        name,
                        "catcode" | "obeyspaces" | "obeylines" | "scantokens" | "ExplSyntaxOn"
                    )
                })
            })
            || blocks
                .iter()
                .filter(|block| block.kind == BlockKind::Condition)
                .any(|block| {
                    markers.iter().any(|(_, _, body)| {
                        block.begin.start < body.end && block.end.end > body.end
                    })
                });
        if risky {
            continue;
        }
        let Ok(formatted) = crate::format(&synthetic, crate::FileKind::Tex, &inner) else {
            continue;
        };
        let declaration_line = &source[line_start(source, definition.start)..definition.start];
        let base = &declaration_line
            [..declaration_line.len() - declaration_line.trim_start_matches([' ', '\t']).len()];
        for (body_index, (body, (begin_marker, end_marker, synthetic_body))) in
            definition.bodies.iter().zip(markers).enumerate()
        {
            let Some(begin_at) = formatted
                .text
                .find(&begin_marker)
                .map(|pos| pos + begin_marker.len() + 1)
            else {
                continue;
            };
            let Some(end_at) = formatted
                .text
                .find(&end_marker)
                .map(|pos| line_start(&formatted.text, pos).saturating_sub(1))
            else {
                continue;
            };
            if begin_at > end_at {
                continue;
            }
            let original = &source[body.clone()];
            let replacement = &formatted.text[begin_at..end_at];
            let old_lines: Vec<_> = original.split_inclusive('\n').collect();
            let mut new_lines: Vec<_> = replacement.split_inclusive('\n').collect();
            if old_lines.last().is_some_and(|line| {
                !line.ends_with('\n') && line.trim_matches([' ', '\t']).is_empty()
            }) && new_lines.len() + 1 == old_lines.len()
            {
                new_lines.push("");
            }
            if old_lines.len() != new_lines.len() {
                continue;
            }
            let mut text = String::with_capacity(replacement.len() + base.len() * new_lines.len());
            let mut offset = synthetic_body.start;
            for (index, (&old, &new)) in old_lines.iter().zip(&new_lines).enumerate() {
                let inline_start = index == 0 && !original.starts_with('\n');
                let raw = overlaps(&(offset..offset + old.len()), &protected);
                if inline_start || raw {
                    text.push_str(old);
                } else {
                    let content = new.trim_start_matches([' ', '\t']);
                    if content.trim_matches(['\r', '\n']).is_empty() {
                        if index + 1 == old_lines.len() && !old.ends_with('\n') {
                            text.push_str(base);
                        } else {
                            text.push_str(content);
                        }
                    } else {
                        text.push_str(base);
                        text.push_str(new);
                    }
                }
                offset += old.len();
            }
            if body_index > 0 {
                // The protected definition includes the gap between its bodies.
                // Reindent a standalone opening brace only after validating the
                // definition; the body and any gap comments remain untouched.
                let opening = body.start - 1;
                let prefix = line_start(source, opening)..opening;
                if source[prefix.clone()].trim_matches([' ', '\t']).is_empty()
                    && source[prefix.clone()] != *base
                {
                    edits.push((prefix, base.to_owned()));
                }
            }
            edits.push((body.clone(), text));
        }
    }
    let mut output = source.to_owned();
    for (range, text) in edits.into_iter().rev() {
        output.replace_range(range, &text);
    }
    Ok(output)
}

fn protected_special_end(
    pos: usize,
    config: &Config,
    matches: &[SpecialMatches],
) -> Result<Option<usize>, FormatError> {
    for (index, rule) in config
        .syntax
        .special
        .iter()
        .enumerate()
        .filter(|(_, r)| r.verbatim)
    {
        if let Some(begin_end) = at_match(&matches[index].begin, pos) {
            let mut cursor = begin_end;
            let mut depth = 1usize;
            loop {
                let end = next_match(&matches[index].end, cursor).ok_or_else(|| {
                    FormatError::Syntax(format!("unclosed protected special block {:?}", rule.name))
                })?;
                let begin = if rule.nested {
                    next_match(&matches[index].begin, cursor)
                        .filter(|begin| begin.start < end.start)
                } else {
                    None
                };
                if let Some(begin) = begin {
                    depth += 1;
                    cursor = begin.end;
                } else {
                    depth -= 1;
                    cursor = end.end;
                    if depth == 0 {
                        break;
                    }
                }
            }
            return Ok(Some(cursor));
        }
    }
    Ok(None)
}

fn inferred_protected_environments(
    source: &str,
    config: &Config,
    protected: &[Range<usize>],
) -> Vec<String> {
    let definitions = macro_definitions_with_ranges(source, protected);
    let environments: HashSet<_> = definitions
        .iter()
        .filter_map(|definition| definition.environment.as_deref())
        .collect();
    if environments.is_empty() {
        return Vec::new();
    }
    let mut body_config = config.clone();
    // Inspect syntax-changing commands themselves without treating their
    // definition bodies as a dynamic document suffix. Literal text and nested
    // definitions retain the usual protection.
    body_config.protect.dynamic_catcodes = false;
    body_config.protect.expl3 = false;
    let mut dependencies: HashMap<String, HashSet<String>> = HashMap::new();
    let mut dangerous = HashSet::new();
    for name in &config.protect.environments {
        dangerous.insert(format!("environment:{name}"));
        dangerous.insert(format!("command:{name}"));
        dangerous.insert(format!("command:end{name}"));
    }
    for name in &config.protect.commands {
        dangerous.insert(format!("command:{}", name.trim_end_matches('*')));
    }
    for definition in &definitions {
        let target = if let Some(name) = &definition.environment {
            format!("environment:{name}")
        } else if let Some(name) = &definition.command {
            format!("command:{name}")
        } else {
            continue;
        };
        let mut body = String::new();
        for range in &definition.bodies {
            body.push_str(&source[range.clone()]);
            body.push('\n');
        }
        let literal = protected_ranges_basic(&body, &body_config).unwrap_or_default();
        let bases = dependencies.entry(target.clone()).or_default();
        let mut pos = 0usize;
        while pos < body.len() {
            let index = literal.partition_point(|range| range.end <= pos);
            if let Some(range) = literal.get(index).filter(|range| range.start <= pos) {
                if let Some((name, begin, _)) = env_at(&body, range.start) {
                    if begin {
                        bases.insert(format!("environment:{name}"));
                    }
                } else if let Some((name, _)) = command_at(&body, range.start) {
                    bases.insert(format!("command:{}", name.trim_end_matches('*')));
                }
                pos = range.end;
                continue;
            }
            match body.as_bytes()[pos] {
                b'%' => pos = line_end(&body, pos),
                b'\\' => {
                    let Some((name, end)) = command_at(&body, pos) else {
                        break;
                    };
                    let name = name.trim_end_matches('*');
                    if config.protect.dynamic_catcodes
                        && matches!(name, "catcode" | "obeyspaces" | "obeylines" | "scantokens")
                        || config.protect.expl3 && name == "ExplSyntaxOn"
                    {
                        dangerous.insert(target.clone());
                    }
                    if let Some((name, begin, finish)) = env_at(&body, pos) {
                        if begin {
                            bases.insert(format!("environment:{name}"));
                        }
                        pos = finish;
                        continue;
                    }
                    bases.insert(format!("command:{name}"));
                    if environments.contains(name) {
                        bases.insert(format!("environment:{name}"));
                    }
                    if let Some(name) = name.strip_prefix("end")
                        && environments.contains(name)
                    {
                        bases.insert(format!("environment:{name}"));
                    }
                    pos = end;
                }
                _ => {
                    pos += body[pos..]
                        .chars()
                        .next()
                        .expect("valid position")
                        .len_utf8()
                }
            }
        }
    }
    loop {
        let mut changed = false;
        for (name, bases) in &dependencies {
            if bases.iter().any(|base| dangerous.contains(base)) {
                changed |= dangerous.insert(name.clone());
            }
        }
        if !changed {
            break;
        }
    }
    let mut names: Vec<_> = environments
        .into_iter()
        .filter(|name| dangerous.contains(&format!("environment:{name}")))
        .map(str::to_owned)
        .collect();
    names.sort();
    names
}

/// Source regions whose original bytes must survive all formatting passes.
pub fn protected_ranges(source: &str, config: &Config) -> Result<Vec<Range<usize>>, FormatError> {
    protected_ranges_impl(source, config, None)
}

#[doc(hidden)]
pub fn protected_ranges_with_syntax(
    source: &str,
    config: &Config,
    compiled: &CompiledSyntax,
) -> Result<Vec<Range<usize>>, FormatError> {
    protected_ranges_impl(source, config, Some(compiled))
}

fn protected_ranges_impl(
    source: &str,
    config: &Config,
    compiled: Option<&CompiledSyntax>,
) -> Result<Vec<Range<usize>>, FormatError> {
    let owned;
    let compiled = if let Some(compiled) = compiled {
        Some(compiled)
    } else if memchr::memchr2(b'\\', b'%', source.as_bytes()).is_some()
        || config.syntax.special.iter().any(|rule| rule.verbatim)
    {
        owned = compile(&config.syntax).map_err(FormatError::Syntax)?;
        Some(&owned)
    } else {
        None
    };
    let mut ranges =
        protected_ranges_with_inferred_environments(source, config, &[], None, compiled)?;
    if !["environment", "DocumentEnvironment"]
        .iter()
        .any(|name| source.contains(name))
    {
        add_conditional_environment_protection(source, config, &mut ranges, compiled)?;
        return Ok(ranges);
    }
    let names = inferred_protected_environments(source, config, &ranges);
    if names.is_empty() {
        add_conditional_environment_protection(source, config, &mut ranges, compiled)?;
        return Ok(ranges);
    }
    // Rescan outside definition text, keeping begin, body and end intact.
    ranges = protected_ranges_with_inferred_environments(source, config, &names, None, compiled)?;
    add_conditional_environment_protection(source, config, &mut ranges, compiled)?;
    Ok(ranges)
}

fn protected_ranges_basic(source: &str, config: &Config) -> Result<Vec<Range<usize>>, FormatError> {
    protected_ranges_with_inferred_environments(source, config, &[], None, None)
}

pub(crate) fn visual_header_bindings(
    source: &str,
    config: &Config,
    protected: &[Range<usize>],
) -> Result<HashMap<usize, bool>, FormatError> {
    let names = inferred_protected_environments(source, config, protected);
    let mut bindings = HashMap::new();
    protected_ranges_with_inferred_environments(source, config, &names, Some(&mut bindings), None)?;
    Ok(bindings)
}

fn inferred_condition(name: &str, source: &str, end: usize) -> bool {
    primitive_condition(name)
        || name.starts_with("if")
            && ![
                "iff",
                "ifthenelse",
                "ifpatchable",
                "iftoggle",
                "ifboolexpr",
                "ifstrequal",
                "ifdef",
                "ifblank",
                "ifstrempty",
                "ifcsdef",
                "ifnumcomp",
                "ifdimcomp",
            ]
            .contains(&name)
            && !source[end..].trim_start().starts_with('{')
}

// A conditional can choose an environment which is closed in a later
// conditional. Track branch outcomes until the possible stacks agree again,
// then preserve that bounded interval. Ordinary balanced branches remain
// visible to the formatter, and errors outside an interval remain strict.
fn add_conditional_environment_protection(
    source: &str,
    config: &Config,
    ranges: &mut Vec<Range<usize>>,
    compiled: Option<&CompiledSyntax>,
) -> Result<(), FormatError> {
    if !source.contains("\\csname")
        && !source
            .match_indices("\\else")
            .chain(source.match_indices("\\or"))
            .any(|(start, _)| {
                !is_escaped(source, start)
                    && !is_protected(start, ranges)
                    && command_at(source, start)
                        .is_some_and(|(name, _)| matches!(name, "else" | "or"))
            })
    {
        return Ok(());
    }
    let owned;
    let compiled = if let Some(compiled) = compiled {
        compiled
    } else {
        owned = compile(&config.syntax).map_err(FormatError::Syntax)?;
        &owned
    };
    let constructed_commands = constant_constructed_commands(source, config, ranges);
    let tokens =
        lex_with_constructed_commands(source, config, compiled, ranges, &constructed_commands)?;
    struct Branch {
        start: usize,
        name: String,
        entered: Vec<Vec<String>>,
        outcomes: Vec<Vec<String>>,
        has_alternative: bool,
        has_else: bool,
    }
    let mut states = vec![Vec::<String>::new()];
    let mut branches: Vec<Branch> = Vec::new();
    let mut ambiguous = None;
    let mut additions = Vec::new();
    let mut known: HashSet<String> = config.syntax.conditions.iter().cloned().collect();
    let mut new_if = false;
    for token in &tokens {
        match token.kind {
            TokenKind::BeginEnvironment => {
                let (name, _, _) = env_at(source, token.span.start).expect("environment token");
                for state in &mut states {
                    state.push(name.into());
                }
            }
            TokenKind::EndEnvironment => {
                let (name, _, _) = env_at(source, token.span.start).expect("environment token");
                let matching = states
                    .iter()
                    .filter(|s| s.last().is_some_and(|n| n == name))
                    .count();
                if matching == 0 || matching != states.len() && branches.is_empty() {
                    break;
                }
                states.retain(|state| state.last().is_some_and(|n| n == name));
                for state in &mut states {
                    state.pop();
                }
            }
            TokenKind::Command => {
                let cached_constructed = constructed_commands.get(&token.span.start);
                let literal_constructed = cached_constructed
                    .is_none()
                    .then(|| literal_csname(source, token.span.start))
                    .flatten();
                let constructed = cached_constructed.or(literal_constructed.as_ref());
                let name = constructed.map_or_else(
                    || {
                        command_at(source, token.span.start)
                            .expect("command token")
                            .0
                    },
                    |(name, _)| name.as_str(),
                );
                let base = name.trim_end_matches('*');
                if new_if {
                    known.insert(base.into());
                    new_if = false;
                } else if base == "newif" {
                    new_if = true;
                } else if base == "else" || base == "or" {
                    let Some(branch) = branches.last_mut() else {
                        break;
                    };
                    if branch.has_else
                        || base == "or"
                            && primitive_condition(&branch.name)
                            && branch.name != "ifcase"
                    {
                        break;
                    }
                    branch.outcomes.append(&mut states);
                    states = branch.entered.clone();
                    branch.has_alternative = true;
                    branch.has_else = base == "else";
                } else if base == "fi" {
                    let Some(mut branch) = branches.pop() else {
                        break;
                    };
                    branch.outcomes.append(&mut states);
                    if !branch.has_alternative {
                        branch.outcomes.append(&mut branch.entered);
                    }
                    branch.outcomes.sort();
                    branch.outcomes.dedup();
                    states = branch.outcomes;
                    if states.len() > 32 {
                        break;
                    }
                    if states.len() > 1 {
                        ambiguous.get_or_insert(
                            branches.first().map_or(branch.start, |outer| outer.start),
                        );
                    } else if ambiguous
                        .is_some_and(|start| !branches.iter().any(|outer| outer.start >= start))
                    {
                        let start = ambiguous.take().expect("ambiguous environment interval");
                        let end = token.span.end;
                        let mut groups = Vec::new();
                        let mut math = Vec::new();
                        let mut specials = Vec::new();
                        let mut balanced = true;
                        for candidate in tokens
                            .iter()
                            .filter(|t| start <= t.span.start && t.span.end <= end)
                        {
                            match candidate.kind {
                                TokenKind::OpenBrace => groups.push(false),
                                TokenKind::CloseBrace => {
                                    balanced &= groups.pop() == Some(false);
                                }
                                TokenKind::Command => {
                                    match command_at(source, candidate.span.start)
                                        .expect("command token")
                                        .0
                                    {
                                        "bgroup" => groups.push(false),
                                        "begingroup" => groups.push(true),
                                        "egroup" => balanced &= groups.pop() == Some(false),
                                        "endgroup" => balanced &= groups.pop() == Some(true),
                                        "[" => math.push("displayMath"),
                                        "(" => math.push("inlineMath"),
                                        "]" => balanced &= math.pop() == Some("displayMath"),
                                        ")" => balanced &= math.pop() == Some("inlineMath"),
                                        _ => (),
                                    }
                                }
                                TokenKind::Math => {
                                    let delimiter =
                                        if candidate.span.len() == 2 { "$$" } else { "$" };
                                    if math.last() == Some(&delimiter) {
                                        math.pop();
                                    } else {
                                        math.push(delimiter);
                                    }
                                }
                                TokenKind::SpecialBegin(rule) => specials.push(rule),
                                TokenKind::SpecialEnd(rule) => {
                                    balanced &= specials.pop() == Some(rule)
                                }
                                _ => (),
                            }
                        }
                        if balanced && groups.is_empty() && math.is_empty() && specials.is_empty() {
                            additions.push(start..end);
                        }
                    }
                } else if known.contains(base)
                    || inferred_condition(base, source, token.span.end)
                    || compiled
                        .condition
                        .as_ref()
                        .is_some_and(|regex| regex.is_match(base).unwrap_or(false))
                {
                    branches.push(Branch {
                        start: token.span.start,
                        name: base.into(),
                        entered: states.clone(),
                        outcomes: Vec::new(),
                        has_alternative: false,
                        has_else: false,
                    });
                }
            }
            _ => (),
        }
    }
    ranges.extend(additions);
    ranges.sort_by_key(|range| range.start);
    let mut merged: Vec<Range<usize>> = Vec::new();
    for range in ranges.drain(..) {
        if let Some(last) = merged.last_mut().filter(|last| range.start <= last.end) {
            last.end = last.end.max(range.end);
        } else {
            merged.push(range);
        }
    }
    *ranges = merged;
    Ok(())
}

fn inferred_environment_end(
    source: &str,
    mut pos: usize,
    environment: &str,
    config: &Config,
) -> usize {
    let mut depth = 1usize;
    while pos < source.len() {
        match source.as_bytes()[pos] {
            b'%' => pos = line_end(source, pos),
            b'\\' => {
                let Some((command, end)) = command_at(source, pos) else {
                    break;
                };
                if let Some((name, begin, finish)) = env_at(source, pos) {
                    if name == environment {
                        if begin {
                            depth += 1;
                        } else {
                            depth -= 1;
                            if depth == 0 {
                                return finish;
                            }
                        }
                    }
                    pos = finish;
                } else if config
                    .protect
                    .commands
                    .iter()
                    .any(|name| name == command || name == command.trim_end_matches('*'))
                    || config
                        .syntax
                        .commands
                        .get(command.trim_end_matches('*'))
                        .is_some_and(|rule| rule.verbatim)
                {
                    pos = inline_end(source, command, end).unwrap_or(source.len());
                } else if command == "string" {
                    let next = skip_space(source, end);
                    pos = command_at(source, next).map_or_else(
                        || {
                            source[next..]
                                .chars()
                                .next()
                                .map_or(source.len(), |token| next + token.len_utf8())
                        },
                        |(_, finish)| finish,
                    );
                } else {
                    pos = end;
                }
            }
            _ => {
                pos += source[pos..]
                    .chars()
                    .next()
                    .expect("valid position")
                    .len_utf8()
            }
        }
    }
    // Changed catcodes can make normal comment or delimiter scanning
    // ambiguous. Preserve the remaining source when an end cannot be located.
    source.len()
}

fn default_catcode(character: u32) -> u8 {
    match char::from_u32(character) {
        Some('\\') => 0,
        Some('{') => 1,
        Some('}') => 2,
        Some('$') => 3,
        Some('&') => 4,
        Some('\n' | '\r') => 5,
        Some('#') => 6,
        Some('^') => 7,
        Some('_') => 8,
        Some('\0') => 9,
        Some(' ' | '\t') => 10,
        Some('~') => 13,
        Some('%') => 14,
        Some('\u{7f}') => 15,
        Some(c) if c.is_ascii_alphabetic() => 11,
        _ => 12,
    }
}

fn literal_catcode_assignment(source: &str, end: usize) -> Option<(u32, u8, usize)> {
    let (character, mut cursor) = literal_integer(source, end)?;
    let character = u32::try_from(character)
        .ok()
        .filter(|&c| char::from_u32(c).is_some())?;
    cursor = skip_trivia(source, cursor);
    if source.as_bytes().get(cursor) == Some(&b'=') {
        cursor += 1;
    }
    let (code, mut finish) = literal_integer(source, cursor)?;
    let code = u8::try_from(code).ok().filter(|&c| c <= 15)?;
    if source.as_bytes().get(finish) == Some(&b'\\') && command_at(source, finish)?.0 != "relax" {
        return None;
    }
    if source
        .as_bytes()
        .get(finish)
        .is_some_and(u8::is_ascii_whitespace)
    {
        finish += 1;
    }
    Some((character, code, finish))
}

// Keep only the interval with altered tokenization. Explicit assignments are
// tracked individually; a local group restores its incoming catcodes. If the
// escape/group delimiters change or an assignment is dynamic, retain the suffix.
fn dynamic_catcode_end(
    source: &str,
    name: &str,
    end: usize,
    scope: Option<&str>,
    global: bool,
) -> usize {
    let mut changed = HashMap::new();
    let mut cursor = end;
    match name {
        "catcode" => {
            let Some((character, code, finish)) = literal_catcode_assignment(source, end) else {
                return source.len();
            };
            cursor = finish;
            if code == default_catcode(character) {
                return finish;
            }
            if character == '\\' as u32
                || character == '{' as u32
                || character == '}' as u32
                || char::from_u32(character).is_some_and(|c| c.is_ascii_alphabetic())
            {
                return source.len();
            }
            changed.insert(character, code);
        }
        "obeyspaces" => {
            changed.insert(' ' as u32, 13);
        }
        "obeylines" => {
            changed.insert('\n' as u32, 13);
            changed.insert('\r' as u32, 13);
        }
        "ExplSyntaxOn" => {
            changed.insert('_' as u32, 11);
            changed.insert(':' as u32, 11);
            changed.insert(' ' as u32, 9);
            changed.insert('\n' as u32, 9);
        }
        _ => return source.len(),
    }
    let mut nested = Vec::<(String, HashMap<u32, u8>)>::new();
    let mut can_restore_scope = !global;
    let mut pending_global = false;
    while cursor < source.len() {
        let c = source[cursor..].chars().next().expect("catcode token");
        let code = changed
            .get(&(c as u32))
            .copied()
            .unwrap_or_else(|| default_catcode(c as u32));
        if code == 14 {
            cursor = line_end(source, cursor);
            continue;
        }
        if c == '\\' && code == 0 {
            let Some((command, finish)) = command_at(source, cursor) else {
                return source.len();
            };
            if command == "global" {
                pending_global = true;
                cursor = finish;
                continue;
            }
            if matches!(command, "long" | "outer" | "protected") {
                cursor = finish;
                continue;
            }
            if command == "catcode" {
                let Some((character, value, next)) = literal_catcode_assignment(source, finish)
                else {
                    return source.len();
                };
                if pending_global {
                    can_restore_scope = false;
                    for (_, snapshot) in &mut nested {
                        if value == default_catcode(character) {
                            snapshot.remove(&character);
                        } else {
                            snapshot.insert(character, value);
                        }
                    }
                }
                pending_global = false;
                if value != default_catcode(character)
                    && (character == '\\' as u32
                        || character == '{' as u32
                        || character == '}' as u32
                        || char::from_u32(character).is_some_and(|c| c.is_ascii_alphabetic()))
                {
                    return source.len();
                }
                if value == default_catcode(character) {
                    changed.remove(&character);
                } else {
                    changed.insert(character, value);
                }
                cursor = next;
                if changed.is_empty() && nested.is_empty() {
                    return cursor;
                }
                continue;
            }
            pending_global = false;
            if name == "ExplSyntaxOn" && command == "ExplSyntaxOff" {
                if nested.is_empty() {
                    return finish;
                }
                for character in ['_', ':', ' ', '\n'] {
                    changed.remove(&(character as u32));
                }
                cursor = finish;
                continue;
            }
            // Branch execution could change which assignments or delimiters
            // take effect; preserve the suffix rather than infer a false restore.
            if primitive_condition(command) || inferred_condition(command, source, finish) {
                return source.len();
            }
            if let Some(next) = primitive_data_end(source, command, finish)
                .or_else(|| let_declaration_end(source, command, finish))
                .or_else(|| primitive_literal_argument_end(source, command, finish))
                .or_else(|| {
                    inline_end(source, command, finish)
                        .filter(|_| matches!(command, "verb" | "verb*"))
                })
                .or_else(|| {
                    macro_body(source, command, finish)
                        .map(|body| body.end)
                        .filter(|_| {
                            latex_macro_definition(command)
                                || matches!(command, "def" | "gdef" | "edef" | "xdef")
                        })
                })
            {
                cursor = next;
                continue;
            }
            if let Some((environment, begin, next)) = env_at(source, cursor) {
                let terminator = format!("\\end{{{environment}}}");
                if begin {
                    nested.push((terminator, changed.clone()));
                } else if nested.last().is_some_and(|(end, _)| end == &terminator) {
                    changed = nested.pop().expect("catcode scope").1;
                } else if nested.is_empty()
                    && can_restore_scope
                    && scope == Some(terminator.as_str())
                {
                    return cursor;
                }
                cursor = next;
                continue;
            }
            if matches!(command, "begingroup" | "bgroup") {
                nested.push((
                    if command == "begingroup" {
                        "\\endgroup"
                    } else {
                        "\\egroup"
                    }
                    .into(),
                    changed.clone(),
                ));
            } else if matches!(command, "endgroup" | "egroup") {
                let terminator = format!("\\{command}");
                if nested.last().is_some_and(|(end, _)| end == &terminator) {
                    changed = nested.pop().expect("catcode scope").1;
                } else if nested.is_empty()
                    && can_restore_scope
                    && scope == Some(terminator.as_str())
                {
                    return cursor;
                }
            }
            cursor = finish;
            continue;
        }
        if c == '{' && code == 1 {
            nested.push(("}".into(), changed.clone()));
        } else if c == '}' && code == 2 {
            if nested.last().is_some_and(|(end, _)| end == "}") {
                changed = nested.pop().expect("catcode scope").1;
            } else if nested.is_empty() && can_restore_scope && scope == Some("}") {
                return cursor;
            }
        }
        cursor += c.len_utf8();
    }
    source.len()
}

fn protected_ranges_with_inferred_environments(
    source: &str,
    config: &Config,
    inferred_environments: &[String],
    mut visual_headers: Option<&mut HashMap<usize, bool>>,
    compiled: Option<&CompiledSyntax>,
) -> Result<Vec<Range<usize>>, FormatError> {
    if memchr::memchr2(b'\\', b'%', source.as_bytes()).is_none()
        && !config.syntax.special.iter().any(|r| r.verbatim)
    {
        return Ok(Vec::new());
    }
    let owned;
    let compiled = if let Some(compiled) = compiled {
        compiled
    } else {
        owned = compile(&config.syntax).map_err(FormatError::Syntax)?;
        &owned
    };
    let special_matches = special_matches(source, config, compiled, true)?;
    let mut ranges = Vec::new();
    let mut pos = 0usize;
    let mut directives = FormatDirectiveState::default();
    let mut noindent: Vec<(&str, usize)> = Vec::new();
    let noindent_markers: Vec<_> = config
        .protect
        .no_indent_blocks
        .iter()
        .map(|name| {
            (
                name.as_str(),
                format!("\\begin{{{name}}}"),
                format!("\\end{{{name}}}"),
            )
        })
        .collect();
    let mut macro_ranges = Vec::new();
    let mut delayed_key_ranges = std::collections::BTreeMap::new();
    let mut xparse_bindings: HashMap<(bool, String), xparse::Signature> = HashMap::new();
    let mut package_verbatim: HashMap<String, bool> = HashMap::new();
    let mut package_commands_redefined = HashSet::new();
    let mut visual_overrides = crate::packages::visual_science::Overrides::default();
    let mut conditional_bindings = ConditionalBindings::default();
    let mut catcode_scopes: Vec<(String, bool)> = Vec::new();
    let mut inline_tikz_start = None;
    let mut active_conditions: Vec<ActiveCondition> = Vec::new();
    let mut conditional_loops = Vec::new();
    let mut pending_unless = false;
    while pos < source.len() {
        if let Some(end) = delayed_key_ranges.remove(&pos) {
            ranges.push(pos..end);
            pos = end;
            continue;
        }
        if inline_tikz_start.is_some_and(|start| start < pos) {
            inline_tikz_start = None;
        }
        if let Some(range) = macro_ranges
            .last()
            .filter(|r: &&Range<usize>| r.start == pos)
        {
            ranges.push((*range).clone());
            pos = range.end;
            continue;
        }
        let c = source.as_bytes()[pos];
        if pending_unless && c != b'\\' && c != b'%' && !c.is_ascii_whitespace() {
            pending_unless = false;
        }
        // Disabled regions can contain arbitrary literal text, including incomplete
        // control sequences. Only their comment markers participate in scanning.
        if (directives.is_disabled() || !noindent.is_empty()) && c != b'%' {
            pos = memchr::memchr(b'%', &source.as_bytes()[pos..]).map_or(source.len(), |i| pos + i);
            continue;
        }
        if c == b'%' {
            let finish = line_end(source, pos);
            let comment = source[pos + 1..finish].trim();
            if is_escaped(source, pos) {
                pos += 1;
                continue;
            }
            let resume = directives.observe(source, pos, finish, &mut ranges);
            for (name, begin_marker, end_marker) in &noindent_markers {
                if comment.starts_with(begin_marker) {
                    noindent.push((name, line_start(source, pos)));
                }
                if comment.starts_with(end_marker)
                    && let Some(index) = noindent.iter().rposition(|(n, _)| n == name)
                {
                    let (_, start) = noindent.remove(index);
                    ranges.push(start..finish);
                }
            }
            pos = resume;
            continue;
        }
        if let Some(end) = protected_special_end(pos, config, &special_matches)? {
            ranges.push(pos..end);
            pos = end;
            continue;
        }
        if c == b'\\' {
            let command = command_at(source, pos);
            if let Some((name, begin, finish)) =
                command.and_then(|(name, end)| env_after_command(source, name, end))
            {
                if begin
                    && crate::packages::scoped_header_signature(name).is_some()
                    && let Some(headers) = visual_headers.as_deref_mut()
                {
                    headers.insert(
                        finish,
                        !config.syntax.environment_arguments.contains_key(name)
                            && !visual_overrides.contains(name),
                    );
                }
                if begin && inferred_environments.iter().any(|n| n == name) {
                    let end = inferred_environment_end(source, finish, name, config);
                    ranges.push(pos..end);
                    pos = end;
                    continue;
                }
                if begin
                    && (config.protect.environments.iter().any(|n| n == name)
                        || config.protect.file_contents.iter().any(|n| n == name)
                        || package_verbatim.contains_key(name)
                        || crate::package_environments::is_source_verbatim_environment_outside(
                            &source[..pos],
                            name,
                            &ranges,
                        )
                        || crate::packages::visual_science::is_verbatim(name)
                            && !config.syntax.environment_arguments.contains_key(name)
                            && !visual_overrides.contains(name))
                {
                    if name == "scontents" {
                        let end =
                            crate::code_environments::nested_environment_end(source, finish, name)
                                .ok_or_else(|| {
                                    FormatError::Syntax(format!(
                                        "unclosed verbatim environment {name:?} at byte {pos}"
                                    ))
                                })?;
                        ranges.push(pos..end);
                        pos = end;
                        continue;
                    }
                    let terminator = format!("\\end{{{name}}}");
                    let line_delimited =
                        name == "comment" || package_verbatim.get(name) == Some(&true);
                    let lua_line_delimited =
                        crate::luadraw_syntax::is_line_delimited_environment(name)
                            || crate::code_environments::is_line_delimited_environment(name);
                    let end = if crate::packages::visual_science::has_exact_line_end(name) {
                        crate::packages::visual_science::exact_line_end(source, finish, name)
                    } else if lua_line_delimited {
                        line_delimited_environment_end(source, finish, &terminator)
                    } else {
                        source[finish..]
                            .match_indices(&terminator)
                            .find(|(offset, _)| {
                                let start = finish + offset;
                                !line_delimited
                                    || (line_start(source, start) == start
                                        && source
                                            [start + terminator.len()..line_end(source, start)]
                                            .trim_end_matches(['\r', '\n', ' '])
                                            .is_empty())
                            })
                            .map(|(i, _)| finish + i + terminator.len())
                    }
                    .ok_or_else(|| {
                        FormatError::Syntax(format!(
                            "unclosed verbatim environment {name:?} at byte {pos}"
                        ))
                    })?;
                    let end = if crate::code_environments::ignores_end_line_tail(name)
                        || crate::packages::visual_science::ignores_end_line_tail(name)
                    {
                        line_end(source, end)
                    } else {
                        end
                    };
                    ranges.push(pos..end);
                    pos = end;
                    continue;
                }
                if begin {
                    let signature = config
                        .syntax
                        .environment_arguments
                        .get(name)
                        .and_then(|rule| rule.signature.as_deref())
                        .filter(|signature| xparse::extended(signature))
                        .and_then(xparse::Signature::parse);
                    let signature = signature
                        .as_ref()
                        .or_else(|| xparse_bindings.get(&(true, name.to_owned())));
                    if let Some(arguments) = signature.and_then(|signature| {
                        signature.literal_ranges(source, finish..source.len())
                    }) {
                        delayed_key_ranges
                            .extend(arguments.into_iter().map(|range| (range.start, range.end)));
                    }
                }
                if begin
                    && !config.syntax.environment_arguments.contains_key(name)
                    && !visual_overrides.contains(name)
                    && let Some(range) = column_specification(source, name, finish)
                {
                    macro_ranges.push(range);
                }
                if begin {
                    conditional_bindings.enter();
                    visual_overrides.enter();
                    if crate::packages::tikz_diagrams::reinstall_picture_commands(name)
                        && (name != "ganttchart"
                            || !catcode_scopes.last().is_some_and(|scope| scope.1))
                        && !visual_overrides.contains(name)
                        && !config.syntax.environment_arguments.contains_key(name)
                    {
                        visual_overrides.restore_default("pie");
                    }
                    let drawing = catcode_scopes.last().is_some_and(|scope| scope.1)
                        || crate::tikz_scan::environment(name)
                            && !visual_overrides.contains(name)
                            && !config.syntax.environment_arguments.contains_key(name);
                    catcode_scopes.push((format!("\\end{{{name}}}"), drawing));
                } else {
                    conditional_bindings.leave();
                    visual_overrides.leave();
                    catcode_scopes.pop();
                }
                if begin
                    && let Some(signature) =
                        crate::packages::scoped_invocation_signature(source, finish, name)
                    && !config.syntax.environment_arguments.contains_key(name)
                    && !visual_overrides.contains(name)
                {
                    // Graphics/animation settings contain delayed environments,
                    // declarations and catcode tokens. Keep this header data out
                    // of body parsing and project declaration inference.
                    let header = crate::layout::signature_argument_ranges(
                        source,
                        finish..source.len(),
                        signature.bytes(),
                    );
                    if let Some(last) = header.last() {
                        // Leave the enclosing delimiters visible to the block
                        // parser so they retain their header-argument ownership.
                        for argument in &header {
                            let delimited = matches!(
                                source.as_bytes()[argument.start],
                                b'{' | b'[' | b'(' | b'<'
                            );
                            let content = if delimited {
                                argument.start + 1..argument.end - 1
                            } else {
                                argument.clone()
                            };
                            if !content.is_empty() {
                                ranges.push(content);
                            }
                        }
                        pos = last.end;
                        continue;
                    }
                }
            }
            if let Some((name, end)) = command {
                let base = name.trim_end_matches('*');
                if latex_macro_definition(name) {
                    if let Some(declaration) = context_macro_declaration(source, pos, name, end) {
                        let key = (declaration.environment, declaration.name);
                        if let Some(signature) = declaration
                            .arguments
                            .and_then(|arguments| arguments.signature)
                            .filter(|signature| xparse::extended(signature))
                            .and_then(|signature| xparse::Signature::parse(&signature))
                        {
                            if catcode_scopes.is_empty() && active_conditions.is_empty() {
                                xparse_bindings.insert(key, signature);
                            }
                        } else {
                            xparse_bindings.remove(&key);
                        }
                    }
                } else if matches!(base, "def" | "gdef" | "edef" | "xdef" | "let" | "futurelet")
                    && let Some((target, _)) = command_at(source, skip_trivia(source, end))
                {
                    xparse_bindings.remove(&(false, target.to_owned()));
                }
                let configured = config.syntax.commands.get(base);
                let signature = configured
                    .and_then(|rule| rule.signature.as_deref())
                    .filter(|signature| xparse::extended(signature))
                    .and_then(xparse::Signature::parse);
                let signature = signature.as_ref().or_else(|| {
                    configured
                        .is_none()
                        .then(|| xparse_bindings.get(&(false, base.to_owned())))
                        .flatten()
                });
                if let Some(signature) = signature {
                    let start = end - usize::from(name.ends_with('*'));
                    if let Some(arguments) = signature.literal_ranges(source, start..source.len()) {
                        delayed_key_ranges.extend(
                            arguments
                                .into_iter()
                                .filter(|range| range.start >= end)
                                .map(|range| (range.start, range.end)),
                        );
                    }
                }

                let uncertain = active_conditions.iter().any(|condition| {
                    condition.meaning == ConditionalMeaning::Other
                        || condition.meaning == ConditionalMeaning::Case
                            && condition.selected.is_none()
                });
                let global_catcode = conditional_bindings.has_global_prefix();
                conditional_bindings.observe_with_uncertainty(source, name, end, config, uncertain);
                if let Some(finish) = primitive_data_end(source, name, end) {
                    ranges.push(pos..finish);
                    pos = finish;
                    continue;
                }
                if latex_macro_definition(base)
                    || matches!(base, "def" | "gdef" | "edef" | "xdef" | "let" | "futurelet")
                {
                    let mut target = skip_trivia(source, end);
                    if source.as_bytes().get(target) == Some(&b'{') {
                        target = skip_trivia(source, target + 1);
                    }
                    if base.contains("environment") || base.contains("Environment") {
                        let start = skip_trivia(source, end);
                        if let Some(finish) = balanced_end(source, start, b'{', b'}') {
                            let environment = source[start + 1..finish - 1].trim();
                            package_verbatim.remove(environment);
                            if !matches!(base, "provideenvironment" | "ProvideDocumentEnvironment")
                                && crate::packages::scoped_header_signature(environment).is_some()
                            {
                                visual_overrides.assign(environment, global_catcode);
                            }
                        }
                    } else if let Some((target, _)) = command_at(source, target) {
                        let environment = crate::packages::scoped_environment_for_command(target)
                            .or_else(|| {
                                (crate::packages::has_scoped_command_data(target)
                                    && (target != "pie"
                                        || catcode_scopes.last().is_some_and(|scope| scope.1)))
                                .then_some(target)
                            });
                        let provide = matches!(base, "providecommand" | "ProvideDocumentCommand");
                        if !provide || environment.is_none() {
                            package_commands_redefined.insert(target.to_owned());
                            if let Some(environment) = environment {
                                visual_overrides.assign(
                                    environment,
                                    global_catcode || matches!(base, "gdef" | "xdef"),
                                );
                            }
                        }
                    }
                }
                if let Some(finish) = let_declaration_end(source, name, end) {
                    pos = finish;
                    continue;
                }
                if let Some(finish) = primitive_literal_argument_end(source, name, end) {
                    ranges.push(end..finish);
                    pos = finish;
                    continue;
                }
                if !config.syntax.commands.contains_key(base)
                    && if crate::packages::has_scoped_command_data(base) {
                        !visual_overrides.contains(base)
                    } else {
                        !package_commands_redefined.contains(base)
                    }
                {
                    if base == "tikz" {
                        let start = skip_trivia(source, end);
                        let body = if source.as_bytes().get(start) == Some(&b'[') {
                            balanced_end(source, start, b'[', b']')
                                .map(|end| skip_trivia(source, end))
                        } else {
                            Some(start)
                        };
                        inline_tikz_start =
                            body.filter(|&start| source.as_bytes().get(start) == Some(&b'{'));
                    }
                    if crate::code_package_syntax::INLINE_CODE_COMMANDS.contains(&base)
                        || crate::code_environments::INLINE_VERBATIM_COMMANDS.contains(&base)
                    {
                        let finish = inline_end(source, name, end).ok_or_else(|| {
                            FormatError::Syntax(format!(
                                "unclosed verbatim command \\{name} at byte {pos}"
                            ))
                        })?;
                        ranges.push(pos..finish);
                        pos = finish;
                        continue;
                    }
                    if let Some(finish) = embedded_lua_end(source, name, end) {
                        ranges.push(pos..finish);
                        pos = finish;
                        continue;
                    }
                    if let Some(declaration) =
                        crate::package_environments::literal_box_declaration(source, base, end)
                    {
                        let mut cursor = pos;
                        while cursor < declaration.end {
                            if source.as_bytes()[cursor] == b'%' {
                                cursor = line_end(source, cursor);
                                continue;
                            }
                            if let Some((command, finish)) = command_at(source, cursor) {
                                if config.protect.dynamic_catcodes
                                    && matches!(
                                        command,
                                        "catcode" | "obeyspaces" | "obeylines" | "scantokens"
                                    )
                                    || config.protect.expl3 && command == "ExplSyntaxOn"
                                {
                                    package_verbatim.insert(declaration.name.clone(), false);
                                    break;
                                }
                                cursor = finish;
                            } else {
                                cursor += source[cursor..].chars().next().unwrap().len_utf8();
                            }
                        }
                        ranges.push(pos..declaration.end);
                        pos = declaration.end;
                        continue;
                    }
                    if let Some(finish) =
                        crate::package_commands::physics_matrix_end(source, name, end)
                    {
                        ranges.push(pos..finish);
                        pos = finish;
                        continue;
                    }
                    delayed_key_ranges.extend(
                        crate::packages::keys::delayed_ranges(source, name, end)
                            .into_iter()
                            .map(|range| (range.start, range.end)),
                    );
                    if let Some(finish) = (!config
                        .protect
                        .commands
                        .iter()
                        .any(|command| command == base || command == name))
                    .then(|| crate::packages::command_data_end(source, name, end))
                    .flatten()
                    {
                        ranges.push(end..finish);
                        pos = finish;
                        continue;
                    }
                    if let Some(range) =
                        crate::packages::tikz_derived::literal_data_range(source, end, name)
                            .or_else(|| {
                                crate::packages::tikz_diagrams::literal_data_range(
                                    source,
                                    end,
                                    name,
                                    catcode_scopes.last().is_some_and(|scope| scope.1),
                                )
                            })
                            .filter(|_| {
                                !config
                                    .protect
                                    .commands
                                    .iter()
                                    .any(|command| command == base || command == name)
                            })
                    {
                        pos = range.end;
                        ranges.push(range);
                        continue;
                    }
                }
                if !config.syntax.commands.contains_key(base)
                    && !package_commands_redefined.contains(base)
                    && let Some(declaration) =
                        crate::package_environments::verbatim_declaration(source, base, end)
                {
                    for name in declaration.names {
                        package_verbatim.insert(name, declaration.line_delimited);
                    }
                    ranges.push(pos..declaration.end);
                    pos = declaration.end;
                    continue;
                }
                if base == "newif" {
                    let target = skip_trivia(source, end);
                    if let Some((target_name, target_end)) = command_at(source, target) {
                        conditional_bindings.assign(target_name, ConditionalMeaning::Other, false);
                        pos = target_end;
                        continue;
                    }
                }
                if matches!(base, "bgroup" | "begingroup") {
                    conditional_bindings.enter();
                    visual_overrides.enter();
                    catcode_scopes.push((
                        if base == "begingroup" {
                            "\\endgroup"
                        } else {
                            "\\egroup"
                        }
                        .into(),
                        catcode_scopes.last().is_some_and(|scope| scope.1),
                    ));
                }
                if matches!(base, "egroup" | "endgroup") {
                    conditional_bindings.leave();
                    visual_overrides.leave();
                    catcode_scopes.pop();
                }
                if base == "loop" && !conditional_bindings.assigned(base) {
                    conditional_loops.push(active_conditions.len());
                } else if base == "repeat"
                    && !conditional_bindings.assigned(base)
                    && let Some(before) = conditional_loops.pop()
                    && active_conditions.len() == before + 1
                {
                    active_conditions.pop();
                }
                let constructed = literal_csname(source, pos);
                let (executed_name, executed_end) = constructed
                    .as_ref()
                    .map_or((base, end), |(value, finish)| (value.as_str(), *finish));
                let numeric = literal_numeric_condition(
                    source,
                    executed_name,
                    executed_end,
                    &conditional_bindings,
                );
                let mut meaning = numeric.map_or_else(
                    || conditional_bindings.meaning(executed_name, config),
                    |(meaning, _)| meaning,
                );
                if meaning == ConditionalMeaning::Unless {
                    pending_unless = true;
                    pos = executed_end;
                    continue;
                }
                if pending_unless {
                    meaning = match meaning {
                        ConditionalMeaning::True => ConditionalMeaning::False,
                        ConditionalMeaning::False => ConditionalMeaning::True,
                        ConditionalMeaning::Case => ConditionalMeaning::Other,
                        other => other,
                    };
                    pending_unless = false;
                }
                let mut skip_to = None;
                let mut preserve_dead = false;
                match meaning {
                    ConditionalMeaning::True
                    | ConditionalMeaning::False
                    | ConditionalMeaning::Case
                    | ConditionalMeaning::Other => {
                        let case_number = (meaning == ConditionalMeaning::Case)
                            .then(|| {
                                literal_case_number(source, executed_end, &conditional_bindings)
                            })
                            .flatten();
                        let selected = case_number.map(|(number, _)| number);
                        active_conditions.push(ActiveCondition {
                            meaning,
                            selected,
                            branch: 0,
                            had_else: false,
                        });
                        if config.protect.inactive_branches
                            && (meaning == ConditionalMeaning::False
                                || meaning == ConditionalMeaning::Case
                                    && selected.is_some_and(|number| number != 0))
                        {
                            preserve_dead = meaning == ConditionalMeaning::False;
                            let start = case_number
                                .or(numeric.map(|(_, finish)| (0, finish)))
                                .map_or(executed_end, |(_, finish)| finish);
                            skip_to = inactive_branch_marker(
                                source,
                                start,
                                config,
                                &conditional_bindings,
                                true,
                                meaning == ConditionalMeaning::Case,
                            )
                            .map(|(_, marker)| (start, marker.start));
                        }
                    }
                    ConditionalMeaning::Else | ConditionalMeaning::Or => {
                        if let Some(condition) = active_conditions
                            .last_mut()
                            .filter(|condition| !condition.had_else)
                        {
                            let was_selected = condition.selected == Some(condition.branch);
                            if meaning == ConditionalMeaning::Else {
                                condition.had_else = true;
                            } else {
                                condition.branch += 1;
                            }
                            if config.protect.inactive_branches {
                                let skip_tail = meaning == ConditionalMeaning::Else
                                    && condition.meaning == ConditionalMeaning::True
                                    || condition.meaning == ConditionalMeaning::Case
                                        && condition.selected.is_some()
                                        && was_selected;
                                let skip_case_branch = meaning == ConditionalMeaning::Or
                                    && condition.meaning == ConditionalMeaning::Case
                                    && condition
                                        .selected
                                        .is_some_and(|number| number != condition.branch);
                                if skip_tail || skip_case_branch {
                                    skip_to = inactive_branch_marker(
                                        source,
                                        executed_end,
                                        config,
                                        &conditional_bindings,
                                        !skip_tail,
                                        !skip_tail,
                                    )
                                    .map(|(_, marker)| (executed_end, marker.start));
                                }
                            }
                        }
                    }
                    ConditionalMeaning::Fi => {
                        active_conditions.pop();
                    }
                    _ => (),
                }
                if let Some((start, finish)) = skip_to {
                    if finish > start
                        && (preserve_dead
                            || dead_branch_needs_protection(
                                source,
                                start..finish,
                                config,
                                compiled,
                                &conditional_bindings,
                            ))
                    {
                        ranges.push(start..finish);
                    }
                    pos = finish;
                    continue;
                }
                if config
                    .protect
                    .commands
                    .iter()
                    .any(|n| n == base || n == name)
                    || config.syntax.commands.get(base).is_some_and(|r| r.verbatim)
                {
                    let finish = inline_end(source, name, end).ok_or_else(|| {
                        FormatError::Syntax(format!(
                            "unclosed verbatim command \\{name} at byte {pos}"
                        ))
                    })?;
                    ranges.push(pos..finish);
                    pos = finish;
                    continue;
                }
                if config.protect.expl3 && name == "ExplSyntaxOn"
                    || config.protect.dynamic_catcodes
                        && matches!(name, "catcode" | "obeyspaces" | "obeylines" | "scantokens")
                {
                    let finish = dynamic_catcode_end(
                        source,
                        name,
                        end,
                        catcode_scopes.last().map(|scope| scope.0.as_str()),
                        global_catcode,
                    );
                    ranges.push(pos..finish);
                    pos = finish;
                    continue;
                }
                if config.protect.macro_bodies
                    && matches!(
                        base,
                        "newcommand"
                            | "renewcommand"
                            | "providecommand"
                            | "DeclareRobustCommand"
                            | "DeclareMathOperator"
                            | "def"
                            | "gdef"
                            | "edef"
                            | "xdef"
                            | "newenvironment"
                            | "renewenvironment"
                            | "newcolumntype"
                            | "NewDocumentCommand"
                            | "RenewDocumentCommand"
                            | "ProvideDocumentCommand"
                            | "DeclareDocumentCommand"
                            | "NewDocumentEnvironment"
                            | "RenewDocumentEnvironment"
                            | "ProvideDocumentEnvironment"
                            | "DeclareDocumentEnvironment"
                    )
                {
                    if let Some(range) = macro_body(source, name, end) {
                        macro_ranges.push(range);
                    }
                } else if let Some(rule) =
                    config.syntax.commands.get(base).filter(|r| r.preserve_body)
                    && let Some(range) = raw_command_body(source, end, rule.arguments)
                {
                    macro_ranges.push(range);
                }
                pos = primitive_test_end(source, name, end)
                    .or_else(|| next_character_test_end(source, name, end))
                    .or_else(|| definition_header_end(source, name, end))
                    .unwrap_or(end);
                continue;
            }
        }
        {
            if c == b'{' {
                conditional_bindings.enter();
                visual_overrides.enter();
                let drawing = catcode_scopes.last().is_some_and(|scope| scope.1)
                    || inline_tikz_start == Some(pos);
                if inline_tikz_start == Some(pos) {
                    visual_overrides.restore_default("pie");
                }
                catcode_scopes.push(("}".into(), drawing));
            }
            if c == b'}' {
                conditional_bindings.leave();
                visual_overrides.leave();
                catcode_scopes.pop();
            }
            pos += source[pos..]
                .chars()
                .next()
                .expect("valid position")
                .len_utf8();
            if config.syntax.special.is_empty() {
                let next = memchr::memchr3(b'\\', b'%', b'{', &source.as_bytes()[pos..])
                    .map_or(source.len(), |i| pos + i);
                let next = memchr::memchr(b'}', &source.as_bytes()[pos..])
                    .map_or(next, |i| next.min(pos + i));
                let next = delayed_key_ranges
                    .range(pos..)
                    .next()
                    .map_or(next, |(&start, _)| next.min(start));
                pos = macro_ranges
                    .last()
                    .filter(|r| r.start >= pos)
                    .map_or(next, |r| next.min(r.start));
            }
        }
    }
    directives.finish(source.len(), &mut ranges);
    for (_, start) in noindent {
        ranges.push(start..source.len());
    }
    ranges.sort_by_key(|r| r.start);
    let mut merged: Vec<Range<usize>> = Vec::new();
    for range in ranges {
        if let Some(last) = merged.last_mut().filter(|last| range.start <= last.end) {
            last.end = last.end.max(range.end);
        } else {
            merged.push(range);
        }
    }
    Ok(merged)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    Command,
    BeginEnvironment,
    EndEnvironment,
    OpenBrace,
    CloseBrace,
    OpenBracket,
    CloseBracket,
    Math,
    Comment,
    Space,
    Newline,
    Text,
    Protected,
    SpecialBegin(usize),
    SpecialMiddle(usize),
    SpecialEnd(usize),
}

#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Range<usize>,
}

pub fn lex(
    source: &str,
    config: &Config,
    compiled: &CompiledSyntax,
) -> Result<Vec<Token>, FormatError> {
    let ranges = protected_ranges(source, config)?;
    lex_with_ranges(source, config, compiled, &ranges)
}

pub fn lex_with_ranges(
    source: &str,
    config: &Config,
    compiled: &CompiledSyntax,
    ranges: &[Range<usize>],
) -> Result<Vec<Token>, FormatError> {
    let constructed_commands = constant_constructed_commands(source, config, ranges);
    lex_with_constructed_commands(source, config, compiled, ranges, &constructed_commands)
}

pub(crate) fn lex_with_analysis(
    source: &str,
    config: &Config,
    compiled: &CompiledSyntax,
    ranges: &[Range<usize>],
    analysis: &crate::source_analysis::SourceAnalysis<'_>,
) -> Result<Vec<Token>, FormatError> {
    let constructed_commands = analysis.constructed_commands(ranges);
    lex_with_constructed_commands(source, config, compiled, ranges, &constructed_commands)
}

fn lex_with_constructed_commands(
    source: &str,
    config: &Config,
    compiled: &CompiledSyntax,
    ranges: &[Range<usize>],
    constructed_commands: &HashMap<usize, (String, usize)>,
) -> Result<Vec<Token>, FormatError> {
    let special_matches = special_matches(source, config, compiled, false)?;
    let mut output = Vec::with_capacity((source.len() / 32).min(65_536));
    let mut pos = 0usize;
    let mut protected_index = 0usize;
    let mut special_stack: Vec<usize> = Vec::new();
    let mut special_depths = Vec::new();
    let mut brace_depth = 0usize;
    let mut dollar_mode = None;
    let mut restricted_text = false;
    let mut pending_text_mode = false;
    let mut math_groups = Vec::new();
    while pos < source.len() {
        while protected_index < ranges.len() && ranges[protected_index].end <= pos {
            protected_index += 1;
        }
        if let Some(range) = ranges.get(protected_index).filter(|r| r.start == pos) {
            output.push(Token {
                kind: TokenKind::Protected,
                span: range.clone(),
            });
            pos = range.end;
            continue;
        }
        let start = pos;
        let mut special_match = None;
        if let Some(&index) = special_stack.last() {
            if let Some(end) = at_match(&special_matches[index].end, pos) {
                special_match = Some((TokenKind::SpecialEnd(index), end));
                special_stack.pop();
                special_depths.pop();
            } else if special_depths.last() == Some(&brace_depth)
                && let Some(end) = at_match(&special_matches[index].middle, pos)
            {
                special_match = Some((TokenKind::SpecialMiddle(index), end));
            }
        }
        if special_match.is_none() {
            for (i, rule) in config
                .syntax
                .special
                .iter()
                .enumerate()
                .filter(|(_, r)| !r.verbatim)
            {
                if special_stack
                    .last()
                    .is_some_and(|&j| j == i && !rule.nested)
                {
                    continue;
                }
                if let Some(end) = at_match(&special_matches[i].begin, pos) {
                    special_match = Some((TokenKind::SpecialBegin(i), end));
                    special_stack.push(i);
                    special_depths.push(brace_depth);
                    break;
                }
            }
        }
        if let Some((kind, end)) = special_match {
            output.push(Token {
                kind,
                span: start..end,
            });
            pos = end;
            continue;
        }
        let kind = match source.as_bytes()[pos] {
            b'\\' => {
                if let Some((_, begin, end)) = env_at(source, pos) {
                    pos = end;
                    pending_text_mode = false;
                    if begin {
                        TokenKind::BeginEnvironment
                    } else {
                        TokenKind::EndEnvironment
                    }
                } else {
                    let (name, end) = command_at(source, pos).ok_or_else(|| {
                        FormatError::Syntax(format!("incomplete control sequence at byte {pos}"))
                    })?;
                    if matches!(name, "bgroup" | "begingroup") {
                        math_groups
                            .push(pending_text_mode.then_some((dollar_mode, restricted_text)));
                        if pending_text_mode {
                            dollar_mode = None;
                            restricted_text = true;
                        }
                    } else if matches!(name, "egroup" | "endgroup")
                        && let Some(Some((saved_mode, saved_text))) = math_groups.pop()
                    {
                        dollar_mode = saved_mode;
                        restricted_text = saved_text;
                    }
                    pending_text_mode = matches!(
                        name.trim_end_matches('*'),
                        "text"
                            | "textrm"
                            | "textsf"
                            | "texttt"
                            | "textnormal"
                            | "textup"
                            | "textit"
                            | "textsl"
                            | "textsc"
                            | "textmd"
                            | "textbf"
                            | "mbox"
                            | "hbox"
                    );
                    if let Some(finish) = primitive_data_end(source, name, end) {
                        pos = finish;
                        TokenKind::Protected
                    } else if let Some(finish) = let_declaration_end(source, name, end) {
                        pos = finish;
                        TokenKind::Command
                    } else {
                        pos = constructed_commands
                            .get(&pos)
                            .map(|(_, finish)| *finish)
                            .or_else(|| literal_csname(source, pos).map(|(_, finish)| finish))
                            .unwrap_or_else(|| {
                                primitive_test_end(source, name, end)
                                    .or_else(|| next_character_test_end(source, name, end))
                                    .or_else(|| definition_header_end(source, name, end))
                                    .unwrap_or(end)
                            });
                        TokenKind::Command
                    }
                }
            }
            b'%' => {
                pos = source[pos..].find('\n').map_or(source.len(), |i| pos + i);
                TokenKind::Comment
            }
            b'{' => {
                pos += 1;
                brace_depth += 1;
                math_groups.push(pending_text_mode.then_some((dollar_mode, restricted_text)));
                if pending_text_mode {
                    dollar_mode = None;
                    restricted_text = true;
                }
                pending_text_mode = false;
                TokenKind::OpenBrace
            }
            b'}' => {
                pos += 1;
                brace_depth = brace_depth.saturating_sub(1);
                if let Some(Some((saved_mode, saved_text))) = math_groups.pop() {
                    dollar_mode = saved_mode;
                    restricted_text = saved_text;
                }
                pending_text_mode = false;
                TokenKind::CloseBrace
            }
            b'[' => {
                pos += 1;
                TokenKind::OpenBracket
            }
            b']' => {
                pos += 1;
                TokenKind::CloseBracket
            }
            b'$' => {
                let width = if dollar_mode == Some(1) {
                    // Closing inline math consumes one shift, including the
                    // first shift between adjacent formulas such as $a$$b$.
                    dollar_mode = None;
                    1
                } else if source[pos..].starts_with("$$") && !restricted_text {
                    dollar_mode = if dollar_mode == Some(2) {
                        None
                    } else {
                        Some(2)
                    };
                    2
                } else {
                    if dollar_mode.is_none() {
                        dollar_mode = Some(1);
                    }
                    1
                };
                pos += width;
                TokenKind::Math
            }
            b'\r' if source.as_bytes().get(pos + 1) == Some(&b'\n') => {
                pos += 2;
                TokenKind::Newline
            }
            b'\n' => {
                pos += 1;
                TokenKind::Newline
            }
            b' ' | b'\t' | b'\r' => {
                pos += 1;
                while matches!(source.as_bytes().get(pos), Some(b' ' | b'\t')) {
                    pos += 1;
                }
                TokenKind::Space
            }
            _ => {
                pos += source[pos..]
                    .chars()
                    .next()
                    .expect("valid position")
                    .len_utf8();
                let special_limit = special_matches
                    .iter()
                    .flat_map(|m| {
                        [
                            next_match(&m.begin, pos),
                            next_match(&m.middle, pos),
                            next_match(&m.end, pos),
                        ]
                    })
                    .flatten()
                    .map(|m| m.start)
                    .min()
                    .unwrap_or(source.len());
                while pos < source.len()
                    && !matches!(
                        source.as_bytes()[pos],
                        b'\\' | b'%' | b'{' | b'}' | b'[' | b']' | b'$'
                    )
                    && ranges.get(protected_index).is_none_or(|r| r.start != pos)
                    && pos < special_limit
                {
                    // Delimiters are ASCII; scanning bytes lands at a UTF-8 boundary.
                    pos += 1;
                }
                TokenKind::Text
            }
        };
        output.push(Token {
            kind,
            span: start..pos,
        });
        if !matches!(
            output.last().expect("token").kind,
            TokenKind::Space | TokenKind::Newline | TokenKind::Comment | TokenKind::Command
        ) {
            pending_text_mode = false;
        }
    }
    Ok(output)
}

#[derive(Debug, Clone)]
pub struct Environment {
    pub name: String,
    pub begin: Range<usize>,
    pub body: Range<usize>,
    pub end: Range<usize>,
}

pub fn environment_ranges(source: &str, config: &Config) -> Result<Vec<Environment>, FormatError> {
    let compiled = compile(&config.syntax).map_err(FormatError::Syntax)?;
    let protected = protected_ranges(source, config)?;
    let tokens = lex_with_ranges(source, config, &compiled, &protected)?;
    environment_ranges_with_tokens(source, &tokens, &protected, config)
}

pub(crate) fn environment_ranges_with_tokens(
    source: &str,
    tokens: &[Token],
    protected: &[Range<usize>],
    config: &Config,
) -> Result<Vec<Environment>, FormatError> {
    let mut stack: Vec<(String, Range<usize>)> = Vec::new();
    let mut output = Vec::new();
    for token in tokens {
        match token.kind {
            TokenKind::BeginEnvironment => {
                let (name, _, _) = env_at(source, token.span.start).expect("environment token");
                stack.push((name.into(), token.span.clone()));
            }
            TokenKind::EndEnvironment => {
                let (name, _, _) = env_at(source, token.span.start).expect("environment token");
                let (open, begin) = stack.pop().ok_or_else(|| {
                    FormatError::Syntax(format!(
                        "unexpected \\end{{{name}}} at byte {}",
                        token.span.start
                    ))
                })?;
                if open != name {
                    return Err(FormatError::Syntax(format!(
                        "environment {open:?} ended by {name:?} at byte {}",
                        token.span.start
                    )));
                }
                output.push(Environment {
                    name: open,
                    body: begin.end..token.span.start,
                    begin,
                    end: token.span.clone(),
                });
            }
            _ => (),
        }
    }
    if let Some((name, begin)) = stack.last() {
        if let Some(suffix) = opaque_suffix(source, protected, config) {
            for (name, begin) in stack {
                output.push(Environment {
                    name,
                    body: begin.end..suffix.start,
                    begin,
                    end: suffix.start..suffix.start,
                });
            }
        } else {
            return Err(FormatError::Syntax(format!(
                "unclosed environment {name:?} at byte {}",
                begin.start
            )));
        }
    }
    Ok(output)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    Loop,
    TikzPath,
    Environment,
    Verbatim,
    Command,
    MandatoryArgument,
    OptionalArgument,
    Group,
    Condition,
    Item,
    Heading,
    Special,
    KeyValue,
    NamedGroup,
}
impl BlockKind {
    pub fn category(self) -> &'static str {
        match self {
            Self::Loop => "loop",
            Self::TikzPath => "tikz-path",
            Self::Environment => "environment",
            Self::Verbatim => "verbatim",
            Self::Command => "command",
            Self::MandatoryArgument => "mandatory-argument",
            Self::OptionalArgument => "optional-argument",
            Self::Group => "group",
            Self::Condition => "condition",
            Self::Item => "item",
            Self::Heading => "heading",
            Self::Special => "special",
            Self::KeyValue => "key-value",
            Self::NamedGroup => "named-group",
        }
    }
}
#[derive(Debug, Clone)]
pub struct Block {
    pub kind: BlockKind,
    pub name: String,
    pub begin: Range<usize>,
    pub body: Range<usize>,
    pub end: Range<usize>,
    pub parent: Option<usize>,
    pub branches: Vec<(bool, Range<usize>)>,
}

pub fn verbatim_blocks(source: &str, config: &Config) -> Result<Vec<Block>, FormatError> {
    let protected = protected_ranges(source, config)?;
    Ok(
        parse_verbatim_blocks_with_ranges(source, &protected, &mut OwnedNames)
            .into_iter()
            .map(Block::from)
            .collect(),
    )
}

pub(crate) fn verbatim_blocks_with_ranges(
    source: &str,
    protected: &[Range<usize>],
) -> Vec<SharedBlock> {
    parse_verbatim_blocks_with_ranges(source, protected, &mut NamePool::default())
}

fn parse_verbatim_blocks_with_ranges<S: NameStore>(
    source: &str,
    protected: &[Range<usize>],
    names: &mut S,
) -> Vec<StructuralBlock<S::Name>> {
    let mut output = Vec::new();
    for range in protected {
        if let Some((name, true, begin_end)) = env_at(source, range.start) {
            let closing = format!("\\end{{{name}}}");
            if !source[range.clone()].ends_with(&closing) {
                continue;
            }
            let end_start = range.end - closing.len();
            output.push(StructuralBlock {
                kind: BlockKind::Verbatim,
                name: names.make(name),
                begin: range.start..begin_end,
                body: begin_end..end_start,
                end: end_start..range.end,
                parent: None,
                branches: Vec::new(),
            });
        }
    }
    output
}

pub fn blocks(
    source: &str,
    config: &crate::CompiledConfig,
) -> Result<(Vec<Token>, Vec<Block>), FormatError> {
    let protected = protected_ranges(source, &config.config)?;
    let analysis = crate::source_analysis::SourceAnalysis::new(source, &config.config, &protected);
    blocks_with_analysis(source, config, &protected, &analysis)
}

pub fn blocks_with_ranges(
    source: &str,
    config: &crate::CompiledConfig,
    protected: &[Range<usize>],
) -> Result<(Vec<Token>, Vec<Block>), FormatError> {
    // Configuration protection supplies declaration context when it is valid.
    // Explicit raw ranges also admit otherwise incomplete source fragments.
    let metadata_protected = config
        .config
        .syntax
        .infer_list_environments
        .then(|| protected_ranges(source, &config.config).ok())
        .flatten();
    let analysis = crate::source_analysis::SourceAnalysis::new(
        source,
        &config.config,
        metadata_protected.as_deref().unwrap_or(protected),
    );
    blocks_with_analysis(source, config, protected, &analysis)
}

pub(crate) fn shared_blocks_with_ranges(
    source: &str,
    config: &crate::CompiledConfig,
    protected: &[Range<usize>],
) -> Result<(Vec<Token>, Vec<SharedBlock>), FormatError> {
    // Configuration protection supplies declaration context when it is valid.
    // Explicit raw ranges also admit otherwise incomplete source fragments.
    let metadata_protected = config
        .config
        .syntax
        .infer_list_environments
        .then(|| protected_ranges(source, &config.config).ok())
        .flatten();
    let analysis = crate::source_analysis::SourceAnalysis::new(
        source,
        &config.config,
        metadata_protected.as_deref().unwrap_or(protected),
    );
    shared_blocks_with_analysis(source, config, protected, &analysis)
}

pub(crate) fn shared_blocks_with_analysis(
    source: &str,
    config: &crate::CompiledConfig,
    protected: &[Range<usize>],
    analysis: &crate::source_analysis::SourceAnalysis<'_>,
) -> Result<(Vec<Token>, Vec<SharedBlock>), FormatError> {
    let tokens = lex_with_analysis(source, &config.config, &config.syntax, protected, analysis)?;
    let blocks = blocks_from_tokens(source, config, protected, analysis, &tokens)?;
    Ok((tokens, blocks))
}

pub(crate) fn blocks_with_analysis(
    source: &str,
    config: &crate::CompiledConfig,
    protected: &[Range<usize>],
    analysis: &crate::source_analysis::SourceAnalysis<'_>,
) -> Result<(Vec<Token>, Vec<Block>), FormatError> {
    let tokens = lex_with_analysis(source, &config.config, &config.syntax, protected, analysis)?;
    let blocks = parse_blocks_from_tokens(
        source,
        config,
        protected,
        analysis,
        &tokens,
        &mut OwnedNames,
    )?;
    Ok((tokens, blocks.into_iter().map(Block::from).collect()))
}

pub(crate) fn blocks_from_tokens(
    source: &str,
    config: &crate::CompiledConfig,
    protected: &[Range<usize>],
    analysis: &crate::source_analysis::SourceAnalysis<'_>,
    tokens: &[Token],
) -> Result<Vec<SharedBlock>, FormatError> {
    parse_blocks_from_tokens(
        source,
        config,
        protected,
        analysis,
        tokens,
        &mut NamePool::default(),
    )
}

fn parse_blocks_from_tokens<S: NameStore>(
    source: &str,
    config: &crate::CompiledConfig,
    protected: &[Range<usize>],
    analysis: &crate::source_analysis::SourceAnalysis<'_>,
    tokens: &[Token],
    names: &mut S,
) -> Result<Vec<StructuralBlock<S::Name>>, FormatError> {
    let mut blocks: Vec<StructuralBlock<S::Name>> = Vec::new();
    let constructed_commands = analysis.constructed_commands(protected);
    let mut stack: Vec<usize> = Vec::new();
    let mut pending: Option<usize> = None;
    let mut new_if = false;
    let mut known_conditions: HashSet<String> =
        config.config.syntax.conditions.iter().cloned().collect();
    let mut conditional_bindings = ConditionalBindings::default();
    let mut condition_meanings = HashMap::new();
    let mut list_environments: HashSet<String> = config
        .config
        .syntax
        .list_environments
        .iter()
        .cloned()
        .collect();
    if config.config.syntax.infer_list_environments {
        list_environments.extend(analysis.inferred_lists(&config.syntax).iter().cloned());
        for name in crate::packages::implicit_list_environments() {
            if analysis.implicit_list(name) {
                list_environments.insert(name.into());
            }
        }
    }
    let mut argument_counts: HashMap<usize, usize> = HashMap::new();
    let mut command_headers: HashMap<usize, Vec<Range<usize>>> = HashMap::new();
    let mut loop_conditions: HashMap<usize, usize> = HashMap::new();
    let mut configured_headers: HashMap<usize, Vec<Range<usize>>> = HashMap::new();
    let mut environment_ends = HashMap::new();
    let mut environments = Vec::new();
    for token in tokens {
        if token.kind == TokenKind::BeginEnvironment {
            environments.push(token.span.start);
        } else if token.kind == TokenKind::EndEnvironment
            && let Some(start) = environments.pop()
        {
            environment_ends.insert(start, token.span.start);
        }
    }
    for token in tokens {
        let parent = stack.last().copied();
        macro_rules! add {
            ($kind:expr, $name:expr, $parent:expr) => {{
                let index = blocks.len();
                blocks.push(StructuralBlock {
                    kind: $kind,
                    name: $name,
                    begin: token.span.clone(),
                    body: token.span.end..source.len(),
                    end: source.len()..source.len(),
                    parent: $parent,
                    branches: Vec::new(),
                });
                index
            }};
        }
        match token.kind {
            TokenKind::BeginEnvironment => {
                let (name, _, _) = env_at(source, token.span.start).expect("environment token");
                let i = add!(BlockKind::Environment, names.make(name), parent);
                if let Some(rule) = config.config.syntax.environment_arguments.get(name) {
                    let limit = environment_ends
                        .get(&token.span.start)
                        .copied()
                        .unwrap_or(source.len());
                    configured_headers.insert(
                        i,
                        crate::layout::configured_environment_argument_ranges(
                            source,
                            token.span.end..limit,
                            rule,
                        ),
                    );
                } else if crate::packages::scoped_header_signature(name).is_some() {
                    let limit = environment_ends
                        .get(&token.span.start)
                        .copied()
                        .unwrap_or(source.len());
                    let body = token.span.end..limit;
                    if let Some(rule) = analysis.box_arguments(name) {
                        configured_headers.insert(
                            i,
                            crate::layout::configured_environment_argument_ranges(
                                source, body, &rule,
                            ),
                        );
                    } else if analysis.visual_header_active(name, token.span.end) {
                        let signature =
                            crate::packages::scoped_invocation_signature(source, body.start, name)
                                .expect("known visual/science header");
                        configured_headers.insert(
                            i,
                            crate::layout::signature_argument_ranges(
                                source,
                                body,
                                signature.bytes(),
                            ),
                        );
                    }
                }
                stack.push(i);
                conditional_bindings.enter();
                pending = Some(i);
            }
            TokenKind::EndEnvironment => {
                let (name, _, _) = env_at(source, token.span.start).expect("environment token");
                let position = stack
                    .iter()
                    .rposition(|&i| blocks[i].kind != BlockKind::Condition)
                    .ok_or_else(|| {
                        FormatError::Syntax(format!(
                            "unexpected \\end{{{name}}} at byte {}",
                            token.span.start
                        ))
                    })?;
                let i = stack.remove(position);
                if blocks[i].kind != BlockKind::Environment || blocks[i].name != name {
                    return Err(FormatError::Syntax(format!(
                        "mismatched \\end{{{name}}} at byte {}",
                        token.span.start
                    )));
                }
                blocks[i].body.end = token.span.start;
                blocks[i].end = token.span.clone();
                conditional_bindings.leave();
                pending = None;
            }
            TokenKind::OpenBrace | TokenKind::OpenBracket => {
                if token.kind == TokenKind::OpenBrace {
                    conditional_bindings.enter();
                }
                let in_math = stack.iter().rev().any(|&i| match blocks[i].kind {
                    BlockKind::Special => matches!(
                        blocks[i].name.as_str(),
                        "displayMath" | "displayMathTeX" | "inlineMath"
                    ),
                    BlockKind::Environment => math_environment(&blocks[i].name),
                    _ => false,
                });
                if token.kind == TokenKind::OpenBracket
                    && in_math
                    && math_optional_argument_end(source, token.span.start).is_none()
                {
                    pending = None;
                    continue;
                }
                let header = parent.and_then(|i| configured_headers.get(&i));
                let explicit_owner = parent.filter(|_| {
                    header.is_some_and(|ranges| {
                        ranges.iter().any(|range| range.start == token.span.start)
                    })
                });
                let owner = explicit_owner.or_else(|| {
                    pending.filter(|&i| {
                        if let Some(ranges) = header {
                            let header_end = ranges.last().map_or_else(
                                || blocks[parent.expect("configured environment")].begin.end,
                                |range| range.end,
                            );
                            if blocks[i].begin.start < header_end
                                || blocks[i].kind == BlockKind::Environment
                            {
                                return false;
                            }
                        }
                        if in_math
                            && token.kind == TokenKind::OpenBracket
                            && math_delimiter_command(&blocks[i].name)
                            && !config
                                .config
                                .syntax
                                .commands
                                .contains_key(blocks[i].name.trim_end_matches('*'))
                        {
                            return false;
                        }
                        if let Some(ranges) = command_headers.get(&i) {
                            return ranges.iter().any(|range| range.start == token.span.start);
                        }
                        if blocks[i].kind != BlockKind::Command {
                            return true;
                        }
                        let Some(rule) = config
                            .config
                            .syntax
                            .commands
                            .get(blocks[i].name.trim_end_matches('*'))
                            .or_else(|| {
                                crate::package_commands::builtin_command_rule(&blocks[i].name)
                                    .filter(|_| {
                                        !analysis.declared_commands().is_ok_and(|names| {
                                            names.contains(blocks[i].name.trim_end_matches('*'))
                                        })
                                    })
                            })
                        else {
                            return true;
                        };
                        if token.kind == TokenKind::OpenBracket {
                            rule.optional_arguments
                        } else {
                            rule.arguments.is_none_or(|limit| {
                                argument_counts.get(&i).copied().unwrap_or(0) < limit
                            })
                        }
                    })
                });
                let preceding =
                    source[..token.span.start].trim_end_matches([' ', '\t', '\r', '\n']);
                let key = preceding
                    .strip_suffix('=')
                    .map(str::trim_end)
                    .and_then(|s| {
                        s.rsplit(|c: char| c.is_whitespace() || "{}[]\\,".contains(c))
                            .next()
                    })
                    .filter(|s| !(s.is_empty() || in_math && token.kind == TokenKind::OpenBracket));
                let word = preceding
                    .rsplit(|c: char| c.is_whitespace() || "{}[]\\,".contains(c))
                    .next()
                    .unwrap_or("");
                let named = !word.is_empty()
                    && word
                        .chars()
                        .all(|c| c.is_alphanumeric() || ".*@><".contains(c))
                    && preceding
                        .as_bytes()
                        .get(preceding.len().saturating_sub(word.len() + 1))
                        != Some(&b'\\')
                    && !(in_math && token.kind == TokenKind::OpenBracket);
                let inside_bracket =
                    parent.is_some_and(|i| source.as_bytes()[blocks[i].begin.start] == b'[');
                if token.kind == TokenKind::OpenBracket
                    && owner.is_none()
                    && key.is_none()
                    && !named
                    && !inside_bracket
                {
                    pending = None;
                    continue;
                }
                let (kind, name, group_parent) = if let Some(i) = owner {
                    if token.kind == TokenKind::OpenBrace
                        && blocks[i].kind == BlockKind::Command
                        && config
                            .config
                            .syntax
                            .commands
                            .get(blocks[i].name.trim_end_matches('*'))
                            .or_else(|| {
                                crate::package_commands::builtin_command_rule(&blocks[i].name)
                                    .filter(|_| {
                                        !analysis.declared_commands().is_ok_and(|names| {
                                            names.contains(blocks[i].name.trim_end_matches('*'))
                                        })
                                    })
                            })
                            .is_some_and(|r| r.arguments.is_some())
                    {
                        *argument_counts.entry(i).or_default() += 1;
                    }
                    (
                        if token.kind == TokenKind::OpenBracket {
                            BlockKind::OptionalArgument
                        } else {
                            BlockKind::MandatoryArgument
                        },
                        blocks[i].name.clone(),
                        Some(i),
                    )
                } else if let Some(key) = key {
                    (BlockKind::KeyValue, names.make(key), parent)
                } else if named {
                    (BlockKind::NamedGroup, names.make(word), parent)
                } else {
                    (BlockKind::Group, S::Name::default(), parent)
                };
                let i = add!(kind, name, group_parent);
                stack.push(i);
                pending = None;
            }
            TokenKind::CloseBrace | TokenKind::CloseBracket => {
                let structural_position = stack
                    .iter()
                    .rposition(|&i| blocks[i].kind != BlockKind::Condition);
                let structural_parent = structural_position.map(|position| stack[position]);
                if token.kind == TokenKind::CloseBracket
                    && structural_parent
                        .is_none_or(|i| source.as_bytes()[blocks[i].begin.start] != b'[')
                {
                    continue;
                }
                let position = structural_position.ok_or_else(|| {
                    FormatError::Syntax(format!(
                        "unexpected closing delimiter at byte {}",
                        token.span.start
                    ))
                })?;
                let i = stack.remove(position);
                let correct = if token.kind == TokenKind::CloseBracket {
                    source.as_bytes()[blocks[i].begin.start] == b'['
                } else {
                    source.as_bytes()[blocks[i].begin.start] == b'{'
                        || blocks[i].kind == BlockKind::Group && blocks[i].name == "bgroup"
                };
                if !correct {
                    return Err(FormatError::Syntax(format!(
                        "mismatched closing delimiter at byte {}",
                        token.span.start
                    )));
                }
                blocks[i].body.end = token.span.start;
                blocks[i].end = token.span.clone();
                if token.kind == TokenKind::CloseBrace {
                    conditional_bindings.leave();
                }
                pending = blocks[i].parent.filter(|&p| {
                    matches!(
                        blocks[p].kind,
                        BlockKind::Command
                            | BlockKind::Item
                            | BlockKind::Environment
                            | BlockKind::Heading
                    )
                });
                if let Some(p) = pending.filter(|&p| {
                    blocks[p].kind == BlockKind::Command || blocks[p].kind == BlockKind::Heading
                }) {
                    blocks[p].body.end = blocks[p].body.end.max(token.span.end);
                    blocks[p].end = blocks[p].body.end..blocks[p].body.end;
                }
            }
            TokenKind::Command => {
                let cached_constructed = constructed_commands.get(&token.span.start);
                let literal_constructed = cached_constructed
                    .is_none()
                    .then(|| literal_csname(source, token.span.start))
                    .flatten();
                let constructed = cached_constructed.or(literal_constructed.as_ref());
                let (raw_name, raw_end) =
                    command_at(source, token.span.start).expect("command token");
                let name = constructed.map_or(raw_name, |(name, _)| name.as_str());
                let base = name.trim_end_matches('*');
                conditional_bindings.observe(
                    source,
                    name,
                    constructed.map_or(raw_end, |(_, finish)| *finish),
                    &config.config,
                );
                let meaning = conditional_bindings.meaning(base, &config.config);
                if new_if {
                    known_conditions.insert(base.into());
                    conditional_bindings.assign(base, ConditionalMeaning::Other, false);
                    new_if = false;
                    pending = None;
                    continue;
                }
                if base == "newif" {
                    new_if = true;
                    pending = None;
                    continue;
                }
                if matches!(base, "bgroup" | "begingroup") {
                    conditional_bindings.enter();
                    let i = add!(BlockKind::Group, names.make(base), parent);
                    stack.push(i);
                    pending = None;
                    continue;
                }
                if matches!(base, "egroup" | "endgroup") {
                    let position = stack
                        .iter()
                        .rposition(|&i| blocks[i].kind != BlockKind::Condition)
                        .ok_or_else(|| {
                            FormatError::Syntax(format!(
                                "unexpected \\{base} at byte {}",
                                token.span.start
                            ))
                        })?;
                    let i = stack[position];
                    let correct = if base == "endgroup" {
                        blocks[i].kind == BlockKind::Group && blocks[i].name == "begingroup"
                    } else {
                        source.as_bytes()[blocks[i].begin.start] == b'{'
                            || blocks[i].kind == BlockKind::Group && blocks[i].name == "bgroup"
                    };
                    if !correct {
                        return Err(FormatError::Syntax(format!(
                            "mismatched \\{base} at byte {}",
                            token.span.start
                        )));
                    }
                    stack.remove(position);
                    conditional_bindings.leave();
                    blocks[i].body.end = token.span.start;
                    blocks[i].end = token.span.clone();
                    pending = None;
                    continue;
                }
                if base == "loop" {
                    let i = add!(BlockKind::Loop, names.make(base), parent);
                    loop_conditions.insert(
                        i,
                        stack
                            .iter()
                            .filter(|&&i| blocks[i].kind == BlockKind::Condition)
                            .count(),
                    );
                    stack.push(i);
                    pending = None;
                    continue;
                }
                if base == "repeat" {
                    let position = stack
                        .iter()
                        .rposition(|&i| blocks[i].kind != BlockKind::Condition)
                        .ok_or_else(|| {
                            FormatError::Syntax(format!(
                                "unexpected \\repeat at byte {}",
                                token.span.start
                            ))
                        })?;
                    let i = stack[position];
                    let conditions: Vec<usize> = stack
                        .iter()
                        .copied()
                        .filter(|&open| {
                            blocks[open].kind == BlockKind::Condition
                                && blocks[open].begin.start > blocks[i].begin.start
                        })
                        .collect();
                    let expected = loop_conditions.get(&i).copied().unwrap_or(0) + 1;
                    if blocks[i].kind != BlockKind::Loop
                        || conditions.len() != 1
                        || stack
                            .iter()
                            .filter(|&&open| blocks[open].kind == BlockKind::Condition)
                            .count()
                            != expected
                    {
                        return Err(FormatError::Syntax(format!(
                            "\\repeat requires one unfinished loop condition at byte {}",
                            token.span.start
                        )));
                    }
                    let condition = conditions[0];
                    stack.retain(|&open| open != i && open != condition);
                    blocks[condition].body.end = token.span.start;
                    blocks[condition].end = token.span.start..token.span.start;
                    blocks[i].body.end = token.span.start;
                    blocks[i].end = token.span.clone();
                    pending = None;
                    continue;
                }
                if matches!(
                    meaning,
                    ConditionalMeaning::Else | ConditionalMeaning::Or | ConditionalMeaning::Fi
                ) {
                    let operation = match meaning {
                        ConditionalMeaning::Else => "else",
                        ConditionalMeaning::Or => "or",
                        _ => "fi",
                    };
                    let position = stack
                        .iter()
                        .rposition(|&i| blocks[i].kind == BlockKind::Condition)
                        .ok_or_else(|| {
                            FormatError::Syntax(format!(
                                "unexpected \\{base} at byte {}",
                                token.span.start
                            ))
                        })?;
                    let i = stack[position];
                    if operation == "fi" {
                        stack.remove(position);
                        blocks[i].body.end = token.span.start;
                        blocks[i].end = token.span.clone();
                    } else if operation == "or"
                        && primitive_condition(&blocks[i].name)
                        && condition_meanings.get(&i) != Some(&ConditionalMeaning::Case)
                    {
                        return Err(FormatError::Syntax(format!(
                            "unexpected \\or for \\{} at byte {}",
                            blocks[i].name, token.span.start
                        )));
                    } else if blocks[i].branches.iter().any(|(is_or, _)| !is_or) {
                        return Err(FormatError::Syntax(format!(
                            "unexpected \\{base} after \\else at byte {}",
                            token.span.start
                        )));
                    } else {
                        blocks[i]
                            .branches
                            .push((operation == "or", token.span.clone()));
                    }
                    pending = None;
                    continue;
                }
                let matched =
                    |regex: &Option<Arc<fancy_regex::Regex>>| -> Result<bool, FormatError> {
                        regex.as_ref().map_or(Ok(false), |re| {
                            re.is_match(name)
                                .map_err(|e| FormatError::Syntax(e.to_string()))
                        })
                    };
                let primitive = !conditional_bindings.assigned(base)
                    && inferred_condition(base, source, token.span.end);
                if matches!(
                    meaning,
                    ConditionalMeaning::True
                        | ConditionalMeaning::False
                        | ConditionalMeaning::Case
                        | ConditionalMeaning::Other
                ) || known_conditions.contains(base) && !conditional_bindings.assigned(base)
                    || primitive
                    || matched(&config.syntax.condition)?
                {
                    let i = add!(BlockKind::Condition, names.make(name), parent);
                    condition_meanings.insert(i, meaning);
                    stack.push(i);
                    pending = None;
                } else if base == "[" || base == "(" {
                    let i = add!(
                        BlockKind::Special,
                        names.make(if base == "[" {
                            "displayMath"
                        } else {
                            "inlineMath"
                        }),
                        parent
                    );
                    stack.push(i);
                    pending = None;
                } else if base == "]" || base == ")" {
                    let position = stack
                        .iter()
                        .rposition(|&i| blocks[i].kind != BlockKind::Condition)
                        .ok_or_else(|| {
                            FormatError::Syntax(format!(
                                "unexpected math terminator at byte {}",
                                token.span.start
                            ))
                        })?;
                    let i = stack.remove(position);
                    let expected = if base == "]" {
                        "displayMath"
                    } else {
                        "inlineMath"
                    };
                    if blocks[i].kind != BlockKind::Special || blocks[i].name != expected {
                        return Err(FormatError::Syntax(format!(
                            "mismatched math terminator at byte {}",
                            token.span.start
                        )));
                    }
                    blocks[i].body.end = token.span.start;
                    blocks[i].end = token.span.clone();
                    conditional_bindings.leave();
                    pending = None;
                } else {
                    let explicit_item = config.config.syntax.items.iter().any(|n| n == base)
                        || matched(&config.syntax.item)?;
                    let environment = stack
                        .iter()
                        .rev()
                        .find(|&&i| blocks[i].kind == BlockKind::Environment)
                        .map(|&i| blocks[i].name.as_str());
                    let package_item = !explicit_item
                        && environment.is_some_and(|environment| {
                            crate::package_environments::is_package_item(base, environment)
                        })
                        && !config.config.syntax.commands.contains_key(base)
                        && !analysis.declared_commands()?.contains(base);
                    let in_list = (explicit_item || package_item)
                        && environment.is_some_and(|name| list_environments.contains(name));
                    let kind = if in_list {
                        BlockKind::Item
                    } else if config.config.syntax.headings.contains_key(base) {
                        BlockKind::Heading
                    } else {
                        BlockKind::Command
                    };
                    let i = add!(kind, names.make(name), parent);
                    if kind == BlockKind::Command || kind == BlockKind::Heading {
                        blocks[i].body.end = token.span.end;
                        blocks[i].end = token.span.end..token.span.end;
                    }
                    let declared_rule = if config.config.syntax.commands.contains_key(base) {
                        None
                    } else {
                        analysis.command_arguments(base)
                    };
                    let user_signature = config
                        .config
                        .syntax
                        .commands
                        .get(base)
                        .and_then(|rule| rule.signature.as_deref())
                        .or_else(|| {
                            declared_rule
                                .as_deref()
                                .and_then(|rule| rule.signature.as_deref())
                        });
                    let package_signature =
                        match crate::packages::command_signature(source, name, token.span.end) {
                            Some(signature)
                                if matches!(kind, BlockKind::Command | BlockKind::Heading)
                                    && !config.config.syntax.commands.contains_key(base)
                                    && !analysis.declared_commands()?.contains(base) =>
                            {
                                Some(signature)
                            }
                            _ => None,
                        };
                    if let Some(signature) = user_signature.or(package_signature) {
                        let limit = stack
                            .iter()
                            .rev()
                            .find_map(|&parent| {
                                (blocks[parent].kind == BlockKind::Environment)
                                    .then(|| {
                                        environment_ends.get(&blocks[parent].begin.start).copied()
                                    })
                                    .flatten()
                            })
                            .unwrap_or(source.len());
                        let start = token.span.end
                            - usize::from(user_signature.is_some() && name.ends_with('*'));
                        let ranges = if xparse::extended(signature) {
                            xparse::Signature::parse(signature)
                                .and_then(|signature| signature.ranges(source, start..limit))
                                .unwrap_or_default()
                        } else {
                            crate::layout::signature_argument_ranges(
                                source,
                                start..limit,
                                signature.bytes(),
                            )
                        };
                        if let Some(last) = ranges.last() {
                            blocks[i].body.end = last.end;
                            blocks[i].end = last.end..last.end;
                        }
                        command_headers.insert(i, ranges);
                    }
                    pending = if config.syntax.command.as_ref().map_or(Ok(true), |re| {
                        re.is_match(name)
                            .map_err(|e| FormatError::Syntax(e.to_string()))
                    })? {
                        Some(i)
                    } else {
                        None
                    };
                }
            }
            TokenKind::Math => {
                let name = if source[token.span.clone()].len() == 2 {
                    "displayMathTeX"
                } else {
                    "inlineMath"
                };
                let structural_position = stack
                    .iter()
                    .rposition(|&i| blocks[i].kind != BlockKind::Condition);
                let structural_parent = structural_position.map(|position| stack[position]);
                if structural_parent
                    .is_some_and(|i| blocks[i].kind == BlockKind::Special && blocks[i].name == name)
                {
                    let i = stack.remove(structural_position.expect("math stack"));
                    conditional_bindings.leave();
                    blocks[i].body.end = token.span.start;
                    blocks[i].end = token.span.clone();
                } else {
                    let i = add!(BlockKind::Special, names.make(name), parent);
                    stack.push(i);
                    conditional_bindings.enter();
                }
                pending = None;
            }
            TokenKind::SpecialBegin(rule) => {
                let i = add!(
                    BlockKind::Special,
                    names.make(config.config.syntax.special[rule].name.as_str()),
                    parent
                );
                stack.push(i);
                pending = None;
            }
            TokenKind::SpecialEnd(rule) => {
                let i = stack
                    .pop()
                    .ok_or_else(|| FormatError::Syntax("unexpected special block end".into()))?;
                if blocks[i].kind != BlockKind::Special
                    || blocks[i].name != config.config.syntax.special[rule].name.as_str()
                {
                    return Err(FormatError::Syntax("mismatched special block end".into()));
                }
                blocks[i].body.end = token.span.start;
                blocks[i].end = token.span.clone();
                pending = None;
            }
            TokenKind::SpecialMiddle(rule) => {
                let i = stack
                    .last()
                    .copied()
                    .ok_or_else(|| FormatError::Syntax("unexpected special block middle".into()))?;
                if blocks[i].kind != BlockKind::Special
                    || blocks[i].name != config.config.syntax.special[rule].name.as_str()
                {
                    // A matching marker inside an ordinary argument, math or
                    // child environment belongs to that nested scope.
                    pending = None;
                    continue;
                }
                blocks[i].branches.push((false, token.span.clone()));
                pending = None;
            }
            TokenKind::Text => {
                let mut keep = pending.is_some_and(|i| {
                    command_headers
                        .get(&i)
                        .and_then(|ranges| ranges.last())
                        .is_some_and(|last| {
                            token.span.start < last.end
                                && (token.span.end <= last.end
                                    || source[last.end..token.span.end].trim().is_empty())
                        })
                });
                if pending.is_some()
                    && let Some(re) = &config.syntax.between
                {
                    let text = &source[token.span.clone()];
                    let mut pos = 0usize;
                    loop {
                        pos = skip_space(text, pos);
                        if pos == text.len() {
                            keep = true;
                            break;
                        }
                        let matched = re
                            .find_from_pos(text, pos)
                            .map_err(|e| FormatError::Syntax(e.to_string()))?;
                        if let Some(m) = matched.filter(|m| m.start() == pos && m.end() > pos) {
                            pos = m.end();
                        } else {
                            break;
                        }
                    }
                }
                if !keep {
                    pending = None;
                }
            }
            TokenKind::Protected => {
                if token_data_range(source, &token.span) {
                    let mut cursor = token.span.start;
                    while cursor < token.span.end {
                        let (name, end) = command_at(source, cursor).expect("token-data command");
                        conditional_bindings.observe(source, name, end, &config.config);
                        cursor = primitive_data_end(source, name, end)
                            .or_else(|| let_declaration_end(source, name, end))
                            .expect("token-data end");
                    }
                }
                // Package metadata can cover complete leading arguments in a
                // single opaque token. The invocation's fixed ranges still
                // identify a later printable argument; retain its command
                // owner until that argument opens. Exact range matching in
                // OpenBrace/OpenBracket keeps following free groups separate.
                pending = pending.filter(|&i| {
                    token.span.start >= blocks[i].begin.end
                        && command_headers.get(&i).is_some_and(|ranges| {
                            ranges.iter().any(|range| range.start >= token.span.end)
                        })
                });
            }
            _ => (),
        }
    }
    if let Some(&i) = stack.last() {
        if let Some(suffix) = opaque_suffix(source, protected, &config.config) {
            // An opaque suffix can contain the closing tokens of the already
            // parsed prefix. Stop its formatting scopes at the protected boundary.
            for &open in &stack {
                blocks[open].body.end = suffix.start;
                blocks[open].end = suffix.start..suffix.start;
            }
        } else {
            return Err(FormatError::Syntax(format!(
                "unclosed {} {:?} at byte {}",
                blocks[i].kind.category(),
                blocks[i].name,
                blocks[i].begin.start
            )));
        }
    }
    // TeX conditionals and groups have independent lifetimes. Crossing scopes
    // share their enclosing parent so indentation only uses containing scopes.
    for i in 0..blocks.len() {
        if matches!(blocks[i].kind, BlockKind::Item | BlockKind::Heading) {
            continue;
        }
        while let Some(parent) = blocks[i].parent {
            if blocks[parent].begin.start <= blocks[i].begin.start
                && blocks[i].end.end <= blocks[parent].end.end
            {
                break;
            }
            blocks[i].parent = blocks[parent].parent;
        }
    }
    // Items and headings span following content until a sibling or enclosing end.
    let mut next_item: HashMap<Option<usize>, usize> = HashMap::new();
    let mut next_headings: HashMap<Option<usize>, Vec<(usize, usize)>> = HashMap::new();
    for i in (0..blocks.len()).rev() {
        if blocks[i].kind == BlockKind::Item {
            let parent = blocks[i].parent;
            let end = next_item
                .get(&parent)
                .copied()
                .unwrap_or_else(|| parent.map_or(source.len(), |p| blocks[p].body.end));
            blocks[i].body.end = end;
            blocks[i].end = end..end;
            next_item.insert(parent, blocks[i].begin.start);
        } else if blocks[i].kind == BlockKind::Heading {
            let base = blocks[i].name.trim_end_matches('*');
            let level = config
                .config
                .syntax
                .headings
                .get(base)
                .map_or(1, |h| h.level);
            let headings = next_headings.entry(blocks[i].parent).or_default();
            while headings
                .last()
                .is_some_and(|&(next_level, _)| next_level > level)
            {
                headings.pop();
            }
            let end = headings.last().map(|&(_, start)| start).unwrap_or_else(|| {
                blocks[i]
                    .parent
                    .map_or(source.len(), |p| blocks[p].body.end)
            });
            blocks[i].body.start = blocks[i].body.end;
            blocks[i].body.end = end;
            blocks[i].end = end..end;
            headings.push((level, blocks[i].begin.start));
        }
    }
    crate::foreach::add_blocks(source, tokens, &mut blocks, config, protected, analysis)?;
    crate::tikz::add_blocks(source, tokens, &mut blocks, config, protected, analysis)?;
    Ok(blocks)
}

fn declaration_reuse_config_supported(config: &Config) -> bool {
    let syntax = &config.syntax;
    syntax.command_pattern.is_none()
        && syntax.environment_pattern.is_none()
        && syntax.condition_pattern.is_none()
        && syntax.item_pattern.is_none()
        && syntax.argument_between.is_none()
        && syntax.special.is_empty()
        && syntax.conditions.is_empty()
        && !syntax
            .commands
            .values()
            .any(|rule| rule.preserve_body || rule.verbatim)
}

fn declaration_cross_segment_command(command: &str) -> bool {
    command != "iff" && command.starts_with("if")
        || command.starts_with("@if")
        || matches!(
            command,
            "fi" | "else"
                | "or"
                | "unless"
                | "newif"
                | "loop"
                | "repeat"
                | "csname"
                | "endcsname"
                | "catcode"
                | "obeyspaces"
                | "obeylines"
                | "scantokens"
                | "ExplSyntaxOn"
                | "ExplSyntaxOff"
        )
}

fn declaration_ordinary_binding_name(name: &str) -> bool {
    !name.ends_with('*')
        && !declaration_cross_segment_command(name)
        && !context_builtin_command(name)
        && !matches!(
            name,
            "futurelet"
                | "global"
                | "long"
                | "outer"
                | "protected"
                | "loop"
                | "repeat"
                | "bgroup"
                | "egroup"
                | "begingroup"
                | "endgroup"
                | "documentclass"
                | "usepackage"
                | "RequirePackage"
                | "LoadClass"
                | "LoadClassWithOptions"
                | "RequirePackageWithOptions"
                | "meaning"
                | "show"
                | "detokenize"
                | "showtokens"
        )
}

fn declaration_stable_let_end(source: &str, end: usize) -> Option<usize> {
    let (target, target_end) = command_at(source, skip_trivia(source, end))?;
    if !declaration_ordinary_binding_name(target) {
        return None;
    }
    let mut operand = skip_trivia(source, target_end);
    if source.as_bytes().get(operand) == Some(&b'=') {
        operand = skip_trivia(source, operand + 1);
    }
    let (value, finish) = command_at(source, operand)?;
    declaration_ordinary_binding_name(value).then_some(finish)
}

/// Whether an already inferred project prefix has a stable declaration
/// boundary. Open ordinary groups and environments retain their scope; token
/// operands, declarations and protected content must be complete. This is a
/// conservative cache predicate and never changes formatter parsing.
pub fn declaration_reuse_boundary(source: &str, config: &Config) -> bool {
    declaration_reuse_boundary_status(source, config) == DeclarationReuseBoundary::Safe
}

/// Internal project-cache distinction; a refused policy never reuses options.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclarationReuseBoundary {
    Safe,
    Pending,
    PolicyRefused,
}

#[doc(hidden)]
pub fn declaration_reuse_boundary_status(
    source: &str,
    config: &Config,
) -> DeclarationReuseBoundary {
    use DeclarationReuseBoundary::Pending;
    if !declaration_reuse_config_supported(config) || !source.is_empty() && !source.ends_with('\n')
    {
        return Pending;
    }
    let Ok(ranges) = protected_ranges(source, config) else {
        return Pending;
    };
    declaration_reuse_boundary_with_ranges(source, config, &ranges)
}

fn declaration_reuse_boundary_with_ranges(
    source: &str,
    config: &Config,
    ranges: &[Range<usize>],
) -> DeclarationReuseBoundary {
    use DeclarationReuseBoundary::{Pending, PolicyRefused, Safe};
    if !declaration_reuse_config_supported(config) || !source.is_empty() && !source.ends_with('\n')
    {
        return Pending;
    }
    // EOF protection may contain a pending raw environment, catcode region,
    // formatter-off block or macro body completed by a later file.
    if ranges.last().is_some_and(|range| range.end == source.len()) {
        return Pending;
    }
    let mut cursor = 0;
    let mut conditions: Vec<(bool, bool)> = Vec::new();
    while cursor < source.len() {
        let index = ranges.partition_point(|range| range.end <= cursor);
        if let Some(range) = ranges.get(index).filter(|range| range.start <= cursor) {
            if range.start == cursor
                && let Some((name, end)) = command_at(source, cursor)
                && (name == "futurelet"
                    || name == "let" && declaration_stable_let_end(source, end).is_none())
            {
                let complete = if name == "futurelet" {
                    tex_token_end(source, end)
                        .and_then(|target| tex_token_end(source, target))
                        .and_then(|first| tex_token_end(source, first))
                        .is_some()
                } else {
                    let_declaration_end(source, name, end).is_some()
                };
                return if complete { PolicyRefused } else { Pending };
            }
            cursor = range.end;
            continue;
        }
        if source.as_bytes()[cursor] == b'%' {
            cursor = line_end(source, cursor);
            continue;
        }
        if source.as_bytes()[cursor] != b'\\' {
            cursor += source[cursor..]
                .chars()
                .next()
                .expect("source character")
                .len_utf8();
            continue;
        }
        let Some((name, end)) = command_at(source, cursor) else {
            return Pending;
        };
        let base = name.trim_end_matches('*');
        if base == "newif" {
            if config.syntax.commands.contains_key(base) {
                return PolicyRefused;
            }
            let Some((target, finish)) = command_at(source, skip_trivia(source, end)) else {
                return Pending;
            };
            if !inferred_condition(target, "", 0)
                || target.ends_with('*')
                || primitive_condition(target)
            {
                return PolicyRefused;
            }
            cursor = finish;
            continue;
        }
        if inferred_condition(base, source, end) {
            conditions.push((base == "ifcase", false));
            cursor = primitive_test_end(source, base, end)
                .or_else(|| next_character_test_end(source, base, end))
                .unwrap_or(end);
            continue;
        }
        if base == "fi" {
            if conditions.pop().is_none() {
                return Pending;
            }
            cursor = end;
            continue;
        }
        if matches!(base, "else" | "or") {
            let Some((case, had_else)) = conditions.last_mut() else {
                return Pending;
            };
            if *had_else || base == "or" && !*case {
                return Pending;
            }
            *had_else = base == "else";
            cursor = end;
            continue;
        }
        if declaration_cross_segment_command(base) {
            return if base == "csname" && constructed_csname(source, cursor).is_none() {
                Pending
            } else {
                PolicyRefused
            };
        }
        let finish = if latex_macro_definition(base)
            || matches!(base, "def" | "gdef" | "edef" | "xdef" | "newcolumntype")
        {
            let mut target = skip_trivia(source, end);
            if source.as_bytes().get(target) == Some(&b'{') {
                target = skip_trivia(source, target + 1);
            }
            if command_at(source, target)
                .is_some_and(|(name, _)| !declaration_ordinary_binding_name(name))
            {
                return if macro_body(source, base, end).is_some() {
                    PolicyRefused
                } else {
                    Pending
                };
            }
            macro_body(source, base, end).map(|body| body.end)
        } else if base == "let" {
            declaration_stable_let_end(source, end)
        } else if base == "futurelet" {
            None
        } else if matches!(base, "string" | "meaning" | "show" | "noexpand") {
            primitive_data_end(source, name, end).or_else(|| {
                // A character after noexpand still executes structurally.
                (base == "noexpand" && tex_token_end(source, end).is_some()).then_some(end)
            })
        } else if matches!(base, "detokenize" | "showtokens") {
            primitive_literal_argument_end(source, name, end)
        } else if matches!(base, "begin" | "end") {
            env_at(source, cursor).map(|(_, _, finish)| finish)
        } else if base == "newlist" {
            let mut finish = end;
            (0..3)
                .all(|_| context_argument(source, &mut finish, b'{', b'}').is_some())
                .then_some(finish)
        } else if has_context_declaration(&source[cursor..end]) {
            context_box_declaration(source, cursor, base, end)
                .map(|declaration| declaration.span.end)
                .or_else(|| {
                    crate::package_environments::verbatim_declaration(source, base, end)
                        .map(|declaration| declaration.end)
                })
        } else {
            Some(end)
        };
        let Some(finish) = finish else {
            return Pending;
        };
        cursor = finish;
    }
    if conditions.is_empty() { Safe } else { Pending }
}

/// Whether a complete independent project segment preserves every inferred
/// declaration and enclosing scope. Uncertain custom syntax and any declaration
/// or token-reading state use the complete project scanner instead.
pub fn declaration_neutral_segment(source: &str, config: &Config) -> bool {
    let Ok(compiled) = compile(&config.syntax) else {
        return false;
    };
    declaration_neutral_segment_with_syntax(source, config, &compiled)
}

/// Check a project segment using syntax already compiled for its configuration.
pub fn declaration_neutral_segment_with_syntax(
    source: &str,
    config: &Config,
    compiled: &CompiledSyntax,
) -> bool {
    if !declaration_reuse_config_supported(config) {
        return false;
    }
    if source.contains("tift:") || source.contains("tex-fmt:") {
        return false;
    }
    if source.trim().is_empty() {
        return true;
    }
    // Reject an early declaration before allocating a full protection scan.
    // Once a possible literal opener is seen, its body needs the range-aware
    // check below because documentation examples can contain declarations.
    for (start, _) in source.match_indices('\\') {
        let Some((name, _)) = command_at(source, start) else {
            return false;
        };
        let base = name.trim_end_matches('*');
        if base == "begin"
            || config
                .protect
                .commands
                .iter()
                .any(|command| command == base || command == name)
            || config
                .syntax
                .commands
                .get(base)
                .is_some_and(|rule| rule.verbatim || rule.preserve_body)
        {
            break;
        }
        if context_declaration_command(base) {
            return false;
        }
    }
    let Ok(ranges) = protected_ranges_with_syntax(source, config, compiled) else {
        return false;
    };
    if ranges.last().is_some_and(|range| range.end == source.len()) {
        return false;
    }
    let mut protected_index = 0;
    for (start, _) in source.match_indices('\\') {
        // Literal examples can contain declaration and reader tokens. Their
        // bodies cannot change the state inherited by the following file.
        // Keep checking the opener itself: a protected macro declaration or
        // catcode command still changes that state.
        while ranges
            .get(protected_index)
            .is_some_and(|range| range.end <= start)
        {
            protected_index += 1;
        }
        if ranges
            .get(protected_index)
            .is_some_and(|range| range.start < start)
        {
            continue;
        }
        let Some((name, end)) = command_at(source, start) else {
            return false;
        };
        let base = name.trim_end_matches('*');
        let conditional =
            inferred_condition(base, source, end) || matches!(base, "fi" | "else" | "or");
        if context_declaration_command(base)
            || declaration_cross_segment_command(base) && !conditional
            || matches!(
                base,
                "string"
                    | "meaning"
                    | "show"
                    | "noexpand"
                    | "detokenize"
                    | "showtokens"
                    | "newcolumntype"
            )
        {
            return false;
        }
    }
    let Ok(tokens) = lex_with_ranges(source, config, compiled, &ranges) else {
        return false;
    };
    let mut groups = 0usize;
    let mut brackets = 0usize;
    let mut environments = Vec::new();
    struct NeutralCondition<'a> {
        case: bool,
        had_else: bool,
        groups: usize,
        brackets: usize,
        environments: Vec<&'a str>,
    }
    let mut conditions: Vec<NeutralCondition<'_>> = Vec::new();
    for token in tokens {
        match token.kind {
            TokenKind::OpenBrace => groups += 1,
            TokenKind::CloseBrace => {
                let Some(depth) = groups.checked_sub(1) else {
                    return false;
                };
                groups = depth;
            }
            TokenKind::OpenBracket => brackets += 1,
            TokenKind::CloseBracket => {
                let Some(depth) = brackets.checked_sub(1) else {
                    return false;
                };
                brackets = depth;
            }
            TokenKind::BeginEnvironment | TokenKind::EndEnvironment => {
                let Some((name, begin, _)) = env_at(source, token.span.start) else {
                    return false;
                };
                if begin {
                    environments.push(name);
                } else if environments.pop() != Some(name) {
                    return false;
                }
            }
            TokenKind::Command => {
                let Some((name, end)) = command_at(source, token.span.start) else {
                    return false;
                };
                let base = name.trim_end_matches('*');
                // The project declaration graph expands loaders only outside
                // groups, conditions and ordinary environments. Scoped loaders
                // remain literal source in that graph; they need not invalidate
                // an otherwise independent, declaration-free segment. Document
                // wrappers retain top-level loader behavior.
                if matches!(
                    base,
                    "input"
                        | "include"
                        | "documentclass"
                        | "usepackage"
                        | "RequirePackage"
                        | "LoadClass"
                        | "LoadClassWithOptions"
                        | "RequirePackageWithOptions"
                ) && groups == 0
                    && brackets == 0
                    && conditions.is_empty()
                    && environments.iter().all(|name| *name == "document")
                {
                    return false;
                }
                if inferred_condition(base, source, end) {
                    conditions.push(NeutralCondition {
                        case: base == "ifcase",
                        had_else: false,
                        groups,
                        brackets,
                        environments: environments.clone(),
                    });
                    continue;
                }
                if matches!(base, "fi" | "else" | "or") {
                    let Some(condition) = conditions.last_mut() else {
                        return false;
                    };
                    if groups != condition.groups
                        || brackets != condition.brackets
                        || environments != condition.environments
                    {
                        return false;
                    }
                    if base == "fi" {
                        conditions.pop();
                    } else {
                        if condition.had_else || base == "or" && !condition.case {
                            return false;
                        }
                        condition.had_else = base == "else";
                    }
                    continue;
                }
                match base {
                    "begingroup" | "bgroup" => groups += 1,
                    "endgroup" | "egroup" => {
                        let Some(depth) = groups.checked_sub(1) else {
                            return false;
                        };
                        groups = depth;
                    }
                    "begin" | "end" => return false,
                    _ => (),
                }
            }
            _ => (),
        }
    }
    groups == 0 && brackets == 0 && environments.is_empty() && conditions.is_empty()
}



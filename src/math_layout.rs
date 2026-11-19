//! Lossless mathematical whitespace edits and native display-line planning.

use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::ops::Range;
use unicode_width::UnicodeWidthStr;

mod modes;
mod symbols;

use crate::{CompiledConfig, FileKind, FormatError, code_layout, syntax};
use modes::MathModes;
use symbols::{DelimiterRole, MathClass};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct MathConfig {
    /// Master switch for mathematical whitespace and display layout.
    pub enabled: bool,
    /// Normalize horizontal whitespace inside recognized mathematics.
    pub spacing: bool,
    /// Layout policy for single-formula displays; inline and grid math stay atomic.
    pub wrap: MathWrap,
    /// Display column budget. None inherits `wrap.columns`.
    pub columns: Option<usize>,
    /// Preferred operator classes, highest first. Empty keeps greedy wrapping.
    pub break_priority: Vec<MathBreakKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MathBreakKind {
    Relation,
    Additive,
    Multiplicative,
    Binary,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MathWrap {
    Preserve,
    SingleLine,
    #[default]
    Break,
}

impl Default for MathConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            spacing: true,
            wrap: MathWrap::Break,
            columns: None,
            break_priority: Vec::new(),
        }
    }
}

pub fn validate(config: &MathConfig) -> Result<(), String> {
    for (index, kind) in config.break_priority.iter().enumerate() {
        if config.break_priority[..index].contains(kind) {
            return Err("math.break-priority must not contain duplicate operator classes".into());
        }
    }
    if config
        .columns
        .is_some_and(|columns| !(1..=1_000_000).contains(&columns))
    {
        return Err("math.columns must be between 1 and 1000000".into());
    }
    Ok(())
}

pub(crate) fn is_active(source: &str, config: &CompiledConfig) -> bool {
    config.config.math.enabled
        && (!config.config.syntax.environment_aliases.is_empty()
            || !config.config.syntax.inferred_environment_aliases.is_empty()
            || !config.config.syntax.environment_alias_events.is_empty()
            || source.contains('$')
            || memchr::memchr_iter(b'\\', source.as_bytes()).any(|start| {
                command_at(source, start).is_some_and(|(name, end)| {
                    matches!(name, "(" | "[")
                        || matches!(name, "hypo" | "infer" | "ellipsis" | "delims")
                        || matches!(
                            name,
                            "NewDerivative"
                                | "RenewDerivative"
                                | "ProvideDerivative"
                                | "DeclareDerivative"
                                | "NewDifferential"
                                | "RenewDifferential"
                                | "ProvideDifferential"
                                | "DeclareDifferential"
                        )
                        || modes::command_signature(name)
                            .is_some_and(|signature| signature.domains.contains(['M', 'I']))
                        || name == "begin"
                            && source[end..]
                                .trim_start()
                                .strip_prefix('{')
                                .and_then(|rest| rest.split_once('}'))
                                .is_some_and(|(name, _)| modes::is_math_environment(name))
                })
            })
            || crate::packages::math::has_extensions(source))
}

pub(crate) fn format_math_with_context(
    source: &str,
    kind: FileKind,
    config: &CompiledConfig,
    parsing: &mut crate::parse_context::ParseContext<'_>,
) -> Result<String, FormatError> {
    let formatted_bodies = if config.config.math.enabled
        && config.config.code.expl3
        && config.config.protect.format_macro_bodies
        && matches!(kind, FileKind::Tex | FileKind::Style | FileKind::Class)
        && (source.contains("\\ExplSyntaxOn") || source.contains("\\ProvidesExpl"))
    {
        let protected = parsing.expl3_protected_ranges(source)?;
        let parsed = parsing.snapshot(source, false, false)?;
        let model = parsed
            .metadata
            .exposed_model(source, &config.config, &protected);
        let mut output = String::new();
        let mut cursor = 0;
        for declaration in model.declarations() {
            if declaration.body.start < cursor
                || !syntax::command_at(source, declaration.span.start)
                    .is_some_and(|(name, _)| name.starts_with("cs_"))
            {
                continue;
            }
            let body = &source[declaration.body.clone()];
            let mut inner = crate::parse_context::ParseContext::new(config);
            let Ok(formatted) = format_math_with_context(body, kind, config, &mut inner) else {
                continue;
            };
            if formatted != body {
                output.push_str(&source[cursor..declaration.body.start]);
                output.push_str(&formatted);
                cursor = declaration.body.end;
            }
        }
        if cursor > 0 {
            output.push_str(&source[cursor..]);
            Some(output)
        } else {
            None
        }
    } else {
        None
    };
    format_math_snapshot(
        formatted_bodies.as_deref().unwrap_or(source),
        kind,
        config,
        parsing,
    )
}

fn format_math_snapshot(
    source: &str,
    kind: FileKind,
    config: &CompiledConfig,
    parsing: &mut crate::parse_context::ParseContext<'_>,
) -> Result<String, FormatError> {
    if !config.config.math.enabled || kind == FileKind::Bibtex {
        return Ok(source.into());
    }
    if kind == FileKind::Dtx {
        return format_documented_math(source, config, parsing);
    }
    let mut protected = math_protection(source, config, parsing)?;
    if kind == FileKind::Ins {
        protected.extend(crate::docstrip::preamble_ranges(source, &protected));
        protected = code_layout::merged(protected);
    }
    let mut modes = MathModes::build(source, config, &protected, parsing)?;
    let mut editor = Editor {
        source,
        config,
        modes: &modes,
        protected: &protected,
        edits: Vec::new(),
        visited: HashSet::new(),
    };
    if config.config.math.spacing {
        for region in &modes.regions {
            if valid_sized_delimiters(source, &region.body, &modes) {
                editor.sequence(region.body.clone(), false, region.text_cases);
            }
        }
    }
    if config.config.math.wrap == MathWrap::Preserve
        || !modes.regions.iter().any(|region| region.display.is_some())
    {
        return Ok(editor.finish(source));
    }
    if editor.edits.is_empty() {
        return layout_displays(source, config, &modes, &protected);
    }
    let (normalized, positions) = editor.finish_mapped(source);
    if normalized == source {
        return layout_displays(source, config, &modes, &protected);
    }
    // Whitespace edits preserve math tokens and their argument domains. Inline
    // documents need no display pass; displays reuse the already parsed modes
    // after projecting their byte positions through the effective edits.
    if modes.remap(&positions).is_some()
        && protected
            .iter_mut()
            .try_for_each(|range| positions.remap_range(range))
            .is_some()
    {
        return layout_displays(&normalized, config, &modes, &protected);
    }
    break_displays(&normalized, kind, config, parsing)
}

/// Effective, nonoverlapping whitespace edits in source order. A boundary
/// inside replaced whitespace is ambiguous and requests the full-parser path.
struct PositionMap {
    edits: Vec<(Range<usize>, usize)>,
}

impl PositionMap {
    fn position(&self, at: usize, after_insertion: bool) -> Option<usize> {
        let index = self.edits.partition_point(|(range, _)| {
            range.end < at || range.end == at && (!range.is_empty() || after_insertion)
        });
        if self
            .edits
            .get(index)
            .is_some_and(|(range, _)| range.start < at && at < range.end)
        {
            return None;
        }
        match index.checked_sub(1).map(|index| &self.edits[index]) {
            Some((range, new_end)) => new_end.checked_add(at - range.end),
            None => Some(at),
        }
    }

    fn remap_range(&self, range: &mut Range<usize>) -> Option<()> {
        // Inserted gaps at a token boundary belong outside that token. This
        // also keeps an unbraced argument from absorbing the following space.
        let start = self.position(range.start, range.start != range.end)?;
        let end = self.position(range.end, false)?;
        if start > end {
            return None;
        }
        *range = start..end;
        Some(())
    }
}

fn format_documented_math(
    source: &str,
    config: &CompiledConfig,
    parsing: &mut crate::parse_context::ParseContext<'_>,
) -> Result<String, FormatError> {
    // Executable code and documentation carry independent TeX state. A code
    // catcode assignment or open group cannot hide documentation environments.
    let code = format_math_with_context(source, FileKind::Style, config, parsing)?;
    let parsed = parsing.snapshot(&code, false, false)?;
    let analysis = crate::source_analysis::SourceAnalysis::with_memo(
        &code,
        &config.config,
        &parsed.protected,
        std::rc::Rc::clone(&parsed.metadata),
    );
    let documentation_config =
        crate::docstrip::documentation_config(analysis.model().document_class(), config)?;
    let documentation_config = documentation_config.as_ref().unwrap_or(config);
    if !documentation_config.config.math.spacing {
        return Ok(code);
    }
    let (documentation, margins) = documentation_source(&code);
    let model = crate::source_model::SourceModel::analyze(&documentation, documentation_config);
    let mut protected = model.protected_ranges().to_vec();
    let mut documentation_parsing =
        crate::parse_context::ParseContext::with_model(documentation_config, Some(model));
    let modes = MathModes::build(
        &documentation,
        documentation_config,
        &protected,
        &mut documentation_parsing,
    )?;
    protected.extend(margins);
    protected = code_layout::merged(protected);
    let mut editor = Editor {
        source: &documentation,
        config: documentation_config,
        modes: &modes,
        protected: &protected,
        edits: Vec::new(),
        visited: HashSet::new(),
    };
    for region in &modes.regions {
        if valid_sized_delimiters(&documentation, &region.body, &modes) {
            editor.sequence(region.body.clone(), false, region.text_cases);
        }
    }
    Ok(editor.finish(&code))
}

fn math_protection(
    source: &str,
    config: &CompiledConfig,
    parsing: &mut crate::parse_context::ParseContext<'_>,
) -> Result<Vec<Range<usize>>, FormatError> {
    if config.config.code.expl3
        && (source.contains("\\ExplSyntaxOn") || source.contains("\\ProvidesExpl"))
    {
        parsing.expl3_protected_ranges(source)
    } else {
        Ok(parsing.snapshot(source, false, false)?.protected.clone())
    }
}

#[derive(Clone)]
struct Atom {
    range: Range<usize>,
    class: MathClass,
    delimiter: Option<DelimiterRole>,
    command: bool,
    unary: bool,
    barrier: bool,
}

fn whitespace(text: &str) -> bool {
    text.bytes().all(|byte| byte.is_ascii_whitespace())
}

fn overlaps(ranges: &[Range<usize>], range: &Range<usize>) -> bool {
    let first = ranges.partition_point(|entry| entry.end <= range.start);
    ranges
        .get(first)
        .is_some_and(|entry| entry.start < range.end)
}

/// TeX math control words end before script markers and punctuation.
fn command_at(source: &str, start: usize) -> Option<(&str, usize)> {
    if source.as_bytes().get(start) != Some(&b'\\') {
        return None;
    }
    let mut end = start + 1;
    let character = source[end..].chars().next()?;
    if character.is_ascii_alphabetic() || character == '@' {
        while source
            .as_bytes()
            .get(end)
            .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'@')
        {
            end += 1
        }
        if source.as_bytes().get(end) == Some(&b'*') {
            end += 1
        }
    } else {
        end += character.len_utf8()
    }
    Some((&source[start + 1..end], end))
}

fn skip_space(source: &str, mut cursor: usize, end: usize) -> usize {
    while cursor < end && source.as_bytes()[cursor].is_ascii_whitespace() {
        cursor += 1
    }
    cursor
}

/// Replace documentation margins in a same-length scan buffer, then keep their
/// original byte ranges protected when projecting whitespace edits.
fn documentation_source(source: &str) -> (String, Vec<Range<usize>>) {
    let mut documentation_bytes: Vec<u8> = source
        .bytes()
        .map(|byte| {
            if matches!(byte, b'\r' | b'\n') {
                byte
            } else {
                b' '
            }
        })
        .collect();
    let mut margins = Vec::new();
    let mut offset = 0;
    let mut macrocode = false;
    for line in source.split_inclusive('\n') {
        let trimmed = line.trim_start_matches([' ', '\t']);
        let documentation = trimmed.strip_prefix('%');
        let content = documentation.map(str::trim_start);
        if content.is_some_and(|text| text.starts_with("\\end{macrocode}")) {
            macrocode = false
        } else if content.is_some_and(|text| text.starts_with("\\begin{macrocode}")) {
            macrocode = true
        } else if !macrocode
            && documentation.is_some_and(|text| {
                !text.starts_with(['%', '<'])
                    && !["\\iffalse", "\\fi"]
                        .iter()
                        .any(|command| text.trim_start().starts_with(command))
            })
        {
            let margin = offset + line.len() - trimmed.len();
            documentation_bytes[offset..offset + line.len()].copy_from_slice(line.as_bytes());
            if !documentation.is_some_and(|text| text.trim_start().starts_with("tift:")) {
                documentation_bytes[margin] = b' ';
            }
            margins.push(margin..margin + 1);
        }
        offset += line.len();
    }
    (
        String::from_utf8(documentation_bytes).expect("whole UTF-8 source lines were copied"),
        margins,
    )
}

fn valid_sized_delimiters(source: &str, range: &Range<usize>, modes: &MathModes) -> bool {
    let bytes = source[range.clone()].as_bytes();
    if ![b"\\left".as_slice(), b"\\right", b"\\middle"]
        .iter()
        .any(|name| memchr::memmem::find(bytes, name).is_some())
    {
        return true;
    }
    let mut depth = 0usize;
    let mut scopes = Vec::new();
    let mut literal = BTreeMap::new();
    let mut cursor = range.start;
    while cursor < range.end {
        if let Some(&end) = literal.get(&cursor) {
            cursor = end;
        } else if source.as_bytes()[cursor] == b'%' {
            cursor = source[cursor..range.end]
                .find('\n')
                .map_or(range.end, |offset| cursor + offset + 1);
        } else if let Some((name, end)) = command_at(source, cursor) {
            if let Some(signature) = modes.signature_at(name, cursor) {
                for argument in modes
                    .arguments_at(source, cursor, end, range.end, signature)
                    .iter()
                {
                    if argument.domain == b'U' {
                        literal.insert(argument.range.start, argument.range.end);
                    }
                }
            } else if !symbols::known_command(name) && !matches!(name, "left" | "right" | "middle")
            {
                let mut next = skip_space(source, end, range.end);
                while source.as_bytes().get(next) == Some(&b'{') {
                    let Some(closing) = syntax::balanced_end(source, next, b'{', b'}')
                        .filter(|closing| *closing <= range.end)
                    else {
                        break;
                    };
                    literal.insert(next, closing);
                    next = skip_space(source, closing, range.end);
                }
            }
            match name {
                "left" => depth += 1,
                "right" if depth == 0 => return false,
                "right" => depth -= 1,
                "middle" if depth == 0 => return false,
                _ => (),
            }
            cursor = end;
        } else {
            if source.as_bytes()[cursor] == b'{' {
                scopes.push(depth);
                depth = 0;
            } else if source.as_bytes()[cursor] == b'}' {
                if depth != 0 {
                    return false;
                }
                depth = scopes.pop().unwrap_or(0);
            }
            cursor += source[cursor..].chars().next().unwrap().len_utf8()
        }
    }
    depth == 0
}

struct Editor<'a> {
    source: &'a str,
    config: &'a CompiledConfig,
    modes: &'a MathModes,
    protected: &'a [Range<usize>],
    edits: Vec<(Range<usize>, Cow<'static, str>)>,
    visited: HashSet<(usize, usize)>,
}

impl Editor<'_> {
    fn blocked(&self, range: &Range<usize>) -> bool {
        let index = self
            .protected
            .partition_point(|entry| entry.end <= range.start);
        self.protected.get(index).is_some_and(|entry| {
            if range.is_empty() {
                entry.start <= range.start && range.start < entry.end
            } else {
                entry.start < range.end && range.start < entry.end
            }
        })
    }

    fn edit(&mut self, range: Range<usize>, value: &str) {
        let authored = &self.source[range.clone()];
        if authored != value
            && self.config.config.math.spacing
            && !(self.config.config.math.wrap == MathWrap::Preserve
                && authored.contains(['\r', '\n']))
            && !self.blocked(&range)
            && whitespace(authored)
        {
            let value = match value {
                "" => Cow::Borrowed(""),
                " " => Cow::Borrowed(" "),
                value => Cow::Owned(value.into()),
            };
            self.edits.push((range, value));
        }
    }

    fn collapse(&mut self, range: Range<usize>) {
        // Empty intervals and conservative boundary exits contain no gap.
        if range.start >= range.end {
            return;
        }
        let authored = &self.source[range.clone()];
        if !authored.contains(['\r', '\n']) && authored.contains("  ") {
            let mut output = String::with_capacity(authored.len());
            let mut space = false;
            for character in authored.chars() {
                if character != ' ' || !space {
                    output.push(character)
                }
                space = character == ' ';
            }
            self.edit(range, &output);
        }
    }

    fn group(&mut self, range: Range<usize>, script: bool) {
        let mut body = range.start + 1..range.end - 1;
        let original_body = (body.start, body.end);
        // TeX comments retain the newline separating them from closing braces.
        if self.source.as_bytes()[range.start] == b'{' && !self.source[body.clone()].contains('%') {
            let text = &self.source[body.clone()];
            let first = text.len() - text.trim_start_matches(char::is_whitespace).len();
            let mut last = text.trim_end_matches(char::is_whitespace).len();
            // A control space is a complete TeX token. Trimming its whitespace
            // would turn the escape into a different command or consume the
            // closing brace during the recursive scan.
            if last < text.len()
                && text[..last]
                    .bytes()
                    .rev()
                    .take_while(|&byte| byte == b'\\')
                    .count()
                    % 2
                    == 1
            {
                let whitespace = text[last..].chars().next().expect("trailing whitespace");
                last += whitespace.len_utf8();
                if whitespace == '\r' && text.as_bytes().get(last) == Some(&b'\n') {
                    last += 1;
                }
            }
            if first <= last {
                self.edit(body.start..body.start + first, "");
                self.edit(body.start + last..body.end, "");
                body = body.start + first..body.start + last;
            } else {
                self.edit(body.clone(), "");
                body.end = body.start;
            }
        }
        self.sequence(body, script, false);
        self.visited.insert(original_body);
    }

    fn atom(&mut self, cursor: usize, end: usize, script: bool) -> Atom {
        let character = self.source[cursor..].chars().next().unwrap();
        let mut range = cursor..cursor + character.len_utf8();
        let mut info = symbols::character_info(character);
        let mut command = false;
        let mut barrier = matches!(character, '%' | '&');
        let protected_index = self.protected.partition_point(|entry| entry.end <= cursor);
        if let Some(protected) = self
            .protected
            .get(protected_index)
            .filter(|entry| entry.start <= cursor)
        {
            range.end = protected.end.min(end);
            barrier = true;
        } else if character == '%' {
            range.end = self.source[cursor..end]
                .find('\n')
                .map_or(end, |offset| cursor + offset);
        } else if character == '{' {
            if let Some(closing) = syntax::balanced_end(self.source, cursor, b'{', b'}')
                .filter(|closing| *closing <= end)
            {
                range.end = closing;
                self.group(range.clone(), script);
                info = Default::default();
            }
        } else if let Some((name, command_end)) = command_at(self.source, cursor) {
            if command_end > end {
                return Atom {
                    range: cursor..end,
                    class: info.class,
                    delimiter: None,
                    command: true,
                    unary: false,
                    barrier: true,
                };
            }
            command = true;
            range.end = command_end;
            let name = name.trim_end_matches('*');
            info = symbols::command_info(name);
            barrier = matches!(name, "\\" | "cr" | "crcr");
            if barrier {
                range.end =
                    crate::layout::row_terminator_end(self.source, command_end, end, name == "\\");
            }
            if name == "begin"
                && let Some(region) = self.modes.environment_at(cursor, end).cloned()
            {
                self.sequence(region.body.clone(), script, region.text_cases);
                range.end = region.environment.unwrap().end;
                info.class = MathClass::Inner;
            } else if matches!(
                name,
                "left"
                    | "right"
                    | "middle"
                    | "bigl"
                    | "bigr"
                    | "Bigl"
                    | "Bigr"
                    | "biggl"
                    | "biggr"
                    | "Biggl"
                    | "Biggr"
            ) {
                let delimiter_start = skip_space(self.source, command_end, end);
                if delimiter_start < end {
                    let delimiter_end = command_at(self.source, delimiter_start).map_or_else(
                        || {
                            delimiter_start
                                + self.source[delimiter_start..]
                                    .chars()
                                    .next()
                                    .unwrap()
                                    .len_utf8()
                        },
                        |(_, end)| end,
                    );
                    if delimiter_end > end {
                        return Atom {
                            range: cursor..end,
                            class: info.class,
                            delimiter: None,
                            command: true,
                            unary: false,
                            barrier: true,
                        };
                    }
                    self.edit(command_end..delimiter_start, "");
                    range.end = delimiter_end;
                    info.class = if name.ends_with('l') || name == "left" {
                        MathClass::Open
                    } else if name.ends_with('r') || name == "right" {
                        MathClass::Close
                    } else {
                        MathClass::Ord
                    };
                    info.delimiter = match info.class {
                        MathClass::Open => Some(DelimiterRole::Open),
                        MathClass::Close => Some(DelimiterRole::Close),
                        _ => None,
                    };
                }
            } else {
                let overridden = self.modes.is_overridden(name, cursor)
                    || self.config.config.syntax.commands.contains_key(name);
                let signature = (!overridden)
                    .then(|| self.modes.signature_at(name, cursor))
                    .flatten();
                if let Some(signature) = signature {
                    let modes = self.modes;
                    for argument in modes
                        .arguments_at(self.source, cursor, command_end, end, signature)
                        .iter()
                    {
                        if matches!(argument.domain, b'M' | b'I') {
                            if argument.range.start + 1 == argument.body.start
                                && argument.body.end + 1 == argument.range.end
                            {
                                self.group(argument.range.clone(), script)
                            } else {
                                self.sequence(argument.body.clone(), script, false)
                            }
                        }
                        range.end = argument.range.end;
                    }
                    if name == "sqrt" {
                        info = Default::default()
                    }
                } else if overridden || !symbols::known_command(name) {
                    // Unknown brace arguments are token data; bare brackets keep
                    // their ordinary mathematical delimiter meaning.
                    let mut next = skip_space(self.source, range.end, end);
                    while self.source.as_bytes().get(next) == Some(&b'{') {
                        let Some(closing) = syntax::balanced_end(self.source, next, b'{', b'}')
                            .filter(|closing| *closing <= end)
                        else {
                            break;
                        };
                        range.end = closing;
                        next = skip_space(self.source, range.end, end);
                    }
                }
            }
        }
        if !barrier {
            loop {
                let marker = skip_space(self.source, range.end, end);
                if marker >= end {
                    break;
                }
                if !matches!(self.source.as_bytes().get(marker), Some(b'_' | b'^')) {
                    break;
                }
                let argument = skip_space(self.source, marker + 1, end);
                if argument >= end {
                    break;
                }
                self.edit(range.end..marker, "");
                self.edit(marker + 1..argument, "");
                let value = self.atom(argument, end, true);
                if value.range.end <= argument {
                    break;
                }
                range.end = value.range.end;
            }
        }
        Atom {
            range,
            class: info.class,
            delimiter: info.delimiter,
            command,
            unary: false,
            barrier,
        }
    }

    fn atoms(&mut self, range: Range<usize>, script: bool, text_cases: bool) -> Vec<Atom> {
        let mut atoms: Vec<Atom> = Vec::new();
        let mut cursor = range.start;
        let mut text_column = false;
        while cursor < range.end {
            if self.source.as_bytes()[cursor].is_ascii_whitespace() {
                cursor += 1;
                continue;
            }
            if text_cases && text_column {
                if self.source[cursor..].starts_with("\\\\") {
                    text_column = false
                } else {
                    cursor += self.source[cursor..].chars().next().unwrap().len_utf8();
                    continue;
                }
            }
            let current = self.atom(cursor, range.end, script);
            cursor = current.range.end;
            if text_cases && &self.source[current.range.clone()] == "&" {
                text_column = true
            }
            if let Some(previous) = atoms.last_mut() {
                let left = &self.source[previous.range.clone()];
                let right = &self.source[current.range.clone()];
                if previous.range.end == current.range.start
                    && !previous.command
                    && !current.command
                    && ((previous.class == MathClass::Rel && current.class == MathClass::Rel)
                        || left.chars().all(|character| character == ':')
                            && matches!(right, ":" | "=")
                        || left == "!" && right == "=")
                {
                    previous.range.end = current.range.end;
                    if current.class == MathClass::Rel {
                        previous.class = MathClass::Rel;
                        previous.delimiter = None
                    }
                    continue;
                }
            }
            atoms.push(current);
        }
        let mut read = 0;
        let mut write = 0;
        while read < atoms.len() {
            if read + 1 < atoms.len()
                && &self.source[atoms[read].range.clone()] == "\\not"
                && atoms[read + 1].class == MathClass::Rel
                && whitespace(&self.source[atoms[read].range.end..atoms[read + 1].range.start])
            {
                atoms[read].range.end = atoms[read + 1].range.end;
                atoms[read].class = MathClass::Rel;
                atoms[read].delimiter = None;
                if write != read {
                    atoms.swap(write, read);
                }
                read += 2;
            } else {
                if write != read {
                    atoms.swap(write, read);
                }
                read += 1;
            }
            write += 1;
        }
        atoms.truncate(write);
        let mut previous = MathClass::Rel;
        let mut opener = false;
        for atom in &mut atoms {
            atom.unary = atom.class == MathClass::Bin
                && (opener
                    || matches!(
                        previous,
                        MathClass::Op | MathClass::Bin | MathClass::Rel | MathClass::Punct
                    ));
            if atom.barrier {
                previous = MathClass::Rel;
                opener = false
            } else {
                previous = atom.class;
                opener = atom.delimiter == Some(DelimiterRole::Open)
            }
        }
        atoms
    }

    fn retained_gaps(&self, atoms: &[Atom]) -> Vec<bool> {
        // Ordinary expressions need no delimiter/slash propagation buffers.
        if atoms.len() < 3
            || !atoms.iter().any(|atom| {
                atom.delimiter == Some(DelimiterRole::Open)
                    || &self.source[atom.range.clone()] == "/"
            })
        {
            return Vec::new();
        }
        // A gap has at most one delimiter partner and two slash neighbours.
        // Store (delimiter index + 1) above two slash bits. Zero is the common
        // unconstrained gap, which needs no propagation or text comparisons.
        const SLASH_PREVIOUS: usize = 1;
        const SLASH_NEXT: usize = 2;
        const DELIMITER_SHIFT: usize = 2;
        let mut gap_links = vec![0usize; atoms.len().saturating_sub(1)];
        let mut keep_gap: Vec<_> = atoms
            .windows(2)
            .map(|pair| {
                let gap = &self.source[pair[0].range.end..pair[1].range.start];
                gap.starts_with(' ') && gap.ends_with(' ') && !gap.contains(['\r', '\n'])
            })
            .collect();
        let mut delimiters = Vec::new();
        for (index, atom) in atoms.iter().enumerate() {
            if atom.delimiter == Some(DelimiterRole::Open) {
                delimiters.push(index)
            } else if atom.delimiter == Some(DelimiterRole::Close)
                && let Some(open) = delimiters.pop()
                && open + 1 < index
            {
                gap_links[open] |= index << DELIMITER_SHIFT;
                gap_links[index - 1] |= (open + 1) << DELIMITER_SHIFT;
            }
            if &self.source[atom.range.clone()] == "/" {
                if index > 0 && index + 1 < atoms.len() {
                    gap_links[index - 1] |= SLASH_NEXT;
                    gap_links[index] |= SLASH_PREVIOUS;
                } else if index > 0 {
                    keep_gap[index - 1] = false
                } else if index == 0 && index + 1 < atoms.len() {
                    keep_gap[index] = false
                }
            }
        }
        // A shared gap must satisfy every adjacent slash/delimiter pair. This
        // propagates one-sided compaction through runs such as `a / /b` once.
        let mut removed: Vec<_> = keep_gap
            .iter()
            .enumerate()
            .filter_map(|(index, keep)| (!keep && gap_links[index] != 0).then_some(index))
            .collect();
        while let Some(index) = removed.pop() {
            let links = gap_links[index];
            if links == 0 {
                continue;
            }
            for partner in [
                (links >> DELIMITER_SHIFT).wrapping_sub(1),
                if links & SLASH_PREVIOUS != 0 {
                    index - 1
                } else {
                    usize::MAX
                },
                if links & SLASH_NEXT != 0 {
                    index + 1
                } else {
                    usize::MAX
                },
            ] {
                if keep_gap.get(partner).copied() == Some(true) {
                    keep_gap[partner] = false;
                    removed.push(partner)
                }
            }
        }
        for (keep, links) in keep_gap.iter_mut().zip(gap_links) {
            *keep &= links != 0;
        }
        keep_gap
    }

    fn sequence(&mut self, range: Range<usize>, script: bool, text_cases: bool) {
        if !self.visited.insert((range.start, range.end)) {
            return;
        }
        let atoms = self.atoms(range.clone(), script, text_cases);
        if let Some(first) = atoms.first() {
            self.collapse(range.start..first.range.start)
        }
        if let Some(last) = atoms.last() {
            self.collapse(last.range.end..range.end)
        }
        let retained = self.retained_gaps(&atoms);
        for (index, pair) in atoms.windows(2).enumerate() {
            let left = &pair[0];
            let right = &pair[1];
            let gap = left.range.end..right.range.start;
            let authored = &self.source[gap.clone()];
            if left.barrier || right.barrier || !whitespace(authored) {
                continue;
            }
            let left_text = &self.source[left.range.clone()];
            let right_text = &self.source[right.range.clone()];
            if retained.get(index).copied().unwrap_or(false) {
                self.collapse(gap);
                continue;
            }
            if left_text == "/" || right_text == "/" {
                self.edit(gap, "");
                continue;
            }
            if left.delimiter == Some(DelimiterRole::Open)
                || right.delimiter == Some(DelimiterRole::Close)
            {
                if !authored.contains(['\r', '\n']) {
                    self.edit(
                        gap,
                        if control_word_seam(left_text, right_text) {
                            " "
                        } else {
                            ""
                        },
                    )
                }
                continue;
            }
            let left_operator = matches!(left.class, MathClass::Bin | MathClass::Rel);
            let right_operator = matches!(right.class, MathClass::Bin | MathClass::Rel);
            if script && (left_operator && left.command || right_operator && right.command) {
                self.collapse(gap);
                continue;
            }
            if script && (left_operator || right_operator) || left.unary {
                if !control_word_seam(left_text, right_text) {
                    self.edit(gap, "")
                } else {
                    self.collapse(gap)
                }
            } else if (left_operator && !left.unary || right_operator && !right.unary)
                && !authored.contains(['\r', '\n'])
            {
                self.edit(gap, " ")
            } else {
                self.collapse(gap)
            }
        }
    }

    fn finish(self, original: &str) -> String {
        self.finish_with_mapping::<false>(original).0
    }

    fn finish_mapped(self, original: &str) -> (String, PositionMap) {
        self.finish_with_mapping::<true>(original)
    }

    fn finish_with_mapping<const MAPPED: bool>(mut self, original: &str) -> (String, PositionMap) {
        // Stable sorting retains insertion order for equal starts. The last
        // edit at a start wins, as in the original ordered map.
        self.edits.sort_by_key(|(range, _)| range.start);
        let mut output = String::with_capacity(original.len());
        let mut positions = PositionMap {
            edits: Vec::with_capacity(if MAPPED { self.edits.len() } else { 0 }),
        };
        let mut cursor = 0;
        let mut edits = self.edits.into_iter().peekable();
        while let Some((range, value)) = edits.next() {
            let (start, end) = (range.start, range.end);
            if edits.peek().is_some_and(|(next, _)| next.start == start) {
                continue;
            }
            if start < cursor {
                continue;
            }
            output.push_str(&original[cursor..start]);
            output.push_str(&value);
            if MAPPED {
                positions.edits.push((start..end, output.len()));
            }
            cursor = end;
        }
        output.push_str(&original[cursor..]);
        (output, positions)
    }
}

fn control_word_seam(left: &str, right: &str) -> bool {
    left.rfind('\\').is_some_and(|start| {
        left[start + 1..]
            .chars()
            .all(|character| character.is_ascii_alphabetic() || character == '@')
            && start + 1 < left.len()
    }) && right
        .as_bytes()
        .first()
        .is_some_and(u8::is_ascii_alphabetic)
}

struct BreakLayout<'a> {
    prefix: &'a str,
    unit: &'a str,
    ending: &'a str,
    unit_width: usize,
    line_width: usize,
    spacing: bool,
    priority: &'a [MathBreakKind],
}

fn break_displays(
    source: &str,
    kind: FileKind,
    config: &CompiledConfig,
    parsing: &mut crate::parse_context::ParseContext<'_>,
) -> Result<String, FormatError> {
    let mut protected = math_protection(source, config, parsing)?;
    if kind == FileKind::Ins {
        protected.extend(crate::docstrip::preamble_ranges(source, &protected));
        protected = code_layout::merged(protected);
    }
    let modes = MathModes::build(source, config, &protected, parsing)?;
    layout_displays(source, config, &modes, &protected)
}

fn layout_displays(
    source: &str,
    config: &CompiledConfig,
    modes: &MathModes,
    protected: &[Range<usize>],
) -> Result<String, FormatError> {
    if config.config.math.wrap == MathWrap::Preserve {
        return Ok(source.into());
    }
    let mut editor = Editor {
        source,
        config,
        modes,
        protected,
        edits: Vec::new(),
        visited: HashSet::new(),
    };
    let mut replacements = Vec::new();
    let ending = if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    for region in &modes.regions {
        let Some((start, name)) = &region.display else {
            continue;
        };
        let body = &source[region.body.clone()];
        if body.contains('%') || body.contains("\\\\") || overlaps(protected, &region.body) {
            continue;
        }
        let line_start = source[..*start].rfind('\n').map_or(0, |offset| offset + 1);
        let prefix = &source[line_start..*start];
        if !prefix
            .chars()
            .all(|character| matches!(character, ' ' | '\t'))
        {
            continue;
        }
        let prefix_width = indent_width(prefix, config.config.indent.width);
        let columns = config
            .config
            .math
            .columns
            .unwrap_or(config.config.wrap.columns);
        let line_width = columns.saturating_sub(prefix_width).max(1);
        let category = if matches!(name.as_str(), "$$" | "displayMath") {
            "special"
        } else {
            "environment"
        };
        let unit = config.config.indent.unit(category, name);
        let unit_width = indent_width(&unit, config.config.indent.width);
        let layout = BreakLayout {
            prefix,
            unit: &unit,
            ending,
            unit_width,
            line_width,
            spacing: config.config.math.spacing,
            priority: &config.config.math.break_priority,
        };
        if config.config.math.wrap == MathWrap::SingleLine {
            if !valid_sized_delimiters(source, &region.body, modes) {
                continue;
            }
            let atoms = editor.atoms(region.body.clone(), false, false);
            if let Some((first, last)) = atoms.first().zip(atoms.last())
                && let Some(flat) = flat_piece(source, &atoms, &(first.range.start..last.range.end))
            {
                let leading = &source[region.body.start..first.range.start];
                let trailing = &source[last.range.end..region.body.end];
                let mut replacement = String::new();
                if leading.contains('\n') {
                    replacement.push_str(ending);
                    replacement.push_str(prefix);
                    replacement.push_str(&unit);
                } else {
                    replacement.push_str(leading);
                }
                replacement.push_str(&flat);
                if trailing.contains('\n') {
                    replacement.push_str(ending);
                    replacement.push_str(prefix);
                } else {
                    replacement.push_str(trailing);
                }
                if replacement != body {
                    replacements.push((region.body.clone(), replacement));
                }
            }
            continue;
        }
        let first_prefix = &source[line_start..region.body.start];
        let overflows = body.lines().enumerate().any(|(index, line)| {
            UnicodeWidthStr::width(line)
                + if index == 0 {
                    UnicodeWidthStr::width(first_prefix)
                } else {
                    0
                }
                > columns
        });
        if !overflows && flat_display_width(body) + unit_width <= line_width {
            continue;
        }
        if !valid_sized_delimiters(source, &region.body, modes) {
            continue;
        }
        let atoms = editor.atoms(region.body.clone(), false, false);
        let candidates = break_candidates(source, &atoms, layout.priority);
        if let Some(replacement) = greedy_breaks(source, &atoms, &candidates, &layout)
            && replacement != body
        {
            replacements.push((region.body.clone(), replacement))
        }
    }
    let mut output = String::with_capacity(source.len());
    let mut cursor = 0;
    for (range, replacement) in replacements {
        if range.start < cursor {
            continue;
        }
        output.push_str(&source[cursor..range.start]);
        output.push_str(&replacement);
        cursor = range.end;
    }
    output.push_str(&source[cursor..]);
    Ok(output)
}

fn indent_width(text: &str, tab_width: usize) -> usize {
    text.chars()
        .map(|character| if character == '\t' { tab_width } else { 1 })
        .sum()
}

fn flat_display_width(body: &str) -> usize {
    body.lines()
        .enumerate()
        .map(|(index, line)| UnicodeWidthStr::width(line.trim()) + usize::from(index > 0))
        .sum()
}

fn break_candidates(source: &str, atoms: &[Atom], priority: &[MathBreakKind]) -> BTreeSet<usize> {
    let mut candidates = BTreeSet::new();
    let mut depth = 0usize;
    let mut conditional = false;
    for atom in atoms {
        let text = &source[atom.range.clone()];
        if !atom.barrier
            && depth == 0
            && (atom.class == MathClass::Bin && !atom.unary
                || atom.class == MathClass::Rel && !conditional)
            && text != "\\mid"
            && (text != "\\cdot" || priority.contains(&MathBreakKind::Multiplicative))
        {
            candidates.insert(atom.range.start);
        }
        if text == "\\mid" && depth == 0 {
            conditional = true
        }
        match atom.delimiter {
            Some(DelimiterRole::Open) => depth += 1,
            Some(DelimiterRole::Close) => depth = depth.saturating_sub(1),
            _ => (),
        }
    }
    candidates
}

fn greedy_breaks(
    source: &str,
    atoms: &[Atom],
    candidates: &BTreeSet<usize>,
    layout: &BreakLayout<'_>,
) -> Option<String> {
    let first = atoms.first()?;
    let last = atoms.last()?;
    let mut pieces = Vec::new();
    let mut start = first.range.start;
    let mut preceding_end = start;
    for atom in atoms {
        if candidates.contains(&atom.range.start) && atom.range.start > start {
            pieces.push(start..preceding_end);
            start = atom.range.start
        }
        preceding_end = atom.range.end;
    }
    pieces.push(start..last.range.end);
    if pieces.len() < 2 {
        return None;
    }
    let first_relation = atoms
        .iter()
        .find(|atom| atom.class == MathClass::Rel && candidates.contains(&atom.range.start));
    let rhs_indent = first_relation
        .and_then(|relation| {
            flat_piece(source, atoms, &(first.range.start..relation.range.end))
                .map(|text| layout.unit_width + UnicodeWidthStr::width(text.as_str()) + 1)
        })
        .unwrap_or(layout.unit_width);
    let relation_indent = first_relation
        .and_then(|relation| {
            flat_piece(source, atoms, &(first.range.start..relation.range.start))
                .map(|text| layout.unit_width + UnicodeWidthStr::width(text.as_str()))
        })
        .unwrap_or(layout.unit_width);
    let ending = layout.ending;
    let mut output = format!("{ending}{}{}", layout.prefix, layout.unit);
    let gap_before = |index: usize| {
        let authored = &source[pieces[index - 1].end..pieces[index].start];
        if layout.spacing || authored.contains(['\r', '\n']) {
            " "
        } else {
            authored
        }
    };
    let atom_at = |index: usize| {
        let index = atoms.partition_point(|atom| atom.range.start < pieces[index].start);
        &atoms[index]
    };
    let indent_at = |index: usize, piece_width: usize| {
        let preferred = if atom_at(index).class == MathClass::Rel {
            relation_indent
        } else {
            rhs_indent
        };
        if preferred + piece_width <= layout.line_width {
            preferred
        } else {
            layout.unit_width
        }
    };
    if !layout.priority.is_empty() {
        let mut ranked = vec![Vec::new(); layout.priority.len()];
        let mut rendered = Vec::with_capacity(pieces.len());
        let mut ends = Vec::with_capacity(pieces.len());
        let mut column = 0;
        for (index, range) in pieces.iter().enumerate() {
            let gap = if index == 0 { "" } else { gap_before(index) };
            column += UnicodeWidthStr::width(gap);
            let text = flat_piece(source, atoms, range)?;
            let width = UnicodeWidthStr::width(text.as_str());
            rendered.push((text, width, column, gap));
            column += width;
            ends.push(column);
            if index > 0 {
                let kind = break_kind(source, atom_at(index));
                if let Some(rank) = layout.priority.iter().position(|&entry| entry == kind) {
                    ranked[rank].push(index);
                }
            }
        }
        let mut start = 0;
        while start < rendered.len() {
            let (_, width, column, _) = &rendered[start];
            let indent = if start == 0 {
                layout.unit_width
            } else {
                indent_at(start, *width)
            };
            let limit = column + layout.line_width.saturating_sub(indent);
            let farthest = ends.partition_point(|&end| end <= limit).max(start + 1);
            // Find the latest acceptable position in each class by searching its
            // own index. A short preferred line does not repeatedly scan its tail.
            let stop = if farthest == rendered.len() {
                farthest
            } else {
                ranked
                    .iter()
                    .find_map(|positions| {
                        let count = positions.partition_point(|&position| position <= farthest);
                        count
                            .checked_sub(1)
                            .map(|last| positions[last])
                            .filter(|&position| position > start)
                    })
                    .unwrap_or(farthest)
            };
            if start > 0 {
                output.push_str(ending);
                output.push_str(layout.prefix);
                output.push_str(layout.unit);
                output.push_str(&" ".repeat(indent.saturating_sub(layout.unit_width)));
            }
            for (index, (text, _, _, gap)) in rendered[start..stop].iter().enumerate() {
                if index > 0 {
                    output.push_str(gap);
                }
                output.push_str(text);
            }
            start = stop;
        }
        output.push_str(ending);
        output.push_str(layout.prefix);
        return Some(output);
    }
    let mut width = layout.unit_width;
    for (index, range) in pieces.iter().enumerate() {
        let text = flat_piece(source, atoms, range)?;
        let piece_width = UnicodeWidthStr::width(text.as_str());
        if index > 0 {
            let gap = gap_before(index);
            let gap_width = UnicodeWidthStr::width(gap);
            if width + gap_width + piece_width <= layout.line_width {
                output.push_str(gap);
                width += gap_width
            } else {
                let indent = indent_at(index, piece_width);
                output.push_str(ending);
                output.push_str(layout.prefix);
                output.push_str(layout.unit);
                output.push_str(&" ".repeat(indent.saturating_sub(layout.unit_width)));
                width = indent;
            }
        }
        output.push_str(&text);
        width += piece_width;
    }
    output.push_str(ending);
    output.push_str(layout.prefix);
    Some(output)
}

fn break_kind(source: &str, atom: &Atom) -> MathBreakKind {
    if atom.class == MathClass::Rel {
        MathBreakKind::Relation
    } else {
        match &source[atom.range.clone()] {
            "+" | "-" | "−" | "±" | "∓" | "\\pm" | "\\mp" => MathBreakKind::Additive,
            "*" | "×" | "÷" | "·" | "\\cdot" | "\\times" | "\\div" => {
                MathBreakKind::Multiplicative
            }
            _ => MathBreakKind::Binary,
        }
    }
}

fn flat_piece(source: &str, atoms: &[Atom], range: &Range<usize>) -> Option<String> {
    let mut output = String::new();
    let mut cursor = range.start;
    let first = atoms.partition_point(|atom| atom.range.start < range.start);
    let mut previous: Option<&Atom> = None;
    for atom in atoms[first..]
        .iter()
        .take_while(|atom| atom.range.start <= range.end)
        .filter(|atom| atom.range.end <= range.end)
    {
        let text = &source[atom.range.clone()];
        if text.contains(['\r', '\n']) {
            return None;
        }
        let gap = &source[cursor..atom.range.start];
        if gap.contains(['\r', '\n']) {
            let delimiter_gap = atom.delimiter == Some(DelimiterRole::Close)
                || previous.is_some_and(|left| left.delimiter == Some(DelimiterRole::Open));
            if !delimiter_gap
                || previous.is_some_and(|left| control_word_seam(&source[left.range.clone()], text))
            {
                output.push(' ')
            }
        } else {
            output.push_str(gap)
        }
        output.push_str(text);
        cursor = atom.range.end;
        previous = Some(atom);
    }
    let tail = &source[cursor..range.end];
    if tail.contains(['\r', '\n']) {
        output.push(' ')
    } else {
        output.push_str(tail)
    }
    Some(output)
}

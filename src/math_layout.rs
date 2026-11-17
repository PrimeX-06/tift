//! Lossless mathematical whitespace edits and native display-line planning.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
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
    pub enabled: bool,
}

impl Default for MathConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

pub(crate) fn is_active(source: &str, config: &CompiledConfig) -> bool {
    config.config.math.enabled
        && (source.contains('$')
            || memchr::memchr_iter(b'\\', source.as_bytes()).any(|start| {
                command_at(source, start).is_some_and(|(name, end)| {
                    matches!(name, "(" | "[")
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

pub(crate) fn format_math(
    source: &str,
    kind: FileKind,
    config: &CompiledConfig,
) -> Result<String, FormatError> {
    if !config.config.math.enabled || kind == FileKind::Bibtex {
        return Ok(source.into());
    }
    let mut protected = code_layout::protected_ranges(source, config, config.config.code.expl3)?;
    if kind == FileKind::Ins {
        protected.extend(crate::docstrip::preamble_ranges(source, &protected));
        protected = code_layout::merged(protected);
    }
    let (virtual_source, margins) = if kind == FileKind::Dtx {
        let (source, margins) = documentation_source(source);
        (std::borrow::Cow::Owned(source), margins)
    } else {
        (std::borrow::Cow::Borrowed(source), Vec::new())
    };
    let modes = MathModes::build(&virtual_source, config, &protected)?;
    protected.extend(margins);
    protected = code_layout::merged(protected);
    let mut editor = Editor {
        source: &virtual_source,
        config,
        modes: &modes,
        protected: &protected,
        edits: BTreeMap::new(),
        visited: BTreeSet::new(),
    };
    for region in &modes.regions {
        if valid_sized_delimiters(&virtual_source, &region.body) {
            editor.sequence(region.body.clone(), false, region.text_cases);
        }
    }
    let normalized = editor.finish(source);
    break_displays(&normalized, kind, config)
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
    let mut bytes = source.as_bytes().to_vec();
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
            bytes[margin] = b' ';
            margins.push(margin..margin + 1);
        }
        offset += line.len();
    }
    (
        String::from_utf8(bytes).expect("only ASCII margins were replaced"),
        margins,
    )
}

fn valid_sized_delimiters(source: &str, range: &Range<usize>) -> bool {
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
            if let Some(signature) = modes::command_signature(name) {
                for (argument, domain) in modes::argument_ranges(source, end, range.end, signature)
                {
                    if domain == b'U' {
                        literal.insert(argument.start, argument.end);
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
    edits: BTreeMap<usize, (usize, String)>,
    visited: BTreeSet<(usize, usize)>,
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
        if !self.blocked(&range)
            && whitespace(&self.source[range.clone()])
            && &self.source[range.clone()] != value
        {
            self.edits.insert(range.start, (range.end, value.into()));
        }
    }

    fn collapse(&mut self, range: Range<usize>) {
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
            let last = text.trim_end_matches(char::is_whitespace).len();
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
                && let Some(region) = self
                    .modes
                    .regions
                    .iter()
                    .find(|region| {
                        region
                            .environment
                            .as_ref()
                            .is_some_and(|range| range.start == cursor && range.end <= end)
                    })
                    .cloned()
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
                let overridden = self.modes.declared.contains(name)
                    || self.config.config.syntax.commands.contains_key(name);
                let signature = (!overridden)
                    .then(|| modes::command_signature(name))
                    .flatten();
                if let Some(signature) = signature {
                    for (argument, domain) in
                        modes::argument_ranges(self.source, command_end, end, signature)
                    {
                        if matches!(domain, b'M' | b'I') {
                            self.group(argument.clone(), script)
                        }
                        range.end = argument.end;
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
        let mut index = 0;
        while index + 1 < atoms.len() {
            if &self.source[atoms[index].range.clone()] == "\\not"
                && atoms[index + 1].class == MathClass::Rel
                && whitespace(&self.source[atoms[index].range.end..atoms[index + 1].range.start])
            {
                let next = atoms.remove(index + 1);
                atoms[index].range.end = next.range.end;
                atoms[index].class = MathClass::Rel;
                atoms[index].delimiter = None;
            }
            index += 1;
        }
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
        let mut paired_gaps = vec![Vec::new(); atoms.len().saturating_sub(1)];
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
                paired_gaps[open].push(index - 1);
                paired_gaps[index - 1].push(open);
            }
            if &self.source[atom.range.clone()] == "/" {
                if index > 0 && index + 1 < atoms.len() {
                    paired_gaps[index - 1].push(index);
                    paired_gaps[index].push(index - 1);
                } else if index > 0 {
                    keep_gap[index - 1] = false
                } else if index + 1 < atoms.len() {
                    keep_gap[index] = false
                }
            }
        }
        // A shared gap must satisfy every adjacent slash/delimiter pair. This
        // propagates one-sided compaction through runs such as `a / /b` once.
        let mut removed: Vec<_> = keep_gap
            .iter()
            .enumerate()
            .filter_map(|(index, keep)| (!keep).then_some(index))
            .collect();
        while let Some(index) = removed.pop() {
            for &partner in &paired_gaps[index] {
                if keep_gap[partner] {
                    keep_gap[partner] = false;
                    removed.push(partner)
                }
            }
        }
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
            if keep_gap[index] && !paired_gaps[index].is_empty() {
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
        let mut output = String::with_capacity(original.len());
        let mut cursor = 0;
        for (start, (end, value)) in self.edits {
            if start < cursor {
                continue;
            }
            output.push_str(&original[cursor..start]);
            output.push_str(&value);
            cursor = end;
        }
        output.push_str(&original[cursor..]);
        output
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
    unit_width: usize,
    line_width: usize,
}

fn break_displays(
    source: &str,
    kind: FileKind,
    config: &CompiledConfig,
) -> Result<String, FormatError> {
    let mut protected = code_layout::protected_ranges(source, config, config.config.code.expl3)?;
    if kind == FileKind::Ins {
        protected.extend(crate::docstrip::preamble_ranges(source, &protected));
        protected = code_layout::merged(protected);
    }
    let modes = MathModes::build(source, config, &protected)?;
    let mut editor = Editor {
        source,
        config,
        modes: &modes,
        protected: &protected,
        edits: BTreeMap::new(),
        visited: BTreeSet::new(),
    };
    let mut replacements = Vec::new();
    for region in &modes.regions {
        let Some((start, name)) = &region.display else {
            continue;
        };
        let body = &source[region.body.clone()];
        if overlaps(&protected, &region.body)
            || !valid_sized_delimiters(source, &region.body)
            || body.contains('%')
            || body.contains("\\\\")
        {
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
        let line_width = config
            .config
            .wrap
            .columns
            .saturating_sub(prefix_width)
            .max(1);
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
            unit_width,
            line_width,
        };
        let first_prefix = &source[line_start..region.body.start];
        let overflows = body.lines().enumerate().any(|(index, line)| {
            UnicodeWidthStr::width(line)
                + if index == 0 {
                    UnicodeWidthStr::width(first_prefix)
                } else {
                    0
                }
                > config.config.wrap.columns
        });
        if !overflows && flat_display_width(body) + unit_width <= line_width {
            continue;
        }
        let atoms = editor.atoms(region.body.clone(), false, false);
        let candidates = break_candidates(source, &atoms);
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

fn break_candidates(source: &str, atoms: &[Atom]) -> BTreeSet<usize> {
    let mut candidates = BTreeSet::new();
    let mut depth = 0usize;
    let mut conditional = false;
    for atom in atoms {
        let text = &source[atom.range.clone()];
        if !atom.barrier
            && depth == 0
            && (atom.class == MathClass::Bin && !atom.unary
                || atom.class == MathClass::Rel && !conditional)
            && !matches!(text, "\\cdot" | "\\mid")
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
    let ending = if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let mut output = format!("{ending}{}{}", layout.prefix, layout.unit);
    let mut width = layout.unit_width;
    for (index, range) in pieces.iter().enumerate() {
        let text = flat_piece(source, atoms, range)?;
        let piece_width = UnicodeWidthStr::width(text.as_str());
        if index > 0 {
            if width + 1 + piece_width <= layout.line_width {
                output.push(' ');
                width += 1
            } else {
                let relation = atoms
                    .binary_search_by_key(&range.start, |atom| atom.range.start)
                    .ok()
                    .is_some_and(|index| atoms[index].class == MathClass::Rel);
                let wanted = if relation {
                    relation_indent
                } else {
                    rhs_indent
                };
                let indent = if wanted + piece_width <= layout.line_width {
                    wanted
                } else {
                    layout.unit_width
                };
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

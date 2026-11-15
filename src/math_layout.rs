//! Math-only whitespace edits over lossless syntax and a static symbol vocabulary.

use badness_formatter::ast::{AstNode, Environment};
use badness_formatter::parser::{Parse, parse_with_flavor};
use badness_formatter::semantic::{DelimiterRole, MathClass, Mode, ModeIndex, math_atoms};
use badness_formatter::syntax::{SyntaxElement, SyntaxKind, SyntaxNode};
use badness_formatter::{MathWrap, WrapMode};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use unicode_width::UnicodeWidthStr;

use crate::{CompiledConfig, FileKind, FormatError, code_layout};

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
    if !config.config.math.enabled {
        return false;
    }
    if source.contains('$') {
        return true;
    }
    use badness_formatter::semantic::signature::{ArgumentDomain, builtin};
    for start in memchr::memchr_iter(b'\\', source.as_bytes()) {
        let Some((name, end)) = crate::syntax::command_at(source, start) else {
            continue;
        };
        if matches!(name, "(" | "[")
            || builtin().command(name).is_some_and(|signature| {
                signature
                    .args
                    .iter()
                    .any(|argument| argument.domain == ArgumentDomain::Math)
            })
        {
            return true;
        }
        if name == "begin"
            && let Some(body) = source[end..]
                .trim_start()
                .strip_prefix('{')
                .and_then(|text| text.split_once('}').map(|(name, _)| name))
            && builtin()
                .environment(body)
                .is_some_and(|signature| signature.math)
        {
            return true;
        }
    }
    false
}

pub(crate) fn format_math(
    source: &str,
    kind: FileKind,
    config: &CompiledConfig,
) -> Result<String, FormatError> {
    format_math_inner(source, kind, config, true)
}

fn format_math_inner(
    source: &str,
    kind: FileKind,
    config: &CompiledConfig,
    reuse: bool,
) -> Result<String, FormatError> {
    if !config.config.math.enabled || kind == FileKind::Bibtex {
        return Ok(source.to_owned());
    }
    let parsed = parse_with_flavor(source, code_layout::lex_config(kind));
    let root = parsed.syntax();
    let modes = ModeIndex::build(&root);
    let errors = error_ranges(&parsed);
    let mut protected = code_layout::protected_ranges(source, config, config.config.code.expl3)?;
    if kind == FileKind::Ins {
        protected.extend(crate::docstrip::preamble_ranges(&root, source.len()));
    }
    let display_protected = code_layout::merged(protected.clone());
    for node in root
        .descendants()
        .filter(|node| node.kind() == SyntaxKind::MATH)
    {
        let range = node_range(&node);
        if overlaps(&errors, &(range.start..range.end.saturating_add(1)))
            || unpaired_sized_delimiter(&node, &modes)
        {
            protected.push(range);
        }
    }
    let protected = code_layout::merged(protected);
    let mut editor = Editor {
        source,
        modes: &modes,
        protected: &protected,
        edits: BTreeMap::new(),
    };
    editor.visit(&root, false);
    let has_displays = root.descendants().any(|node| {
        node.kind() == SyntaxKind::DISPLAY_MATH
            || Environment::cast(node)
                .and_then(|environment| environment.name())
                .is_some_and(|name| {
                    matches!(name.as_str(), "equation" | "equation*" | "displaymath")
                })
    });
    let normalized = editor.finish();
    if has_displays {
        if reuse && normalized == source {
            break_displays_parsed(source, &root, &modes, &display_protected, &errors, config)
        } else {
            break_displays(&normalized, kind, config)
        }
    } else {
        Ok(normalized)
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

fn node_range(node: &SyntaxNode) -> Range<usize> {
    let range = node.text_range();
    usize::from(range.start())..usize::from(range.end())
}

fn error_ranges(parsed: &Parse) -> Vec<Range<usize>> {
    code_layout::merged(
        parsed
            .errors
            .iter()
            .map(|error| error.start..error.end.saturating_add(1))
            .collect(),
    )
}

fn overlaps(ranges: &[Range<usize>], range: &Range<usize>) -> bool {
    let first = ranges.partition_point(|entry| entry.end <= range.start);
    ranges
        .get(first)
        .is_some_and(|entry| entry.start < range.end)
}

fn trivia(kind: SyntaxKind) -> bool {
    matches!(kind, SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE)
}

fn whitespace(text: &str) -> bool {
    text.bytes().all(|byte| byte.is_ascii_whitespace())
}

fn group_elements(node: &SyntaxNode) -> Vec<SyntaxElement> {
    node.children_with_tokens()
        .filter(|element| {
            !matches!(
                element.kind(),
                SyntaxKind::L_BRACE
                    | SyntaxKind::R_BRACE
                    | SyntaxKind::L_BRACKET
                    | SyntaxKind::R_BRACKET
            )
        })
        .collect()
}

fn unpaired_sized_delimiter(node: &SyntaxNode, modes: &ModeIndex) -> bool {
    node.descendants_with_tokens()
        .filter_map(SyntaxElement::into_token)
        .any(|token| {
            matches!(token.text(), "\\left" | "\\right" | "\\middle")
                && modes.mode_at(usize::from(token.text_range().start())) == Mode::Math
                && !token
                    .parent_ancestors()
                    .take_while(|ancestor| ancestor != node)
                    .any(|ancestor| ancestor.kind() == SyntaxKind::LEFT_RIGHT)
        })
}

struct Editor<'a> {
    source: &'a str,
    modes: &'a ModeIndex,
    protected: &'a [Range<usize>],
    edits: BTreeMap<usize, (usize, String)>,
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

    fn visit(&mut self, node: &SyntaxNode, script: bool) {
        let range = node_range(node);
        if self.blocked(&range) {
            // Intersecting containers still need their unprotected descendants.
            let first = self
                .protected
                .partition_point(|entry| entry.end <= range.start);
            if self
                .protected
                .get(first)
                .is_some_and(|entry| entry.start <= range.start && range.end <= entry.end)
            {
                return;
            }
        }
        match node.kind() {
            SyntaxKind::MATH => self.sequence(node.children_with_tokens().collect(), script),
            SyntaxKind::GROUP | SyntaxKind::OPTIONAL
                if self.modes.mode_at(usize::from(node.text_range().start())) == Mode::Math =>
            {
                self.group(node, script);
            }
            _ => {
                for child in node.children() {
                    self.visit(&child, script);
                }
            }
        }
    }

    fn group(&mut self, node: &SyntaxNode, script: bool) {
        let elements = group_elements(node);
        let range = node_range(node);
        let opening = range.start + 1;
        let closing = range.end.saturating_sub(1);
        // A closing brace must stay after the newline ending a TeX comment.
        let contains_comment = node
            .descendants_with_tokens()
            .any(|element| element.kind() == SyntaxKind::COMMENT);
        if node.kind() == SyntaxKind::GROUP && opening <= closing && !contains_comment {
            let body = &self.source[opening..closing];
            let start = body.len() - body.trim_start_matches(char::is_whitespace).len();
            let end = body.trim_end_matches(char::is_whitespace).len();
            if start <= end {
                self.edit(opening..opening + start, "");
                self.edit(opening + end..closing, "");
            } else {
                self.edit(opening..closing, "");
            }
        }
        self.sequence(elements, script);
    }

    fn scripts(&mut self, node: &SyntaxNode, outer_script: bool) {
        let mut previous_end = None;
        for element in node.children_with_tokens() {
            if trivia(element.kind()) {
                continue;
            }
            if let Some(child) = element.as_node() {
                if matches!(
                    child.kind(),
                    SyntaxKind::SUBSCRIPT | SyntaxKind::SUPERSCRIPT
                ) {
                    let start = usize::from(child.text_range().start());
                    if let Some(end) = previous_end {
                        self.edit(end..start, "");
                    }
                    self.script(child);
                } else {
                    self.visit(child, outer_script);
                }
            }
            previous_end = Some(usize::from(element.text_range().end()));
        }
    }

    fn script(&mut self, node: &SyntaxNode) {
        let mut marker_end = None;
        for element in node.children_with_tokens() {
            if trivia(element.kind()) {
                continue;
            }
            if matches!(element.kind(), SyntaxKind::CARET | SyntaxKind::UNDERSCORE) {
                marker_end = Some(usize::from(element.text_range().end()));
                continue;
            }
            if let Some(end) = marker_end.take() {
                self.edit(end..usize::from(element.text_range().start()), "");
            }
            if let Some(child) = element.as_node() {
                match child.kind() {
                    SyntaxKind::GROUP => self.group(child, true),
                    SyntaxKind::SCRIPTED => self.scripts(child, true),
                    _ => self.visit(child, true),
                }
            }
        }
    }

    fn left_right(&mut self, node: &SyntaxNode, script: bool) {
        let mut delimiter_command = None;
        for element in node.children_with_tokens() {
            if trivia(element.kind()) {
                continue;
            }
            if let Some(child) = element.as_node() {
                self.visit(child, script);
            }
            if let Some(end) = delimiter_command.take() {
                self.edit(end..usize::from(element.text_range().start()), "");
            }
            if element
                .as_token()
                .is_some_and(|token| matches!(token.text(), "\\left" | "\\right" | "\\middle"))
            {
                delimiter_command = Some(usize::from(element.text_range().end()));
            }
        }
    }

    fn atoms(&mut self, elements: Vec<SyntaxElement>, script: bool) -> Vec<Atom> {
        let mut atoms = Vec::new();
        // An unknown command signature can attach adjacent brackets to a math
        // symbol. Zero-argument symbols keep their following operand separate,
        // so spacing never changes the atom structure on the next pass.
        let mut pending = elements;
        pending.reverse();
        while let Some(element) = pending.pop() {
            if let Some(node) = element.as_node()
                && node.kind() == SyntaxKind::COMMAND
                && argumentless_symbol(node, self.source)
            {
                let appended = pending.len();
                pending.extend(node.children_with_tokens());
                pending[appended..].reverse();
                continue;
            }
            if trivia(element.kind()) {
                continue;
            }
            if let Some(node) = element.as_node() {
                match node.kind() {
                    SyntaxKind::SCRIPTED => self.scripts(node, script),
                    SyntaxKind::GROUP | SyntaxKind::OPTIONAL => self.group(node, script),
                    SyntaxKind::LEFT_RIGHT => self.left_right(node, script),
                    _ => self.visit(node, script),
                }
            }
            let barrier = matches!(
                element.kind(),
                SyntaxKind::COMMENT
                    | SyntaxKind::DOC_MARGIN
                    | SyntaxKind::GUARD
                    | SyntaxKind::LINE_BREAK
                    | SyntaxKind::AMPERSAND
            );
            let command =
                element.kind() == SyntaxKind::COMMAND || element.kind() == SyntaxKind::CONTROL_WORD;
            for atom in math_atoms(&element) {
                let range = usize::from(atom.range.start())..usize::from(atom.range.end());
                let info = if command {
                    crate::syntax::command_at(self.source, range.start)
                        .map(|(name, _)| crate::math_symbols::command_info(name))
                } else {
                    None
                };
                let current = Atom {
                    barrier: barrier || self.blocked(&range),
                    range,
                    class: info.map_or(atom.class, |info| info.class),
                    delimiter: info.map_or(atom.delimiter, |info| info.delimiter),
                    command,
                    unary: false,
                };
                // Relation runs and colon-equals spellings remain indivisible.
                if let Some(previous) = atoms.last_mut() {
                    let previous: &mut Atom = previous;
                    let left = &self.source[previous.range.clone()];
                    let right = &self.source[current.range.clone()];
                    if previous.range.end == current.range.start
                        && !previous.command
                        && !current.command
                        && ((previous.class == MathClass::Rel && current.class == MathClass::Rel)
                            || left.chars().all(|character| character == ':')
                                && (right == ":" || right == "=")
                            || left == "!" && right == "=")
                    {
                        previous.range.end = current.range.end;
                        if current.class == MathClass::Rel {
                            previous.class = MathClass::Rel;
                        }
                        continue;
                    }
                }
                atoms.push(current);
            }
        }
        // A negation prefix belongs to the following relation, including its
        // authored control-word separator.
        let mut read = 0;
        let mut write = 0;
        while read < atoms.len() {
            if write != read {
                atoms[write] = atoms[read].clone();
            }
            read += 1;
            if read < atoms.len()
                && &self.source[atoms[write].range.clone()] == "\\not"
                && atoms[read].class == MathClass::Rel
                && whitespace(&self.source[atoms[write].range.end..atoms[read].range.start])
            {
                atoms[write].range.end = atoms[read].range.end;
                atoms[write].class = MathClass::Rel;
                atoms[write].delimiter = None;
                read += 1;
            }
            write += 1;
        }
        atoms.truncate(write);
        let mut previous_class = MathClass::Rel;
        let mut opener = false;
        for atom in &mut atoms {
            atom.unary = atom.class == MathClass::Bin
                && (opener
                    || matches!(
                        previous_class,
                        MathClass::Op | MathClass::Bin | MathClass::Rel | MathClass::Punct
                    ));
            if atom.barrier {
                previous_class = MathClass::Rel;
                opener = false;
            } else {
                previous_class = atom.class;
                opener = atom.delimiter == Some(DelimiterRole::Open);
            }
        }
        atoms
    }

    fn sequence(&mut self, elements: Vec<SyntaxElement>, script: bool) {
        let atoms = self.atoms(elements, script);
        for pair in atoms.windows(2) {
            let left = &pair[0];
            let right = &pair[1];
            let gap = left.range.end..right.range.start;
            let authored = &self.source[gap.clone()];
            if left.barrier || right.barrier || !whitespace(authored) {
                continue;
            }
            let left_text = &self.source[left.range.clone()];
            let right_text = &self.source[right.range.clone()];
            if left_text == "/" || right_text == "/" {
                self.edit(gap, "");
                continue;
            }
            // Delimiter-facing gaps retain their exact authored bytes.
            if left.delimiter == Some(DelimiterRole::Open)
                || right.delimiter == Some(DelimiterRole::Close)
            {
                continue;
            }
            let left_operator = matches!(left.class, MathClass::Bin | MathClass::Rel);
            let right_operator = matches!(right.class, MathClass::Bin | MathClass::Rel);
            if script && (left_operator && left.command || right_operator && right.command) {
                continue;
            }
            if script && (left_operator || right_operator) || left.unary {
                // Never merge a control word with the next ASCII letter.
                if !control_word_seam(left_text, right_text) {
                    self.edit(gap, "");
                }
            } else if (left_operator && !left.unary || right_operator && !right.unary)
                && !authored.contains(['\r', '\n'])
            {
                self.edit(gap, " ");
            }
        }
    }

    fn finish(self) -> String {
        let mut output = String::with_capacity(self.source.len());
        let mut cursor = 0;
        for (start, (end, value)) in self.edits {
            if start < cursor {
                continue;
            }
            output.push_str(&self.source[cursor..start]);
            output.push_str(&value);
            cursor = end;
        }
        output.push_str(&self.source[cursor..]);
        output
    }
}

fn control_word_seam(left: &str, right: &str) -> bool {
    left.starts_with('\\')
        && left[1..]
            .chars()
            .all(|character| character.is_ascii_alphabetic())
        && right
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphabetic)
}

fn argumentless_symbol(node: &SyntaxNode, source: &str) -> bool {
    let Some((name, _)) = crate::syntax::command_at(source, node_range(node).start) else {
        return false;
    };
    let info = crate::math_symbols::command_info(name);
    matches!(info.class, MathClass::Bin | MathClass::Rel | MathClass::Op)
        && badness_formatter::semantic::signature::builtin()
            .command(name)
            .is_none_or(|signature| signature.args.is_empty())
}

/// One lossless, non-whitespace lexical atom. WORD runs are split at scalar
/// boundaries so inserting operator spaces cannot change the token projection.
fn skeleton(node: &SyntaxNode) -> Vec<Range<usize>> {
    let mut result = Vec::new();
    for token in node
        .descendants_with_tokens()
        .filter_map(SyntaxElement::into_token)
    {
        if trivia(token.kind()) {
            continue;
        }
        let start = usize::from(token.text_range().start());
        if token.kind() == SyntaxKind::WORD {
            for (offset, character) in token.text().char_indices() {
                result.push(start + offset..start + offset + character.len_utf8());
            }
        } else {
            result.push(start..usize::from(token.text_range().end()));
        }
    }
    result
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
    let parsed = parse_with_flavor(source, code_layout::lex_config(kind));
    let root = parsed.syntax();
    let mut protected = code_layout::protected_ranges(source, config, config.config.code.expl3)?;
    if kind == FileKind::Ins {
        protected.extend(crate::docstrip::preamble_ranges(&root, source.len()));
    }
    let protected = code_layout::merged(protected);
    let modes = ModeIndex::build(&root);
    let errors = error_ranges(&parsed);
    break_displays_parsed(source, &root, &modes, &protected, &errors, config)
}

fn break_displays_parsed(
    source: &str,
    root: &SyntaxNode,
    modes: &ModeIndex,
    protected: &[Range<usize>],
    errors: &[Range<usize>],
    config: &CompiledConfig,
) -> Result<String, FormatError> {
    let mut atom_editor = Editor {
        source,
        modes,
        protected,
        edits: BTreeMap::new(),
    };
    let mut replacements = Vec::new();
    for node in root.descendants() {
        let display = node.kind() == SyntaxKind::DISPLAY_MATH
            || Environment::cast(node.clone())
                .and_then(|environment| environment.name())
                .is_some_and(|name| {
                    matches!(name.as_str(), "equation" | "equation*" | "displaymath")
                });
        if !display {
            continue;
        }
        let Some(body) = node
            .children()
            .find(|child| child.kind() == SyntaxKind::MATH)
        else {
            continue;
        };
        let range = node_range(&body);
        if overlaps(protected, &range)
            || overlaps(errors, &(range.start..range.end.saturating_add(1)))
            || unpaired_sized_delimiter(&body, atom_editor.modes)
            || body.descendants_with_tokens().any(|element| {
                matches!(
                    element.kind(),
                    SyntaxKind::COMMENT
                        | SyntaxKind::DOC_MARGIN
                        | SyntaxKind::GUARD
                        | SyntaxKind::LINE_BREAK
                )
            })
        {
            continue;
        }
        let body_text = &source[range.clone()];
        let start = usize::from(node.text_range().start());
        let line_start = source[..start].rfind('\n').map_or(0, |offset| offset + 1);
        let prefix = &source[line_start..start];
        if !prefix
            .chars()
            .all(|character| matches!(character, ' ' | '\t'))
        {
            continue;
        }
        let prefix_width = prefix
            .chars()
            .map(|character| {
                if character == '\t' {
                    config.config.indent.width
                } else {
                    1
                }
            })
            .sum::<usize>();
        let line_width = config
            .config
            .wrap
            .columns
            .saturating_sub(prefix_width)
            .max(1);
        let environment =
            Environment::cast(node.clone()).and_then(|environment| environment.name());
        let unit = if let Some(name) = &environment {
            config.config.indent.unit("environment", name)
        } else {
            let name = if source[start..].starts_with("$$") {
                "$$"
            } else {
                "displayMath"
            };
            config.config.indent.unit("special", name)
        };
        let unit_width = unit
            .chars()
            .map(|character| {
                if character == '\t' {
                    config.config.indent.width
                } else {
                    1
                }
            })
            .sum::<usize>();
        let layout = BreakLayout {
            prefix,
            unit: &unit,
            unit_width,
            line_width,
        };
        // Fitting authored display lines retain their boundaries. The operator
        // planner is needed only for an actual overflow.
        let first_line_prefix = &source[line_start..range.start];
        let mut overflows = false;
        for (index, line) in body_text.lines().enumerate() {
            let width = UnicodeWidthStr::width(line)
                + if index == 0 {
                    UnicodeWidthStr::width(first_line_prefix)
                } else {
                    0
                };
            overflows |= width > config.config.wrap.columns;
        }
        let flat_width = flat_display_width(body_text);
        if !overflows && flat_width + unit_width <= line_width {
            continue;
        }
        let atoms = atom_editor.atoms(body.children_with_tokens().collect(), false);
        let candidates = break_candidates(source, &atoms);
        if candidates.is_empty() {
            continue;
        }
        // The planner measures logical tokens, independent of an authored or
        // previously generated leading indent. Projection below preserves
        // noncandidate gaps and all atom contents in the original source.
        let planning_body = planning_body(source, &atoms);
        let wrapped = format!("\\[{planning_body}\\]");
        let mut style = code_layout::style(config);
        style.wrap = WrapMode::Preserve;
        style.math_wrap = MathWrap::Break;
        style.line_width = line_width;
        style.indent_width = unit_width;
        let Ok(formatted) = badness_formatter::formatter::format_with_style_flavored(
            &wrapped,
            style,
            code_layout::lex_config(FileKind::Tex),
        ) else {
            continue;
        };
        let formatted_root =
            parse_with_flavor(&formatted, code_layout::lex_config(FileKind::Tex)).syntax();
        let Some(formatted_body) = formatted_root
            .descendants()
            .find(|child| child.kind() == SyntaxKind::MATH)
        else {
            continue;
        };
        if let Some(mut projected) = project_breaks(
            body_text,
            &body,
            &formatted,
            &formatted_body,
            &layout,
            &candidates,
        ) {
            // The upstream planner can know fewer command aliases or measure
            // shorter ordinary gaps. A native pass handles those remaining
            // overflows using the same proven top-level operator boundaries.
            if projected
                .lines()
                .any(|line| UnicodeWidthStr::width(line) > config.config.wrap.columns)
                && let Some(fallback) = greedy_breaks(source, &body, &atoms, &candidates, &layout)
            {
                projected = fallback;
            }
            if projected != body_text {
                replacements.push((range, projected));
            }
        }
    }
    let mut output = String::with_capacity(source.len());
    let mut cursor = 0;
    replacements.sort_by_key(|(range, _)| range.start);
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

fn flat_display_width(body: &str) -> usize {
    body.lines()
        .enumerate()
        .map(|(index, line)| UnicodeWidthStr::width(line.trim()) + usize::from(index > 0))
        .sum()
}

fn project_breaks(
    original: &str,
    original_node: &SyntaxNode,
    formatted: &str,
    formatted_node: &SyntaxNode,
    layout: &BreakLayout<'_>,
    candidates: &BTreeSet<usize>,
) -> Option<String> {
    let original_atoms = skeleton(original_node);
    let formatted_atoms = skeleton(formatted_node);
    let base = usize::from(original_node.text_range().start());
    if original_atoms.len() != formatted_atoms.len()
        || original_atoms
            .iter()
            .zip(&formatted_atoms)
            .any(|(left, right)| {
                original[left.start - base..left.end - base] != formatted[right.clone()]
            })
    {
        return None;
    }
    let formatted_base = usize::from(formatted_node.text_range().start());
    let mut output = String::with_capacity(original.len());
    let mut previous = 0;
    let mut formatted_previous = formatted_base;
    let ending = if original.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    for (range, printed) in original_atoms.iter().zip(&formatted_atoms) {
        let start = range.start - base;
        let gap = &original[previous..start];
        let planned = &formatted[formatted_previous..printed.start];
        if planned.contains('\n')
            && whitespace(planned)
            && (previous == 0 || candidates.contains(&range.start))
        {
            output.push_str(&planned_gap(planned, ending, layout));
        } else if gap.contains('\n') && candidates.contains(&range.start) {
            output.push(' ');
        } else {
            output.push_str(gap);
        }
        output.push_str(&original[range.start - base..range.end - base]);
        previous = range.end - base;
        formatted_previous = printed.end;
    }
    let tail = &formatted[formatted_previous..usize::from(formatted_node.text_range().end())];
    if tail.contains('\n') && whitespace(tail) {
        output.push_str(&planned_gap(tail, ending, layout));
    } else {
        output.push_str(&original[previous..]);
    }
    Some(output)
}

fn planned_gap(gap: &str, ending: &str, layout: &BreakLayout<'_>) -> String {
    let BreakLayout {
        prefix,
        unit,
        unit_width,
        ..
    } = *layout;
    let normalized = gap.replace("\r\n", "\n");
    let mut output = String::new();
    for (index, part) in normalized.split('\n').enumerate() {
        if index == 0 {
            output.push_str(part);
            continue;
        }
        output.push_str(ending);
        output.push_str(prefix);
        if unit_width > 0 && part.len() >= unit_width && part.bytes().all(|byte| byte == b' ') {
            output.push_str(unit);
            output.push_str(&part[unit_width..]);
        } else {
            output.push_str(part);
        }
    }
    output
}

fn break_candidates(source: &str, atoms: &[Atom]) -> BTreeSet<usize> {
    let mut candidates = BTreeSet::new();
    let mut depth = 0isize;
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
            Some(DelimiterRole::Close) => depth = (depth - 1).max(0),
            _ => (),
        }
    }
    candidates
}

fn greedy_breaks(
    source: &str,
    body: &SyntaxNode,
    atoms: &[Atom],
    candidates: &BTreeSet<usize>,
    layout: &BreakLayout<'_>,
) -> Option<String> {
    let BreakLayout {
        prefix,
        unit,
        unit_width,
        line_width,
    } = *layout;
    let first = atoms.first()?;
    let last = atoms.last()?;
    let mut pieces = Vec::new();
    let mut start = first.range.start;
    let mut preceding_end = start;
    for atom in atoms {
        if candidates.contains(&atom.range.start) && atom.range.start > start {
            pieces.push(start..preceding_end);
            start = atom.range.start;
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
                .map(|text| unit_width + UnicodeWidthStr::width(text.as_str()) + 1)
        })
        .unwrap_or(unit_width);
    let relation_indent = first_relation
        .and_then(|relation| {
            flat_piece(source, atoms, &(first.range.start..relation.range.start))
                .map(|text| unit_width + UnicodeWidthStr::width(text.as_str()))
        })
        .unwrap_or(unit_width);
    let body_range = node_range(body);
    let ending = if source[body_range].contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let mut output = format!("{ending}{prefix}{unit}");
    let mut width = unit_width;
    for (index, range) in pieces.iter().enumerate() {
        let text = flat_piece(source, atoms, range)?;
        let piece_width = UnicodeWidthStr::width(text.as_str());
        if index > 0 {
            if width + 1 + piece_width <= line_width {
                output.push(' ');
                width += 1;
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
                let indent = if wanted + piece_width <= line_width {
                    wanted
                } else {
                    unit_width
                };
                output.push_str(ending);
                output.push_str(prefix);
                output.push_str(unit);
                output.push_str(&" ".repeat(indent.saturating_sub(unit_width)));
                width = indent;
            }
        }
        output.push_str(&text);
        width += piece_width;
    }
    output.push_str(ending);
    output.push_str(prefix);
    Some(output)
}

fn flat_piece(source: &str, atoms: &[Atom], range: &Range<usize>) -> Option<String> {
    let mut output = String::new();
    let mut cursor = range.start;
    let first = atoms.partition_point(|atom| atom.range.start < range.start);
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
            output.push(' ');
        } else {
            output.push_str(gap);
        }
        output.push_str(text);
        cursor = atom.range.end;
    }
    let tail = &source[cursor..range.end];
    if tail.contains(['\r', '\n']) {
        output.push(' ');
    } else {
        output.push_str(tail);
    }
    Some(output)
}

fn planning_body(source: &str, atoms: &[Atom]) -> String {
    let mut output = String::new();
    let mut previous = None;
    for atom in atoms {
        if let Some(end) = previous {
            let gap = &source[end..atom.range.start];
            if gap.contains(['\r', '\n']) {
                output.push(' ');
            } else {
                output.push_str(gap);
            }
        }
        output.push_str(&source[atom.range.clone()]);
        previous = Some(atom.range.end);
    }
    output
}



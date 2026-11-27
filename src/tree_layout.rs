//! Conservative layouts for literal bracket trees.

use crate::{CompiledConfig, FormatError, syntax};
use serde::{Deserialize, Serialize};
use std::ops::Range;
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TreeMode {
    #[default]
    Preserve,
    Auto,
    Expand,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct TreeConfig {
    pub mode: TreeMode,
    pub columns: Option<usize>,
    pub forest: bool,
    pub qtree: bool,
    pub dirtree: bool,
}

impl Default for TreeConfig {
    fn default() -> Self {
        Self {
            mode: TreeMode::Preserve,
            columns: None,
            forest: true,
            qtree: true,
            dirtree: true,
        }
    }
}

pub(crate) fn validate(config: &TreeConfig) -> Result<(), String> {
    if config
        .columns
        .is_some_and(|width| !(1..=1_000_000).contains(&width))
    {
        return Err("trees.columns must be between 1 and 1000000".into());
    }
    Ok(())
}

pub(crate) fn tree_edits(
    source: &str,
    config: &CompiledConfig,
    parsing: &mut crate::parse_context::ParseContext<'_>,
) -> Result<crate::edit_plan::RuleEdits, FormatError> {
    let settings = &config.config.trees;
    let bracket_trees = settings.mode != TreeMode::Preserve;
    let forest = bracket_trees && settings.forest && source.contains("forest");
    let qtree = bracket_trees && settings.qtree && source.contains("\\Tree");
    let directory = settings.dirtree && source.contains(r"\dirtree");
    if !(forest || qtree || directory) {
        return Ok(Default::default());
    }
    let parsed = parsing.snapshot(source, true, false)?;
    let tokens = parsed.tokens.as_deref().expect("tree tokens");
    let analysis = parsed.analysis(&config.config);
    let mut roots = Vec::new();
    let mut plan = crate::edit_plan::RuleEdits::default();
    if forest
        && !analysis.declared_environments().contains("forest")
        && !config
            .config
            .syntax
            .environment_arguments
            .contains_key("forest")
    {
        for env in parsed.environment_ranges(&config.config)? {
            if env.name != "forest" || syntax::is_protected(env.begin.start, &parsed.protected) {
                continue;
            }
            let header = crate::layout::environment_header_end(
                source,
                env.body.clone(),
                "forest",
                &config.config,
                &analysis,
            );
            let line = source[..env.begin.start].rfind('\n').map_or(0, |at| at + 1);
            plan.reads.push(line..env.body.end);
            // Forest options preceding the root stay opaque.
            if let Some(root) = forest_root(tokens, header..env.body.end)
                && !syntax::overlaps(&root, &parsed.protected)
            {
                roots.push((root, false));
            }
        }
    }
    if qtree
        && !analysis.declared_commands().contains("Tree")
        && !config.config.syntax.commands.contains_key("Tree")
    {
        for token in tokens {
            if token.kind != syntax::TokenKind::Command
                || syntax::is_protected(token.span.start, &parsed.protected)
            {
                continue;
            }
            if let Some(("Tree", end)) = analysis
                .model()
                .control_at(token.span.start)
                .or_else(|| syntax::command_at(source, token.span.start))
            {
                let start = crate::trivia::skip(source, end);
                let Some(end) = syntax::balanced_end(source, start, b'[', b']') else {
                    continue;
                };
                let line = source[..token.span.start]
                    .rfind('\n')
                    .map_or(0, |at| at + 1);
                plan.reads.push(line..end);
                if let Some(literal) = crate::packages::tikz_diagrams::literal_data_range(
                    source,
                    token.span.end,
                    "Tree",
                    false,
                ) && literal.end >= end
                    && !parsed
                        .protected
                        .iter()
                        .any(|range| range.start < end && start < range.end && *range != literal)
                {
                    roots.push((start..end, true));
                }
            }
        }
    }
    roots.sort_by_key(|(range, _)| range.start);
    if directory
        && !config.config.syntax.commands.contains_key("dirtree")
        && !analysis.declared_commands().contains("dirtree")
    {
        for token in tokens {
            if token.kind == syntax::TokenKind::Command
                && &source[token.span.clone()] == r"\dirtree"
            {
                let line = source[..token.span.start]
                    .rfind('\n')
                    .map_or(0, |at| at + 1);
                let open = crate::trivia::skip(source, token.span.end);
                let Some(end) = syntax::balanced_end(source, open, b'{', b'}') else {
                    continue;
                };
                let tail = source[end..].find('\n').map_or(source.len(), |at| end + at);
                plan.reads.push(line..tail);
                if let Some((span, text)) = directory_tree(
                    source,
                    token.span.start,
                    open..end,
                    &parsed.protected,
                    config,
                ) {
                    plan.edits.replace(span, text);
                }
            }
        }
    }
    let mut last_end = 0;
    for (root, qtree) in roots {
        if root.start < last_end {
            continue;
        }
        last_end = root.end;
        let line = source[..root.start].rfind('\n').map_or(0, |at| at + 1);
        let prefix = &source[line..root.start];
        let indent: String = prefix
            .chars()
            .take_while(|c| matches!(c, ' ' | '\t'))
            .collect();
        let width = settings.columns.unwrap_or(config.config.wrap.columns);
        if settings.mode == TreeMode::Auto
            && !source[line..root.end]
                .lines()
                .any(|line| UnicodeWidthStr::width(line) > width)
        {
            continue;
        }
        if let Some(text) = render(source, root.clone(), qtree, &indent, config) {
            plan.edits.replace(root, text);
        }
    }
    Ok(plan)
}

fn directory_tree(
    source: &str,
    command: usize,
    argument: Range<usize>,
    protected: &[Range<usize>],
    config: &CompiledConfig,
) -> Option<(Range<usize>, String)> {
    let base = source[..command].rsplit('\n').next()?;
    if !base.trim_matches([' ', '\t']).is_empty() {
        return None;
    }
    let open = argument.start;
    let end = argument.end;
    let tail = source[end..].split('\n').next()?.trim();
    if (!tail.is_empty() && !tail.starts_with('%')) || syntax::overlaps(&(command..end), protected)
    {
        return None;
    }
    let body = open + 1..end - 1;
    let (header, nodes) = source[body.clone()].split_once('\n')?;
    if !header.trim().is_empty() && !header.trim_start().starts_with('%') {
        return None;
    }
    let unit = config.config.indent.unit("environment", "dirtree");
    let mut edits = crate::edit_plan::EditPlan::new();
    let mut previous = 0usize;
    let mut offset = header.len() + 1;
    for line in nodes.split_inclusive('\n') {
        let text = line.trim_start_matches([' ', '\t']);
        if text.trim().is_empty() || text.starts_with('%') {
            offset += line.len();
            continue;
        }
        let mut code = text;
        if let Some((comment, _)) = text
            .match_indices('%')
            .find(|(at, _)| !syntax::is_escaped(text, *at))
        {
            code = &text[..comment];
            if !code.ends_with([' ', '\t']) {
                return None;
            }
        }
        let record = code
            .trim_end_matches(['\r', '\n', ' ', '\t'])
            .strip_prefix('.')?;
        let (digits, name) = record.split_once([' ', '\t'])?;
        if !digits.bytes().all(|b| b.is_ascii_digit()) || !literal_directory_name(name) {
            return None;
        }
        let depth = digits.parse::<usize>().ok()?;
        if depth == 0 || depth > previous + 1 {
            return None;
        }
        previous = depth;
        let levels = if let Some(max) = config.config.indent.maximum {
            let width = |s| crate::wrap::display_width(s, 0, config.config.indent.width.max(1));
            depth.min(max.saturating_sub(width(base)) / width(&unit).max(1))
        } else {
            depth
        };
        let leading = line.len() - text.len();
        let mut indentation = base.to_owned();
        indentation.push_str(&unit.repeat(levels));
        if indentation != line[..leading] {
            edits.replace(offset..offset + leading, indentation);
        }
        offset += line.len();
    }
    let output = edits.apply_ordered(&source[body.clone()]).ok()?;
    (previous > 0 && output != source[body.clone()]).then_some((body, output))
}

fn literal_directory_name(name: &str) -> bool {
    let Some(label) = name.strip_suffix('.') else {
        return false;
    };
    let mut pos = 0;
    while pos < label.len() {
        match label.as_bytes()[pos] {
            b'{' => {
                let Some(end) = syntax::balanced_end(label, pos, b'{', b'}') else {
                    return false;
                };
                pos = end;
            }
            b'\\' => {
                let Some((_, end)) = syntax::command_at(label, pos) else {
                    return false;
                };
                pos = end;
            }
            b'%' | b'}' => return false,
            b'.' if label
                .as_bytes()
                .get(pos + 1)
                .is_some_and(u8::is_ascii_whitespace) =>
            {
                return false;
            }
            _ => pos += label[pos..].chars().next().unwrap().len_utf8(),
        }
    }
    true
}

fn forest_root(tokens: &[syntax::Token], body: Range<usize>) -> Option<Range<usize>> {
    use syntax::TokenKind;

    let first = tokens.partition_point(|token| token.span.start < body.start);
    let mut start = None;
    let mut braces = 0usize;
    let mut nodes = 0usize;
    for token in tokens[first..]
        .iter()
        .take_while(|t| t.span.start < body.end)
    {
        match token.kind {
            TokenKind::OpenBrace => braces += 1,
            TokenKind::CloseBrace => braces = braces.checked_sub(1)?,
            TokenKind::OpenBracket if braces == 0 => {
                start.get_or_insert(token.span.start);
                nodes += 1;
            }
            TokenKind::CloseBracket if braces == 0 => {
                nodes = nodes.checked_sub(1)?;
                if nodes == 0 {
                    return Some(start?..token.span.end);
                }
            }
            _ => (),
        }
    }
    None
}

pub(crate) enum ForestGroup<'s> {
    Node,
    KeyValue(&'s str),
    LiteralBracket,
}

/// Braces shield node content and PGF values from Forest's bracket parser.
/// Paired brackets inside a brace may still be ordinary TeX command options.
pub(crate) fn forest_groups<'s>(
    source: &'s str,
    body: Range<usize>,
    tokens: &[syntax::Token],
    groups: &mut std::collections::HashMap<usize, ForestGroup<'s>>,
) {
    use syntax::TokenKind;

    let Some(root) = forest_root(tokens, body.clone()) else {
        return;
    };
    let first = tokens.partition_point(|token| token.span.start < body.start);
    let mut braces: Vec<(Option<&str>, usize)> = Vec::new();
    let mut inner_brackets = Vec::new();
    let mut key = None;
    for token in tokens[first..]
        .iter()
        .take_while(|t| t.span.start < root.end)
    {
        match token.kind {
            TokenKind::OpenBracket if braces.is_empty() && token.span.start >= root.start => {
                groups.insert(token.span.start, ForestGroup::Node);
                key = None;
            }
            TokenKind::OpenBracket if !braces.is_empty() => {
                inner_brackets.push(token.span.start);
                key = None;
            }
            TokenKind::CloseBracket => {
                if braces
                    .last()
                    .is_some_and(|&(_, base)| inner_brackets.len() > base)
                {
                    inner_brackets.pop();
                }
                key = None;
            }
            TokenKind::Text if braces.last().is_none_or(|(owner, _)| owner.is_some()) => {
                // A PGF key may take adjacent arguments, e.g. if level=1{...}{...}.
                let text = &source[token.span.clone()];
                let entry = if let Some((_, last)) = text.rsplit_once(',') {
                    key = None;
                    last
                } else {
                    text
                };
                if let Some((name, _)) = entry.split_once('=') {
                    key = Some(name.trim()).filter(|name| !name.is_empty());
                }
            }
            TokenKind::OpenBrace => {
                if let Some(name) = key {
                    groups.insert(token.span.start, ForestGroup::KeyValue(name));
                }
                braces.push((key.take(), inner_brackets.len()));
            }
            TokenKind::CloseBrace => {
                if let Some((owner, base)) = braces.pop() {
                    key = owner;
                    for at in inner_brackets.drain(base..) {
                        groups.insert(at, ForestGroup::LiteralBracket);
                    }
                }
            }
            TokenKind::Comment | TokenKind::Space | TokenKind::Newline => (),
            _ => key = None,
        }
    }
}

fn render(
    source: &str,
    root: Range<usize>,
    qtree: bool,
    prefix: &str,
    config: &CompiledConfig,
) -> Option<String> {
    let unit = config
        .config
        .indent
        .unit("environment", if qtree { "Tree" } else { "forest" });
    let newline = if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let mut stack = Vec::<bool>::new();
    let mut boundaries = Vec::new();
    let mut at = root.start;
    while at < root.end {
        match source.as_bytes()[at] {
            b'{' => at = syntax::balanced_end(source, at, b'{', b'}')?,
            b'%' => {
                let end = source[at..root.end]
                    .find('\n')
                    .map_or(root.end, |n| at + n + 1);
                if matches!(
                    source[at + 1..end].trim(),
                    "tift: off" | "tift: on" | "tex-fmt: off" | "tex-fmt: on"
                ) {
                    return None;
                }
                at = end;
            }
            b'$' => {
                let delimiter = if source[at..].starts_with("$$") {
                    "$$"
                } else {
                    "$"
                };
                let mut end = at + delimiter.len();
                while end < root.end
                    && (!source[end..].starts_with(delimiter) || syntax::is_escaped(source, end))
                {
                    end += source[end..].chars().next()?.len_utf8();
                }
                if end >= root.end {
                    return None;
                }
                at = end + delimiter.len();
            }
            b'\\' => {
                let (name, end) = syntax::command_at(source, at)?;
                if matches!(
                    name,
                    "node" | "edge" | "verb" | "verb*" | "catcode" | "scantokens"
                ) || source.as_bytes().get(end) == Some(&b'[')
                {
                    return None;
                }
                if matches!(name, "(" | "[") {
                    let close = if name == "(" { "\\)" } else { "\\]" };
                    at = end + source[end..root.end].find(close)? + close.len();
                } else {
                    at = end;
                }
            }
            b'[' => {
                if stack.len() >= 128 {
                    return None;
                }
                if let Some(parent) = stack.last_mut() {
                    *parent = true;
                    boundaries.push((at, stack.len()));
                }
                stack.push(false);
                at += 1;
            }
            b']' => {
                let children = stack.pop()?;
                if children {
                    boundaries.push((at, stack.len()));
                }
                at += 1;
            }
            _ => at += source[at..].chars().next()?.len_utf8(),
        }
    }
    if !stack.is_empty() {
        return None;
    }
    let mut edits = Vec::new();
    for (position, depth) in boundaries {
        let mut start = position;
        while start > root.start && source.as_bytes()[start - 1].is_ascii_whitespace() {
            start -= 1;
        }
        let gap = &source[start..position];
        if gap.bytes().filter(|b| *b == b'\n').count() > 1 {
            continue;
        }
        // qtree uses whitespace to delimit labels; a tight seam is ambiguous.
        if qtree && gap.is_empty() {
            continue;
        }
        let mut text = String::new();
        if gap.is_empty() {
            text.push('%');
        }
        text.push_str(newline);
        text.push_str(prefix);
        text.push_str(&unit.repeat(depth));
        edits.push((start..position, text));
    }
    let mut plan = crate::edit_plan::EditPlan::new();
    for (range, text) in edits {
        plan.replace(range.start - root.start..range.end - root.start, text);
    }
    plan.apply_ordered(&source[root]).ok()
}

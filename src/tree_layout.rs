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
}

impl Default for TreeConfig {
    fn default() -> Self {
        Self {
            mode: TreeMode::Preserve,
            columns: None,
            forest: true,
            qtree: true,
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

pub(crate) fn format_with_context(
    source: &str,
    config: &CompiledConfig,
    parsing: &mut crate::parse_context::ParseContext<'_>,
) -> Result<String, FormatError> {
    let settings = &config.config.trees;
    let forest = settings.forest && source.contains("forest");
    let qtree = settings.qtree && source.contains("\\Tree");
    if settings.mode == TreeMode::Preserve || !(forest || qtree) {
        return Ok(source.into());
    }
    let parsed = parsing.snapshot(source, true, false)?;
    let tokens = parsed.tokens.as_deref().expect("tree tokens");
    let analysis = crate::source_analysis::SourceAnalysis::with_memo(
        source,
        &config.config,
        &parsed.protected,
        std::rc::Rc::clone(&parsed.metadata),
    );
    let mut roots = Vec::new();
    if forest
        && !analysis.declared_environments()?.contains("forest")
        && !config
            .config
            .syntax
            .environment_arguments
            .contains_key("forest")
    {
        for env in syntax::environment_ranges_with_tokens(
            source,
            tokens,
            &parsed.protected,
            &config.config,
        )? {
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
            // Forest options preceding the root stay opaque.
            if let Some(start) = forest_root(source, header..env.body.end)
                && let Some(end) = syntax::balanced_end(source, start, b'[', b']')
                && end <= env.body.end
                && !parsed
                    .protected
                    .iter()
                    .any(|range| range.start < end && start < range.end)
            {
                roots.push((start..end, false));
            }
        }
    }
    if qtree
        && !analysis.declared_commands()?.contains("Tree")
        && !config.config.syntax.commands.contains_key("Tree")
    {
        for token in tokens {
            if token.kind != syntax::TokenKind::Command
                || syntax::is_protected(token.span.start, &parsed.protected)
            {
                continue;
            }
            if let Some(("Tree", end)) = syntax::command_at(source, token.span.start) {
                let start = crate::trivia::skip(source, end);
                if let Some(end) = syntax::balanced_end(source, start, b'[', b']')
                    && let Some(literal) = crate::packages::tikz_diagrams::literal_data_range(
                        source,
                        token.span.end,
                        "Tree",
                        false,
                    )
                    && literal.end >= end
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
    let mut edits = Vec::new();
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
            edits.push((root, text));
        }
    }
    crate::layout::apply_edits(source, edits)
}

fn forest_root(source: &str, range: Range<usize>) -> Option<usize> {
    let mut at = range.start;
    while at < range.end {
        match source.as_bytes()[at] {
            b'[' => return Some(at),
            b'{' => at = syntax::balanced_end(source, at, b'{', b'}')?,
            b'%' => at = source[at..].find('\n').map_or(range.end, |n| at + n + 1),
            b'\\' => at = syntax::command_at(source, at)?.1,
            _ => at += source[at..].chars().next()?.len_utf8(),
        }
    }
    None
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
    let mut output = String::new();
    let mut copied = root.start;
    for (range, text) in edits {
        output.push_str(&source[copied..range.start]);
        output.push_str(&text);
        copied = range.end;
    }
    output.push_str(&source[copied..root.end]);
    Some(output)
}

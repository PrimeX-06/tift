//! Literal TikZ statement boundaries. TeX node text and option values stay opaque.
use std::collections::HashSet;
use std::ops::Range;

use crate::block_storage::{BlockName, StructuralBlock as Block};
use crate::config::BreakMode;
use crate::source_analysis::SourceAnalysis;
use crate::syntax::{self, BlockKind, Token, TokenKind};
use crate::tikz_scan::{comment_end, group_end, node_body, path_command};
use crate::{CompiledConfig, FormatError};

fn math_end(source: &str, start: usize, limit: usize) -> Option<usize> {
    let double = source.as_bytes().get(start + 1) == Some(&b'$');
    let mut cursor = start + 1 + usize::from(double);
    while cursor < limit {
        match source.as_bytes()[cursor] {
            b'%' => cursor = comment_end(source, cursor, limit),
            b'\\' => cursor = syntax::command_at(source, cursor)?.1,
            b'$' if !double || source.as_bytes().get(cursor + 1) == Some(&b'$') => {
                return Some(cursor + 1 + usize::from(double));
            }
            _ => cursor += 1,
        }
    }
    None
}

struct Statement {
    end: usize,
    children: Vec<Range<usize>>,
    matrix: bool,
}

fn child_scope(
    source: &str,
    start: usize,
    limit: usize,
    protected: &[Range<usize>],
) -> Option<Range<usize>> {
    let mut cursor = crate::trivia::skip_until(source, start, limit);
    if source.as_bytes().get(cursor) == Some(&b'[') {
        cursor =
            crate::trivia::skip_until(source, group_end(source, cursor, limit, protected)?, limit);
    }
    if source.as_bytes().get(cursor) != Some(&b'{') {
        return None;
    }
    let end = group_end(source, cursor, limit, protected)?;
    Some(cursor + 1..end - 1)
}

fn keyword(source: &str, cursor: usize, limit: usize, name: &str) -> bool {
    source[cursor..limit].starts_with(name)
        && (cursor == 0 || !source.as_bytes()[cursor - 1].is_ascii_alphabetic())
        && source
            .as_bytes()
            .get(cursor + name.len())
            .is_some_and(|byte| {
                byte.is_ascii_whitespace() || matches!(byte, b'[' | b'(' | b'{' | b'%' | b';')
            })
}

fn child_details(
    source: &str,
    body: Range<usize>,
    protected: &[Range<usize>],
    text: &mut Vec<Range<usize>>,
    children: &mut Vec<Range<usize>>,
    depth: usize,
) {
    if depth > 256 {
        return;
    }
    let mut cursor = body.start;
    while cursor < body.end {
        if let Some(range) = protected
            .get(protected.partition_point(|range| range.end <= cursor))
            .filter(|range| range.start <= cursor)
        {
            cursor = range.end.min(body.end);
            continue;
        }
        match source.as_bytes()[cursor] {
            b'%' => cursor = comment_end(source, cursor, body.end),
            b'\\' => cursor = syntax::command_at(source, cursor).map_or(cursor + 1, |(_, end)| end),
            b'n' if keyword(source, cursor, body.end, "node") => {
                if let Some(node) = node_body(source, cursor + 4, body.end, protected) {
                    cursor = node.end + 1;
                    text.push(node);
                } else {
                    cursor += 4;
                }
            }
            b'c' if keyword(source, cursor, body.end, "child") => {
                if let Some(child) = child_scope(source, cursor + 5, body.end, protected) {
                    cursor = child.end + 1;
                    children.push(child.clone());
                    child_details(source, child, protected, text, children, depth + 1);
                } else {
                    cursor += 5;
                }
            }
            b'{' | b'[' | b'(' => {
                let Some(end) = group_end(source, cursor, body.end, protected) else {
                    return;
                };
                cursor = end;
            }
            _ => cursor += 1,
        }
    }
}

fn matrix_option(source: &str, start: usize, limit: usize, protected: &[Range<usize>]) -> bool {
    let cursor = crate::trivia::skip_until(source, start, limit);
    if source.as_bytes().get(cursor) != Some(&b'[') {
        return false;
    }
    let Some(end) = group_end(source, cursor, limit, protected) else {
        return false;
    };
    let mut cursor = cursor + 1;
    let mut segment = cursor;
    let mut matrix = false;
    while cursor < end {
        match source.as_bytes()[cursor] {
            b'{' | b'[' | b'(' => {
                let Some(next) = group_end(source, cursor, end, protected) else {
                    return false;
                };
                cursor = next;
            }
            b'%' => cursor = comment_end(source, cursor, end),
            b'\\' => {
                cursor = syntax::command_at(source, cursor).map_or(cursor + 1, |(_, next)| next)
            }
            b',' | b']' => {
                let mut option = source[segment..cursor].splitn(2, '=');
                let key = option.next().unwrap_or("").trim();
                if matches!(
                    key,
                    "matrix" | "matrix of nodes" | "matrix of math nodes" | "/tikz/matrix"
                ) {
                    matrix = option.next().is_none_or(|value| value.trim() == "true");
                }
                cursor += 1;
                segment = cursor;
            }
            _ => cursor += 1,
        }
    }
    matrix
}

fn statement(
    source: &str,
    name: &str,
    start: usize,
    limit: usize,
    protected: &[Range<usize>],
    text: &mut Vec<Range<usize>>,
    mut operation: impl FnMut(usize),
) -> Option<Statement> {
    let mut cursor = start;
    let mut children = Vec::new();
    let matrix =
        name == "matrix" || name == "node" && matrix_option(source, start, limit, protected);
    if matches!(name, "node" | "matrix")
        && let Some(body) = node_body(source, cursor, limit, protected)
    {
        cursor = body.end + 1;
        text.push(body);
    }
    while cursor < limit {
        if let Some(range) = protected
            .get(protected.partition_point(|range| range.end <= cursor))
            .filter(|range| range.start <= cursor)
        {
            if range.end > limit {
                return None;
            }
            cursor = range.end;
            continue;
        }
        match source.as_bytes()[cursor] {
            b'%' => cursor = comment_end(source, cursor, limit),
            b'-' if source[cursor..limit].starts_with("--") => {
                operation(cursor);
                cursor += 2;
            }
            b'.' if source[cursor..limit].starts_with("..") => {
                let next = crate::trivia::skip_until(source, cursor + 2, limit);
                if keyword(source, next, limit, "controls") {
                    operation(cursor);
                }
                cursor += 2;
            }
            b'{' | b'[' | b'(' => cursor = group_end(source, cursor, limit, protected)?,
            b'$' => cursor = math_end(source, cursor, limit)?,
            b'\\' => {
                let (command, end) = syntax::command_at(source, cursor)?;
                if matches!(command, "begin" | "end") || path_command(command) {
                    return None;
                }
                if matches!(command, "(" | "[") {
                    let close = if command == "(" { "\\)" } else { "\\]" };
                    cursor = source[end..limit]
                        .find(close)
                        .map(|offset| end + offset + 2)?;
                } else {
                    cursor = end;
                }
            }
            b'}' => return None,
            b';' => {
                return Some(Statement {
                    end: cursor + 1,
                    children,
                    matrix,
                });
            }
            b'n' if keyword(source, cursor, limit, "node") => {
                operation(cursor);
                if let Some(body) = node_body(source, cursor + 4, limit, protected) {
                    cursor = body.end + 1;
                    text.push(body);
                } else {
                    cursor += 4;
                }
            }
            b'c' if keyword(source, cursor, limit, "child") => {
                if let Some(child) = child_scope(source, cursor + 5, limit, protected) {
                    cursor = child.end + 1;
                    children.push(child.clone());
                    child_details(source, child, protected, text, &mut children, 0);
                } else {
                    cursor += 5;
                }
            }
            _ => cursor += 1,
        }
    }
    None
}

struct Scanner<'a, N: BlockName> {
    source: &'a str,
    tokens: &'a [Token],
    protected: &'a [Range<usize>],
    declared: &'a HashSet<String>,
    config: &'a CompiledConfig,
    starts: Vec<usize>,
    ends: Vec<(usize, usize)>,
    headers: Vec<Range<usize>>,
    output: Vec<Block<N>>,
}

impl<N: BlockName> Scanner<'_, N> {
    fn collect(&mut self, scope: Range<usize>, tikz: bool, depth: usize) {
        if depth > 256 {
            return;
        }
        let first = self
            .tokens
            .partition_point(|token| token.span.start < scope.start);
        let last = self
            .tokens
            .partition_point(|token| token.span.start < scope.end);
        let mut starts = self.starts.partition_point(|&start| start < scope.start);
        let mut ends = self.ends.partition_point(|&(end, _)| end <= scope.start);
        let mut active = 0usize;
        let mut skip = scope.start;
        for index in first..last {
            let token = &self.tokens[index];
            let pos = token.span.start;
            while starts < self.starts.len() && self.starts[starts] <= pos {
                active += 1;
                starts += 1;
            }
            while ends < self.ends.len() && self.ends[ends].0 <= pos {
                if self.ends[ends].1 >= scope.start {
                    active = active.saturating_sub(1);
                }
                ends += 1;
            }
            if pos < skip
                || token.kind != TokenKind::Command
                || syntax::is_protected(pos, &self.headers)
            {
                continue;
            }
            let Some((name, command_end)) = syntax::command_at(self.source, pos) else {
                continue;
            };
            if self.declared.contains(name) || self.config.config.syntax.commands.contains_key(name)
            {
                continue;
            }
            if name == "tikz" {
                let mut cursor = crate::trivia::skip_until(self.source, command_end, scope.end);
                if self.source.as_bytes().get(cursor) == Some(&b'[') {
                    let Some(end) = group_end(self.source, cursor, scope.end, self.protected)
                    else {
                        continue;
                    };
                    cursor = crate::trivia::skip_until(self.source, end, scope.end);
                }
                if self.source.as_bytes().get(cursor) == Some(&b'{') {
                    if let Some(end) = group_end(self.source, cursor, scope.end, self.protected) {
                        self.output.push(Block {
                            kind: BlockKind::TikzPath,
                            name: "tikz-scope".into(),
                            begin: pos..cursor + 1,
                            body: cursor + 1..end - 1,
                            end: end - 1..end,
                            parent: None,
                            branches: Vec::new(),
                        });
                        self.collect(cursor + 1..end - 1, true, depth + 1);
                        skip = end;
                    }
                } else if let Some((path, path_end)) = syntax::command_at(self.source, cursor)
                    && path_command(path)
                    && !self.declared.contains(path)
                    && !self.config.config.syntax.commands.contains_key(path)
                {
                    let mut text = Vec::new();
                    if let Some(statement) = statement(
                        self.source,
                        path,
                        path_end,
                        scope.end,
                        self.protected,
                        &mut text,
                        |_| {},
                    ) {
                        skip = statement.end;
                        self.push_path("tikz-inline", pos..path_end, statement, text, depth);
                    } else {
                        skip = skip.max(self.preserve_text(text, depth));
                    }
                }
            } else if matches!(name, "tikzset" | "pgfkeys" | "pgfqkeys") {
                let mut cursor = crate::trivia::skip_until(self.source, command_end, scope.end);
                let arguments = if name == "pgfqkeys" { 2 } else { 1 };
                for _ in 0..arguments {
                    if self.source.as_bytes().get(cursor) != Some(&b'{') {
                        break;
                    }
                    let Some(end) = group_end(self.source, cursor, scope.end, self.protected)
                    else {
                        break;
                    };
                    skip = end;
                    cursor = crate::trivia::skip_until(self.source, end, scope.end);
                }
            } else if path_command(name) && (tikz || active > 0) {
                let mut text = Vec::new();
                if let Some(statement) = statement(
                    self.source,
                    name,
                    command_end,
                    scope.end,
                    self.protected,
                    &mut text,
                    |_| {},
                ) {
                    skip = statement.end;
                    self.push_path(name, pos..command_end, statement, text, depth);
                } else {
                    skip = skip.max(self.preserve_text(text, depth));
                }
            }
        }
    }

    fn preserve_text(&mut self, text: Vec<Range<usize>>, depth: usize) -> usize {
        let mut end = 0;
        for text in text {
            end = end.max(text.end + 1);
            self.collect(text, false, depth + 1);
        }
        end
    }

    fn push_path(
        &mut self,
        name: &str,
        begin: Range<usize>,
        statement: Statement,
        text: Vec<Range<usize>>,
        depth: usize,
    ) {
        let mut body_start = begin.end;
        let mut first = true;
        for (index, text) in text
            .iter()
            .map(Some)
            .chain(std::iter::once(None))
            .enumerate()
        {
            let body_end = text.map_or(statement.end - 1, |text| text.start - 1);
            self.output.push(Block {
                kind: BlockKind::TikzPath,
                name: name.into(),
                begin: if first {
                    begin.clone()
                } else {
                    body_start..body_start
                },
                body: body_start..body_end,
                end: if first {
                    statement.end - 1..statement.end
                } else {
                    body_end..body_end
                },
                parent: None,
                branches: Vec::new(),
            });
            if let Some(text) = text {
                // Commands in node text are TeX commands. Explicit nested pictures opt in again.
                let matrix = statement.matrix && index == 0;
                if matrix {
                    self.output.push(Block {
                        kind: BlockKind::TikzPath,
                        name: "tikz-matrix-body".into(),
                        begin: text.start - 1..text.start,
                        body: text.clone(),
                        end: text.end..text.end + 1,
                        parent: None,
                        branches: Vec::new(),
                    });
                }
                self.collect(text.clone(), matrix, depth + 1);
                body_start = text.end + 1;
            }
            first = false;
        }
        for child in statement.children {
            self.output.push(Block {
                kind: BlockKind::TikzPath,
                name: "tikz-child".into(),
                begin: child.start - 1..child.start,
                body: child.clone(),
                end: child.end..child.end + 1,
                parent: None,
                branches: Vec::new(),
            });
        }
    }
}

pub(crate) fn add_blocks<N: BlockName>(
    source: &str,
    tokens: &[Token],
    blocks: &mut Vec<Block<N>>,
    config: &CompiledConfig,
    protected: &[Range<usize>],
    analysis: &SourceAnalysis<'_>,
) -> Result<(), FormatError> {
    if !source.contains("\\tikz")
        && !tokens.iter().any(|token| {
            token.kind == TokenKind::Command
                && syntax::command_at(source, token.span.start)
                    .is_some_and(|(name, _)| path_command(name))
        })
    {
        return Ok(());
    }
    let declared = analysis.declared_commands()?;
    let declared_envs = analysis.declared_environments()?;
    let mut starts = Vec::new();
    let mut ends = Vec::new();
    for block in blocks.iter().filter(|block| {
        block.kind == BlockKind::Environment
            && crate::tikz_scan::environment(&block.name)
            && !declared_envs.contains(block.name.as_str())
            && !config
                .config
                .syntax
                .environment_arguments
                .contains_key(block.name.as_str())
    }) {
        starts.push(block.body.start);
        ends.push((block.body.end, block.body.start));
    }
    starts.sort_unstable();
    ends.sort_unstable();
    let mut headers: Vec<Range<usize>> = blocks
        .iter()
        .filter_map(|block| {
            if block.kind == BlockKind::Loop {
                Some(block.begin.clone())
            } else if matches!(
                block.kind,
                BlockKind::MandatoryArgument | BlockKind::OptionalArgument
            ) && block
                .parent
                .is_some_and(|parent| blocks[parent].kind == BlockKind::Environment)
            {
                Some(block.begin.start..block.end.end)
            } else {
                None
            }
        })
        .collect();
    headers.sort_unstable_by_key(|range| range.start);
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(headers.len());
    for range in headers {
        if let Some(last) = merged.last_mut().filter(|last| range.start <= last.end) {
            last.end = last.end.max(range.end);
        } else {
            merged.push(range);
        }
    }
    let mut scanner = Scanner {
        source,
        tokens,
        protected,
        declared: &declared,
        config,
        starts,
        ends,
        headers: merged,
        output: Vec::new(),
    };
    scanner.collect(0..source.len(), false, 0);
    // Literal `child { ... }` is already a named group in the general parser.
    // Reuse that indentation; bracketed child headers need the specialized scope.
    let existing_children: HashSet<(usize, usize)> = blocks
        .iter()
        .filter(|block| block.kind == BlockKind::NamedGroup && block.name == "child")
        .map(|block| (block.begin.start, block.end.end))
        .collect();
    blocks.extend(scanner.output.into_iter().filter(|block| {
        block.name != "tikz-child"
            || !existing_children.contains(&(block.begin.start, block.end.end))
    }));
    Ok(())
}

/// Only commit breaks after the entire list is recognized. Error tuples and
/// point metadata belong to their preceding coordinate. Existing empty lines
/// can mark scanlines in a surface plot and must survive.
fn coordinate_breaks(
    source: &str,
    header_end: usize,
    limit: usize,
    protected: &[Range<usize>],
) -> Option<Vec<usize>> {
    let open =
        crate::packages::circuits_timing::coordinate_list_open(&source[..limit], header_end)?;
    let close = group_end(source, open, limit, protected)? - 1;
    if syntax::overlaps(&(open..close + 1), protected) {
        return None;
    }
    let skip = |start| crate::trivia::skip_until(source, start, close);
    let mut cursor = skip(open + 1);
    let mut breaks = Vec::new();
    while cursor < close {
        if source.as_bytes()[cursor] != b'(' {
            return None;
        }
        breaks.push(cursor);
        cursor = skip(group_end(source, cursor, close, protected)?);
        while matches!(
            source.as_bytes().get(cursor..cursor + 2),
            Some(b"+-" | b"+=" | b"-=")
        ) {
            let error = skip(cursor + 2);
            if source.as_bytes().get(error) != Some(&b'(') {
                return None;
            }
            cursor = skip(group_end(source, error, close, protected)?);
        }
        if source.as_bytes()[cursor] == b'[' {
            cursor = skip(group_end(source, cursor, close, protected)?);
        }
    }
    if !breaks.is_empty() {
        breaks.push(close);
    }
    Some(breaks)
}

pub(crate) fn boundaries<N: BlockName>(
    source: &str,
    blocks: &[Block<N>],
    config: &CompiledConfig,
    protected: &[Range<usize>],
) -> Vec<(usize, BreakMode, bool)> {
    let mut result = Vec::new();
    if !config.config.breaks.enabled {
        return result;
    }
    let mut matrices: Vec<Range<usize>> = blocks
        .iter()
        .filter(|block| block.kind == BlockKind::TikzPath && block.name == "tikz-matrix-body")
        .map(|block| block.body.clone())
        .collect();
    matrices.sort_unstable_by_key(|range| range.start);
    let mut matrix_regions: Vec<Range<usize>> = Vec::with_capacity(matrices.len());
    for range in matrices {
        if let Some(last) = matrix_regions
            .last_mut()
            .filter(|last| range.start <= last.end)
        {
            last.end = last.end.max(range.end);
        } else {
            matrix_regions.push(range);
        }
    }
    for block in blocks.iter().filter(|block| {
        block.kind == BlockKind::TikzPath
            && !block.begin.is_empty()
            && !matches!(
                block.name.as_str(),
                "tikz-scope" | "tikz-child" | "tikz-matrix-body"
            )
            && !config.config.breaks.rules.contains_key(block.name.as_str())
            && !config.config.breaks.categories.contains_key("tikz-path")
            && !syntax::is_protected(block.begin.start, &matrix_regions)
    }) {
        let inline = block.name == "tikz-inline";
        if !inline {
            let before = if block.begin.start > 0
                && !source.as_bytes()[block.begin.start - 1].is_ascii_whitespace()
            {
                BreakMode::CommentLine
            } else {
                BreakMode::Line
            };
            let after = if source
                .as_bytes()
                .get(block.end.end)
                .is_some_and(|byte| !byte.is_ascii_whitespace())
            {
                BreakMode::CommentLine
            } else {
                BreakMode::Line
            };
            result.push((block.begin.start, before, true));
            result.push((block.end.end, after, false));
        }
        let name = if inline {
            source[block.begin.clone()]
                .rsplit('\\')
                .next()
                .unwrap_or("")
        } else {
            block.name.as_str()
        };
        if config.config.breaks.pgfplots_coordinates
            && name == "addplot"
            && let Some(points) =
                coordinate_breaks(source, block.begin.end, block.end.start, protected)
        {
            result.extend(points.into_iter().map(|pos| (pos, BreakMode::Line, true)));
        }
        if let Some(columns) = config.config.breaks.tikz_path_columns
            && matches!(
                name,
                "draw"
                    | "path"
                    | "fill"
                    | "filldraw"
                    | "shade"
                    | "shadedraw"
                    | "clip"
                    | "useasboundingbox"
            )
            && source[block.begin.start..block.end.end]
                .lines()
                .any(|line| unicode_width::UnicodeWidthStr::width(line.trim_start()) > columns)
        {
            statement(
                source,
                name,
                block.begin.end,
                block.end.end,
                protected,
                &mut Vec::new(),
                |pos| result.push((pos, BreakMode::Line, true)),
            );
        }
    }
    result
}

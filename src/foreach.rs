//! Literal pgffor headers and TeX-delimited loop bodies.
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Range;

use crate::block_storage::{BlockName, StructuralBlock as Block};
use crate::source_analysis::SourceAnalysis;
use crate::syntax::{self, BlockKind, Token, TokenKind};
use crate::tikz_scan::{comment_end, group_end, node_body, path_command};
use crate::trivia::skip_until as trivia;
use crate::{CompiledConfig, FormatError};

#[derive(Clone)]
struct Loop {
    begin: Range<usize>,
    body: Range<usize>,
    end: Range<usize>,
    full_end: usize,
    group: Option<Range<usize>>,
}

fn token_end(
    source: &str,
    start: usize,
    limit: usize,
    protected: &[Range<usize>],
) -> Option<usize> {
    let end = if source.as_bytes().get(start) == Some(&b'{') {
        group_end(source, start, limit, protected)?
    } else if source.as_bytes().get(start) == Some(&b'\\') {
        let (name, end) = syntax::command_at(source, start)?;
        end - usize::from(name.len() > 1 && name.ends_with('*'))
    } else {
        start + source[start..].chars().next()?.len_utf8()
    };
    (end <= limit).then_some(end)
}

fn header_end(
    source: &str,
    start: usize,
    limit: usize,
    protected: &[Range<usize>],
) -> Option<usize> {
    let (name, mut cursor) = syntax::command_at(source, start)?;
    if name != "foreach" {
        return None;
    }
    let mut variables = 0;
    while cursor < limit {
        cursor = trivia(source, cursor, limit);
        if cursor >= limit {
            return None;
        }
        if source[cursor..].starts_with("in") {
            if variables == 0 {
                return None;
            }
            let list = trivia(source, cursor + 2, limit);
            return token_end(source, list, limit, protected);
        }
        match source.as_bytes()[cursor] {
            b'[' => cursor = group_end(source, cursor, limit, protected)?,
            b'/' => cursor += 1,
            b'\\' => {
                variables += 1;
                cursor = token_end(source, cursor, limit, protected)?;
            }
            _ => return None,
        }
    }
    None
}

fn statement_end(
    source: &str,
    mut cursor: usize,
    limit: usize,
    protected: &[Range<usize>],
) -> Option<usize> {
    while cursor < limit {
        let index = protected.partition_point(|range| range.end <= cursor);
        if let Some(range) = protected.get(index).filter(|range| range.start <= cursor) {
            cursor = range.end;
            continue;
        }
        match source.as_bytes()[cursor] {
            b'%' => cursor = comment_end(source, cursor, limit),
            b'{' => cursor = group_end(source, cursor, limit, protected)?,
            b'\\' => cursor = syntax::command_at(source, cursor)?.1,
            b';' => return Some(cursor + 1),
            b'}' => return None,
            _ => cursor += source[cursor..].chars().next()?.len_utf8(),
        }
    }
    None
}

fn parse(
    source: &str,
    start: usize,
    limit: usize,
    protected: &[Range<usize>],
    memo: &mut HashMap<usize, Option<Loop>>,
    depth: usize,
) -> Option<Loop> {
    if let Some(result) = memo.get(&start) {
        return result.clone();
    }
    if depth > 256 {
        return None;
    }
    let header = header_end(source, start, limit, protected)?;
    let body_start = trivia(source, header, limit);
    let result = if source[body_start..].starts_with("\\foreach")
        && syntax::command_at(source, body_start).is_some_and(|(name, _)| name == "foreach")
    {
        let child = parse(source, body_start, limit, protected, memo, depth + 1)?;
        Loop {
            begin: start..header,
            body: body_start..child.full_end,
            end: child.full_end..child.full_end,
            full_end: child.full_end,
            group: None,
        }
    } else if source.as_bytes().get(body_start) == Some(&b'{') {
        let finish = group_end(source, body_start, limit, protected)?;
        if finish > limit {
            return None;
        }
        Loop {
            begin: start..body_start + 1,
            body: body_start + 1..finish - 1,
            end: finish - 1..finish,
            full_end: finish,
            group: Some(body_start..finish),
        }
    } else {
        let finish = statement_end(source, body_start, limit, protected)?;
        Loop {
            begin: start..header,
            body: body_start..finish,
            end: finish..finish,
            full_end: finish,
            group: None,
        }
    };
    memo.insert(start, Some(result.clone()));
    Some(result)
}

fn enclosing_limit<N: BlockName>(token: &Token, blocks: &[Block<N>], source_end: usize) -> usize {
    blocks
        .iter()
        .filter(|block| {
            block.begin.start < token.span.start
                && block.body.start <= token.span.start
                && token.span.end <= block.body.end
                && !matches!(block.kind, BlockKind::Command | BlockKind::Heading)
        })
        .map(|block| block.body.end)
        .min()
        .unwrap_or(source_end)
}

struct ScopeIndex {
    limits: Vec<(usize, usize)>,
    tikz: Vec<Range<usize>>,
}

impl ScopeIndex {
    fn new<N: BlockName>(blocks: &[Block<N>], source_end: usize) -> Self {
        let mut events = Vec::new();
        let mut tikz = Vec::new();
        for block in blocks {
            if !matches!(block.kind, BlockKind::Command | BlockKind::Heading) {
                let start = block.body.start.max(block.begin.start.saturating_add(1));
                if start < block.body.end {
                    events.push((start, true, block.body.end));
                    events.push((block.body.end, false, block.body.end));
                }
            }
            if block.kind == BlockKind::Environment && crate::tikz_scan::environment(&block.name)
                || block.kind == BlockKind::Command && block.name == "tikz"
            {
                tikz.push(block.body.clone());
            }
        }
        events.sort_unstable();
        let mut active = BTreeMap::<usize, usize>::new();
        let mut limits = Vec::new();
        let mut cursor = 0;
        let mut last = source_end;
        while cursor < events.len() {
            let pos = events[cursor].0;
            while cursor < events.len() && events[cursor].0 == pos {
                let (_, start, end) = events[cursor];
                if start {
                    *active.entry(end).or_default() += 1;
                } else if let Some(count) = active.get_mut(&end) {
                    *count -= 1;
                    if *count == 0 {
                        active.remove(&end);
                    }
                }
                cursor += 1;
            }
            let limit = active.first_key_value().map_or(source_end, |(&end, _)| end);
            if last != limit {
                limits.push((pos, limit));
                last = limit;
            }
        }
        tikz.sort_unstable_by_key(|range| range.start);
        let mut regions: Vec<Range<usize>> = Vec::with_capacity(tikz.len());
        for range in tikz {
            if let Some(last) = regions.last_mut().filter(|last| range.start <= last.end) {
                last.end = last.end.max(range.end);
            } else {
                regions.push(range);
            }
        }
        Self {
            limits,
            tikz: regions,
        }
    }

    fn limit<N: BlockName>(&self, token: &Token, blocks: &[Block<N>], source_end: usize) -> usize {
        let previous = self
            .limits
            .partition_point(|&(start, _)| start <= token.span.start);
        let limit = previous
            .checked_sub(1)
            .map_or(source_end, |index| self.limits[index].1);
        if token.span.end <= limit {
            limit.min(source_end)
        } else {
            // Partial/custom token grammars can straddle a structural boundary.
            enclosing_limit(token, blocks, source_end).min(source_end)
        }
    }

    fn in_tikz(&self, token: &Token) -> bool {
        self.tikz
            .get(
                self.tikz
                    .partition_point(|range| range.end <= token.span.start),
            )
            .is_some_and(|range| range.start <= token.span.start && token.span.end <= range.end)
    }
}

struct PathScanner<'a, N: BlockName> {
    source: &'a str,
    tokens: &'a [Token],
    blocks: &'a [Block<N>],
    config: &'a CompiledConfig,
    declared: &'a HashSet<String>,
    protected: &'a [Range<usize>],
    scopes: &'a ScopeIndex,
    paths: Vec<Range<usize>>,
}

impl<N: BlockName> PathScanner<'_, N> {
    fn collect(&mut self, scope: Range<usize>, tikz: bool, depth: usize) {
        if depth > 256 {
            self.paths.push(scope);
            return;
        }
        let source = self.source;
        let tokens = self.tokens;
        let first = tokens.partition_point(|token| token.span.start < scope.start);
        let last = tokens.partition_point(|token| token.span.start < scope.end);
        let mut skip_until = scope.start;
        for token in &tokens[first..last] {
            if token.kind != TokenKind::Command || token.span.start < skip_until {
                continue;
            }
            let (name, _) = syntax::command_at(source, token.span.start).expect("command token");
            // Variables, options, and list tokens belong to the foreach
            // header even when their control sequences name TikZ paths.
            if name == "foreach" {
                if let Some(end) = header_end(source, token.span.start, scope.end, self.protected) {
                    skip_until = end;
                }
                continue;
            }
            if self.declared.contains(name) || self.config.config.syntax.commands.contains_key(name)
            {
                continue;
            }
            let path = path_command(name) && (tikz || self.scopes.in_tikz(token));
            if name != "tikz" && !path {
                continue;
            }
            let limit = self.scopes.limit(token, self.blocks, scope.end);
            if name == "tikz" {
                let mut cursor = trivia(source, token.span.end, limit);
                if source.as_bytes().get(cursor) == Some(&b'[') {
                    let Some(end) = group_end(source, cursor, limit, self.protected) else {
                        continue;
                    };
                    cursor = trivia(source, end, limit);
                }
                if source.as_bytes().get(cursor) == Some(&b'{') {
                    if let Some(end) = group_end(source, cursor, limit, self.protected) {
                        self.with_body(token.span.start..end, cursor + 1..end - 1, depth);
                        skip_until = end;
                    }
                } else if let Some(end) = statement_end(source, cursor, limit, self.protected) {
                    self.paths.push(token.span.start..end);
                    skip_until = end;
                }
            } else if path
                && let Some(end) = statement_end(source, token.span.end, limit, self.protected)
            {
                let body = matches!(name, "node" | "matrix")
                    .then(|| node_body(source, token.span.end, end, self.protected))
                    .flatten();
                if let Some(body) = body {
                    self.with_body(token.span.start..end, body, depth);
                } else {
                    self.paths.push(token.span.start..end);
                }
                skip_until = end;
            }
        }
    }

    fn with_body(&mut self, path: Range<usize>, body: Range<usize>, depth: usize) {
        self.paths.push(path.start..body.start);
        self.collect(body.clone(), true, depth + 1);
        self.paths.push(body.end..path.end);
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
    if !config.config.syntax.foreach
        || !source.contains("\\foreach")
        || config.config.syntax.commands.contains_key("foreach")
    {
        return Ok(());
    }
    let declared = analysis.declared_commands()?;
    if declared.contains("foreach") {
        return Ok(());
    }
    let scopes = ScopeIndex::new(blocks, source.len());
    let mut scanner = PathScanner {
        source,
        tokens,
        blocks,
        config,
        declared: &declared,
        protected,
        scopes: &scopes,
        paths: Vec::new(),
    };
    scanner.collect(0..source.len(), false, 0);
    let paths = scanner.paths;
    let mut memo = HashMap::new();
    let mut loops = Vec::new();
    for token in tokens.iter().filter(|token| {
        token.kind == TokenKind::Command && &source[token.span.clone()] == "\\foreach"
    }) {
        if syntax::is_protected(token.span.start, &paths) {
            continue;
        }
        let limit = scopes.limit(token, blocks, source.len());
        if let Some(loop_) = parse(source, token.span.start, limit, protected, &mut memo, 0) {
            loops.push(loop_);
        }
    }
    loops.sort_by_key(|loop_| loop_.begin.start);
    let original_count = blocks.len();
    for loop_ in &loops {
        let mut body_group = None;
        if let Some(group) = &loop_.group {
            for (i, block) in blocks.iter_mut().take(original_count).enumerate() {
                if block.begin.start == group.start
                    && block.end.end == group.end
                    && block.body == loop_.body
                    && matches!(block.kind, BlockKind::Group | BlockKind::MandatoryArgument)
                {
                    block.name = "foreach-body".into();
                    body_group = Some(i);
                }
            }
        }
        let parent = blocks
            .iter()
            .enumerate()
            .filter(|(_, block)| {
                block.begin.start < loop_.begin.start && loop_.full_end <= block.body.end
            })
            .max_by_key(|(_, block)| block.begin.start)
            .map(|(i, _)| i);
        blocks.push(Block {
            kind: BlockKind::Loop,
            name: "foreach".into(),
            begin: loop_.begin.clone(),
            body: loop_.body.clone(),
            end: loop_.end.clone(),
            parent,
            branches: Vec::new(),
        });
        if let Some(i) = body_group {
            blocks[i].parent = Some(blocks.len() - 1);
        }
    }
    // Existing body groups and direct commands keep their structure, with the
    // nearest new loop as an additional parent. Header groups stay outside.
    for i in 0..original_count {
        let candidate = loops
            .iter()
            .enumerate()
            .filter(|(_, loop_)| {
                loop_.body.start <= blocks[i].begin.start && blocks[i].end.end <= loop_.body.end
            })
            .max_by_key(|(_, loop_)| loop_.begin.start);
        if let Some((loop_index, loop_)) = candidate {
            let parent_inside = blocks[i].parent.is_some_and(|parent| {
                loop_.body.start <= blocks[parent].begin.start
                    && blocks[parent].end.end <= loop_.body.end
            });
            if !parent_inside {
                blocks[i].parent = Some(original_count + loop_index);
            }
        }
    }
    Ok(())
}

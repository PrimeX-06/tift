//! Lossless token partitions over an immutable source snapshot.

use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::{Deref, Range};

use crate::source_document::SourceDocument;
use crate::syntax;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceTokenKind {
    Control,
    OpenGroup,
    CloseGroup,
    Space,
    Newline,
    Comment,
    Text,
    Opaque,
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceToken {
    pub kind: SourceTokenKind,
    pub span: Range<usize>,
}

#[derive(Default)]
pub(crate) struct PrefixGroupPairs {
    scanned: usize,
    open: Vec<usize>,
    pairs: HashMap<usize, usize>,
}

impl PrefixGroupPairs {
    pub(crate) fn retain_prefix(&mut self, keep: usize) {
        if self.scanned > keep {
            self.scanned = 0;
            self.open.clear();
            self.pairs.clear();
        }
    }

    pub(crate) fn closing_group(&mut self, tokens: &[SourceToken], start: usize) -> Option<usize> {
        if tokens.get(start)?.kind != SourceTokenKind::OpenGroup {
            return None;
        }
        if start < self.scanned
            && let Some(&end) = self.pairs.get(&start)
        {
            return Some(end);
        }
        for (index, token) in tokens.iter().enumerate().skip(self.scanned) {
            match token.kind {
                SourceTokenKind::OpenGroup => self.open.push(index),
                SourceTokenKind::CloseGroup => {
                    if let Some(open) = self.open.pop() {
                        self.pairs.insert(open, index);
                        if open == start {
                            self.scanned = index + 1;
                            return Some(index);
                        }
                    }
                }
                _ => (),
            }
        }
        self.scanned = tokens.len();
        None
    }
}

/// One token in TeX's character-translated input, with its exact raw spelling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappedToken {
    pub kind: SourceTokenKind,
    pub analysis: Range<usize>,
    pub source: Range<usize>,
}

impl SourceToken {
    pub(crate) fn control<'a>(&self, source: &'a str) -> Option<(&'a str, usize)> {
        (self.kind == SourceTokenKind::Control && self.span.len() > 1)
            .then(|| (&source[self.span.start + 1..self.span.end], self.span.end))
    }
}

/// The protection reader drives raw tokenization up to its current source
/// position. Later passes can rewind their cursor over the buffered prefix.
pub(crate) struct RawTokenStream<'a> {
    reader: SourceTokens<'a>,
    tokens: Cow<'a, [SourceToken]>,
}

impl<'a> RawTokenStream<'a> {
    pub(crate) fn new(source: &'a str, previous: Option<LexicalIndex>) -> Self {
        let mut reader = SourceTokens::new(source, &[]);
        let tokens = previous.map_or_else(Vec::new, |index| {
            reader.position = source.len();
            index.into_tokens()
        });
        Self {
            reader,
            tokens: Cow::Owned(tokens),
        }
    }

    pub(crate) fn buffered(source: &'a str, tokens: &'a [SourceToken]) -> Self {
        let mut reader = SourceTokens::new(source, &[]);
        reader.position = source.len();
        Self {
            reader,
            tokens: Cow::Borrowed(tokens),
        }
    }

    pub(crate) fn control_at(&mut self, at: usize, cursor: &mut usize) -> Option<(&'a str, usize)> {
        while self.reader.position <= at {
            let token = self.reader.next()?;
            self.tokens.to_mut().push(token);
        }
        while self
            .tokens
            .get(*cursor)
            .is_some_and(|token| token.span.end <= at)
        {
            *cursor += 1;
        }
        self.tokens
            .get(*cursor)
            .filter(|token| token.span.start == at)
            .and_then(|token| token.control(self.reader.source))
    }

    pub(crate) fn complete(&mut self) -> &[SourceToken] {
        if self.reader.position < self.reader.source.len() {
            self.tokens.to_mut().extend(self.reader.by_ref());
        }
        &self.tokens
    }

    pub(crate) fn finish(mut self) -> LexicalIndex {
        self.complete();
        LexicalIndex {
            tokens: self.tokens.into_owned(),
            protected: Box::default(),
        }
    }
}

/// Reusable token partition consumed by the source model and parsing readers.
#[derive(Debug, Clone)]
pub(crate) struct LexicalIndex {
    tokens: Vec<SourceToken>,
    protected: Box<[Range<usize>]>,
}

impl LexicalIndex {
    pub(crate) fn new(source: &str, protected: &[Range<usize>]) -> Self {
        Self {
            tokens: SourceTokens::new(source, protected).collect(),
            protected: protected.into(),
        }
    }

    /// Apply reader protection to the existing raw partition. A boundary
    /// inside a raw token (for example a verb delimiter inside a raw comment)
    /// restarts the ordinary lexer until its start agrees with the partition.
    pub(crate) fn with_protection(self, source: &str, ranges: &[Range<usize>]) -> Self {
        if ranges.is_empty() {
            return self;
        }
        let mut tokens = Vec::with_capacity(self.tokens.len());
        project_tail(source, ranges, &self.tokens, &mut tokens);
        Self {
            tokens,
            protected: ranges.into(),
        }
    }

    /// Copy complete tokens on each side of the edit. Restart from the token
    /// touching the first changed byte, since a command, comment, space run or
    /// CRLF can consume text appended at its end.
    pub(crate) fn relex(&self, old: &str, new: &str, protected: &[Range<usize>]) -> Self {
        let mut prefix = old
            .bytes()
            .zip(new.bytes())
            .position(|(a, b)| a != b)
            .unwrap_or(old.len().min(new.len()));
        let same_ranges = self
            .protected
            .iter()
            .zip(protected)
            .take_while(|(a, b)| a == b)
            .count();
        for next in [self.protected.get(same_ranges), protected.get(same_ranges)]
            .into_iter()
            .flatten()
        {
            prefix = prefix.min(next.start);
        }
        // Two Unicode scalars can start with the same byte while only one
        // continues a control word. Restart before that whole scalar.
        prefix = old.floor_char_boundary(prefix);
        let mut suffix = old.as_bytes()[prefix..]
            .iter()
            .rev()
            .zip(new.as_bytes()[prefix..].iter().rev())
            .take_while(|(a, b)| a == b)
            .count();
        // A shifted protection interval agrees only when both boundaries
        // have the same distance from the end of their respective source.
        let old_ranges = &self.protected;
        let aligned_ranges = old_ranges
            .iter()
            .rev()
            .zip(protected.iter().rev())
            .take_while(|(a, b)| {
                old.len() - a.end == new.len() - b.end && old.len() - a.start == new.len() - b.start
            })
            .count();
        if let Some(range) = old_ranges.iter().rev().nth(aligned_ranges) {
            suffix = suffix.min(old.len() - range.end);
        }
        if let Some(range) = protected.iter().rev().nth(aligned_ranges) {
            suffix = suffix.min(new.len() - range.end);
        }
        if prefix == 0 && suffix == 0 {
            return Self::new(new, protected);
        }

        let prefix_tokens = self.tokens.partition_point(|token| token.span.end < prefix);
        let restart = self.tokens.get(prefix_tokens).map_or(0, |t| t.span.start);
        let mut tokens = Vec::with_capacity(self.tokens.len().min(new.len()));
        tokens.extend_from_slice(&self.tokens[..prefix_tokens]);
        let mut reader = SourceTokens {
            source: new,
            ranges: protected,
            position: restart,
            protected_index: protected.partition_point(|range| range.end <= restart),
        };
        let old_tail = old.len() - suffix;
        let new_tail = new.len() - suffix;
        let mut next_old = self
            .tokens
            .partition_point(|token| token.span.start < old_tail);
        while reader.position < new.len() {
            if reader.position >= new_tail {
                let aligned = old_tail + (reader.position - new_tail);
                while self
                    .tokens
                    .get(next_old)
                    .is_some_and(|t| t.span.start < aligned)
                {
                    next_old += 1;
                }
                if self
                    .tokens
                    .get(next_old)
                    .is_some_and(|t| t.span.start == aligned)
                {
                    if old.len() == new.len() {
                        tokens.extend_from_slice(&self.tokens[next_old..]);
                    } else {
                        tokens.extend(self.tokens[next_old..].iter().map(|t| SourceToken {
                            kind: t.kind,
                            span: new_tail + (t.span.start - old_tail)
                                ..new_tail + (t.span.end - old_tail),
                        }));
                    }
                    break;
                }
            }
            tokens.push(reader.next().expect("remaining source"));
        }
        Self {
            tokens,
            protected: protected.into(),
        }
    }

    pub(crate) fn as_slice(&self) -> &[SourceToken] {
        &self.tokens
    }

    pub(crate) fn protected_ranges(&self) -> &[Range<usize>] {
        &self.protected
    }

    pub(crate) fn into_tokens(self) -> Vec<SourceToken> {
        self.tokens
    }
}

fn project_tail(
    source: &str,
    ranges: &[Range<usize>],
    raw: &[SourceToken],
    tokens: &mut Vec<SourceToken>,
) {
    let position = tokens.last().map_or(0, |token| token.span.end);
    let mut reader = SourceTokens {
        source,
        ranges,
        position,
        protected_index: ranges.partition_point(|range| range.end <= position),
    };
    let mut raw_index = raw.partition_point(|token| token.span.end <= position);
    while reader.position < source.len() {
        let pos = reader.position;
        raw_index += raw[raw_index..].partition_point(|token| token.span.end <= pos);
        while ranges
            .get(reader.protected_index)
            .is_some_and(|range| range.end <= pos)
        {
            reader.protected_index += 1;
        }
        let limit = ranges
            .get(reader.protected_index)
            .map_or(source.len(), |range| range.start);
        let count = if raw
            .get(raw_index)
            .is_some_and(|token| token.span.start == pos)
        {
            raw[raw_index..].partition_point(|token| token.span.end <= limit)
        } else {
            0
        };
        if count > 0 {
            tokens.extend_from_slice(&raw[raw_index..raw_index + count]);
            raw_index += count;
            reader.position = raw[raw_index - 1].span.end;
        } else {
            tokens.push(reader.next().expect("remaining source"));
        }
    }
}

#[derive(Default)]
pub(crate) struct AppendProtectedTokens {
    tokens: Vec<SourceToken>,
    ranges: Vec<Range<usize>>,
}

impl AppendProtectedTokens {
    pub(crate) fn update(
        &mut self,
        source: &str,
        ranges: &[Range<usize>],
    ) -> (&[SourceToken], usize, usize) {
        let end = self.tokens.last().map_or(0, |token| token.span.end);
        assert!(
            source.len() >= end,
            "project alias source must retain its prefix"
        );
        let common = self
            .ranges
            .iter()
            .zip(ranges)
            .take_while(|(old, new)| old == new)
            .count();
        let old = self.ranges.get(common);
        let new = ranges.get(common);
        if source.len() == end && old.is_none() && new.is_none() {
            return (&self.tokens, self.tokens.len(), 0);
        }
        let change = old
            .into_iter()
            .chain(new)
            .fold(end, |at, range| at.min(range.start));
        let mut keep = self
            .tokens
            .partition_point(|token| token.span.end <= change);
        if let Some(token) = self.tokens[..keep]
            .last()
            .filter(|token| token.span.end == change)
            && let Some(&next) = source.as_bytes().get(change)
            && match token.kind {
                SourceTokenKind::Control => token.span.len() == 1 || !text_delimiter(next),
                SourceTokenKind::Comment => next != b'\n',
                SourceTokenKind::Space => matches!(next, b' ' | b'\t'),
                SourceTokenKind::Text => {
                    source.as_bytes()[token.span.start] != b'=' && !text_delimiter(next)
                }
                SourceTokenKind::Newline => next == b'\n' && source.as_bytes()[change - 1] == b'\r',
                _ => false,
            }
        {
            keep -= 1;
        }
        self.tokens.truncate(keep);
        let position = self.tokens.last().map_or(0, |token| token.span.end);
        self.tokens.extend(SourceTokens {
            source,
            ranges,
            position,
            protected_index: ranges.partition_point(|range| range.end <= position),
        });
        self.ranges.truncate(common);
        self.ranges.extend_from_slice(&ranges[common..]);
        (&self.tokens, keep, source.len() - position)
    }
}

#[derive(Default)]
pub(crate) struct AppendLexicalIndex {
    raw: Vec<SourceToken>,
    protected: Vec<Range<usize>>,
    projected: Vec<SourceToken>,
}

impl AppendLexicalIndex {
    pub(crate) fn raw(&self) -> &[SourceToken] {
        &self.raw
    }

    pub(crate) fn append_raw(&mut self, source: &str) -> (usize, usize) {
        let end = self.raw.last().map_or(0, |token| token.span.end);
        if source.len() == end {
            return (end, 0);
        }
        assert!(
            source.len() > end,
            "project export source must retain its prefix"
        );
        let mut reader = SourceTokens::new(source, &[]);
        reader.position = end;
        let mut restart = end;
        let mut scanned_from = end;
        if let Some(last) = self.raw.last_mut() {
            let next = source.as_bytes()[end];
            match last.kind {
                SourceTokenKind::Comment if next != b'\n' => {
                    restart = last.span.start;
                    reader.position = source[end..].find('\n').map_or(source.len(), |at| end + at);
                    last.span.end = reader.position;
                }
                SourceTokenKind::Space if matches!(next, b' ' | b'\t') => {
                    restart = last.span.start;
                    last.span.end = reader.next().unwrap().span.end;
                }
                SourceTokenKind::Text
                    if source.as_bytes()[last.span.start] != b'=' && !text_delimiter(next) =>
                {
                    restart = last.span.start;
                    last.span.end = reader.next().unwrap().span.end;
                }
                SourceTokenKind::Control => {
                    restart = last.span.start;
                    scanned_from = restart;
                    reader.position = restart;
                    self.raw.pop();
                }
                SourceTokenKind::Newline
                    if next == b'\n' && source.as_bytes()[end - 1] == b'\r' =>
                {
                    restart = last.span.start;
                    reader.position += 1;
                    last.span.end = reader.position;
                }
                _ => (),
            }
        }
        self.raw.extend(reader);
        (restart, source.len() - scanned_from)
    }

    pub(crate) fn project(
        &mut self,
        source: &str,
        ranges: &[Range<usize>],
        restart: usize,
    ) -> (&[SourceToken], usize) {
        if ranges.is_empty() {
            let restart = if self.protected.is_empty() {
                restart
            } else {
                0
            };
            self.protected.clear();
            self.projected.clear();
            return (
                &self.raw,
                self.raw.partition_point(|token| token.span.end <= restart),
            );
        }
        let common = self
            .protected
            .iter()
            .zip(ranges)
            .take_while(|(old, new)| old == new)
            .count();
        if restart == source.len() && common == ranges.len() && common == self.protected.len() {
            return (&self.projected, self.projected.len());
        }
        let change = self
            .protected
            .get(common)
            .into_iter()
            .chain(ranges.get(common))
            .fold(restart, |at, range| at.min(range.start));
        let keep = self
            .projected
            .partition_point(|token| token.span.end < change);
        self.projected.truncate(keep);
        self.protected.truncate(common);
        self.protected.extend_from_slice(&ranges[common..]);
        project_tail(source, ranges, &self.raw, &mut self.projected);
        (&self.projected, keep)
    }
}

impl Deref for LexicalIndex {
    type Target = [SourceToken];

    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

/// Streaming partition shared by stored indexes and filtered readers.
pub(crate) struct SourceTokens<'a> {
    source: &'a str,
    ranges: &'a [Range<usize>],
    pub(crate) position: usize,
    protected_index: usize,
}

fn text_delimiter(byte: u8) -> bool {
    matches!(
        byte,
        b'\\' | b'%' | b'{' | b'}' | b'\r' | b'\n' | b' ' | b'\t' | b'='
    )
}

impl<'a> SourceTokens<'a> {
    pub(crate) fn mapped<'s>(
        document: &'s SourceDocument,
        protection: &'s [Range<usize>],
    ) -> impl Iterator<Item = MappedToken> + 's {
        SourceTokens::new(document.analysis_text(), protection).map(|token| {
            let source = document
                .to_source_range(token.span.clone())
                .expect("lexical byte boundary");
            MappedToken {
                kind: token.kind,
                analysis: token.span,
                source,
            }
        })
    }
    pub(crate) fn new(source: &'a str, ranges: &'a [Range<usize>]) -> Self {
        Self {
            source,
            ranges,
            position: 0,
            protected_index: 0,
        }
    }
}

impl Iterator for SourceTokens<'_> {
    type Item = SourceToken;

    fn next(&mut self) -> Option<Self::Item> {
        let source = self.source;
        let start = self.position;
        if start == source.len() {
            return None;
        }
        while self
            .ranges
            .get(self.protected_index)
            .is_some_and(|range| range.end <= start)
        {
            self.protected_index += 1;
        }
        let protected = self.ranges.get(self.protected_index);
        let limit = protected.map_or(source.len(), |range| range.start.max(start));
        let mut pos = start;
        let kind = if let Some(range) = protected.filter(|range| range.start <= start) {
            pos = range.end.min(source.len());
            SourceTokenKind::Opaque
        } else {
            match source.as_bytes()[pos] {
                b'\\' => {
                    pos = syntax::command_at(source, pos).map_or(pos + 1, |(_, end)| end);
                    SourceTokenKind::Control
                }
                b'%' => {
                    pos = source[pos..].find('\n').map_or(source.len(), |n| pos + n);
                    SourceTokenKind::Comment
                }
                b'{' => {
                    pos += 1;
                    SourceTokenKind::OpenGroup
                }
                b'}' => {
                    pos += 1;
                    SourceTokenKind::CloseGroup
                }
                b'\r' | b'\n' => {
                    pos += if source[pos..].starts_with("\r\n") {
                        2
                    } else {
                        1
                    };
                    SourceTokenKind::Newline
                }
                b' ' | b'\t' => {
                    pos += 1;
                    while pos < limit && matches!(source.as_bytes()[pos], b' ' | b'\t') {
                        pos += 1;
                    }
                    SourceTokenKind::Space
                }
                b'=' => {
                    pos += 1;
                    SourceTokenKind::Text
                }
                _ => {
                    pos += 1;
                    while pos < limit && !text_delimiter(source.as_bytes()[pos]) {
                        pos += 1;
                    }
                    SourceTokenKind::Text
                }
            }
        };
        self.position = pos;
        Some(SourceToken {
            kind,
            span: start..pos,
        })
    }
}

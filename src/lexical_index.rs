//! Lossless token partitions over an immutable source snapshot.

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
    tokens: Vec<SourceToken>,
}

impl<'a> RawTokenStream<'a> {
    pub(crate) fn new(source: &'a str, previous: Option<LexicalIndex>) -> Self {
        let mut reader = SourceTokens::new(source, &[]);
        let tokens = previous.map_or_else(Vec::new, |index| {
            reader.position = source.len();
            index.into_tokens()
        });
        Self { reader, tokens }
    }

    pub(crate) fn control_at(&mut self, at: usize, cursor: &mut usize) -> Option<(&'a str, usize)> {
        while self.reader.position <= at {
            self.tokens.push(self.reader.next()?);
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
        self.tokens.extend(self.reader.by_ref());
        &self.tokens
    }

    pub(crate) fn finish(mut self) -> LexicalIndex {
        self.complete();
        LexicalIndex {
            tokens: self.tokens,
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
        let mut raw = self.tokens.iter().peekable();
        let mut reader = SourceTokens::new(source, ranges);
        while reader.position < source.len() {
            let pos = reader.position;
            while raw.peek().is_some_and(|t| t.span.end <= pos) {
                raw.next();
            }
            while ranges
                .get(reader.protected_index)
                .is_some_and(|r| r.end <= pos)
            {
                reader.protected_index += 1;
            }
            let limit = ranges
                .get(reader.protected_index)
                .map_or(source.len(), |r| r.start);
            if let Some(token) = raw
                .peek()
                .filter(|t| t.span.start == pos && t.span.end <= limit)
            {
                tokens.push((*token).clone());
                reader.position = token.span.end;
            } else {
                tokens.push(reader.next().expect("remaining source"));
            }
        }
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
                    while pos < limit
                        && !matches!(
                            source.as_bytes()[pos],
                            b'\\' | b'%' | b'{' | b'}' | b'\r' | b'\n' | b' ' | b'\t' | b'='
                        )
                    {
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

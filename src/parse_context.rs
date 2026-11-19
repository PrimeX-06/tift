//! Reuse structural parsing while a pipeline stage keeps the same text.
use std::ops::Range;
use std::rc::Rc;

use crate::source_analysis::{AnalysisMemo, SourceAnalysis};
use crate::syntax::{self, SharedBlock as Block, Token};
use crate::{CompiledConfig, FormatError};

pub(crate) struct ParseContext<'a> {
    config: &'a CompiledConfig,
    parsed: Option<ParsedText>,
}

pub(crate) struct ParsedText {
    fingerprint: blake3::Hash,
    pub(crate) protected: Vec<Range<usize>>,
    pub(crate) tokens: Option<Vec<Token>>,
    pub(crate) blocks: Option<Vec<Block>>,
    pub(crate) metadata: Rc<AnalysisMemo>,
    expl3_protected: Option<Vec<Range<usize>>>,
}

impl<'a> ParseContext<'a> {
    pub(crate) fn with_model(
        config: &'a CompiledConfig,
        model: Option<crate::source_model::SourceModel>,
    ) -> Self {
        let parsed = model.map(|mut model| {
            let fingerprint = blake3::hash(model.source_bytes());
            let protected = model.protected_ranges().to_vec();
            let (tokens, blocks) = model
                .structure
                .take()
                .map_or((None, None), |(tokens, blocks)| {
                    (Some(tokens), Some(blocks))
                });
            ParsedText {
                fingerprint,
                protected,
                tokens,
                blocks,
                metadata: Rc::new(AnalysisMemo::with_model(model)),
                expl3_protected: None,
            }
        });
        Self { config, parsed }
    }
    pub(crate) fn new(config: &'a CompiledConfig) -> Self {
        Self {
            config,
            parsed: None,
        }
    }

    /// Preserve structural ownership across line indentation edits; semantic
    /// metadata and lexical tokens are rebuilt for the updated source.
    pub(crate) fn reuse_indented_snapshot(&mut self, before: &str, after: &str) -> bool {
        if before == after {
            return true;
        }
        let grammar = &self.config.config.syntax;
        if grammar.command_pattern.is_some()
            || grammar.environment_pattern.is_some()
            || grammar.condition_pattern.is_some()
            || grammar.item_pattern.is_some()
            || grammar.argument_between.is_some()
            || !grammar.special.is_empty()
        {
            return false;
        }
        let Some(parsed) = self.parsed.as_ref().filter(|parsed| {
            parsed.blocks.is_some() && parsed.fingerprint == blake3::hash(before.as_bytes())
        }) else {
            return false;
        };
        let Some(edits) = IndentationMap::between(before, after) else {
            return false;
        };
        let Some(tokens) = &parsed.tokens else {
            return false;
        };
        for (range, _) in &edits.edits {
            // At the end of a protected range, an insertion could belong to
            // the following line. The position map uses right-biased ends.
            let index = parsed.protected.partition_point(|r| {
                r.end < range.start || (!range.is_empty() && r.end == range.start)
            });
            if parsed
                .protected
                .get(index)
                .is_some_and(|r| r.start < range.end)
            {
                return false;
            }
            let first = tokens.partition_point(|token| token.span.end <= range.start);
            if tokens[first..]
                .iter()
                .take_while(|token| token.span.start < range.end)
                .any(|token| {
                    !matches!(
                        token.kind,
                        syntax::TokenKind::Space | syntax::TokenKind::Text
                    )
                })
                || range.is_empty()
                    && tokens.get(first).is_some_and(|token| {
                        token.span.start < range.start
                            && !matches!(
                                token.kind,
                                syntax::TokenKind::Space | syntax::TokenKind::Text
                            )
                    })
                || range.is_empty()
                    && first.checked_sub(1).is_some_and(|previous| {
                        let token = &tokens[previous];
                        token.span.end == range.start
                            && token.kind == syntax::TokenKind::Command
                            && before[token.span.clone()].ends_with('\n')
                    })
            {
                return false;
            }
        }
        let mut parsed = self.parsed.take().expect("checked indentation snapshot");
        let mut remap = || -> Option<()> {
            for range in &mut parsed.protected {
                edits.range(range)?;
            }
            for block in parsed.blocks.as_mut()? {
                edits.range(&mut block.begin)?;
                edits.range(&mut block.body)?;
                edits.range(&mut block.end)?;
                for (_, branch) in &mut block.branches {
                    edits.range(branch)?;
                }
            }
            Some(())
        };
        if remap().is_none() {
            return false;
        }
        parsed.fingerprint = blake3::hash(after.as_bytes());
        parsed.tokens = None;
        parsed.metadata = Rc::default();
        parsed.expl3_protected = None;
        self.parsed = Some(parsed);
        true
    }

    pub(crate) fn snapshot(
        &mut self,
        source: &str,
        tokens: bool,
        blocks: bool,
    ) -> Result<&ParsedText, FormatError> {
        let fingerprint = blake3::hash(source.as_bytes());
        if self
            .parsed
            .as_ref()
            .is_none_or(|parsed| parsed.fingerprint != fingerprint)
        {
            let ranges = syntax::protected_ranges_with_syntax(
                source,
                &self.config.config,
                &self.config.syntax,
            )?;
            let model =
                crate::source_model::SourceModel::with_ranges(source, &self.config.config, &ranges);
            self.parsed = Some(ParsedText {
                fingerprint,
                protected: model.protected_ranges().to_vec(),
                tokens: None,
                blocks: None,
                metadata: Rc::new(AnalysisMemo::with_model(model)),
                expl3_protected: None,
            });
        }
        let parsed = self.parsed.as_mut().expect("current source snapshot");
        if (tokens || blocks) && parsed.tokens.is_none() {
            let analysis = SourceAnalysis::with_memo(
                source,
                &self.config.config,
                &parsed.protected,
                Rc::clone(&parsed.metadata),
            );
            parsed.tokens = Some(syntax::lex_with_analysis(
                source,
                &self.config.config,
                &self.config.syntax,
                &parsed.protected,
                &analysis,
            )?);
        }
        if blocks && parsed.blocks.is_none() {
            let analysis = SourceAnalysis::with_memo(
                source,
                &self.config.config,
                &parsed.protected,
                Rc::clone(&parsed.metadata),
            );
            parsed.blocks = Some(syntax::blocks_from_tokens(
                source,
                self.config,
                &parsed.protected,
                &analysis,
                parsed.tokens.as_deref().expect("tokens before structure"),
            )?);
        }
        Ok(parsed)
    }

    pub(crate) fn expl3_protected_ranges(
        &mut self,
        source: &str,
    ) -> Result<Vec<Range<usize>>, FormatError> {
        let parsed = self.snapshot(source, false, false)?;
        if let Some(ranges) = &parsed.expl3_protected {
            return Ok(ranges.clone());
        }
        let ranges = crate::code_layout::protected_ranges(source, self.config, true)?;
        self.parsed
            .as_mut()
            .expect("current source snapshot")
            .expl3_protected = Some(ranges.clone());
        Ok(ranges)
    }
}

struct IndentationMap {
    edits: Vec<(Range<usize>, usize)>,
}

impl IndentationMap {
    fn between(before: &str, after: &str) -> Option<Self> {
        let mut old_lines = before.split_inclusive('\n');
        let mut new_lines = after.split_inclusive('\n');
        let (mut old_offset, mut new_offset) = (0, 0);
        let mut edits = Vec::new();
        loop {
            match (old_lines.next(), new_lines.next()) {
                (None, None) => return Some(Self { edits }),
                (Some(old), Some(new)) => {
                    if old != new {
                        let old_body = old.trim_end_matches(['\r', '\n']);
                        let new_body = new.trim_end_matches(['\r', '\n']);
                        if old[old_body.len()..] != new[new_body.len()..] {
                            return None;
                        }
                        let old_start =
                            old_body.len() - old_body.trim_start_matches([' ', '\t']).len();
                        let new_start =
                            new_body.len() - new_body.trim_start_matches([' ', '\t']).len();
                        let old_end = old_body.trim_end_matches([' ', '\t']).len().max(old_start);
                        let new_end = new_body.trim_end_matches([' ', '\t']).len().max(new_start);
                        if old_body[old_start..old_end] != new_body[new_start..new_end] {
                            return None;
                        }
                        if old_body[..old_start] != new_body[..new_start] {
                            edits
                                .push((old_offset..old_offset + old_start, new_offset + new_start));
                        }
                        if old_body[old_end..] != new_body[new_end..] {
                            edits.push((
                                old_offset + old_end..old_offset + old_body.len(),
                                new_offset + new_body.len(),
                            ));
                        }
                    }
                    old_offset += old.len();
                    new_offset += new.len();
                }
                _ => return None,
            }
        }
    }

    fn position(&self, at: usize) -> Option<usize> {
        let index = self.edits.partition_point(|(range, _)| range.end <= at);
        if self
            .edits
            .get(index)
            .is_some_and(|(range, _)| range.start < at)
        {
            return None;
        }
        match index.checked_sub(1).map(|index| &self.edits[index]) {
            Some((range, new_end)) => new_end.checked_add(at - range.end),
            None => Some(at),
        }
    }

    fn range(&self, range: &mut Range<usize>) -> Option<()> {
        *range = self.position(range.start)?..self.position(range.end)?;
        Some(())
    }
}

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

    pub(crate) fn current_snapshot(&self, source: &str) -> Option<&ParsedText> {
        let parsed = self.parsed.as_ref()?;
        (parsed.fingerprint == blake3::hash(source.as_bytes())).then_some(parsed)
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

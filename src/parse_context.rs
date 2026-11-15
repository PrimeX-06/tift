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
}

impl<'a> ParseContext<'a> {
    pub(crate) fn new(config: &'a CompiledConfig) -> Self {
        Self {
            config,
            parsed: None,
        }
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
            self.parsed = Some(ParsedText {
                fingerprint,
                protected: syntax::protected_ranges(source, &self.config.config)?,
                tokens: None,
                blocks: None,
                metadata: Rc::default(),
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
}



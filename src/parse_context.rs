//! Reuse structural parsing while a pipeline stage keeps the same text.
mod comment;
mod prose;

use prose::TextCheckpoint;
use std::cell::OnceCell;
use std::ops::Range;
use std::rc::Rc;

use crate::source_analysis::{AnalysisMemo, SourceAnalysis};
use crate::source_document::{SourceChange, SourceDocument};
use crate::syntax::{self, SharedBlock as Block, Token};
use crate::{CompiledConfig, FormatError};

fn position_independent_grammar(config: &CompiledConfig) -> bool {
    let grammar = &config.config.syntax;
    !(!grammar.special.is_empty()
        || grammar.command_pattern.is_some()
        || grammar.environment_pattern.is_some()
        || grammar.item_pattern.is_some()
        || grammar.condition_pattern.is_some()
        || grammar.argument_between.is_some()
        || grammar
            .package_load_events
            .iter()
            .any(|e| e.at != 0 && e.loader.is_none())
        || grammar
            .command_import_events
            .iter()
            .any(|e| e.at != 0 && e.loader.is_none())
        || grammar
            .environment_alias_events
            .iter()
            .any(|e| e.at != 0 && e.loader.is_none())
        || grammar
            .environment_argument_events
            .iter()
            .any(|e| e.at != 0 && e.loader.is_none()))
}

pub(crate) struct ParseContext<'a> {
    config: &'a CompiledConfig,
    parsed: Option<ParsedText>,
}

pub(crate) struct ParsedText {
    pub(crate) document: SourceDocument,
    pub(crate) protected: Vec<Range<usize>>,
    pub(crate) tokens: Option<Vec<Token>>,
    pub(crate) blocks: Option<Vec<Block>>,
    pub(crate) metadata: Rc<AnalysisMemo>,
    environment_ranges: OnceCell<Vec<syntax::Environment>>,
    text_readers: OnceCell<Vec<TextCheckpoint>>,
    expl3_protected: Option<Vec<Range<usize>>>,
}

impl ParsedText {
    pub(crate) fn analysis<'a>(&'a self, config: &'a crate::Config) -> SourceAnalysis<'a> {
        SourceAnalysis::with_memo(
            self.document.as_str(),
            config,
            &self.protected,
            Rc::clone(&self.metadata),
        )
    }

    pub(crate) fn environment_ranges(
        &self,
        config: &crate::Config,
    ) -> Result<&[syntax::Environment], FormatError> {
        if let Some(ranges) = self.environment_ranges.get() {
            return Ok(ranges);
        }
        let ranges = syntax::environment_ranges_with_tokens(
            self.document.as_str(),
            self.tokens.as_deref().expect("environment tokens"),
            &self.protected,
            config,
        )?;
        Ok(self.environment_ranges.get_or_init(|| ranges))
    }
}

impl<'a> ParseContext<'a> {
    pub(crate) fn with_model(
        config: &'a CompiledConfig,
        model: Option<crate::source_model::SourceModel>,
    ) -> Self {
        let parsed = model.map(|mut model| {
            let document = model.document().clone();
            let protected = model.protected_ranges().to_vec();
            let (tokens, blocks) = model
                .structure
                .take()
                .map_or((None, None), |(tokens, blocks)| {
                    (Some(tokens), Some(blocks))
                });
            ParsedText {
                document,
                protected,
                tokens,
                blocks,
                metadata: Rc::new(AnalysisMemo::with_model(model)),
                environment_ranges: OnceCell::new(),
                text_readers: OnceCell::new(),
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

    /// Project structural spans through line-margin edits from this snapshot's edit plan.
    /// Edits that touch TeX tokens, custom grammar, or newlines need a fresh parse.
    pub(crate) fn reuse_whitespace_snapshot(
        &mut self,
        before: &str,
        after: &str,
        mappings: &[(Range<usize>, Range<usize>)],
    ) -> bool {
        if mappings.is_empty() {
            return before == after;
        }
        if !position_independent_grammar(self.config) {
            return false;
        }
        let Some(parsed) = self
            .parsed
            .as_ref()
            .filter(|parsed| parsed.document.matches(before))
        else {
            return false;
        };
        let Some(tokens) = parsed.tokens.as_deref() else {
            return false;
        };
        let Some(model) = parsed.metadata.existing_model() else {
            return false;
        };
        let mut before_end = 0;
        let mut after_end = 0;
        for (range, output) in mappings {
            let horizontal = |s: &str| s.bytes().all(|b| matches!(b, b' ' | b'\t'));
            if !before.get(range.clone()).is_some_and(horizontal)
                || !after.get(output.clone()).is_some_and(horizontal)
                || range.start < before_end
                || output.start < after_end
                || before[before_end..range.start] != after[after_end..output.start]
            {
                return false;
            }
            before_end = range.end;
            after_end = output.end;
            let leading = before[..range.start]
                .trim_end_matches([' ', '\t'])
                .as_bytes()
                .last()
                .is_none_or(|&b| b == b'\n');
            let tail = before[range.end..].trim_start_matches([' ', '\t']);
            if !(leading || tail.is_empty() || tail.starts_with('\n') || tail.starts_with("\r\n")) {
                return false;
            }
            // Starts and ends at a line's final command move in different
            // directions when new space is inserted just after the command.
            if range.is_empty() && !leading {
                return false;
            }
            // Whole-line protection can absorb a new indent at either boundary.
            let first = parsed.protected.partition_point(|r| {
                r.end < range.start || !range.is_empty() && r.end == range.start
            });
            if parsed.protected.get(first).is_some_and(|r| {
                r.start < range.end
                    || range.is_empty()
                        && r.start == range.start
                        && (r.end == before.len() || before[r.clone()].contains('\n'))
            }) {
                return false;
            }
            let first = tokens.partition_point(|token| token.span.end <= range.start);
            let allowed = |token: &Token| {
                matches!(
                    token.kind,
                    syntax::TokenKind::Space | syntax::TokenKind::Text
                )
            };
            if tokens[first..]
                .iter()
                .take_while(|token| token.span.start < range.end)
                .any(|token| !allowed(token))
                || range.is_empty()
                    && tokens
                        .get(first)
                        .is_some_and(|t| t.span.start < range.start && !allowed(t))
                || range.is_empty() && range.start > 0 && syntax::is_escaped(before, range.start)
                || range.is_empty()
                    && first.checked_sub(1).is_some_and(|i| {
                        let previous = &tokens[i];
                        previous.span.end == range.start
                            && previous.kind == syntax::TokenKind::Command
                            && before[previous.span.clone()].ends_with('\n')
                    })
            {
                return false;
            }
        }
        if before[before_end..] != after[after_end..] {
            return false;
        }
        // These feed lexical analysis. The public protection includes ranges
        // discovered by semantic analysis, such as ignored command arguments.
        let mut lexical_protection = model.lexical_protection().to_vec();
        let mut windows = model.reader_windows().to_vec();
        // Take ownership first. Any ambiguous boundary discards the partially projected
        // snapshot, so neither the old nor the new source can observe shifted spans.
        let mut parsed = self.parsed.take().expect("validated whitespace snapshot");
        let position = |at| {
            let i = mappings.partition_point(|(range, _)| range.end <= at);
            if mappings.get(i).is_some_and(|(range, _)| range.start < at) {
                return None;
            }
            match i.checked_sub(1).map(|i| &mappings[i]) {
                Some((range, output)) => output.end.checked_add(at - range.end),
                None => Some(at),
            }
        };
        let spans = parsed
            .protected
            .iter_mut()
            .chain(&mut lexical_protection)
            .chain(windows.iter_mut().map(|w| &mut w.span))
            .chain(parsed.blocks.iter_mut().flatten().flat_map(|block| {
                [&mut block.begin, &mut block.body, &mut block.end]
                    .into_iter()
                    .chain(block.branches.iter_mut().map(|(_, span)| span))
            }));
        for span in spans {
            let (Some(start), Some(end)) = (position(span.start), position(span.end)) else {
                return false;
            };
            *span = start..end;
        }
        parsed.document = SourceDocument::new(after);
        parsed.tokens = None;
        parsed.metadata = Rc::new(AnalysisMemo::with_document(
            parsed.document.clone(),
            lexical_protection,
            windows,
        ));
        parsed.environment_ranges.take();
        parsed.text_readers.take();
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
        if self
            .parsed
            .as_ref()
            .is_none_or(|parsed| !parsed.document.matches(source))
        {
            let document = SourceDocument::new(source);
            if !self.project_plain_text(&document) {
                let model = crate::source_model::SourceModel::rescan_document(
                    document.clone(),
                    self.config,
                    self.parsed
                        .as_ref()
                        .and_then(|parsed| parsed.metadata.existing_model()),
                )?;
                self.parsed = Some(ParsedText {
                    document,
                    protected: model.protected_ranges().to_vec(),
                    tokens: None,
                    blocks: None,
                    metadata: Rc::new(AnalysisMemo::with_model(model)),
                    environment_ranges: OnceCell::new(),
                    text_readers: OnceCell::new(),
                    expl3_protected: None,
                });
            }
        }
        let parsed = self.parsed.as_mut().expect("current source snapshot");
        if (tokens || blocks) && parsed.tokens.is_none() {
            let analysis = parsed.analysis(&self.config.config);
            parsed.tokens = Some(syntax::lex_with_analysis(
                source,
                &self.config.config,
                &self.config.syntax,
                &parsed.protected,
                &analysis,
            )?);
        }
        if blocks && parsed.blocks.is_none() {
            let analysis = parsed.analysis(&self.config.config);
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

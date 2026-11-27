//! Reader checkpoints and local reconstruction for ordinary TeX prose.
use super::*;
use crate::source_model::{paragraph_environment, printed_command};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct TextReaderState {
    groups: usize,
    environments: usize,
}

#[derive(Debug, Clone)]
pub(super) struct TextCheckpoint {
    span: Range<usize>,
    state: TextReaderState,
    reader: usize,
}

fn prose_token(text: &str) -> bool {
    text.chars().all(|ch| {
        ch.is_alphanumeric()
            || matches!(
                ch,
                ' ' | '\t' | '\n' | '\r' | '.' | ',' | '!' | '?' | ';' | ':' | '-' | '\''
            )
    })
}

fn prose_environment(name: &str) -> bool {
    paragraph_environment(name)
        || matches!(name, "document" | "itemize" | "enumerate" | "description")
}

fn prose_block(block: &Block, config: &CompiledConfig) -> bool {
    let name = block.name.as_str();
    match block.kind {
        syntax::BlockKind::Group | syntax::BlockKind::NamedGroup | syntax::BlockKind::Item => true,
        syntax::BlockKind::Environment => {
            prose_environment(name)
                && !config
                    .config
                    .syntax
                    .environment_arguments
                    .contains_key(name)
        }
        syntax::BlockKind::Heading
        | syntax::BlockKind::Command
        | syntax::BlockKind::MandatoryArgument => {
            (printed_command(name)
                || matches!(name, "section" | "subsection")
                || prose_environment(name))
                && !config.config.syntax.commands.contains_key(name)
        }
        _ => false,
    }
}

impl ParsedText {
    /// Intersect structural text tokens with the protection scanner's observed
    /// reader state. Independent readers can resume after a protected region.
    pub(super) fn text_checkpoints(&self, config: &CompiledConfig) -> &[TextCheckpoint] {
        self.text_readers.get_or_init(|| {
            if !super::position_independent_grammar(config) {
                return Vec::new();
            }
            let Some(model) = self.metadata.existing_model() else {
                return Vec::new();
            };
            let windows = model.reader_windows();
            let source = self.document.as_str();
            let mut state = TextReaderState::default();
            let mut checkpoints = Vec::new();
            let mut reader = 0;
            let mut operand_end = 0;
            for token in self.tokens.as_deref().unwrap_or_default() {
                match token.kind {
                    syntax::TokenKind::Command if &source[token.span.clone()] == "\\input" => {
                        let start = crate::trivia::skip(source, token.span.end);
                        if source.as_bytes().get(start) != Some(&b'{') {
                            operand_end = source[start..]
                                .find(|c: char| c.is_whitespace() || c == '%')
                                .map_or(source.len(), |i| start + i);
                        }
                    }
                    syntax::TokenKind::BeginEnvironment => state.environments += 1,
                    syntax::TokenKind::EndEnvironment => {
                        state.environments = state.environments.saturating_sub(1)
                    }
                    syntax::TokenKind::OpenBrace => state.groups += 1,
                    syntax::TokenKind::CloseBrace => state.groups = state.groups.saturating_sub(1),
                    syntax::TokenKind::Text => {
                        while reader < windows.len() && windows[reader].span.end <= token.span.start
                        {
                            reader += 1;
                        }
                        for (i, window) in windows[reader..]
                            .iter()
                            .enumerate()
                            .take_while(|(_, w)| w.span.start < token.span.end)
                        {
                            let span = token.span.start.max(window.span.start).max(operand_end)
                                ..token.span.end.min(window.span.end);
                            if !span.is_empty() && prose_token(&source[span.clone()]) {
                                checkpoints.push(TextCheckpoint {
                                    span,
                                    state,
                                    reader: reader + i,
                                });
                            }
                        }
                    }
                    _ => (),
                }
            }
            checkpoints
        })
    }
}

impl ParseContext<'_> {
    pub(super) fn project_plain_text(&mut self, document: &SourceDocument) -> bool {
        let Some(parsed) = self
            .parsed
            .as_ref()
            .filter(|p| p.tokens.is_some() && p.blocks.is_some())
        else {
            return false;
        };
        let Some(model) = parsed.metadata.existing_model() else {
            return false;
        };
        if model.has_errors() {
            return false;
        }
        let change = SourceChange::between(parsed.document.as_str(), document.as_str());
        if parsed.can_project_comment(document, &change, self.config)
            || parsed.can_project_literal(document, &change, self.config)
        {
            self.project_inert_text(document, &change);
            return true;
        }
        let old = &parsed.document.as_str()[change.old.clone()];
        let new = &document.as_str()[change.new.clone()];
        if old.contains('\\') || new.contains('\\') {
            return self.project_printed_paragraph(document, &change);
        }
        if parsed.document.as_str()[change.old.clone()].contains(['{', '}'])
            || document.as_str()[change.new.clone()].contains(['{', '}'])
        {
            return self.project_group_edit(document, &change);
        }
        if !prose_token(&document.as_str()[change.new.clone()])
            || !prose_token(&parsed.document.as_str()[change.old.clone()])
        {
            return false;
        }
        let checkpoints = parsed.text_checkpoints(self.config);
        let i = checkpoints.partition_point(|c| c.span.end <= change.old.start);
        let Some(checkpoint) = checkpoints.get(i) else {
            return false;
        };
        if change.old.start <= checkpoint.span.start
            || change.old.end >= checkpoint.span.end
            || !model.reader_windows()[checkpoint.reader]
                .accepts(&document.as_str()[change.new.clone()])
        {
            return false;
        }
        // A word immediately before an opening group can supply that group's
        // name. Its spelling must be re-read by the full structural parser.
        if matches!(
            parsed.document.as_bytes().get(checkpoint.span.end),
            Some(b'{' | b'[')
        ) {
            return false;
        }
        // Verify that the semantic environment stack and the structural reader
        // agree here before carrying their state through an inert text edit.
        let inside_environment = model
            .invocation_context_at(change.old.start)
            .environment()
            .is_some();
        if (checkpoint.state.environments > 0) != inside_environment {
            return false;
        }
        if parsed.blocks.as_deref().unwrap().iter().any(|b| {
            [&b.begin, &b.end]
                .iter()
                .any(|r| r.start < change.old.end && change.old.start < r.end)
                || b.body.start < change.old.end
                    && change.old.start < b.body.end
                    && !prose_block(b, self.config)
        }) {
            return false;
        }
        self.project_inert_text(document, &change);
        true
    }

    fn project_inert_text(&mut self, document: &SourceDocument, change: &SourceChange) {
        let parsed = self.parsed.as_mut().expect("validated reader");
        let model = parsed.metadata.existing_model().expect("semantic snapshot");
        let updated = model.project_text(document.clone(), change);
        parsed.protected = updated.protected_ranges().to_vec();
        parsed.metadata = Rc::new(AnalysisMemo::with_model(updated));
        for token in parsed.tokens.iter_mut().flatten() {
            change.project(&mut token.span);
        }
        for block in parsed.blocks.iter_mut().flatten() {
            for range in [&mut block.begin, &mut block.body, &mut block.end] {
                change.project(range);
            }
            for (_, range) in &mut block.branches {
                change.project(range);
            }
        }
        for checkpoint in parsed.text_readers.get_mut().into_iter().flatten() {
            change.project(&mut checkpoint.span);
        }
        parsed.document = document.clone();
        parsed.environment_ranges.take();
        parsed.expl3_protected = None;
    }

    /// Reparse a complete plain group when its delimiters stay outside the
    /// edit. Its old entry state and matching closing delimiter bound the
    /// replacement; the local parser verifies any new nested group stack.
    fn project_group_edit(&mut self, document: &SourceDocument, change: &SourceChange) -> bool {
        let parsed = self.parsed.as_ref().expect("source snapshot");
        let blocks = parsed.blocks.as_deref().expect("structural snapshot");
        let Some((owner, outer)) = blocks
            .iter()
            .enumerate()
            .filter(|(_, b)| {
                (matches!(
                    b.kind,
                    syntax::BlockKind::Group | syntax::BlockKind::NamedGroup
                ) || b.kind == syntax::BlockKind::MandatoryArgument
                    && (prose_environment(&b.name) || printed_command(&b.name)))
                    && b.body.start < change.old.start
                    && change.old.end < b.body.end
            })
            .min_by_key(|(_, b)| b.body.len())
        else {
            return false;
        };
        let body = outer.body.clone();
        let after = change.position(body.start)..change.position(body.end);
        let plain_groups = |text: &str| text.split(['{', '}']).all(prose_token);
        if !plain_groups(&parsed.document.as_str()[body.clone()])
            || !plain_groups(&document.as_str()[after.clone()])
        {
            return false;
        }
        let readers = parsed.text_checkpoints(self.config);
        let i = readers.partition_point(|r| r.span.end <= body.start);
        if readers.get(i).is_none_or(|r| {
            r.span.start < body.start || r.span.end > body.end || r.state.groups == 0 || {
                let window = &parsed.metadata.existing_model().unwrap().reader_windows()[r.reader];
                window.span.start > body.start
                    || window.span.end < body.end
                    || !window.accepts(&document.as_str()[after.clone()])
            }
        }) {
            return false;
        }
        // Appended package blocks do not necessarily follow source order.
        // Only splice an ordered structural index with contiguous descendants.
        if blocks
            .windows(2)
            .any(|p| p[0].begin.start > p[1].begin.start)
        {
            return false;
        }
        let children_end = blocks.partition_point(|b| b.begin.start < body.end);
        if blocks[owner + 1..children_end].iter().any(|b| {
            !matches!(
                b.kind,
                syntax::BlockKind::Group | syntax::BlockKind::NamedGroup
            ) || b.end.end > body.end
        }) {
            return false;
        }
        let mut parent = blocks[owner].parent;
        while let Some(id) = parent {
            if !prose_block(&blocks[id], self.config) {
                return false;
            }
            parent = blocks[id].parent;
        }
        let local = &document.as_str()[after.clone()];
        let analysis = SourceAnalysis::new(local, &self.config.config, &[]);
        let Ok((local_tokens, local_blocks)) =
            syntax::shared_blocks_with_analysis(local, self.config, &[], &analysis)
        else {
            return false;
        };
        if local_blocks.iter().any(|b| {
            !matches!(
                b.kind,
                syntax::BlockKind::Group | syntax::BlockKind::NamedGroup
            )
        }) {
            return false;
        }
        let updated = parsed
            .metadata
            .existing_model()
            .expect("semantic snapshot")
            .project_text(document.clone(), change);
        self.splice_structure(
            change,
            body,
            owner + 1..children_end,
            Some(owner),
            (local_tokens, local_blocks),
            updated,
        );
        true
    }

    /// A run between two unchanged prose tokens can acquire or lose complete
    /// printed arguments without changing the outer environment or bindings.
    fn project_printed_paragraph(
        &mut self,
        document: &SourceDocument,
        change: &SourceChange,
    ) -> bool {
        let parsed = self.parsed.as_ref().expect("source snapshot");
        let old = parsed.document.as_str();
        let new = document.as_str();
        let printed = |s: &str| {
            s.match_indices('\\').all(|(at, _)| {
                syntax::command_at(s, at).is_some_and(|(name, end)| match name {
                    name if printed_command(name) => s.as_bytes().get(end) == Some(&b'{'),
                    "begin" | "end" => {
                        syntax::env_after_command(s, name, end).is_some_and(|(env, _, _)| {
                            paragraph_environment(env)
                                && !self
                                    .config
                                    .config
                                    .syntax
                                    .inferred_environment_arguments
                                    .contains_key(env)
                                && !self
                                    .config
                                    .config
                                    .syntax
                                    .environment_arguments
                                    .contains_key(env)
                        })
                    }
                    _ => false,
                })
            }) && s.split(['\\', '{', '}']).all(prose_token)
        };
        if !printed(&old[change.old.clone()]) || !printed(&new[change.new.clone()]) {
            return false;
        }
        let tokens = parsed.tokens.as_deref().expect("tokens");
        let first = tokens.partition_point(|t| t.span.end < change.old.start);
        let end = tokens.partition_point(|t| t.span.end <= change.old.end);
        let Some(start) = tokens.get(first).filter(|t| {
            matches!(t.kind, syntax::TokenKind::Text | syntax::TokenKind::Newline)
                && t.span.start < change.old.start
        }) else {
            return false;
        };
        let suffix = &tokens[end..];
        let tail = suffix
            .iter()
            .take_while(|t| {
                matches!(
                    t.kind,
                    syntax::TokenKind::Space | syntax::TokenKind::Newline
                )
            })
            .count();
        let finish = suffix
            .get(tail)
            .filter(|t| t.kind == syntax::TokenKind::Text)
            .or_else(|| {
                tail.checked_sub(1)
                    .and_then(|i| suffix.get(i))
                    .filter(|t| t.kind == syntax::TokenKind::Newline)
            });
        let Some(finish) = finish.filter(|t| t.span.end > change.old.end) else {
            return false;
        };
        let body = start.span.start..finish.span.end;
        let after = body.start..change.position(body.end);
        if !printed(&old[body.clone()])
            || !printed(&new[after.clone()])
            || matches!(old.as_bytes().get(body.end), Some(b'{' | b'['))
            || !parsed
                .text_checkpoints(self.config)
                .iter()
                .any(|r| r.span.start < body.end && body.start < r.span.end)
            || parsed
                .protected
                .iter()
                .any(|r| r.start < body.end && body.start < r.end)
        {
            return false;
        }
        let blocks = parsed.blocks.as_deref().expect("blocks");
        if blocks
            .windows(2)
            .any(|p| p[0].begin.start > p[1].begin.start)
        {
            return false;
        }
        let mut parent = None;
        let children = blocks.partition_point(|b| b.begin.start < body.start)
            ..blocks.partition_point(|b| b.begin.start < body.end);
        for (i, block) in blocks.iter().enumerate().take(children.end) {
            if children.contains(&i) {
                if block.end.end > body.end || !prose_block(block, self.config) {
                    return false;
                }
            } else if block.body.end > body.start {
                if block.body.start > body.start
                    || block.body.end < body.end
                    || !matches!(
                        block.kind,
                        syntax::BlockKind::Environment | syntax::BlockKind::Group
                    )
                    || !prose_block(block, self.config)
                {
                    return false;
                }
                parent = Some(i);
            }
        }
        let model = parsed.metadata.existing_model().expect("semantic snapshot");
        let before_context = model.invocation_context_at(body.start);
        let grammar = &self.config.config.syntax;
        if before_context.environment() != model.invocation_context_at(body.end - 1).environment()
            || !grammar.command_import_events.is_empty()
            || !grammar.environment_alias_events.is_empty()
            || !grammar.inferred_environment_aliases.is_empty()
            || !grammar.environment_argument_events.is_empty()
        {
            return false;
        }
        let local_model = |text: &str| {
            crate::source_model::SourceModel::rescan_document(
                SourceDocument::new(text),
                self.config,
                None,
            )
        };
        let (Ok(previous), Ok(next)) = (local_model(&old[body.clone()]), local_model(&new[after]))
        else {
            return false;
        };
        let local = next.source();
        let analysis =
            SourceAnalysis::for_model(local, &self.config.config, next.protected_ranges(), &next);
        let Ok(structure) = syntax::shared_blocks_with_analysis(
            local,
            self.config,
            next.protected_ranges(),
            &analysis,
        ) else {
            return false;
        };
        if structure.1.iter().any(|b| !prose_block(b, self.config)) {
            return false;
        }
        let Some(updated) = model.project_printed_commands(
            document.clone(),
            change,
            body.clone(),
            &previous,
            &next,
        ) else {
            return false;
        };
        self.splice_structure(change, body, children, parent, structure, updated);
        true
    }

    fn splice_structure(
        &mut self,
        change: &SourceChange,
        body: Range<usize>,
        children: Range<usize>,
        parent: Option<usize>,
        (mut local_tokens, mut local_blocks): (Vec<Token>, Vec<Block>),
        updated: crate::source_model::SourceModel,
    ) {
        let offset = body.start;
        let inserted = local_blocks.len();
        for token in &mut local_tokens {
            token.span = offset + token.span.start..offset + token.span.end;
        }
        for block in &mut local_blocks {
            for range in [&mut block.begin, &mut block.body, &mut block.end] {
                *range = offset + range.start..offset + range.end;
            }
            block.parent = block.parent.map(|i| children.start + i).or(parent);
        }
        let parsed = self.parsed.as_mut().expect("source snapshot");
        let tokens = parsed.tokens.as_mut().expect("structural tokens");
        let first = tokens.partition_point(|t| t.span.end <= body.start);
        let last = tokens.partition_point(|t| t.span.start < body.end);
        for token in &mut tokens[last..] {
            change.project(&mut token.span);
        }
        tokens.splice(first..last, local_tokens);
        let blocks = parsed.blocks.as_mut().expect("structural snapshot");
        for (i, block) in blocks.iter_mut().enumerate() {
            if children.contains(&i) {
                continue;
            }
            for range in [&mut block.begin, &mut block.body, &mut block.end] {
                change.project(range);
            }
            for (_, range) in &mut block.branches {
                change.project(range);
            }
            if let Some(parent) = &mut block.parent
                && *parent >= children.end
            {
                *parent = children.start + inserted + (*parent - children.end);
            }
        }
        blocks.splice(children, local_blocks);
        parsed.protected = updated.protected_ranges().to_vec();
        parsed.document = updated.document().clone();
        parsed.metadata = Rc::new(AnalysisMemo::with_model(updated));
        parsed.text_readers.take();
        parsed.environment_ranges.take();
        parsed.expl3_protected = None;
    }
}

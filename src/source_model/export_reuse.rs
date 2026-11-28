use super::{Config, SourceToken, SourceTokenKind};

#[derive(Default)]
pub(super) struct ExportProjection {
    tokens: Vec<SourceToken>,
    active: bool,
}

impl ExportProjection {
    pub(super) fn update<'a>(
        &'a mut self,
        source: &str,
        config: &Config,
        tokens: &'a [SourceToken],
        keep: usize,
    ) -> (&'a [SourceToken], usize) {
        let syntax = &config.syntax;
        let active = syntax.command_import_events.is_empty()
            && syntax.environment_alias_events.is_empty()
            && syntax.environment_argument_events.is_empty()
            && syntax.verbatim_environment_events.is_empty()
            && syntax.package_load_events.is_empty();
        let keep = if self.active == active { keep } else { 0 };
        self.active = active;
        if !active {
            self.tokens.clear();
            return (tokens, keep);
        }
        const BLOCK: usize = 64;
        let restart = keep - keep % BLOCK;
        let start = tokens
            .get(restart)
            .map_or(source.len(), |token| token.span.start);
        let retained = self
            .tokens
            .partition_point(|token| token.span.start < start);
        self.tokens.truncate(retained);
        for block in tokens[restart..].chunks(BLOCK) {
            let mut first_text = None;
            let mut last_text = None;
            let mut last_passive = None;
            for token in block {
                match token.kind {
                    SourceTokenKind::Space
                    | SourceTokenKind::Newline
                    | SourceTokenKind::Comment => {
                        last_passive = Some(token);
                    }
                    SourceTokenKind::Text if &source[token.span.clone()] != "=" => {
                        first_text.get_or_insert(token);
                        last_text = Some(token);
                        last_passive = Some(token);
                    }
                    _ => {
                        self.finish_run(first_text.take(), last_text.take(), last_passive.take());
                        self.tokens.push(token.clone());
                    }
                }
            }
            self.finish_run(first_text, last_text, last_passive);
        }
        (&self.tokens, retained)
    }

    fn finish_run(
        &mut self,
        first_text: Option<&SourceToken>,
        last_text: Option<&SourceToken>,
        last_passive: Option<&SourceToken>,
    ) {
        let mut previous = None;
        for token in [first_text, last_text, last_passive].into_iter().flatten() {
            if previous != Some(token.span.start) {
                self.tokens.push(token.clone());
                previous = Some(token.span.start);
            }
        }
    }
}

use super::{CompiledConfig, ParsedText, SourceChange, SourceDocument, syntax};

impl ParsedText {
    /// A closed literal reader resumes at exactly its saved end when only its
    /// payload changes. The first matching delimiter must still be that end.
    pub(super) fn can_project_literal(
        &self,
        document: &SourceDocument,
        change: &SourceChange,
        config: &CompiledConfig,
    ) -> bool {
        let Some(model) = self.metadata.existing_model() else {
            return false;
        };
        let ranges = model.lexical_protection();
        let i = ranges.partition_point(|r| r.end <= change.old.start);
        let Some(range) = ranges
            .get(i)
            .filter(|r| r.start < change.old.start && change.old.end < r.end)
        else {
            return false;
        };
        let Some(tokens) = self.tokens.as_deref() else {
            return false;
        };
        let t = tokens.partition_point(|t| t.span.end <= range.start);
        if tokens
            .get(t)
            .is_none_or(|t| t.kind != syntax::TokenKind::Protected || t.span != *range)
            || !super::position_independent_grammar(config)
            || crate::source_model::SourceModel::has_anchored_loader_events(&config.config)
            || model
                .diagnostics()
                .iter()
                .any(|d| d.span.start < range.end && d.span.end > range.start)
        {
            return false;
        }
        let old = &self.document.as_str()[range.clone()];
        let new = &document.as_str()[range.start..change.position(range.end)];
        let payload_start = change.old.start - range.start;
        let payload_end = change.old.end - range.start;
        if let Some((name @ ("verb" | "verb*"), end)) = syntax::command_at(old, 0)
            && let Some(delimiter) = old[end..].chars().next()
        {
            return payload_start >= end + delimiter.len_utf8()
                && payload_end <= old.len() - delimiter.len_utf8()
                && syntax::inline_end(old, name, end) == Some(old.len())
                && syntax::inline_end(new, name, end) == Some(new.len());
        }
        if let Some((name @ ("verbatim" | "verbatim*"), true, end)) = syntax::env_at(old, 0) {
            let close = format!("\\end{{{name}}}");
            return old.ends_with(&close)
                && payload_start >= end
                && payload_end <= old.len() - close.len()
                && old[end..].find(&close) == Some(old.len() - end - close.len())
                && new.ends_with(&close)
                && new[end..].find(&close) == Some(new.len() - end - close.len());
        }
        false
    }

    /// A comment's payload can change without changing its reader. Directives,
    /// translated characters and file-relative imports can affect later input.
    pub(super) fn can_project_comment(
        &self,
        document: &SourceDocument,
        change: &SourceChange,
        config: &CompiledConfig,
    ) -> bool {
        let Some(tokens) = self.tokens.as_deref() else {
            return false;
        };
        let i = tokens.partition_point(|token| token.span.end <= change.old.start);
        let Some(token) = tokens.get(i).filter(|t| {
            t.kind == syntax::TokenKind::Comment
                && t.span.start < change.old.start
                && change.old.end < t.span.end
        }) else {
            return false;
        };
        if !super::position_independent_grammar(config) {
            return false;
        }
        let old = &self.document.as_str()[token.span.clone()];
        let new = &document.as_str()[token.span.start..change.position(token.span.end)];
        let ordinary = |comment: &str| {
            comment.strip_prefix('%').is_some_and(|tail| {
                tail.trim_end_matches(['\r', '\n']).chars().all(|c| {
                    c.is_alphanumeric() || matches!(c, ' ' | '\t' | '.' | ',' | '-' | '\'')
                })
            })
        };
        ordinary(old)
            && ordinary(new)
            && !self.document.as_str()[change.old.clone()].contains(['\r', '\n'])
            && !document.as_str()[change.new.clone()].contains(['\r', '\n'])
    }
}

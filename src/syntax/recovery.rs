//! Re-enter normal layout after a delimited, structurally invalid source unit.

use std::ops::Range;

use crate::{
    CompiledConfig, Diagnostic, FileKind, FormatError, FormatOutput, Severity,
    source_model::{RuleOrigin, SourceModel},
};

use super::{command_at, env_at, line_end, line_start};

pub(crate) struct Recovery {
    range: Range<usize>,
    opener: usize,
    diagnostic: Diagnostic,
}

impl Recovery {
    fn find(source: &str, model: &SourceModel, config: &CompiledConfig) -> Option<Self> {
        let mut errors = model
            .diagnostics()
            .iter()
            .filter(|d| d.severity == Severity::Error);
        let error = errors.next()?;
        if errors.next().is_some() {
            return None;
        }
        let mismatch = model.mismatch.as_ref()?;
        let open = mismatch.start;
        let close = error.span.start;
        let close_end = mismatch.end;
        if open >= close || !source.is_char_boundary(open) || !source.is_char_boundary(close_end) {
            return None;
        }
        let range = line_start(source, open)..line_end(source, close_end);
        // A virtual boundary must coincide with a real line boundary. Leave
        // commands adjacent to the failing unit to the application's strict path.
        if !source[range.start..open]
            .trim_matches([' ', '\t'])
            .is_empty()
            || !source[close_end..range.end].trim().is_empty()
            || source[..range.start].trim().is_empty() && source[range.end..].trim().is_empty()
        {
            return None;
        }
        let mut pos = open;
        let mut groups = 0usize;
        while pos < close_end {
            let ch = source[pos..].chars().next()?;
            if ch == '\\' {
                if let Some((name, _, end)) = env_at(source, pos) {
                    if groups != 0
                        || model.has_environment_declaration(name)
                        || !name
                            .chars()
                            .all(|c| c.is_alphanumeric() || matches!(c, '*' | '-' | '@'))
                        || source[pos + 1..end].contains(['\\', '%', '^'])
                    {
                        return None;
                    }
                    pos = end;
                } else {
                    let (name, end) = command_at(source, pos)?;
                    if !matches!(name, "emph" | "textbf" | "textit")
                        || source.as_bytes().get(end) != Some(&b'{')
                        || config.config.syntax.commands.contains_key(name)
                        || model
                            .invocation_rule(pos)
                            .is_some_and(|r| r.origin != RuleOrigin::Builtin)
                        || model.declarations().iter().any(|d| d.name == name)
                        || config
                            .config
                            .syntax
                            .command_import_events
                            .iter()
                            .any(|event| event.commands.contains_key(name))
                    {
                        return None;
                    }
                    pos = end;
                }
            } else if ch == '{' {
                groups += 1;
                pos += 1;
            } else if ch == '}' {
                groups = groups.checked_sub(1)?;
                pos += 1;
            } else if ch.is_alphanumeric() || ch.is_whitespace() || "[].,;:-!?".contains(ch) {
                pos += ch.len_utf8();
            } else {
                return None;
            }
        }
        if pos != close_end || groups != 0 {
            return None;
        }
        Some(Self {
            range: range.clone(),
            opener: open - range.start,
            diagnostic: Diagnostic {
                severity: Severity::Warning,
                span: error.span.clone(),
                message: format!(
                    "{}; source bytes {}..{} preserved",
                    error.message, range.start, range.end
                ),
            },
        })
    }

    pub(crate) fn format(
        source: &str,
        kind: FileKind,
        config: &CompiledConfig,
        model: &SourceModel,
        format_masked: impl FnOnce(&str, SourceModel) -> Result<FormatOutput, FormatError>,
    ) -> Option<FormatOutput> {
        if kind != FileKind::Tex
            || source
                .match_indices('\r')
                .any(|(at, _)| source.as_bytes().get(at + 1) != Some(&b'\n'))
        {
            return None;
        }
        let mut recovery = Self::find(source, model, config)?;
        let mut masked = source.to_owned();
        let mut marks = Vec::new();
        let mut ids = 0usize..;
        let masked_model = loop {
            // Every substitution is byte-length preserving, so all diagnoses and
            // file-relative import events remain in the caller's coordinate space.
            let range = recovery.range.clone();
            let marker = ids
                .by_ref()
                .map(|id| format!("%r{id}!"))
                .find(|m| !source.contains(m))?;
            let damaged = &source[range.clone()];
            let content = damaged
                .strip_suffix("\r\n")
                .or_else(|| damaged.strip_suffix('\n'))
                .unwrap_or(damaged);
            let padding = content.len().checked_sub(marker.len())?;
            let mut placeholder = marker.clone();
            placeholder.extend(std::iter::repeat_n(' ', padding));
            masked.replace_range(range.start..range.start + content.len(), &placeholder);
            let next = SourceModel::analyze(&masked, config);
            let at = next
                .tokens()
                .partition_point(|token| token.span.end <= range.start);
            let comment = next.tokens().get(at)?;
            if comment.kind != crate::source_model::SourceTokenKind::Comment
                || comment.span.start != range.start
            {
                return None;
            }
            marks.push((recovery, marker));
            if !next.has_errors() {
                break next;
            }
            recovery = Self::find(&masked, &next, config)?;
            if recovery.range.start < range.end {
                return None;
            }
        };
        let mut output = format_masked(&masked, masked_model).ok()?;
        if output
            .diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error)
        {
            return None;
        }

        // Restore from the right: each newly exposed mismatch must be the
        // first parser error, with the same opener and closer in its own bytes.
        let mut next_start = output.text.len();
        for (recovery, marker) in marks.into_iter().rev() {
            let mut positions = output.text.match_indices(&marker);
            let (start, _) = positions.next()?;
            if positions.next().is_some() {
                return None;
            }
            let line =
                line_start(&output.text, start)..line_end(&output.text, start + marker.len());
            if line.end > next_start || output.text[line.clone()].trim() != marker {
                return None;
            }
            let range = recovery.range;
            let damaged = &source[range.clone()];
            output.text.replace_range(line.clone(), damaged);
            let restored = SourceModel::analyze(&output.text, config);
            let actual = Self::find(&output.text, &restored, config)?;
            if actual.range != (line.start..line.start + damaged.len())
                || actual.opener != recovery.opener
                || actual.diagnostic.span.start - line.start
                    != recovery.diagnostic.span.start - range.start
            {
                return None;
            }
            next_start = line.start;
            output.diagnostics.push(recovery.diagnostic);
        }
        output
            .diagnostics
            .sort_by_key(|diagnostic| diagnostic.span.start);
        Some(output)
    }
}

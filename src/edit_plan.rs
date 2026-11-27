//! Source-coordinate edits and conflict resolution against one immutable snapshot.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::ops::Range;

use crate::FormatError;

type EditMapping = (Range<usize>, Range<usize>);
type AppliedMappings = Result<(String, Vec<EditMapping>), FormatError>;

/// Local layout rules register the source spans they inspect, including
/// candidates that currently fit on a line and therefore produce no edit.
#[derive(Default)]
pub(crate) struct RuleEdits {
    pub(crate) edits: EditPlan<'static>,
    pub(crate) reads: Vec<Range<usize>>,
}

/// Edits in this batch all address the same immutable snapshot. A later
/// reader can share it while its inputs are independent of pending writes.
#[derive(Default)]
pub(crate) struct LayoutBatch {
    edits: EditPlan<'static>,
    requires_snapshot: bool,
}

impl LayoutBatch {
    pub(crate) fn requires_snapshot(&self) -> bool {
        self.requires_snapshot
    }

    pub(crate) fn affects(&self, reads: &[Range<usize>]) -> bool {
        reads.iter().any(|read| {
            let edits = &self.edits.edits;
            let start = edits.partition_point(|edit| edit.range.end <= read.start);
            edits
                .get(start)
                .is_some_and(|edit| edit.range.start < read.end)
        })
    }

    pub(crate) fn queue(&mut self, source: &str, rule: RuleEdits) {
        let mut next = rule.edits.resolve_first_wins();
        next.edits
            .retain(|edit| source[edit.range.clone()] != *edit.replacement);
        if next.is_empty() {
            return;
        }
        // Reordering a control sequence or changing the global newline
        // preference may affect readers even outside the edited construct.
        self.requires_snapshot |= next.edits.iter().any(|edit| {
            let old = &source[edit.range.clone()];
            let new = edit.replacement.as_ref();
            old.contains(['\\', '^'])
                || new.contains(['\\', '^'])
                || old.contains("\r\n") && !new.contains("\r\n")
        });
        self.edits.edits.extend(next.edits);
        self.edits
            .edits
            .sort_by_key(|edit| (edit.range.start, edit.range.end));
    }

    pub(crate) fn flush(&mut self, text: &mut String) -> Result<(), FormatError> {
        let batch = std::mem::take(self);
        if !batch.edits.is_empty() {
            *text = batch.edits.apply_ordered(text)?;
        }
        Ok(())
    }
}

/// Maps diagnostics back through consecutive edit plans, each relative to
/// the previous output. A diagnostic within generated text covers its input.
#[derive(Clone, Default)]
pub(crate) struct EditHistory(Vec<Vec<EditMapping>>);

impl EditHistory {
    pub(crate) fn append(&mut self, other: Self) {
        self.0.extend(other.0);
    }

    pub(crate) fn record(&mut self, edits: Vec<EditMapping>) {
        if !edits.is_empty() {
            self.0.push(edits);
        }
    }

    pub(crate) fn original_range(&self, mut range: Range<usize>) -> Range<usize> {
        for edits in self.0.iter().rev() {
            let map = |at: usize, start: bool| {
                let i = edits.partition_point(|(_, out)| out.end < at || start && out.end == at);
                match edits.get(i) {
                    Some((input, out)) if at <= out.start => input.start - (out.start - at),
                    Some((input, _)) if start => input.start,
                    Some((input, _)) => input.end,
                    None => {
                        let (input, out) = edits.last().expect("nonempty edit stage");
                        input.end + (at - out.end)
                    }
                }
            };
            let start = map(range.start, true);
            range = start..map(range.end, false).max(start);
        }
        range
    }
}

#[derive(Debug, Clone)]
struct TextEdit<'a> {
    range: Range<usize>,
    replacement: Cow<'a, str>,
    origin: EditOrigin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EditOrigin {
    Layout,
    TableRow,
    TableRule,
    TikzPath,
    ConsecutiveArguments,
    TrailingComment,
    Category,
    Named,
}

impl EditOrigin {
    pub(crate) fn priority(self) -> u8 {
        match self {
            Self::Layout
            | Self::TableRow
            | Self::TableRule
            | Self::TikzPath
            | Self::ConsecutiveArguments
            | Self::TrailingComment => 0,
            Self::Category => 1,
            Self::Named => 2,
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct EditPlan<'a> {
    edits: Vec<TextEdit<'a>>,
}

impl<'a> EditPlan<'a> {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn len(&self) -> usize {
        self.edits.len()
    }

    pub(crate) fn truncate(&mut self, len: usize) {
        self.edits.truncate(len);
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.edits.is_empty()
    }

    pub(crate) fn last_replacement(&self) -> Option<(&Range<usize>, &str)> {
        self.edits
            .last()
            .map(|edit| (&edit.range, edit.replacement.as_ref()))
    }

    /// Render a local span using ordered edits contained in it. A reader can
    /// measure a line after earlier proposals without materializing the document.
    pub(crate) fn projected_span<'s>(&self, source: &'s str, span: Range<usize>) -> Cow<'s, str> {
        let start = self
            .edits
            .partition_point(|edit| edit.range.end <= span.start);
        let edits = &self.edits[start..];
        let count = edits.partition_point(|edit| edit.range.start < span.end);
        if count == 0 {
            return Cow::Borrowed(&source[span]);
        }
        let mut text = String::with_capacity(span.len());
        let mut cursor = span.start;
        for edit in &edits[..count] {
            debug_assert!(cursor <= edit.range.start && edit.range.end <= span.end);
            text.push_str(&source[cursor..edit.range.start]);
            text.push_str(&edit.replacement);
            cursor = edit.range.end;
        }
        text.push_str(&source[cursor..span.end]);
        Cow::Owned(text)
    }

    pub(crate) fn replace(&mut self, range: Range<usize>, replacement: impl Into<Cow<'a, str>>) {
        self.replace_from(EditOrigin::Layout, range, replacement);
    }

    pub(crate) fn replace_from(
        &mut self,
        origin: EditOrigin,
        range: Range<usize>,
        replacement: impl Into<Cow<'a, str>>,
    ) {
        self.edits.push(TextEdit {
            range,
            replacement: replacement.into(),
            origin,
        });
    }

    /// Higher-priority rules own their intervals. Equal-priority proposals
    /// use source order with stable ties, as in apply_first_wins.
    pub(crate) fn resolve_priorities(mut self) -> Self {
        if self.edits.iter().all(|edit| edit.origin.priority() == 0) {
            return self.resolve_first_wins();
        }
        self.edits.sort_by_key(|edit| {
            (
                std::cmp::Reverse(edit.origin.priority()),
                edit.range.start,
                edit.range.end,
            )
        });
        let mut accepted = BTreeMap::new();
        for edit in self.edits {
            let start = edit.range.start;
            let end = edit.range.end;
            let key = (start, end);
            if accepted
                .range(..key)
                .next_back()
                .is_some_and(|(&(_, prev_end), _)| prev_end > start)
                || accepted
                    .range(key..)
                    .next()
                    .is_some_and(|(&next, _)| next.0 < end || next == key)
            {
                continue;
            }
            accepted.insert(key, edit);
        }
        Self {
            edits: accepted.into_values().collect(),
        }
    }

    /// Prefer the first edit in source order, including one insertion before a
    /// replacement at the same coordinate. Stable sorting resolves exact ties.
    pub(crate) fn apply_first_wins(self, source: &str) -> Result<String, FormatError> {
        self.resolve_first_wins().apply_ordered(source)
    }

    pub(crate) fn resolve_first_wins(mut self) -> Self {
        self.edits
            .sort_by_key(|edit| (edit.range.start, edit.range.end));
        let mut cursor = 0;
        let mut last_insertion = None;
        self.edits.retain(|edit| {
            let range = &edit.range;
            if range.start < cursor || range.is_empty() && last_insertion == Some(range.start) {
                return false;
            }
            cursor = range.end;
            last_insertion = range.is_empty().then_some(range.start);
            true
        });
        self
    }

    pub(crate) fn apply_ordered(self, source: &str) -> Result<String, FormatError> {
        self.apply_ordered_inner::<false>(source)
            .map(|(output, _)| output)
    }

    pub(crate) fn apply_ordered_with_ranges(
        self,
        source: &str,
    ) -> Result<(String, Vec<Range<usize>>), FormatError> {
        self.apply_ordered_with_mappings(source)
            .map(|(output, mappings)| {
                (
                    output,
                    mappings
                        .into_iter()
                        .map(|(_, output_range)| output_range)
                        .collect(),
                )
            })
    }

    /// Keep the last edit at each start, then skip edits overlapping earlier ranges.
    /// The accepted edits retain source order for one-pass application.
    pub(crate) fn apply_with_last_start_wins(self, source: &str) -> Result<String, FormatError> {
        self.resolve_last_start_wins().apply_ordered(source)
    }

    pub(crate) fn apply_with_last_start_wins_and_mappings(self, source: &str) -> AppliedMappings {
        self.resolve_last_start_wins()
            .apply_ordered_with_mappings(source)
    }

    fn resolve_last_start_wins(self) -> Self {
        let mut edits = self.edits;
        edits.sort_by_key(|edit| edit.range.start);
        let mut selected = Vec::with_capacity(edits.len());
        let mut cursor = 0;
        let mut edits = edits.into_iter().peekable();
        while let Some(edit) = edits.next() {
            if edits
                .peek()
                .is_some_and(|next| next.range.start == edit.range.start)
            {
                continue;
            }
            if edit.range.start < cursor {
                continue;
            }
            cursor = edit.range.end;
            selected.push(edit);
        }
        Self { edits: selected }
    }

    pub(crate) fn apply_ordered_with_mappings(self, source: &str) -> AppliedMappings {
        self.apply_ordered_inner::<true>(source)
    }

    pub(crate) fn apply_recording(
        mut self,
        source: &str,
        history: Option<&mut EditHistory>,
    ) -> Result<String, FormatError> {
        self.edits
            .sort_by_key(|edit| (edit.range.start, edit.range.end));
        let Some(history) = history else {
            return self.apply_ordered(source);
        };
        let (output, edits) = self.apply_ordered_with_mappings(source)?;
        history.record(edits);
        Ok(output)
    }

    fn apply_ordered_inner<const MAPPED: bool>(self, source: &str) -> AppliedMappings {
        if self.edits.is_empty() {
            return Ok((source.to_owned(), Vec::new()));
        }
        let mut capacity = source.len();
        let mut previous_end = 0;
        let mut previous_was_insertion = false;
        for edit in &self.edits {
            if edit.range.start > edit.range.end
                || edit.range.end > source.len()
                || !source.is_char_boundary(edit.range.start)
                || !source.is_char_boundary(edit.range.end)
                || edit.range.start < previous_end
                || edit.range.is_empty()
                    && previous_was_insertion
                    && edit.range.start == previous_end
            {
                return Err(FormatError::Replacement(format!(
                    "overlapping or invalid edits near byte {}",
                    edit.range.start
                )));
            }
            capacity = capacity
                .saturating_sub(edit.range.len())
                .saturating_add(edit.replacement.len());
            previous_end = edit.range.end;
            previous_was_insertion = edit.range.is_empty();
        }

        let mut output = String::with_capacity(capacity);
        let mut changed_ranges = Vec::with_capacity(if MAPPED { self.edits.len() } else { 0 });
        let mut cursor = 0;
        for edit in self.edits {
            output.push_str(&source[cursor..edit.range.start]);
            let start = output.len();
            output.push_str(edit.replacement.as_ref());
            if MAPPED {
                changed_ranges.push((edit.range.clone(), start..output.len()));
            }
            cursor = edit.range.end;
        }
        output.push_str(&source[cursor..]);
        Ok((output, changed_ranges))
    }
}

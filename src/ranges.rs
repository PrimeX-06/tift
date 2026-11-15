//! Physical-line selections with complete-document formatting context.

use std::fmt;
use std::ops::Range;
use std::str::FromStr;
use std::time::Instant;

use similar::{Algorithm, DiffTag, capture_diff_slices};

use crate::layout::StageTrace;
use crate::{CompiledConfig, Diagnostic, FileKind, FormatError, FormatOutput, Severity};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Interval {
    start: usize,
    end: usize,
}

/// A selection of one-based physical source lines.
///
/// Comma-separated single lines and closed ranges are accepted, such as
/// `3,7-12,!9`. An exclusion-only selection addresses every other line.
/// Endpoints past the input's final line are harmless. Adjacent ranges merge;
/// reversed endpoints are normalized. Inserted output lines belong to the
/// preceding source line, or to the first source line at the start of a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineRanges {
    included: Vec<Interval>,
    excluded: Vec<Interval>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct LineRangeError(String);

impl LineRanges {
    pub fn parse(value: &str) -> Result<Self, LineRangeError> {
        value.parse()
    }

    /// Returns whether this one-based line is selected.
    pub fn contains(&self, line: usize) -> bool {
        line != 0
            && (self.included.is_empty() || includes(&self.included, line))
            && !includes(&self.excluded, line)
    }

    fn selected(&self, source: &str) -> Vec<bool> {
        source
            .split_inclusive('\n')
            .enumerate()
            .map(|(index, _)| self.contains(index + 1))
            .collect()
    }
}

impl FromStr for LineRanges {
    type Err = LineRangeError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let mut included = Vec::new();
        let mut excluded = Vec::new();
        for entry in value.split(',') {
            let entry = entry.trim();
            let (negated, range) = entry
                .strip_prefix('!')
                .map_or((false, entry), |range| (true, range.trim()));
            let mut parts = range.split('-');
            let first = endpoint(parts.next().unwrap_or_default(), entry)?;
            let last = parts
                .next()
                .map(|part| endpoint(part, entry))
                .transpose()?
                .unwrap_or(first);
            if parts.next().is_some() {
                return Err(LineRangeError(format!("invalid line range {entry:?}")));
            }
            let interval = Interval {
                start: first.min(last),
                end: first.max(last),
            };
            if negated {
                excluded.push(interval);
            } else {
                included.push(interval);
            }
        }
        normalize(&mut included);
        normalize(&mut excluded);
        Ok(Self { included, excluded })
    }
}

impl fmt::Display for LineRanges {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for (negated, intervals) in [(false, &self.included), (true, &self.excluded)] {
            for interval in intervals {
                if !first {
                    formatter.write_str(",")?;
                }
                first = false;
                if negated {
                    formatter.write_str("!")?;
                }
                write!(formatter, "{}", interval.start)?;
                if interval.start != interval.end {
                    write!(formatter, "-{}", interval.end)?;
                }
            }
        }
        Ok(())
    }
}

fn endpoint(value: &str, entry: &str) -> Result<usize, LineRangeError> {
    let value = value.trim();
    value
        .bytes()
        .all(|byte| byte.is_ascii_digit())
        .then(|| value.parse::<usize>().ok())
        .flatten()
        .filter(|line| *line != 0)
        .ok_or_else(|| {
            LineRangeError(format!(
                "invalid line range {entry:?}: endpoints must be positive integers"
            ))
        })
}

fn normalize(intervals: &mut Vec<Interval>) {
    intervals.sort_unstable_by_key(|interval| interval.start);
    let mut merged: Vec<Interval> = Vec::with_capacity(intervals.len());
    for interval in intervals.drain(..) {
        if let Some(previous) = merged.last_mut()
            && interval.start <= previous.end.saturating_add(1)
        {
            previous.end = previous.end.max(interval.end);
            continue;
        }
        merged.push(interval);
    }
    *intervals = merged;
}

fn includes(intervals: &[Interval], line: usize) -> bool {
    intervals
        .get(
            intervals
                .partition_point(|interval| interval.start <= line)
                .wrapping_sub(1),
        )
        .is_some_and(|interval| line <= interval.end)
}

/// Format selected lines using syntax and layout from the entire document.
///
/// Unselected input lines retain every byte. A transformation that joins or
/// redistributes text across the selection boundary is retained in its original
/// form and reported as a warning. Selecting its complete paragraph or block
/// permits that transformation.
pub fn format_ranges(
    source: &str,
    kind: FileKind,
    config: &CompiledConfig,
    ranges: &LineRanges,
) -> Result<FormatOutput, FormatError> {
    if !ranges.selected(source).iter().any(|selected| *selected) {
        return Ok(FormatOutput {
            text: source.into(),
            diagnostics: Vec::new(),
        });
    }
    let formatted = crate::format(source, kind, config)?;
    Ok(restrict_output(source, formatted, ranges))
}

/// As [`format_ranges`], with full-context stage timing and a final range filter.
pub fn format_ranges_traced(
    source: &str,
    kind: FileKind,
    config: &CompiledConfig,
    ranges: &LineRanges,
) -> Result<(FormatOutput, Vec<StageTrace>), FormatError> {
    if !ranges.selected(source).iter().any(|selected| *selected) {
        return Ok((
            FormatOutput {
                text: source.into(),
                diagnostics: Vec::new(),
            },
            Vec::new(),
        ));
    }
    let (formatted, mut trace) = crate::layout::format_document_traced(source, kind, config)?;
    let started = Instant::now();
    let input_bytes = formatted.text.len();
    let before = formatted.text.clone();
    let output = restrict_output(source, formatted, ranges);
    trace.push(StageTrace {
        name: "line-ranges",
        changed: output.text != before,
        input_bytes,
        output_bytes: output.text.len(),
        elapsed: started.elapsed(),
    });
    Ok((output, trace))
}

/// Restrict an already-formatted document to a physical-line selection.
///
/// This permits callers with their own tracing to reuse the complete-document
/// pass. Existing diagnostic spans are retained. Range-boundary warnings use
/// byte offsets into `source`, including any UTF-8 BOM and CRLF endings.
pub fn restrict_output(
    source: &str,
    mut formatted: FormatOutput,
    ranges: &LineRanges,
) -> FormatOutput {
    let selected = ranges.selected(source);
    if selected.is_empty() || selected.iter().all(|selected| !selected) {
        formatted.text = source.into();
        return formatted;
    }
    if selected.iter().all(|selected| *selected) || source == formatted.text {
        return formatted;
    }
    let old: Vec<_> = source.split_inclusive('\n').collect();
    let new: Vec<_> = formatted.text.split_inclusive('\n').collect();
    // Whitespace-normalized keys align indentation and cell-spacing edits even
    // when all neighboring source lines also change. Unicode spaces, comments,
    // escapes and control-space tokens retain their non-ASCII/token content.
    let old_keys: Vec<_> = old.iter().map(|line| line_key(line)).collect();
    let new_keys: Vec<_> = new.iter().map(|line| line_key(line)).collect();
    let mut offsets = Vec::with_capacity(old.len() + 1);
    offsets.push(0);
    for line in &old {
        offsets.push(offsets.last().expect("initial offset") + line.len());
    }
    let mut projection = Projection {
        old: &old,
        new: &new,
        old_keys: &old_keys,
        new_keys: &new_keys,
        selected: &selected,
        offsets: &offsets,
        text: String::with_capacity(source.len().max(formatted.text.len())),
        diagnostics: &mut formatted.diagnostics,
    };
    for operation in capture_diff_slices(Algorithm::Myers, &old_keys, &new_keys) {
        let (tag, old_range, new_range) = operation.as_tag_tuple();
        if tag == DiffTag::Equal {
            for (old_index, new_index) in old_range.zip(new_range) {
                projection.text.push_str(if selected[old_index] {
                    new[new_index]
                } else {
                    old[old_index]
                });
            }
        } else {
            projection.changed(old_range, new_range);
        }
    }
    formatted.text = projection.text;
    formatted
}

fn line_key(line: &str) -> String {
    let line = line
        .strip_suffix("\r\n")
        .or_else(|| line.strip_suffix('\n'))
        .unwrap_or(line);
    let mut body = line;
    let mut escaped = false;
    for (index, byte) in line.bytes().enumerate() {
        if byte == b'%' && !escaped {
            // A newly inserted empty end comment keeps a tight TeX boundary.
            // Comment-only lines retain their marker: they differ from blank
            // lines that produce paragraph tokens. Text comments and escaped
            // percent symbols remain part of the key.
            if line[index + 1..].trim_matches([' ', '\t']).is_empty()
                && !line[..index]
                    .chars()
                    .all(|character| matches!(character, ' ' | '\t' | '\u{feff}'))
            {
                body = &line[..index];
            }
            break;
        }
        escaped = byte == b'\\' && !escaped;
    }
    let mut key = String::with_capacity(body.len());
    escaped = false;
    for character in body.chars() {
        if escaped || !matches!(character, ' ' | '\t' | '\r' | '\n') {
            key.push(character);
        }
        escaped = character == '\\' && !escaped;
    }
    key
}

struct Projection<'a, 'd> {
    old: &'a [&'a str],
    new: &'a [&'a str],
    old_keys: &'a [String],
    new_keys: &'a [String],
    selected: &'a [bool],
    offsets: &'a [usize],
    text: String,
    diagnostics: &'d mut Vec<Diagnostic>,
}

impl Projection<'_, '_> {
    fn changed(&mut self, old: Range<usize>, new: Range<usize>) {
        if old.is_empty() {
            let owner = old.start.saturating_sub(1);
            if self.selected.get(owner).is_some_and(|selected| *selected) {
                self.append_new(new);
            }
            return;
        }
        if self.selected[old.clone()].iter().all(|selected| *selected) {
            self.append_new(new);
            return;
        }
        if self.selected[old.clone()].iter().all(|selected| !selected) {
            self.append_old(old);
            return;
        }
        let old_content: String = self.old_keys[old.clone()].concat();
        let new_content: String = self.new_keys[new.clone()].concat();
        if old_content != new_content {
            self.preserve_boundary(old);
            return;
        }
        // Shared token boundaries divide a changed hunk into the smallest
        // complete source/output line groups. Split rows can be formatted
        // independently; joins spanning selected and unselected lines stay intact.
        let mut old_start = old.start;
        let mut new_start = new.start;
        while old_start < old.end && new_start < new.end {
            let mut old_end = old_start + 1;
            let mut new_end = new_start + 1;
            let mut old_bytes = self.old_keys[old_start].len();
            let mut new_bytes = self.new_keys[new_start].len();
            while old_bytes != new_bytes {
                if old_bytes < new_bytes && old_end < old.end {
                    old_bytes += self.old_keys[old_end].len();
                    old_end += 1;
                } else if new_bytes < old_bytes && new_end < new.end {
                    new_bytes += self.new_keys[new_end].len();
                    new_end += 1;
                } else {
                    self.preserve_boundary(old_start..old.end);
                    return;
                }
            }
            self.atomic(old_start..old_end, new_start..new_end);
            old_start = old_end;
            new_start = new_end;
        }
        if old_start < old.end {
            // Equal concatenated contents imply these remaining lines are blank.
            for index in old_start..old.end {
                if !self.selected[index] {
                    self.text.push_str(self.old[index]);
                }
            }
        }
        if new_start < new.end {
            self.changed(old.end..old.end, new_start..new.end);
        }
    }

    fn atomic(&mut self, old: Range<usize>, new: Range<usize>) {
        if self.selected[old.clone()].iter().all(|selected| *selected) {
            self.append_new(new);
        } else if self.selected[old.clone()].iter().all(|selected| !selected) {
            self.append_old(old);
        } else {
            self.preserve_boundary(old);
        }
    }

    fn append_old(&mut self, lines: Range<usize>) {
        for line in &self.old[lines] {
            self.text.push_str(line);
        }
    }

    fn append_new(&mut self, lines: Range<usize>) {
        for line in &self.new[lines] {
            self.text.push_str(line);
        }
    }

    fn preserve_boundary(&mut self, lines: Range<usize>) {
        let span = self.offsets[lines.start]..self.offsets[lines.end];
        self.append_old(lines);
        self.diagnostics.push(Diagnostic {
            severity: Severity::Warning,
            message: "layout crossing the line selection boundary was preserved; select the complete paragraph or block to format it".into(),
            span,
        });
    }
}

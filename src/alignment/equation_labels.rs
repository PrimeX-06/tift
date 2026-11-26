use super::{AlignmentConfig, MeasuredRow, Width, measure_width, trim_cell_end};
use crate::syntax;
use std::borrow::Cow;
use std::ops::Range;

pub(super) const ADDITIONAL_ENVIRONMENTS: &[&str] =
    &["gather", "gather*", "IEEEeqnarray", "IEEEeqnarray*"];

pub(super) fn environment(name: &str) -> bool {
    matches!(
        name.trim_end_matches('*'),
        "align" | "alignat" | "xalignat" | "flalign" | "gather" | "eqnarray" | "IEEEeqnarray"
    )
}

pub(super) fn supports_header(name: &str, header: &str) -> bool {
    if name == "empheq" {
        return super::column_spec::empheq_environment(header).is_some_and(|s| {
            matches!(
                s.trim_end_matches('*'),
                "align" | "alignat" | "gather" | "flalign"
            )
        });
    }
    if name.trim_end_matches('*') == "IEEEeqnarray" {
        // IEEE also permits text columns and arbitrary user-defined templates.
        // Only its standard math columns discard inserted source spaces.
        return super::column_spec::equation_header_argument(header)
            .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b"lcrLCR".contains(&b)));
    }
    environment(name)
}

pub(super) fn intertext_end(source: &str, start: usize, end: usize) -> Option<usize> {
    let ("intertext" | "shortintertext", after) = syntax::command_at(source, start)? else {
        return None;
    };
    let group = crate::trivia::skip_until(source, after, end);
    let close = syntax::balanced_end(&source[..end], group, b'{', b'}')?;
    let line_end = super::line_end(source, close, end);
    let tail = source[close..line_end].trim();
    (tail.is_empty() || tail.starts_with('%')).then_some(line_end)
}

#[derive(Clone, Copy)]
pub(super) struct Layout<'a> {
    pub(super) limit: usize,
    pub(super) opaque: &'a [Range<usize>],
}

impl Layout<'_> {
    pub(super) fn extract<'a>(
        self,
        rows: &mut [MeasuredRow<'a>],
        source: &str,
        width: Width,
    ) -> Option<Labels<'a>> {
        let mut found = false;
        for row in rows
            .iter_mut()
            .filter(|r| r.eligible && r.starts_physical_line && !r.folded)
        {
            if row.cells.iter().any(|c| c.text.contains('\n')) {
                continue;
            }
            let Some(cell) = row.cells.last_mut() else {
                continue;
            };
            let Cow::Borrowed(text) = cell.text else {
                continue;
            };
            if !text.contains(r"\label") {
                continue;
            }
            let raw = &source[cell.source.clone()];
            let offset = cell.source.start + raw.len()
                - raw.trim_start_matches([' ', '\t', '\r', '\n']).len();
            if let Some(start) = trailing_label(text, offset, self.opaque) {
                let formula = trim_cell_end(&text[..start]);
                if formula.is_empty() && row.delimiters.is_empty() {
                    continue;
                }
                row.equation_label = Some(&text[start..]);
                cell.text = formula.into();
                cell.width = measure_width(formula, width);
                found = true;
            }
        }
        found.then_some(Labels {
            lines: Vec::new(),
            limit: self.limit,
        })
    }
}

// Consume whole control sequences and groups so a label in an argument or an
// escaped backslash cannot be mistaken for the outer equation label.
fn trailing_label(text: &str, offset: usize, opaque: &[Range<usize>]) -> Option<usize> {
    let mut pos = 0;
    let mut spans = opaque[opaque.partition_point(|r| r.end <= offset)..]
        .iter()
        .peekable();
    while pos < text.len() {
        while spans.next_if(|r| r.end <= offset + pos).is_some() {}
        if let Some(span) = spans.next_if(|r| r.start <= offset + pos) {
            pos = span.end - offset;
            continue;
        }
        match text.as_bytes()[pos] {
            b'{' => pos = syntax::balanced_end(text, pos, b'{', b'}')?,
            b'\\' => {
                let (name, end) = syntax::command_at(text, pos)?;
                if name == "label" {
                    let key = crate::trivia::skip(text, end);
                    return (syntax::balanced_end(text, key, b'{', b'}')? == text.len())
                        .then_some(pos);
                }
                pos = end;
            }
            b'%' | b'}' => return None,
            _ => pos += text[pos..].chars().next()?.len_utf8(),
        }
    }
    None
}

struct Line<'a> {
    gap: Range<usize>,
    column: usize,
    label: Option<&'a str>,
    label_width: usize,
    align_break: bool,
    has_break: bool,
    break_width: usize,
}

pub(super) struct Labels<'a> {
    lines: Vec<Line<'a>>,
    limit: usize,
}

impl<'a> Labels<'a> {
    pub(super) fn record(
        &mut self,
        output: &str,
        end: usize,
        row: &MeasuredRow<'a>,
        align_break: bool,
        settings: &AlignmentConfig,
        tab_width: usize,
    ) {
        if !row.starts_physical_line || (row.equation_label.is_none() && !align_break) {
            return;
        }
        let line = output[..end].rsplit('\n').next().unwrap_or("");
        let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
        let column = super::trailing_indent_width(&line[..indent], tab_width)
            + measure_width(&line[indent..], settings.width);
        self.lines.push(Line {
            gap: end..output.len(),
            column,
            label: row.equation_label,
            label_width: row
                .equation_label
                .map_or(0, |s| measure_width(s, settings.width)),
            align_break,
            has_break: row.has_row_break,
            break_width: measure_width(&row.suffix[..row.terminator_length], settings.width),
        });
    }

    pub(super) fn write(self, output: &mut String, settings: &AlignmentConfig) {
        let label_column = self
            .lines
            .iter()
            .filter(|r| r.label.is_some() && r.column + 1 + r.label_width <= self.limit)
            .map(|r| r.column + 1)
            .max()
            .unwrap_or(0);
        let label_start = |r: &Line<'_>| {
            if label_column + r.label_width <= self.limit {
                label_column.max(r.column + 1)
            } else {
                r.column + 1
            }
        };
        let content_end = |r: &Line<'_>| {
            if r.label.is_some() {
                label_start(r) + r.label_width
            } else {
                r.column
            }
        };
        let natural_break = |r: &Line<'_>| content_end(r) + settings.spaces_before_row_break;
        let break_column = self
            .lines
            .iter()
            .filter(|r| r.align_break && natural_break(r) + r.break_width <= self.limit)
            .map(natural_break)
            .max()
            .unwrap_or(0);

        // Copy each piece of the rendered section once, including every
        // original terminator, modifier, comment and physical line ending.
        let Some(first) = self.lines.first() else {
            return;
        };
        let start = first.gap.start;
        let mut tail = String::with_capacity(output.len() - start);
        let mut pos = start;
        for line in &self.lines {
            tail.push_str(&output[pos..line.gap.start]);
            if let Some(label) = line.label {
                tail.extend(std::iter::repeat_n(' ', label_start(line) - line.column));
                tail.push_str(label);
            }
            if line.has_break {
                let natural = natural_break(line);
                let target = if line.align_break && break_column + line.break_width <= self.limit {
                    break_column.max(natural)
                } else {
                    natural
                };
                tail.extend(std::iter::repeat_n(' ', target - content_end(line)));
            }
            pos = line.gap.end;
        }
        tail.push_str(&output[pos..]);
        output.truncate(start);
        output.push_str(&tail);
    }
}

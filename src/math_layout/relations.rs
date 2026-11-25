//! Prefer boundaries between complete relations to breaks inside either relation.

use super::{Atom, BreakLayout, DelimiterRole, MathClass, flat_piece, greedy_breaks};
use std::{collections::BTreeSet, ops::Range};
use unicode_width::UnicodeWidthStr;

pub(super) fn is_sign(text: &str) -> bool {
    matches!(text, "+" | "-" | "−" | "\\pm" | "\\mp" | "±" | "∓")
}

struct Part<'a> {
    atom: usize,
    text: &'a str,
    class: MathClass,
    unary_sign: bool,
}

impl Part<'_> {
    fn operand(&self) -> bool {
        matches!(self.class, MathClass::Ord | MathClass::Inner)
    }

    fn separator(&self) -> bool {
        matches!(self.text, "," | ";" | "\\quad" | "\\qquad")
    }

    fn grouped_command(&self) -> bool {
        self.operand() && self.text.starts_with('\\') && self.text.ends_with('}')
    }

    fn starts_operand(&self) -> bool {
        self.operand() || self.class == MathClass::Op || self.unary_sign
    }
}

// Work on the outermost level. Each nested group, with all its punctuation
// and relations, occupies a single slot in the boundary scan.
fn top_level<'a>(source: &'a str, atoms: &[Atom]) -> Option<Vec<Part<'a>>> {
    let mut parts = Vec::new();
    let mut i = 0;
    while i < atoms.len() {
        let start = i;
        let atom = &atoms[i];
        let mut class = atom.class;
        i += 1;
        match atom.delimiter {
            Some(DelimiterRole::Open) => {
                let mut depth = 1;
                while depth > 0 {
                    let inner = atoms.get(i)?;
                    match inner.delimiter {
                        Some(DelimiterRole::Open) => depth += 1,
                        Some(DelimiterRole::Close) => depth -= 1,
                        _ => (),
                    }
                    i += 1;
                }
                class = MathClass::Inner;
            }
            Some(DelimiterRole::Close) => return None,
            None => (),
        }
        parts.push(Part {
            atom: start,
            text: &source[atom.range.start..atoms[i - 1].range.end],
            class,
            unary_sign: atom.unary && is_sign(&source[atom.range.clone()]),
        });
    }
    Some(parts)
}

fn gap<'a>(source: &'a str, atoms: &[Atom], before: usize) -> &'a str {
    &source[atoms[before - 1].range.end..atoms[before].range.start]
}

pub(super) fn mark_authored_signs(source: &str, atoms: &mut [Atom]) {
    let Some(mut parts) = top_level(source, atoms) else {
        return;
    };
    // Only promote a sign when its line starts a confirmed relation. Ordinary
    // subtraction may itself start a continuation line.
    for i in 1..parts.len().saturating_sub(1) {
        let at = parts[i].atom;
        if is_sign(parts[i].text)
            && gap(source, atoms, at).contains('\n')
            && gap(source, atoms, parts[i + 1].atom).is_empty()
            && parts[i + 1].starts_operand()
        {
            parts[i].unary_sign = true;
        }
    }
    let mut previous: Option<usize> = None;
    for (i, part) in parts.iter().enumerate() {
        if part.class != MathClass::Rel {
            continue;
        }
        if let Some(prev) = previous
            && prev > 0
            && parts[prev - 1].operand()
            && let Some(right) = parts.get(i + 1)
            && let Some(run) = boundary(source, atoms, &parts[prev + 1..i], right)
        {
            let sign = &mut atoms[run.end];
            if is_sign(&source[sign.range.clone()]) {
                sign.unary = true;
            }
        }
        previous = Some(i);
    }
}

// A separator spans zero or more parts. Its two edges are both available
// when preserving authored line endings around a connector on its own line.
fn boundary(
    source: &str,
    atoms: &[Atom],
    between: &[Part<'_>],
    right: &Part<'_>,
) -> Option<Range<usize>> {
    let first = between.first()?;
    let last = between.last()?;
    if !first.starts_operand() || !last.operand() || !right.starts_operand() {
        return None;
    }
    let mut found: Option<Range<usize>> = None;
    for (i, _) in between
        .iter()
        .enumerate()
        .filter(|(_, part)| part.separator())
    {
        if let Some(run) = &mut found {
            // A single command framed by two wide skips belongs with the
            // skips. Its name and argument text have no bearing on the split.
            if run.end != i
                && !(run.end + 1 == i
                    && between[run.end].grouped_command()
                    && matches!(between[run.end - 1].text, "\\quad" | "\\qquad")
                    && matches!(between[i].text, "\\quad" | "\\qquad"))
            {
                return None;
            }
            run.end = i + 1;
        } else {
            found = Some(i..i + 1);
        }
    }
    if let Some(run) = found {
        let prev = between.get(run.start.checked_sub(1)?)?;
        let next = between.get(run.end)?;
        if !prev.operand() || !next.starts_operand() {
            return None;
        }
        let start = between[run.start].atom;
        if run.len() == 1 {
            let marker = &between[run.start];
            let before = gap(source, atoms, start);
            let after = gap(source, atoms, next.atom);
            // Digit grouping and unit annotations can occur inside products.
            if marker.text == ","
                && before.is_empty()
                && after.is_empty()
                && prev.text.ends_with(|c: char| c.is_ascii_digit())
                && next.text.starts_with(|c: char| c.is_ascii_digit())
            {
                return None;
            }
        }
        return Some(start..next.atom);
    }
    let mut lines = between.windows(2).filter(|pair| {
        pair[0].operand()
            && pair[1].starts_operand()
            && gap(source, atoms, pair[1].atom).contains('\n')
    });
    let first = lines.next()?;
    let start = first[1].atom;
    match lines.next() {
        None => Some(start..start),
        Some(second)
            if first[1].grouped_command() && second[0].atom == start && lines.next().is_none() =>
        {
            Some(start..second[1].atom)
        }
        _ => None,
    }
}

pub(super) fn layout(
    source: &str,
    atoms: &[Atom],
    candidates: &BTreeSet<usize>,
    layout: &BreakLayout<'_>,
) -> Option<String> {
    if atoms
        .iter()
        .filter(|a| a.class == MathClass::Rel && candidates.contains(&a.range.start))
        .take(2)
        .count()
        < 2
        || atoms.iter().any(|a| a.barrier)
    {
        return None;
    }
    let parts = top_level(source, atoms)?;
    let relations = parts
        .iter()
        .enumerate()
        .filter(|(_, p)| {
            p.class == MathClass::Rel && candidates.contains(&atoms[p.atom].range.start)
        })
        .map(|(i, _)| i)
        .collect::<Vec<_>>();
    let mut cuts = Vec::new();
    for pair in relations.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let Some(right) = parts.get(b + 1) else { break };
        if a > 0
            && parts[a - 1].operand()
            && let Some(run) = boundary(source, atoms, &parts[a + 1..b], right)
        {
            let suffix = (run.start..run.end)
                .find(|&at| !matches!(&source[atoms[at].range.clone()], "," | ";"))
                .unwrap_or(run.end);
            let before = cuts.len();
            for at in run.start..=run.end {
                if gap(source, atoms, at).contains('\n') {
                    cuts.push((at, true, suffix.min(at)));
                }
            }
            if before == cuts.len() {
                cuts.push((run.end, false, suffix));
            }
        }
    }
    if cuts.is_empty() {
        return None;
    }
    let mut output = String::new();
    let mut width = layout.line_width;
    let mut start = 0;
    let mut forced = true;
    cuts.push((atoms.len(), false, atoms.len()));
    for (end, hard_break, suffix) in cuts {
        let clause = &atoms[start..end];
        let range = clause[0].range.start..clause.last()?.range.end;
        let flat = flat_piece(source, clause, &range)?;
        let size = UnicodeWidthStr::width(flat.as_str());
        let seam = if start == 0 {
            ""
        } else {
            gap(source, atoms, start)
        };
        let joined = width + UnicodeWidthStr::width(seam) + size;
        if !forced && joined <= layout.line_width {
            output.push_str(seam);
            output.push_str(&flat);
            width = joined;
        } else {
            if start > 0 {
                output.push_str(layout.ending);
            }
            width = layout.unit_width + size;
            let content = &atoms[start..suffix.max(start)];
            if width > layout.line_width
                && let Some(last) = content.last()
                && let Some(wrapped) = greedy_breaks(source, content, candidates, layout)
            {
                output.push_str(&wrapped);
                output.push_str(&source[last.range.end..range.end]);
                // Start the following complete clause on a fresh line.
                width = layout.line_width;
            } else {
                output.push_str(layout.prefix);
                output.push_str(layout.unit);
                output.push_str(&flat);
            }
        }
        start = end;
        forced = hard_break;
    }
    Some(output)
}

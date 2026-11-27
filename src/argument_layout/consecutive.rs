//! Align only the separators between complete, adjacent command calls.
use crate::{
    CompiledConfig, FormatError,
    parse_context::ParseContext,
    source_model::{RuleMeaning, SourceModel},
    syntax,
    wrap::display_width,
};
use std::ops::Range;

struct Row<'a> {
    name: &'a str,
    indent: &'a str,
    signature: String,
    slots: Vec<usize>,
    cells: Vec<&'a str>,
    gaps: Vec<Range<usize>>,
}

impl Row<'_> {
    fn matches(&self, other: &Self) -> bool {
        self.name == other.name
            && self.indent == other.indent
            && self.signature == other.signature
            && self.slots == other.slots
            && self.cells.len() == other.cells.len()
    }
}

pub(crate) fn format(
    source: &str,
    config: &CompiledConfig,
    parsing: &mut ParseContext<'_>,
) -> Result<String, FormatError> {
    collect_edits(source, config, parsing)?.apply_ordered(source)
}

pub(crate) fn collect_edits(
    source: &str,
    config: &CompiledConfig,
    parsing: &mut ParseContext<'_>,
) -> Result<crate::edit_plan::EditPlan<'static>, FormatError> {
    let parsed = parsing.snapshot(source, false, false)?;
    let analysis = parsed.analysis(&config.config);
    let model = analysis.model();
    let limit = config
        .config
        .arguments
        .line_width
        .unwrap_or(config.config.wrap.columns);
    let tab = config.config.indent.width.max(1);
    let mut group: Vec<Row<'_>> = Vec::new();
    let mut edits = crate::edit_plan::EditPlan::new();
    let mut offset = 0;
    for physical in source.split_inclusive('\n') {
        let line = physical.trim_end_matches(['\r', '\n']);
        let next = row(source, line, offset, model, config);
        if !group
            .last()
            .zip(next.as_ref())
            .is_some_and(|(last, next)| last.matches(next))
        {
            align(source, &group, limit, tab, &mut edits);
            group.clear();
        }
        if let Some(next) = next {
            group.push(next);
        }
        offset += physical.len();
    }
    align(source, &group, limit, tab, &mut edits);
    Ok(edits)
}

fn row<'a>(
    source: &str,
    line: &'a str,
    offset: usize,
    model: &SourceModel,
    config: &CompiledConfig,
) -> Option<Row<'a>> {
    let trimmed = line.trim_start_matches([' ', '\t']);
    let indent = &line[..line.len() - trimmed.len()];
    let (control, end) = syntax::command_at(line, indent.len())?;
    let name = control.trim_end_matches('*');
    let at = offset + indent.len();
    if syntax::is_protected(at, model.protected_ranges())
        || config.config.protect.commands.iter().any(|n| n == name)
        || config
            .config
            .syntax
            .commands
            .get(name)
            .is_some_and(|r| r.verbatim || r.preserve_body)
    {
        return None;
    }
    let rule = model.invocation_rule(at)?;
    let RuleMeaning::Arguments(arguments) = &rule.meaning else {
        return None;
    };
    let raw: String = arguments.slots().map(char::from).collect();
    // The internal compact grammar spells tight brackets O and delimited
    // options p or d. Translate them to xparse before reading spacing policy.
    let spec = if !syntax::xparse::extended(&raw) {
        raw.chars()
            .map(|c| match c {
                'o' => "o",
                'O' => "!o",
                'p' => "d()",
                'd' => "d<>",
                'm' => "m",
                'g' => "g",
                _ => "",
            })
            .collect::<String>()
    } else {
        raw.clone()
    };
    if raw.contains('l') && !syntax::xparse::extended(&raw) {
        return None;
    }
    let grammar = syntax::xparse::Signature::parse(&spec)?;
    let spacing = grammar.alignment_slots()?;
    let start = grammar.invocation_cursor(line, end);
    if control.ends_with('*') && start == end {
        return None;
    }
    let args = grammar.slot_ranges(line, start..line.len())?;
    let final_arg = args.last()?;
    let last = final_arg.range.end;
    // A trailing optional argument may start on the next physical line.
    if final_arg.slot + 1 < spacing.len()
        && grammar.end(source, offset + start..source.len())? != offset + last
    {
        return None;
    }
    let tail = line[last..].trim_start_matches([' ', '\t']);
    if !tail.is_empty() && !tail.starts_with('%') {
        return None;
    }
    // A domain reader can consume different boundaries from its shorthand
    // signature. Use only a matching interpretation of this actual call.
    if let Some(exact) = model.invocation_argument_ranges_at(at)
        && (exact.len() != args.len()
            || exact
                .iter()
                .zip(&args)
                .any(|(a, b)| a.range != (offset + b.range.start..offset + b.range.end)))
    {
        return None;
    }
    let mut from = 0;
    let mut prev = start;
    let mut cells = Vec::new();
    let mut gaps = Vec::new();
    let mut slots = Vec::new();
    for arg in args {
        let text = &line[arg.range.clone()];
        if !line[prev..arg.range.start]
            .bytes()
            .all(|b| matches!(b, b' ' | b'\t'))
        {
            return None;
        }
        if text == "*" && arg.slot == 0 && grammar.boolean_slots() & 1 != 0 {
            prev = arg.range.end;
            continue;
        }
        if arg.body == arg.range || text.starts_with('\\') {
            return None;
        }
        if !slots.is_empty() && spacing[arg.slot] {
            cells.push(&line[from..prev]);
            gaps.push(offset + prev..offset + arg.range.start);
            from = arg.range.start;
        }
        slots.push(arg.slot);
        prev = arg.range.end;
    }
    if gaps.is_empty() {
        return None;
    }
    cells.push(&line[from..]);
    Some(Row {
        name,
        indent,
        signature: spec,
        slots,
        cells,
        gaps,
    })
}

// Measure from the actual column so embedded tabs and wide glyphs align.
fn starts(rows: &[Row<'_>], limit: usize, tab: usize) -> Option<Vec<usize>> {
    let mut result = vec![0];
    let mut start = 0;
    for col in 0..rows.first()?.cells.len() {
        let end = rows
            .iter()
            .map(|r| start + display_width(r.cells[col], start, tab))
            .max()?;
        if end > limit {
            return None;
        }
        start = end + 1;
        result.push(start);
    }
    Some(result)
}

fn align(
    source: &str,
    mut rows: &[Row<'_>],
    limit: usize,
    tab: usize,
    edits: &mut crate::edit_plan::EditPlan<'_>,
) {
    while rows.len() >= 2 {
        if starts(&rows[..2], limit, tab).is_none() {
            rows = &rows[1..];
            continue;
        }
        // Find the maximal fitting prefix without remeasuring every prefix.
        let mut count = 2;
        while count < rows.len() {
            let probe = (count * 2).min(rows.len());
            if starts(&rows[..probe], limit, tab).is_some() {
                count = probe;
                continue;
            }
            let mut high = probe;
            while high - count > 1 {
                let mid = count + (high - count) / 2;
                if starts(&rows[..mid], limit, tab).is_some() {
                    count = mid;
                } else {
                    high = mid;
                }
            }
            break;
        }
        let (group, rest) = rows.split_at(count);
        let columns = starts(group, limit, tab).expect("fitting prefix");
        for row in group {
            for (col, gap) in row.gaps.iter().enumerate() {
                let end = columns[col] + display_width(row.cells[col], columns[col], tab);
                let spaces = columns[col + 1] - end;
                if gap.len() != spaces || source[gap.clone()].contains('\t') {
                    edits.replace_from(
                        crate::edit_plan::EditOrigin::ConsecutiveArguments,
                        gap.clone(),
                        " ".repeat(spaces),
                    );
                }
            }
        }
        rows = rest;
    }
}

//! Source alignment for the labelled items of each description environment.

use crate::parse_context::ParseContext;
use crate::syntax::{self, BlockKind, SharedBlock};
use crate::wrap::display_width;
use crate::{CompiledConfig, FormatError};
use std::{collections::BTreeMap, ops::Range};

struct Item<'a> {
    indent: &'a str,
    label_width: usize,
    gap: Range<usize>,
    body_end: usize,
}

fn item<'a>(source: &'a str, block: &SharedBlock, tab_width: usize) -> Option<Item<'a>> {
    let marker = block.begin.start;
    let line = source[..marker].rsplit('\n').next()?;
    if !line.trim_matches([' ', '\t']).is_empty() {
        return None;
    }
    let tail = source.get(marker..block.body.end)?.strip_prefix(r"\item")?;
    let label = tail.trim_start_matches([' ', '\t']);
    let finish = syntax::balanced_end(label, 0, b'[', b']')?;
    let end = block.body.end - label.len() + finish;
    let marker_text = &source[marker..end];
    if marker_text.contains(['\n', '%']) {
        return None;
    }
    let rest = &source[end..block.body.end];
    let body = rest.trim_start_matches([' ', '\t', '\r', '\n']);
    let gap = end..block.body.end - body.len();
    if body.is_empty()
        || body.starts_with('%')
        || source[gap.clone()].bytes().filter(|b| *b == b'\n').count() > 1
    {
        return None;
    }
    Some(Item {
        indent: line,
        label_width: display_width(marker_text, 0, tab_width),
        gap,
        body_end: block.body.end,
    })
}

pub(crate) fn format(
    source: &str,
    config: &CompiledConfig,
    parsing: &mut ParseContext<'_>,
) -> Result<String, FormatError> {
    if !source.contains("description") || !source.contains(r"\item") {
        return Ok(source.into());
    }
    let parsed = parsing.snapshot(source, false, true)?;
    let blocks = parsed.blocks.as_deref().expect("description list blocks");
    let tab_width = config.config.indent.width;
    let mut lists = BTreeMap::<usize, Vec<Item<'_>>>::new();
    let mut opaque = parsed.protected.clone();
    for block in blocks {
        if matches!(
            block.kind,
            BlockKind::Group
                | BlockKind::NamedGroup
                | BlockKind::MandatoryArgument
                | BlockKind::OptionalArgument
                | BlockKind::Special
        ) && source[block.begin.start..block.end.end].contains('\n')
        {
            opaque.push(block.begin.start..block.end.end);
        }
        if block.kind == BlockKind::Item
            && block.name == "item"
            && let Some(parent) = block.parent
            && blocks[parent].kind == BlockKind::Environment
            && blocks[parent].name == "description"
            && let Some(item) = item(source, block, tab_width)
        {
            lists.entry(parent).or_default().push(item);
        }
    }
    let opaque = crate::code_layout::merged(opaque);
    let newline = if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let unit = config.config.indent.unit("item", "item");
    let mut edits = Vec::new();
    for items in lists.values() {
        let fits = |item: &Item<'_>| {
            item.label_width
                < config.config.wrap.columns.saturating_sub(display_width(
                    item.indent,
                    0,
                    tab_width,
                )) / 2
        };
        let common = items
            .iter()
            .filter(|item| fits(item))
            .map(|item| item.label_width + 1)
            .max();
        for item in items {
            let hanging = match common {
                Some(width) => format!("{}{}", item.indent, " ".repeat(width)),
                None => format!("{}{}", item.indent, unit),
            };
            let gap = if fits(item) && !source[item.gap.clone()].contains('\n') {
                " ".repeat(common.unwrap() - item.label_width)
            } else {
                format!("{newline}{hanging}")
            };
            if gap != source[item.gap.clone()] {
                edits.push((item.gap.clone(), gap));
            }
            let mut cursor = item.gap.end;
            for (n, line) in source[cursor..item.body_end]
                .split_inclusive('\n')
                .enumerate()
            {
                let start = cursor;
                cursor += line.len();
                let text = line.trim_start_matches([' ', '\t']);
                let pos = cursor - text.len();
                if text.trim().is_empty()
                    || text.starts_with('%')
                    || text.starts_with(r"\begin")
                    || (n > 0
                        && !crate::wrap::allows_prose_start(text, &config.text)
                            .map_err(FormatError::Syntax)?)
                {
                    break;
                }
                if n > 0 && !syntax::is_protected(start, &opaque) && line[..pos - start] != hanging
                {
                    edits.push((start..pos, hanging.clone()));
                }
            }
        }
    }
    crate::layout::apply_edits(source, edits)
}

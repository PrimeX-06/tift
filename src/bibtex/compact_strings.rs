use super::{skip_whitespace, value_token_end};
use crate::{CompiledConfig, FormatError, wrap::display_width};
use std::ops::Range;

pub(super) struct Candidate {
    entry: Range<usize>,
    left: String,
    right: String,
    indent: usize,
    left_width: usize,
    width: usize,
}

impl Candidate {
    pub(super) fn parse(
        source: &str,
        entry: Range<usize>,
        opening: usize,
        base: &str,
        config: &CompiledConfig,
    ) -> Option<Self> {
        let text = &source[entry.clone()];
        if text.contains('%') {
            return None;
        }
        let end = text.len() - 1;
        let (key, _) = text[opening..end].split_once('=')?;
        let start = skip_whitespace(text, opening + key.len() + 1, end);
        if !matches!(text.as_bytes().get(start), Some(b'{' | b'"')) {
            return None;
        }
        let finish = value_token_end(text, start, end).ok()?;
        let value = &text[start..finish];
        if value.contains(['\r', '\n']) {
            return None;
        }
        let comma = text[finish..end].trim();
        if !matches!(comma, "" | ",") {
            return None;
        }
        let left = format!("{}{}", &text[..opening], key.trim());
        let right = format!("= {value}{comma}{}", &text[end..]);
        let tab = config.config.indent.width.max(1);
        let indent = display_width(base, 0, tab);
        let left_width = display_width(&left, indent, tab);
        let offset = indent + left_width + 1;
        let width = offset + display_width(&right, offset, tab);
        (width <= config.config.wrap.columns && !right.contains('\t')).then_some(Self {
            entry,
            left,
            right,
            indent,
            left_width,
            width,
        })
    }
}

pub(super) fn render(
    source: String,
    entries: &[Candidate],
    limit: usize,
) -> Result<String, FormatError> {
    if entries.is_empty() {
        return Ok(source);
    }
    let mut edits = Vec::new();
    for group in entries.chunk_by(|a, b| {
        let gap = &source[a.entry.end..b.entry.start];
        a.indent == b.indent
            && gap.bytes().filter(|&c| c == b'\n').count() == 1
            && gap.trim().is_empty()
    }) {
        let widest = group.iter().map(|e| e.left_width).max().unwrap_or(0);
        for entry in group {
            let pad = widest - entry.left_width;
            if entry.width + pad <= limit {
                edits.push((
                    entry.entry.clone(),
                    format!("{}{}{}", entry.left, " ".repeat(pad + 1), entry.right),
                ));
            }
        }
    }
    crate::layout::apply_edits(&source, edits)
}

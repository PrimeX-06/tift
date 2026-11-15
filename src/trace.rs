//! Detailed, opt-in records of actual edits made by each formatting stage.
//!
//! Line and byte ranges address the input and output of the named stage. The
//! context and rule fields describe the enclosing structure and applicable
//! configuration; they do not claim a token-by-token execution history.

use std::fmt;
use std::ops::Range;

use similar::TextDiff;

use crate::CompiledConfig;
use crate::syntax::{self, Block, BlockKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceAction {
    Structure,
    Indentation,
    Alignment,
    Arguments,
    Text,
    MacroBody,
    Replacement,
    Whitespace,
    FileBoundary,
    Bibtex,
    Math,
    Selection,
    Other,
}

impl fmt::Display for TraceAction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Structure => "structural line breaks",
            Self::Indentation => "indentation",
            Self::Alignment => "column alignment",
            Self::Arguments => "argument layout",
            Self::Text => "text layout",
            Self::MacroBody => "macro body indentation",
            Self::Replacement => "replacement",
            Self::Whitespace => "whitespace cleanup",
            Self::FileBoundary => "file boundaries",
            Self::Bibtex => "BibTeX layout",
            Self::Math => "math layout",
            Self::Selection => "selected line edits",
            Self::Other => "source edit",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetailTrace {
    pub stage: &'static str,
    pub action: TraceAction,
    /// One-based, end-exclusive physical line range in this stage's input.
    /// An insertion has an empty range at the insertion point.
    pub input_lines: Range<usize>,
    /// One-based, end-exclusive physical line range in this stage's output.
    pub output_lines: Range<usize>,
    pub input_bytes: Range<usize>,
    pub output_bytes: Range<usize>,
    /// Exact removed/replaced text, including original line endings.
    pub before: String,
    /// Exact inserted/replacement text, including new line endings.
    pub after: String,
    pub context: Option<String>,
    /// Configuration associated with this stage and enclosing structure.
    pub rule: Option<String>,
}

fn action(stage: &str) -> TraceAction {
    match stage {
        "structure" => TraceAction::Structure,
        "indentation" => TraceAction::Indentation,
        "alignment" => TraceAction::Alignment,
        "arguments" => TraceAction::Arguments,
        "text-before" | "text-after" | "docstrip-text" => TraceAction::Text,
        "math" | "math-final" => TraceAction::Math,
        "macro-bodies" => TraceAction::MacroBody,
        "replacements-before" | "replacements-after" => TraceAction::Replacement,
        "whitespace-before" | "whitespace-after" => TraceAction::Whitespace,
        "file-boundaries" => TraceAction::FileBoundary,
        "bibtex" => TraceAction::Bibtex,
        "selection" => TraceAction::Selection,
        _ => TraceAction::Other,
    }
}

fn line_offsets(text: &str) -> Vec<usize> {
    let mut offsets = vec![0];
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        offset += line.len();
        offsets.push(offset);
    }
    offsets
}

fn structure<'a>(blocks: &'a [Block], range: &Range<usize>) -> Option<&'a Block> {
    blocks
        .iter()
        .filter(|block| {
            !matches!(
                block.kind,
                BlockKind::MandatoryArgument | BlockKind::OptionalArgument
            ) && block.begin.start <= range.start
                && range.start < block.end.end
        })
        .min_by_key(|block| block.end.end - block.begin.start)
}

fn context(block: &Block) -> String {
    let kind = match block.kind {
        BlockKind::Loop => "loop",
        BlockKind::Environment => "environment",
        BlockKind::Verbatim => "verbatim",
        BlockKind::Command | BlockKind::Heading => "command",
        BlockKind::Condition => "condition",
        BlockKind::Item => "item",
        BlockKind::Special => "special",
        BlockKind::NamedGroup => "named group",
        BlockKind::KeyValue => "key/value",
        _ => "group",
    };
    if block.name.is_empty() {
        kind.into()
    } else {
        format!("{kind} {}", block.name)
    }
}

fn rule(stage: &str, block: Option<&Block>, config: &CompiledConfig) -> Option<String> {
    let name = block.map(|block| block.name.trim_end_matches('*'));
    match stage {
        "math" | "math-final" => Some("math.enabled / wrap.columns".into()),
        "structure" => Some(name.map_or_else(
            || "breaks".into(),
            |name| {
                if config.config.breaks.rules.contains_key(name) {
                    format!("breaks.rules.{name}")
                } else {
                    "breaks defaults/categories".into()
                }
            },
        )),
        "indentation" => Some(name.map_or_else(
            || "indent".into(),
            |name| {
                if config.config.indent.rules.contains_key(name) {
                    format!("indent.rules.{name}")
                } else {
                    "indent defaults/categories".into()
                }
            },
        )),
        "alignment" => Some(name.map_or_else(
            || "alignment".into(),
            |name| {
                if config.config.alignment.rules.contains_key(name) {
                    format!("alignment.rules.{name}")
                } else {
                    "alignment defaults".into()
                }
            },
        )),
        "arguments" => Some(name.map_or_else(
            || "arguments".into(),
            |name| {
                if config.config.arguments.rules.contains_key(name) {
                    format!("arguments.rules.{name}")
                } else {
                    "arguments defaults".into()
                }
            },
        )),
        "text-before" | "text-after" => Some(
            if config.config.sentences.enabled {
                "wrap/sentences"
            } else {
                "wrap"
            }
            .into(),
        ),
        "macro-bodies" => Some("protect.format-macro-bodies".into()),
        "replacements-before" | "replacements-after" => Some("replacements".into()),
        "whitespace-before" | "whitespace-after" => Some("whitespace".into()),
        "file-boundaries" => Some("whitespace.line-ending/final-newline".into()),
        "bibtex" => Some("indent/whitespace".into()),
        _ => None,
    }
}

/// Describe each contiguous changed region in the named stage.
///
/// The caller opts into the cost of computing a diff and structure context by
/// invoking this function. Equal text returns immediately without parsing.
/// Failed context analysis never changes formatting success or diagnostics.
pub fn stage_changes(
    stage: &'static str,
    before: &str,
    after: &str,
    config: &CompiledConfig,
) -> Vec<DetailTrace> {
    if before == after {
        return Vec::new();
    }
    let old_offsets = line_offsets(before);
    let new_offsets = line_offsets(after);
    let blocks = if matches!(
        action(stage),
        TraceAction::Structure
            | TraceAction::Indentation
            | TraceAction::Alignment
            | TraceAction::Arguments
            | TraceAction::MacroBody
    ) {
        syntax::blocks(before, config).map_or_else(|_| Vec::new(), |(_, blocks)| blocks)
    } else {
        Vec::new()
    };
    let diff = TextDiff::from_lines(before, after);
    let mut result = Vec::new();
    for group in diff.grouped_ops(0) {
        let Some(first) = group.first() else {
            continue;
        };
        let last = group.last().expect("nonempty diff group");
        let old_lines = first.old_range().start..last.old_range().end;
        let new_lines = first.new_range().start..last.new_range().end;
        let input_bytes = old_offsets[old_lines.start]..old_offsets[old_lines.end];
        let output_bytes = new_offsets[new_lines.start]..new_offsets[new_lines.end];
        let block = structure(&blocks, &input_bytes);
        result.push(DetailTrace {
            stage,
            action: action(stage),
            input_lines: old_lines.start + 1..old_lines.end + 1,
            output_lines: new_lines.start + 1..new_lines.end + 1,
            before: before[input_bytes.clone()].into(),
            after: after[output_bytes.clone()].into(),
            input_bytes,
            output_bytes,
            context: block.map(context),
            rule: rule(stage, block, config),
        });
    }
    result
}

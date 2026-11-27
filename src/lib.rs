#![forbid(unsafe_code)]

pub mod alignment;
pub mod argument_layout;
pub mod bibtex;
mod block_storage;
mod bst;
pub mod code_layout;
pub mod config;
mod description;
mod docstrip;
mod edit_plan;
mod foreach;
pub mod layout;
mod lexical_index;
pub mod literal_expansion;
pub mod math_layout;
#[path = "packages/package_support_registry.rs"]
mod packages;
mod parse_context;
mod pgfplots_table;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod playground;
pub mod ranges;
pub mod replacements;
mod semantic_index;
mod source_analysis;
mod source_document;
pub mod source_model;
pub mod syntax;
mod tikz;
mod tikz_scan;
pub mod trace;
pub mod tree_layout;
mod trivia;
pub mod wrap;

pub(crate) use packages::{
    classes as class_environments, code as code_package_syntax, code_environments,
    commands as package_commands, documents as document_environments,
    environments as package_environments, equation_floats as additional_package_syntax,
    lua as luadraw_syntax, table_rules, tables as package_tables,
};

pub use config::{CompiledConfig, Config, ConfigError, FileKind};
pub use ranges::{LineRangeError, LineRanges, format_ranges};
use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub message: String,
    pub span: Range<usize>,
}

#[derive(Debug, Clone)]
pub struct FormatOutput {
    pub text: String,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, thiserror::Error)]
pub enum FormatError {
    #[error("{0}")]
    Syntax(String),
    #[error("{closing} at byte {at}; {opening} at byte {opener}", at = .closer.start)]
    StructuralMismatch {
        opening: String,
        closing: String,
        opener: usize,
        closer: Range<usize>,
    },
    #[error("replacement failed: {0}")]
    Replacement(String),
}

impl FormatError {
    pub(crate) fn byte_offset(&self) -> Option<usize> {
        if let Self::StructuralMismatch { closer, .. } = self {
            return Some(closer.start);
        }
        let Self::Syntax(message) = self else {
            return None;
        };
        let (_, rest) = message.split_once(" at byte ")?;
        rest[..rest.bytes().take_while(u8::is_ascii_digit).count()]
            .parse()
            .ok()
    }

    /// Syntax errors can include both the failing token and the opener.
    pub(crate) fn map_byte_offsets(self, offset: impl Fn(usize) -> Option<usize>) -> Self {
        if let Self::StructuralMismatch {
            opening,
            closing,
            opener,
            closer,
        } = self
        {
            return Self::StructuralMismatch {
                opening,
                closing,
                opener: offset(opener).unwrap_or(opener),
                closer: offset(closer.start).unwrap_or(closer.start)
                    ..offset(closer.end).unwrap_or(closer.end),
            };
        }
        let Self::Syntax(message) = self else {
            return self;
        };
        let mut parts = message.split(" at byte ");
        let mut mapped = String::from(parts.next().unwrap_or_default());
        for part in parts {
            mapped.push_str(" at byte ");
            let count = part.bytes().take_while(u8::is_ascii_digit).count();
            if let Some(at) = part[..count].parse().ok().and_then(&offset) {
                mapped.push_str(&at.to_string());
                mapped.push_str(&part[count..]);
            } else {
                mapped.push_str(part);
            }
        }
        Self::Syntax(mapped)
    }
}

pub fn compile_config(config: Config) -> Result<CompiledConfig, ConfigError> {
    CompiledConfig::new(config)
}

pub fn format(
    source: &str,
    kind: FileKind,
    config: &CompiledConfig,
) -> Result<FormatOutput, FormatError> {
    layout::format_document(source, kind, config)
}

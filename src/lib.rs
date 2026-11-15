#![forbid(unsafe_code)]

pub mod alignment;
pub mod argument_layout;
pub mod bibtex;
mod block_storage;
pub mod config;
mod foreach;
pub mod layout;
pub mod literal_expansion;
mod packages;
mod parse_context;
pub mod ranges;
pub mod replacements;
mod source_analysis;
pub mod syntax;
mod tikz;
mod tikz_scan;
pub mod trace;
mod trivia;
pub mod wrap;

pub(crate) use packages::{
    additional as additional_package_syntax, classes as class_environments,
    code as code_package_syntax, code_environments, commands as package_commands,
    documents as document_environments, environments as package_environments,
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
    #[error("replacement failed: {0}")]
    Replacement(String),
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

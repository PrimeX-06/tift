use crate::{CompiledConfig, Config, ConfigError, FileKind, Severity, compile_config, format};
use std::sync::LazyLock;
use wasm_bindgen::prelude::*;

static CONFIG: LazyLock<Result<CompiledConfig, ConfigError>> =
    LazyLock::new(|| compile_config(Config::default()));

#[wasm_bindgen(getter_with_clone)]
pub struct PlaygroundOutput {
    pub text: String,
    pub diagnostic: Option<String>,
    pub has_error: bool,
}

#[wasm_bindgen]
pub fn format_snippet(source: &str, bibtex: bool) -> Result<PlaygroundOutput, JsError> {
    let kind = if bibtex {
        FileKind::Bibtex
    } else {
        FileKind::Tex
    };
    let config = CONFIG
        .as_ref()
        .map_err(|error| JsError::new(&error.to_string()))?;
    let output = format(source, kind, config).map_err(|error| JsError::new(&error.to_string()))?;
    let error = output
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.severity == Severity::Error);
    let diagnostic = error
        .or_else(|| output.diagnostics.first())
        .map(|diagnostic| diagnostic.message.clone());
    Ok(PlaygroundOutput {
        text: output.text,
        diagnostic,
        has_error: error.is_some(),
    })
}

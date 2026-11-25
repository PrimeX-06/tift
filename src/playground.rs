use crate::{
    CompiledConfig, Config, FileKind, Severity, compile_config, config::IndentStyle, format,
};
use std::cell::RefCell;
use wasm_bindgen::prelude::*;

thread_local! {
    static CONFIG: RefCell<Option<CompiledConfig>> = const { RefCell::new(None) };
}

#[wasm_bindgen(getter_with_clone)]
pub struct PlaygroundOutput {
    pub text: String,
    pub diagnostic: Option<String>,
    pub has_error: bool,
}

#[wasm_bindgen]
pub fn format_snippet(
    source: &str,
    bibtex: bool,
    indent_width: Option<usize>,
    use_tabs: bool,
) -> Result<PlaygroundOutput, JsError> {
    let kind = if bibtex {
        FileKind::Bibtex
    } else {
        FileKind::Tex
    };
    let width = indent_width.unwrap_or(4);
    let style = if use_tabs {
        IndentStyle::Tabs
    } else {
        IndentStyle::Spaces
    };
    let output = CONFIG.with_borrow_mut(|cached| {
        let config = match cached {
            Some(config)
                if config.config.indent.width == width && config.config.indent.style == style =>
            {
                config
            }
            _ => {
                let mut settings = Config::default();
                settings.indent.width = width;
                settings.indent.style = style;
                cached.insert(
                    compile_config(settings).map_err(|error| JsError::new(&error.to_string()))?,
                )
            }
        };
        format(source, kind, config).map_err(|error| JsError::new(&error.to_string()))
    })?;
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

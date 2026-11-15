//! Documented TeX source and installation-script layout.

use badness_formatter::ast::{AstNode, Command, Environment};
use badness_formatter::parser::parse_with_flavor;
use badness_formatter::syntax::SyntaxNode;

use std::ops::Range;

use crate::{CompiledConfig, FileKind, FormatError, code_layout};

pub(crate) fn format_docstrip(
    source: &str,
    kind: FileKind,
    config: &CompiledConfig,
) -> Result<String, FormatError> {
    let lex = code_layout::lex_config(kind);
    let parsed = parse_with_flavor(source, lex);
    let root = parsed.syntax();
    let mut ranges = code_layout::protected_ranges(source, config, config.config.code.expl3)?;
    ranges.extend(code_layout::math_ranges(&root));
    if !config.config.code.expl3 {
        ranges.extend(
            badness_formatter::formatter::core::expl3_regions(&root)
                .into_iter()
                .map(|range| usize::from(range.start())..usize::from(range.end())),
        );
    }
    if !config.config.code.algorithm2e {
        for environment in root.descendants().filter_map(Environment::cast) {
            if environment.name().is_some_and(|name| {
                matches!(
                    name.as_str(),
                    "algorithm"
                        | "algorithm*"
                        | "algorithm2e"
                        | "algorithm2e*"
                        | "algocf"
                        | "algocf*"
                )
            }) {
                let range = environment.syntax().text_range();
                ranges.push(usize::from(range.start())..usize::from(range.end()));
            }
        }
    }
    if kind == FileKind::Ins {
        // Installation preambles are copied into generated files as literal data.
        ranges.extend(preamble_ranges(&root, source.len()));
    }
    let opaque = code_layout::Opaque::new(source, ranges);
    let formatted = badness_formatter::formatter::format_with_style_flavored(
        opaque.source(),
        code_layout::style(config),
        lex,
    )
    .map_err(|error| FormatError::Syntax(format!("docstrip syntax: {error}")))?;
    let mut output = opaque.restore(&formatted).ok_or_else(|| {
        FormatError::Syntax("docstrip layout could not retain an opaque source region".into())
    })?;
    if !source.ends_with('\n') {
        output = output.trim_end_matches(['\r', '\n']).to_owned();
    }
    Ok(output)
}

pub(crate) fn preamble_ranges(root: &SyntaxNode, source_len: usize) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut open = None;
    for command in root.descendants().filter_map(Command::cast) {
        match command.name().as_deref() {
            Some(name @ ("preamble" | "postamble")) if open.is_none() => {
                open = Some((
                    usize::from(command.syntax().text_range().start()),
                    name.to_owned(),
                ))
            }
            Some(name @ ("endpreamble" | "endpostamble")) => {
                if open
                    .as_ref()
                    .is_some_and(|(_, begin)| name.strip_prefix("end") == Some(begin.as_str()))
                {
                    let (start, _) = open.take().expect("matching installation preamble");
                    ranges.push(start..usize::from(command.syntax().text_range().end()));
                }
            }
            _ => {}
        }
    }
    if let Some((start, _)) = open {
        ranges.push(start..source_len);
    }
    ranges
}

pub(crate) fn documentation_is_active(config: &CompiledConfig) -> bool {
    config.config.wrap.mode != "preserve" || config.config.sentences.enabled
}

pub(crate) fn format_documentation(
    source: &str,
    config: &CompiledConfig,
) -> Result<String, FormatError> {
    if !documentation_is_active(config) {
        return Ok(source.to_owned());
    }
    let lines: Vec<_> = source.split_inclusive('\n').collect();
    let protected = code_layout::protected_ranges(source, config, false)?;
    let mut prose_config = config.config.clone();
    prose_config.wrap.columns = prose_config.wrap.columns.saturating_sub(2).max(1);
    // Only the display-column budget differs; validated patterns remain shared.
    let prose_config = CompiledConfig {
        config: prose_config,
        replacements: config.replacements.clone(),
        alignment: config.alignment.clone(),
        text: config.text.clone(),
        syntax: config.syntax.clone(),
    };
    let mut offsets = Vec::with_capacity(lines.len());
    let mut offset = 0;
    for line in &lines {
        offsets.push(offset);
        offset += line.len();
    }
    let mut output = String::with_capacity(source.len());
    let mut position = 0;
    let eligible = |index: usize| {
        let line = lines[index];
        let start = offsets[index];
        if protected
            .iter()
            .any(|range| range.start < start + line.len() && start < range.end)
        {
            return false;
        }
        let Some(body) = line.strip_prefix('%') else {
            return false;
        };
        !body.starts_with(['%', '<'])
            && !body.trim().starts_with("\\begin{macrocode")
            && !body.trim().starts_with("\\end{macrocode")
    };
    while position < lines.len() {
        if !eligible(position) {
            output.push_str(lines[position]);
            position += 1;
            continue;
        }
        let mut virtual_source = String::new();
        while position < lines.len() && eligible(position) {
            let body = &lines[position][1..];
            virtual_source.push_str(body.strip_prefix(' ').unwrap_or(body));
            position += 1;
        }
        let mut formatted =
            crate::wrap::format_text(&virtual_source, &prose_config, crate::wrap::Stage::Before)?;
        formatted = crate::wrap::format_text(&formatted, &prose_config, crate::wrap::Stage::After)?;
        for line in formatted.split_inclusive('\n') {
            output.push('%');
            if !line.trim_matches(['\r', '\n']).is_empty() {
                output.push(' ');
            }
            output.push_str(line);
        }
    }
    Ok(output)
}

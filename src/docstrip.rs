//! Documented TeX source and installation-script layout.

use std::ops::Range;

use crate::{CompiledConfig, FileKind, FormatError, FormatOutput, code_layout};

pub(crate) fn format_docstrip(
    source: &str,
    kind: FileKind,
    config: &CompiledConfig,
) -> Result<FormatOutput, FormatError> {
    let mut protection = config.config.clone();
    if config.config.code.expl3 {
        protection.protect.expl3 = false;
    }
    let (mut ranges, diagnostics) = crate::syntax::protected_ranges_with_diagnostics(
        source,
        &protection,
        Some(&config.syntax),
    )?;
    let class = (kind == FileKind::Dtx)
        .then(|| {
            crate::source_model::SourceModel::with_ranges(source, &config.config, &ranges)
                .document_class()
                .map(str::to_owned)
        })
        .flatten();
    if kind == FileKind::Ins {
        ranges.extend(preamble_ranges(source, &ranges));
    }
    let opaque = code_layout::Opaque::new(source, ranges);
    let mut settings = config.config.clone();
    settings.code.algorithm2e = false;
    settings.code.expl3 = false;
    settings.math.enabled = false;
    settings.wrap.mode = "preserve".into();
    settings.sentences.enabled = false;
    settings.protect.expl3 = true;
    settings.replacements.clear();
    settings
        .syntax
        .commands
        .entry("generate".into())
        .or_default()
        .arguments = Some(1);
    let structural =
        CompiledConfig::new(settings).map_err(|error| FormatError::Syntax(error.to_string()))?;
    let input = opaque.source();
    let formatted = if kind == FileKind::Dtx {
        let documentation = documentation_config(class.as_deref(), &structural)?;
        let mut code = config.config.clone();
        code.replacements.clear();
        let code =
            CompiledConfig::new(code).map_err(|error| FormatError::Syntax(error.to_string()))?;
        format_dtx_structure(input, documentation.as_ref().unwrap_or(&structural), &code)?
    } else {
        crate::format(input, FileKind::Style, &structural)?.text
    };
    let restored = opaque.restore(&formatted).ok_or_else(|| {
        FormatError::Syntax("docstrip layout could not retain an opaque source region".into())
    })?;
    let mut output = if kind == FileKind::Dtx {
        restored
    } else {
        code_layout::format_code(&restored, kind, config)?.text
    };
    if !source.ends_with('\n') {
        output = output.trim_end_matches(['\r', '\n']).to_owned();
    }
    Ok(FormatOutput {
        text: output,
        diagnostics,
    })
}

pub(crate) fn documentation_config(
    class: Option<&str>,
    config: &CompiledConfig,
) -> Result<Option<CompiledConfig>, FormatError> {
    let Some(delimiter) = crate::packages::documents::documentation_short_verbatim(class) else {
        return Ok(None);
    };
    if config
        .config
        .syntax
        .special
        .iter()
        .any(|rule| rule.begin == delimiter)
    {
        return Ok(None);
    }
    let mut settings = config.config.clone();
    settings.syntax.special.push(crate::config::SpecialRule {
        name: "ltxdocShortVerb".into(),
        begin: delimiter.into(),
        end: delimiter.into(),
        verbatim: true,
        ..Default::default()
    });
    CompiledConfig::new(settings)
        .map(Some)
        .map_err(|error| FormatError::Syntax(error.to_string()))
}

pub(crate) fn macrocode_boundary(line: &str) -> Option<bool> {
    let body = line
        .trim_start_matches([' ', '\t'])
        .strip_prefix('%')?
        .trim_start();
    if body.starts_with("\\begin{macrocode}") || body.starts_with("\\begin{macrocode*}") {
        Some(true)
    } else if body.starts_with("\\end{macrocode}") || body.starts_with("\\end{macrocode*}") {
        Some(false)
    } else {
        None
    }
}

fn format_dtx_structure(
    source: &str,
    documentation_config: &CompiledConfig,
    code_config: &CompiledConfig,
) -> Result<String, FormatError> {
    let mut output = String::new();
    let mut code = String::new();
    let mut documentation = String::new();
    let flush_code = |output: &mut String, code: &mut String| -> Result<(), FormatError> {
        if code.is_empty() {
            return Ok(());
        }
        match crate::format(code, FileKind::Style, code_config) {
            Ok(formatted) => output.push_str(&formatted.text),
            Err(FormatError::Syntax(_)) => output.push_str(code),
            Err(error) => return Err(error),
        }
        code.clear();
        Ok(())
    };
    let flush_documentation =
        |output: &mut String, documentation: &mut String| -> Result<(), FormatError> {
            if documentation.is_empty() {
                return Ok(());
            }
            let stripped: String = documentation
                .split_inclusive('\n')
                .map(|line| {
                    let body = &line[1..];
                    body.strip_prefix(' ').unwrap_or(body)
                })
                .collect();
            match crate::format(&stripped, FileKind::Tex, documentation_config) {
                Ok(formatted)
                    if !formatted
                        .diagnostics
                        .iter()
                        .any(|d| d.severity == crate::Severity::Error) =>
                {
                    for line in formatted.text.split_inclusive('\n') {
                        output.push('%');
                        if !line.trim_matches(['\r', '\n']).is_empty() {
                            output.push(' ');
                        }
                        output.push_str(line);
                    }
                }
                Ok(_) | Err(FormatError::Syntax(_)) => output.push_str(documentation),
                Err(error) => return Err(error),
            }
            documentation.clear();
            Ok(())
        };
    let mut macrocode = false;
    for line in source.split_inclusive('\n') {
        if let Some(open) = macrocode_boundary(line) {
            flush_documentation(&mut output, &mut documentation)?;
            flush_code(&mut output, &mut code)?;
            output.push_str(line);
            macrocode = open;
            continue;
        }
        if macrocode {
            if line.starts_with("%<") {
                flush_code(&mut output, &mut code)?;
                output.push_str(line);
            } else {
                code.push_str(line);
            }
            continue;
        }
        if let Some(body) = line.strip_prefix('%') {
            flush_code(&mut output, &mut code)?;
            if body.starts_with(['%', '<']) {
                flush_documentation(&mut output, &mut documentation)?;
                output.push_str(line);
            } else {
                documentation.push_str(line);
            }
        } else {
            flush_documentation(&mut output, &mut documentation)?;
            code.push_str(line);
        }
    }
    flush_code(&mut output, &mut code)?;
    flush_documentation(&mut output, &mut documentation)?;
    Ok(output)
}

pub(crate) fn preamble_ranges(source: &str, protected: &[Range<usize>]) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut open = None;
    let mut cursor = 0;
    while cursor < source.len() {
        if crate::syntax::is_protected(cursor, protected) {
            cursor = protected
                .iter()
                .find(|range| range.contains(&cursor))
                .expect("protected range")
                .end;
        } else if source.as_bytes()[cursor] == b'%' {
            cursor = source[cursor..]
                .find('\n')
                .map_or(source.len(), |offset| cursor + offset + 1);
        } else if let Some((name, end)) = crate::syntax::command_at(source, cursor) {
            match name {
                "preamble" | "postamble" if open.is_none() => open = Some((cursor, name)),
                "endpreamble" | "endpostamble"
                    if open
                        .as_ref()
                        .is_some_and(|(_, begin)| name.strip_prefix("end") == Some(*begin)) =>
                {
                    let (start, _) = open.take().expect("matching installation preamble");
                    ranges.push(start..end);
                }
                _ => (),
            }
            cursor = end;
        } else {
            cursor += source[cursor..]
                .chars()
                .next()
                .expect("source character")
                .len_utf8();
        }
    }
    if let Some((start, _)) = open {
        ranges.push(start..source.len());
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
    let merged = code_layout::merged(code_layout::protected_ranges(source, config, false)?);
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
    let mut line_state = Vec::with_capacity(lines.len());
    let mut offset = 0;
    let mut macrocode = false;
    for line in &lines {
        let boundary = macrocode_boundary(line);
        line_state.push((offset, !macrocode && boundary.is_none()));
        if let Some(open) = boundary {
            macrocode = open;
        }
        offset += line.len();
    }
    let mut output = String::with_capacity(source.len());
    let mut position = 0;
    let eligible = |index: usize| {
        let line = lines[index];
        let (start, documentation) = line_state[index];
        if !documentation {
            return false;
        }
        let next = merged.partition_point(|range| range.end <= start);
        if merged
            .get(next)
            .is_some_and(|range| range.start < start + line.len())
        {
            return false;
        }
        let Some(body) = line.strip_prefix('%') else {
            return false;
        };
        !body.starts_with(['%', '<'])
    };
    while position < lines.len() {
        if !eligible(position) {
            output.push_str(lines[position]);
            position += 1;
            continue;
        }
        let first_line = position;
        let mut virtual_source = String::new();
        while position < lines.len() && eligible(position) {
            let body = &lines[position][1..];
            virtual_source.push_str(body.strip_prefix(' ').unwrap_or(body));
            position += 1;
        }
        // Keep the complete structural indentation established by the
        // docstrip printer. A nested table is not a standalone prose root.
        if virtual_source.contains("\\begin{") || virtual_source.contains("\\end{") {
            for line in &lines[first_line..position] {
                output.push_str(line);
            }
            continue;
        }
        let formatted =
            crate::wrap::format_text(&virtual_source, &prose_config, crate::wrap::Stage::Before)
                .and_then(|text| {
                    crate::wrap::format_text(&text, &prose_config, crate::wrap::Stage::After)
                });
        let formatted = match formatted {
            Ok(text) => text,
            // Documentation environments can cross intervening macrocode or
            // guard lines. A partial excerpt keeps its original margins.
            Err(FormatError::Syntax(_)) => {
                for line in &lines[first_line..position] {
                    output.push_str(line);
                }
                continue;
            }
            Err(error) => return Err(error),
        };
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

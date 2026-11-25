use crate::config::{BreakMode, FinalNewline, LineEnding};
use crate::parse_context::ParseContext;
use crate::source_analysis::SourceAnalysis;
use crate::syntax::{self, BlockKind, SharedBlock as Block};
use crate::{CompiledConfig, Diagnostic, FileKind, FormatError, FormatOutput, Severity};
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::ops::Range;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct StageTrace {
    pub name: &'static str,
    pub changed: bool,
    pub input_bytes: usize,
    pub output_bytes: usize,
    pub elapsed: Duration,
}

fn stage(
    name: &'static str,
    source: &str,
    trace: &mut Option<&mut Vec<StageTrace>>,
    details: &mut Option<&mut Vec<crate::trace::DetailTrace>>,
    config: &CompiledConfig,
    operation: impl FnOnce() -> Result<String, FormatError>,
) -> Result<String, FormatError> {
    let started = trace.is_some().then(Instant::now);
    let text = operation()?;
    let elapsed = started.map(|started| started.elapsed());
    if let Some(details) = details.as_deref_mut() {
        details.extend(crate::trace::stage_changes(name, source, &text, config));
    }
    if let Some(trace) = trace.as_deref_mut() {
        trace.push(StageTrace {
            name,
            changed: text != source,
            input_bytes: source.len(),
            output_bytes: text.len(),
            elapsed: elapsed.expect("tracing timer"),
        });
    }
    Ok(text)
}

fn optional_stage(
    name: &'static str,
    text: &mut String,
    trace: &mut Option<&mut Vec<StageTrace>>,
    details: &mut Option<&mut Vec<crate::trace::DetailTrace>>,
    config: &CompiledConfig,
    enabled: bool,
    operation: impl FnOnce(&str) -> Result<String, FormatError>,
) -> Result<bool, FormatError> {
    if !enabled {
        if let Some(trace) = trace.as_deref_mut() {
            trace.push(StageTrace {
                name,
                changed: false,
                input_bytes: text.len(),
                output_bytes: text.len(),
                elapsed: Duration::ZERO,
            });
        }
        return Ok(false);
    }
    let processed = stage(name, text, trace, details, config, || operation(text))?;
    let changed = processed != *text;
    *text = processed;
    Ok(changed)
}

pub fn format_document(
    source: &str,
    kind: FileKind,
    config: &CompiledConfig,
) -> Result<FormatOutput, FormatError> {
    format_document_inner(source, kind, config, None, None)
}

pub fn format_document_traced(
    source: &str,
    kind: FileKind,
    config: &CompiledConfig,
) -> Result<(FormatOutput, Vec<StageTrace>), FormatError> {
    let mut trace = Vec::new();
    let output = format_document_inner(source, kind, config, Some(&mut trace), None)?;
    Ok((output, trace))
}

/// Return stage timings and detailed records of the edits made by each stage.
pub fn format_document_detailed(
    source: &str,
    kind: FileKind,
    config: &CompiledConfig,
) -> Result<
    (
        FormatOutput,
        Vec<StageTrace>,
        Vec<crate::trace::DetailTrace>,
    ),
    FormatError,
> {
    let mut trace = Vec::new();
    let mut details = Vec::new();
    let output = format_document_inner(source, kind, config, Some(&mut trace), Some(&mut details))?;
    Ok((output, trace, details))
}

fn format_document_inner(
    source: &str,
    kind: FileKind,
    config: &CompiledConfig,
    mut trace: Option<&mut Vec<StageTrace>>,
    mut details: Option<&mut Vec<crate::trace::DetailTrace>>,
) -> Result<FormatOutput, FormatError> {
    if kind == FileKind::Tex
        && !config.config.replacement_only
        && source.contains("\\documentclass")
    {
        for (at, marker) in source.match_indices("\\end{document}") {
            let end = at + marker.len();
            if source[end..].trim().is_empty() {
                continue;
            }
            let prefix = crate::source_model::SourceModel::analyze(&source[..end], config);
            let closes_document = prefix.structure.as_ref().is_some_and(|(_, blocks)| {
                blocks.iter().any(|block| {
                    block.kind == BlockKind::Environment
                        && block.name == "document"
                        && block.parent.is_none()
                        && block.end.end == end
                })
            });
            if prefix.document_class().is_none() || prefix.has_errors() || !closes_document {
                continue;
            }
            let tail = if source[end..].starts_with("\r\n") {
                end + 2
            } else if source[end..].starts_with('\n') {
                end + 1
            } else {
                end
            };
            let mut output = format_document_inner(&source[..tail], kind, config, trace, details)?;
            if output
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.severity == Severity::Error)
            {
                return Ok(output);
            }
            output.text.push_str(&source[tail..]);
            output.diagnostics.push(Diagnostic {
                severity: Severity::Warning,
                message: "Nonexecuted source after \\end{document} retained unchanged".into(),
                span: tail..source.len(),
            });
            return Ok(output);
        }
    }
    match format_document_stages(
        source,
        kind,
        config,
        trace.as_deref_mut(),
        details.as_deref_mut(),
    ) {
        Err(FormatError::Syntax(message)) => {
            // A later stage can expose syntax hidden by the original opaque
            // regions. Its offsets belong to an intermediate snapshot, so the
            // recovery diagnostic covers the original file as a whole.
            if let Some(trace) = trace {
                trace.clear();
            }
            if let Some(details) = details {
                details.clear();
            }
            Ok(FormatOutput {
                text: source.into(),
                diagnostics: vec![Diagnostic {
                    severity: Severity::Error,
                    message: format!(
                        "Source retained because a formatting stage could not safely analyze an intermediate snapshot: {message}"
                    ),
                    span: 0..source.len(),
                }],
            })
        }
        result => result,
    }
}

fn format_document_stages(
    source: &str,
    kind: FileKind,
    config: &CompiledConfig,
    mut trace: Option<&mut Vec<StageTrace>>,
    mut details: Option<&mut Vec<crate::trace::DetailTrace>>,
) -> Result<FormatOutput, FormatError> {
    let original_source = source;
    let model = if !config.config.replacement_only
        && matches!(
            kind,
            FileKind::Tex | FileKind::Tikz | FileKind::Style | FileKind::Class
        ) {
        Some(crate::source_model::SourceModel::analyze(source, config))
    } else {
        None
    };
    if let Some(model) = &model
        && model.has_errors()
    {
        return Ok(FormatOutput {
            text: model.reconstruct(),
            diagnostics: model.diagnostics().to_vec(),
        });
    }
    let bom = source.starts_with('\u{feff}');
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let mut diagnostics = model
        .as_ref()
        .map_or_else(Vec::new, |model| model.diagnostics().to_vec());
    let mut parsing = ParseContext::with_model(config, model);
    let embedded;
    let source = if config.config.bibtex.format_filecontents
        && !config.config.replacement_only
        && matches!(
            kind,
            FileKind::Tex
                | FileKind::Tikz
                | FileKind::Class
                | FileKind::Style
                | FileKind::Dtx
                | FileKind::Ins
        )
        && source.contains("filecontents")
    {
        embedded = stage(
            "embedded-bibtex",
            source,
            &mut trace,
            &mut details,
            config,
            || {
                crate::bibtex::format_embedded(
                    source,
                    config,
                    &mut parsing,
                    usize::from(bom) * 3,
                    &mut diagnostics,
                )
            },
        )?;
        if diagnostics.iter().any(|d| d.severity == Severity::Error) {
            return Ok(FormatOutput {
                text: original_source.into(),
                diagnostics,
            });
        }
        embedded.as_str()
    } else {
        source
    };
    let mut text = stage(
        "replacements-before",
        source,
        &mut trace,
        &mut details,
        config,
        || {
            crate::replacements::apply_for_kind(
                source,
                kind,
                config,
                crate::replacements::When::Before,
            )
        },
    )?;
    if !config.config.replacement_only {
        if matches!(kind, FileKind::Bibtex | FileKind::Bst) {
            let name = if kind == FileKind::Bst {
                "bst"
            } else {
                "bibtex"
            };
            text = stage(name, &text, &mut trace, &mut details, config, || {
                let output = if kind == FileKind::Bst {
                    crate::bst::format_style(&text, config)?
                } else {
                    crate::bibtex::format_bibtex_detailed(&text, config)?
                };
                diagnostics.extend(output.diagnostics.into_iter().map(|mut diagnostic| {
                    if bom {
                        diagnostic.span.start += 3;
                        diagnostic.span.end += 3;
                    }
                    diagnostic
                }));
                Ok(output.text)
            })?;
            if diagnostics
                .iter()
                .any(|diagnostic| diagnostic.severity == Severity::Error)
            {
                return Ok(FormatOutput {
                    text: original_source.into(),
                    diagnostics,
                });
            }
        } else if matches!(kind, FileKind::Dtx | FileKind::Ins) {
            text = stage("docstrip", &text, &mut trace, &mut details, config, || {
                let output = crate::docstrip::format_docstrip(&text, kind, config)?;
                diagnostics.extend(output.diagnostics);
                Ok(output.text)
            })?;
            let math_active = crate::math_layout::is_active(&text, config);
            optional_stage(
                "math",
                &mut text,
                &mut trace,
                &mut details,
                config,
                math_active,
                |text| {
                    crate::math_layout::format_math_with_context(text, kind, config, &mut parsing)
                },
            )?;
            optional_stage(
                "docstrip-text",
                &mut text,
                &mut trace,
                &mut details,
                config,
                kind == FileKind::Dtx && crate::docstrip::documentation_is_active(config),
                |text| crate::docstrip::format_documentation(text, config),
            )?;
        } else {
            if config.config.whitespace.before_processing {
                text = stage(
                    "whitespace-before",
                    &text,
                    &mut trace,
                    &mut details,
                    config,
                    || clean_whitespace(&text, config, &mut parsing),
                )?;
            }
            if config.config.breaks.enabled {
                text = stage("structure", &text, &mut trace, &mut details, config, || {
                    apply_breaks(&text, config, &mut parsing)
                })?;
            }
            text = stage(
                "indentation",
                &text,
                &mut trace,
                &mut details,
                config,
                || indent(&text, kind, config, &mut parsing),
            )?;
            let macro_active = syntax::macro_layout_active(&text, config);
            let macro_changed = optional_stage(
                "macro-bodies",
                &mut text,
                &mut trace,
                &mut details,
                config,
                macro_active,
                |text| syntax::format_macro_definitions_with_context(text, config, &mut parsing),
            )?;
            // Formula spacing contributes to the width of text arguments and
            // prose, so normalize math before either text stage measures it.
            let math_active = crate::math_layout::is_active(&text, config);
            let mut needs_cleanup = optional_stage(
                "math",
                &mut text,
                &mut trace,
                &mut details,
                config,
                math_active,
                |text| {
                    crate::math_layout::format_math_with_context(text, kind, config, &mut parsing)
                },
            )?;
            let arguments_active = crate::argument_layout::is_active(&text, config);
            needs_cleanup |= optional_stage(
                "arguments",
                &mut text,
                &mut trace,
                &mut details,
                config,
                arguments_active,
                |text| {
                    crate::argument_layout::format_arguments_with_context(
                        text,
                        config,
                        kind,
                        &mut parsing,
                    )
                },
            )?;
            let collapse_prose = kind == FileKind::Tex
                && config.config.wrap.mode == "preserve"
                && config.config.wrap.collapse_spaces
                && text.contains("  ");
            optional_stage(
                "prose-spaces",
                &mut text,
                &mut trace,
                &mut details,
                config,
                collapse_prose,
                |text| crate::wrap::collapse_prose_spaces(text, config, &mut parsing),
            )?;
            // Resolve structure first so wrapping always measures final indentation.
            optional_stage(
                "description",
                &mut text,
                &mut trace,
                &mut details,
                config,
                config.config.wrap.align_description,
                |text| crate::description::format(text, config, &mut parsing),
            )?;
            needs_cleanup |= optional_stage(
                "text-before",
                &mut text,
                &mut trace,
                &mut details,
                config,
                crate::wrap::is_active(config, crate::wrap::Stage::Before),
                |text| {
                    crate::wrap::format_text_with_context(
                        text,
                        config,
                        crate::wrap::Stage::Before,
                        &mut parsing,
                    )
                },
            )?;
            let processed = stage("alignment", &text, &mut trace, &mut details, config, || {
                crate::alignment::format_alignment_after_indent(&text, config, &mut parsing)
            })?;
            needs_cleanup |= processed != text;
            text = processed;
            needs_cleanup |= optional_stage(
                "text-after",
                &mut text,
                &mut trace,
                &mut details,
                config,
                crate::wrap::is_active(config, crate::wrap::Stage::After),
                |text| {
                    crate::wrap::format_text_with_context(
                        text,
                        config,
                        crate::wrap::Stage::After,
                        &mut parsing,
                    )
                },
            )?;
            if needs_cleanup {
                text = stage(
                    "whitespace-after",
                    &text,
                    &mut trace,
                    &mut details,
                    config,
                    || clean_whitespace(&text, config, &mut parsing),
                )?;
            }
            let code_activity = crate::code_layout::activity(&text, config);
            let code_active = code_activity.active();
            let mut math_recheck = false;
            let code_changed = optional_stage(
                "code",
                &mut text,
                &mut trace,
                &mut details,
                config,
                code_active,
                |text| {
                    let output = crate::code_layout::format_code_with_context(
                        text,
                        kind,
                        config,
                        &mut parsing,
                        code_activity,
                    )?;
                    math_recheck = output.math_recheck;
                    Ok(output.text)
                },
            )?;
            // Code preserves inline math islands exactly. A changed display
            // prefix or an uncovered mathematical argument needs rechecking.
            let math_active =
                code_changed && math_recheck && crate::math_layout::is_active(&text, config);
            optional_stage(
                "math-final",
                &mut text,
                &mut trace,
                &mut details,
                config,
                math_active,
                |text| {
                    crate::math_layout::format_math_with_context(text, kind, config, &mut parsing)
                },
            )?;
            optional_stage(
                "macro-bodies-final",
                &mut text,
                &mut trace,
                &mut details,
                config,
                code_changed || needs_cleanup,
                |text| syntax::format_macro_definitions_with_context(text, config, &mut parsing),
            )?;
            optional_stage(
                "whitespace-final",
                &mut text,
                &mut trace,
                &mut details,
                config,
                code_active || macro_changed,
                |text| clean_whitespace(text, config, &mut parsing),
            )?;
            optional_stage(
                "column-specs",
                &mut text,
                &mut trace,
                &mut details,
                config,
                config.config.alignment.column_spec != crate::alignment::ColumnSpecLayout::Preserve
                    || config
                        .config
                        .alignment
                        .rules
                        .values()
                        .any(|rule| rule.column_spec.is_some()),
                |text| crate::alignment::format_preambles(text, config, &mut parsing),
            )?;
            let tree_layout = config.config.trees.mode != crate::tree_layout::TreeMode::Preserve
                || config.config.trees.dirtree && text.contains(r"\dirtree");
            optional_stage(
                "trees",
                &mut text,
                &mut trace,
                &mut details,
                config,
                tree_layout,
                |text| crate::tree_layout::format_with_context(text, config, &mut parsing),
            )?;
            let plot_tables = config.config.breaks.enabled
                && (config.config.breaks.pgfplots_tables && text.contains(r"\addplot")
                    || config.config.breaks.pgfplots_named_tables
                        && text.contains(r"\pgfplotstableread"));
            optional_stage(
                "pgfplots-tables",
                &mut text,
                &mut trace,
                &mut details,
                config,
                plot_tables,
                |text| crate::pgfplots_table::format(text, config, &mut parsing),
            )?;
            optional_stage(
                "foreach-lists",
                &mut text,
                &mut trace,
                &mut details,
                config,
                config.config.arguments.foreach_lists
                    != crate::argument_layout::ForeachListMode::Preserve,
                |text| crate::foreach::format_lists(text, config, &mut parsing),
            )?;
            optional_stage(
                "consecutive-arguments",
                &mut text,
                &mut trace,
                &mut details,
                config,
                config.config.arguments.align_consecutive,
                |text| crate::argument_layout::consecutive::format(text, config, &mut parsing),
            )?;
            optional_stage(
                "trailing-comments",
                &mut text,
                &mut trace,
                &mut details,
                config,
                config.config.comments.align_trailing,
                |text| crate::wrap::align_trailing_comments(text, config, &mut parsing),
            )?;
        }
    }
    optional_stage(
        "replacements-after",
        &mut text,
        &mut trace,
        &mut details,
        config,
        crate::replacements::is_active(config, crate::replacements::When::After),
        |text| {
            crate::replacements::apply_for_kind(
                text,
                kind,
                config,
                crate::replacements::When::After,
            )
        },
    )?;
    let final_started = trace.is_some().then(Instant::now);
    let before_final = trace.is_some().then(|| text.clone());
    let ending = preferred_ending(source, config);
    let mut boundary_protection = |text: &str| -> Result<Vec<Range<usize>>, FormatError> {
        if kind == FileKind::Bst {
            Ok(crate::bst::protected_ranges(text, false))
        } else if kind == FileKind::Bibtex {
            crate::bibtex::protected_ranges(text)
        } else {
            Ok(parsing.snapshot(text, false, false)?.protected.clone())
        }
    };
    if config.config.output.line_ending != LineEnding::Preserve {
        text = convert_endings(&text, ending, &boundary_protection(&text)?);
    }
    match config.config.output.final_newline {
        FinalNewline::Always
            if !text.is_empty()
                && !text.ends_with('\n')
                && !syntax::is_protected(text.len() - 1, &boundary_protection(&text)?) =>
        {
            text.push_str(ending)
        }
        FinalNewline::Never if text.ends_with('\n') => {
            let protected = boundary_protection(&text)?;
            while text.ends_with('\n') && !syntax::is_protected(text.len() - 1, &protected) {
                text.pop();
                if text.ends_with('\r') {
                    text.pop();
                }
            }
        }
        _ => (),
    }
    if bom {
        text.insert(0, '\u{feff}');
    }
    if let Some(trace) = trace {
        let original = before_final.expect("final trace input");
        if let Some(details) = details {
            details.extend(crate::trace::stage_changes(
                "file-boundaries",
                &original,
                &text,
                config,
            ));
        }
        trace.push(StageTrace {
            name: "file-boundaries",
            changed: original != text,
            input_bytes: original.len(),
            output_bytes: text.len(),
            elapsed: final_started.expect("final trace timer").elapsed(),
        });
    }
    Ok(FormatOutput { text, diagnostics })
}

fn preferred_ending<'a>(source: &str, config: &CompiledConfig) -> &'a str {
    match config.config.output.line_ending {
        LineEnding::Lf => "\n",
        LineEnding::Crlf => "\r\n",
        LineEnding::Preserve => {
            if source.contains("\r\n") {
                "\r\n"
            } else {
                "\n"
            }
        }
    }
}

fn convert_endings(source: &str, ending: &str, protected: &[Range<usize>]) -> String {
    let mut output = String::with_capacity(source.len());
    let mut offset = 0;
    for line in source.split_inclusive('\n') {
        if line.ends_with('\n') && !syntax::is_protected(offset + line.len() - 1, protected) {
            output.push_str(
                line.strip_suffix("\r\n")
                    .unwrap_or_else(|| line.strip_suffix('\n').expect("line ending")),
            );
            output.push_str(ending);
        } else {
            output.push_str(line);
        }
        offset += line.len();
    }
    output
}

pub(crate) fn clean_whitespace(
    source: &str,
    config: &CompiledConfig,
    parsing: &mut ParseContext<'_>,
) -> Result<String, FormatError> {
    // Most layout stages leave trailing whitespace and blank-line runs clean.
    // Check the bytes before rebuilding semantic protection for a new snapshot.
    // Protected text can trigger this conservative check, but cannot hide work.
    if !whitespace_cleanup_needed(source, config) {
        return Ok(source.into());
    }
    clean_whitespace_parsed(source, config, parsing)
}

fn whitespace_cleanup_needed(source: &str, config: &CompiledConfig) -> bool {
    let options = &config.config.whitespace;
    let maximum = options.blank_lines.limit();
    if !options.remove_trailing && maximum.is_none() {
        return false;
    }
    let mut blanks = 0usize;
    for line in source.split_inclusive('\n') {
        let body = line.strip_suffix('\n').unwrap_or(line);
        let body = body.strip_suffix('\r').unwrap_or(body);
        if options.remove_trailing && body.ends_with([' ', '\t']) {
            return true;
        }
        if maximum.is_some() && body.trim_matches([' ', '\t']).is_empty() {
            blanks += 1;
            if maximum.is_some_and(|maximum| blanks > maximum) {
                return true;
            }
        } else {
            blanks = 0;
        }
    }
    false
}

fn clean_whitespace_parsed(
    source: &str,
    config: &CompiledConfig,
    parsing: &mut ParseContext<'_>,
) -> Result<String, FormatError> {
    let protected = &parsing.snapshot(source, false, false)?.protected;
    let mut output = String::with_capacity(source.len());
    let mut offset = 0usize;
    let mut blanks = 0usize;
    for line in source.split_inclusive('\n') {
        let ending = if line.ends_with("\r\n") {
            "\r\n"
        } else if line.ends_with('\n') {
            "\n"
        } else {
            ""
        };
        let body = &line[..line.len() - ending.len()];
        let protected_line = syntax::overlaps(&(offset..offset + line.len()), protected);
        if body.trim_matches([' ', '\t']).is_empty() && !protected_line {
            blanks += 1;
            let maximum = config.config.whitespace.blank_lines.limit();
            if maximum.is_some_and(|maximum| blanks > maximum) {
                offset += line.len();
                continue;
            }
        } else {
            blanks = 0;
        }
        let trimmed = trailing_content_end(body);
        if config.config.whitespace.remove_trailing
            && !syntax::overlaps(&(offset + trimmed..offset + body.len()), protected)
        {
            output.push_str(&body[..trimmed]);
        } else {
            output.push_str(body);
        }
        output.push_str(ending);
        offset += line.len();
    }
    Ok(output)
}

// The first whitespace character in a trailing control-space token is source
// content. TeX consumes the remaining ordinary spaces without affecting it.
fn trailing_content_end(body: &str) -> usize {
    let trimmed = body.trim_end_matches([' ', '\t']).len();
    if trimmed < body.len() && syntax::is_escaped(body, trimmed) {
        trimmed + 1
    } else {
        trimmed
    }
}

#[derive(Debug)]
struct Event<'a> {
    pos: usize,
    block: usize,
    start: bool,
    unit: &'a str,
}

struct IndentMarker {
    command: &'static str,
    environment: &'static str,
    signature: &'static [u8],
    unit_category: &'static str,
}

fn marker_indent_events<'a>(
    source: &str,
    blocks: &[Block],
    marker: &IndentMarker,
    unit: &'a str,
    events: &mut Vec<Event<'a>>,
) {
    let mut active = BTreeMap::new();
    for (i, block) in blocks.iter().enumerate() {
        if block.name != marker.command
            || !(block.kind == BlockKind::Command
                || marker.command == "column" && block.kind == BlockKind::Environment)
        {
            continue;
        }
        let Some(parent) = block.parent.filter(|&p| {
            blocks[p].kind == BlockKind::Environment && blocks[p].name == marker.environment
        }) else {
            continue;
        };
        let limit = blocks[parent].body.end;
        let mut after = block.begin.end;
        if marker.command == "onslide" {
            after += usize::from(source.as_bytes().get(after) == Some(&b'+'));
            let Some(spec) = environment_argument_range(source, after, limit, b'd') else {
                continue;
            };
            after = spec.end;
            // A braced onslide, even on the following line, owns only its argument.
            if source
                .as_bytes()
                .get(crate::trivia::skip_until(source, after, limit))
                == Some(&b'{')
            {
                continue;
            }
        }
        if let Some(previous) = active.remove(&parent) {
            events.push(Event {
                pos: block.begin.start,
                block: previous,
                start: false,
                unit: "",
            });
        }
        if block.kind != BlockKind::Command
            || !source[..block.begin.start]
                .rsplit('\n')
                .next()
                .unwrap_or("")
                .trim_matches([' ', '\t'])
                .is_empty()
        {
            continue;
        }
        // A Beamer column header is recognized on one physical line. Plot
        // options and bibliography labels may themselves span several lines.
        let header_limit = if marker.command == "column" {
            source[block.begin.end..limit]
                .find('\n')
                .map_or(limit, |n| block.begin.end + n)
        } else {
            limit
        };
        let args = signature_argument_ranges(
            source,
            after..header_limit,
            marker.signature.iter().copied(),
        );
        if marker.signature.ends_with(b"g")
            && args
                .last()
                .is_none_or(|a| source.as_bytes()[a.start] != b'{')
        {
            continue;
        }
        let header_end = args.last().map_or(after, |a| a.end);
        let Some(end) = source[header_end..limit].find('\n').map(|n| header_end + n) else {
            continue;
        };
        let rest = source[header_end..end].trim_start_matches([' ', '\t', '\r']);
        if rest.is_empty() || rest.starts_with('%') {
            events.push(Event {
                pos: end,
                block: i,
                start: true,
                unit,
            });
            active.insert(parent, i);
        }
    }
    for (parent, last) in active {
        events.push(Event {
            pos: blocks[parent].body.end,
            block: last,
            start: false,
            unit: "",
        });
    }
}

fn indentation_unit<'a>(
    block: &Block,
    config: &'a CompiledConfig,
    spaces: &'a str,
) -> Result<&'a str, FormatError> {
    if block.name == "foreach-body"
        && matches!(block.kind, BlockKind::Group | BlockKind::MandatoryArgument)
    {
        return Ok("");
    }
    if block.kind == BlockKind::Heading
        && !config
            .config
            .syntax
            .headings
            .get(block.name.trim_end_matches('*'))
            .is_some_and(|h| h.indent)
    {
        return Ok("");
    }
    if block.kind == BlockKind::Environment
        && let Some(pattern) = &config.syntax.environment
        && !pattern
            .is_match(&block.name)
            .map_err(|e| FormatError::Syntax(e.to_string()))?
    {
        return Ok("");
    }
    Ok(config
        .config
        .indent
        .unit_with_spaces(block.kind.category(), &block.name, spaces))
}

pub(crate) fn indent_after_arguments(
    source: &str,
    config: &CompiledConfig,
    kind: FileKind,
    parsing: &mut ParseContext<'_>,
    arguments: &[Range<usize>],
) -> Result<String, FormatError> {
    indent_with_argument_ranges(source, kind, config, parsing, arguments)
}

fn indent(
    source: &str,
    kind: FileKind,
    config: &CompiledConfig,
    parsing: &mut ParseContext<'_>,
) -> Result<String, FormatError> {
    indent_with_argument_ranges(source, kind, config, parsing, &[])
}

fn indent_with_argument_ranges(
    source: &str,
    kind: FileKind,
    config: &CompiledConfig,
    parsing: &mut ParseContext<'_>,
    arguments: &[Range<usize>],
) -> Result<String, FormatError> {
    let parsed = parsing.snapshot(source, true, true)?;
    let protected = &parsed.protected;
    let argument_literals = if arguments.is_empty() {
        Vec::new()
    } else {
        let analysis = SourceAnalysis::with_memo(
            source,
            &config.config,
            protected,
            std::rc::Rc::clone(&parsed.metadata),
        );
        crate::argument_layout::literal_argument_ranges(
            source,
            config,
            protected,
            analysis.declared_commands()?.as_ref(),
        )
    };
    let original_blocks = parsed.blocks.as_ref().expect("parsed structure");
    let mut projected_blocks = None;
    let mut configuration_ranges = Vec::new();
    for (owner, block) in original_blocks.iter().enumerate() {
        if block.kind != BlockKind::Command {
            continue;
        }
        let Some(range) = protected
            .get(protected.partition_point(|range| range.end <= block.begin.end))
            .filter(|range| range.start == block.begin.end && range.end == block.end.end)
        else {
            continue;
        };
        let Some(local) = crate::packages::indexing::configuration_layout(
            &source[block.begin.start..range.end],
            &block.name,
            config,
        ) else {
            continue;
        };
        let blocks = projected_blocks.get_or_insert_with(|| original_blocks.clone());
        let base = blocks.len();
        for mut child in local.into_iter().skip(1) {
            child.begin =
                child.begin.start + block.begin.start..child.begin.end + block.begin.start;
            child.body = child.body.start + block.begin.start..child.body.end + block.begin.start;
            child.end = child.end.start + block.begin.start..child.end.end + block.begin.start;
            child.parent = child.parent.map(|parent| {
                if parent == 0 {
                    owner
                } else {
                    base + parent - 1
                }
            });
            for (_, branch) in &mut child.branches {
                *branch = branch.start + block.begin.start..branch.end + block.begin.start;
            }
            blocks.push(child);
        }
        configuration_ranges.push(range.clone());
    }
    let blocks = projected_blocks.as_ref().unwrap_or(original_blocks);
    let mut events = Vec::new();
    // Validated indentation widths are bounded by 256. Events borrow slices
    // of one buffer or the configured literal instead of allocating per block.
    let spaces = " ".repeat(256);
    let mut branches = BTreeMap::new();
    let newlines: Vec<usize> = memchr::memchr_iter(b'\n', source.as_bytes()).collect();
    let indent = &config.config.indent;
    let algorithm_lines = if indent.algpseudocode && source.contains("algorithmic") {
        let analysis = SourceAnalysis::with_memo(
            source,
            &config.config,
            protected,
            std::rc::Rc::clone(&parsed.metadata),
        );
        crate::packages::code_algorithms::indent_lines(source, config, protected, &analysis)?
    } else {
        BTreeMap::new()
    };
    let algorithm_unit = indent.unit_with_spaces("environment", "algorithmic", &spaces);
    let markers = [
        (
            indent.beamer_columns && source.contains(r"\column"),
            IndentMarker {
                command: "column",
                environment: "columns",
                signature: b"dog",
                unit_category: "environment",
            },
        ),
        (
            indent.pgfplots_panels && source.contains(r"\nextgroupplot"),
            IndentMarker {
                command: "nextgroupplot",
                environment: "groupplot",
                signature: b"o",
                unit_category: "environment",
            },
        ),
        (
            indent.bibliography_items && source.contains(r"\bibitem"),
            IndentMarker {
                command: "bibitem",
                environment: "thebibliography",
                signature: b"og",
                unit_category: "item",
            },
        ),
        (
            indent.beamer_overprint && source.contains(r"\onslide"),
            IndentMarker {
                command: "onslide",
                environment: "overprint",
                signature: b"",
                unit_category: "environment",
            },
        ),
    ];
    let mut markers = markers
        .iter()
        .filter(|(enabled, marker)| {
            *enabled
                && !config.config.syntax.commands.contains_key(marker.command)
                && !config
                    .config
                    .syntax
                    .environment_arguments
                    .contains_key(marker.environment)
        })
        .peekable();
    if markers.peek().is_some() {
        let analysis = SourceAnalysis::with_memo(
            source,
            &config.config,
            protected,
            std::rc::Rc::clone(&parsed.metadata),
        );
        let commands = analysis.declared_commands()?;
        let environments = analysis.declared_environments()?;
        for (_, marker) in markers {
            if !commands.contains(marker.command) && !environments.contains(marker.environment) {
                marker_indent_events(
                    source,
                    blocks,
                    marker,
                    indent.unit_with_spaces(marker.unit_category, marker.command, &spaces),
                    &mut events,
                );
            }
        }
    }
    if source.contains(r"\CodeAfter") {
        let analysis = SourceAnalysis::with_memo(
            source,
            &config.config,
            protected,
            std::rc::Rc::clone(&parsed.metadata),
        );
        for (marker, parent) in
            crate::package_tables::code_after_markers(blocks, &config.config, &analysis)?
        {
            let block = &blocks[marker];
            let environment = &blocks[parent];
            let after =
                environment_argument_range(source, block.begin.end, environment.body.end, b'o')
                    .map_or(block.begin.end, |range| range.end);
            events.push(Event {
                pos: after,
                block: marker,
                start: true,
                unit: indent.unit_with_spaces("environment", &environment.name, &spaces),
            });
            events.push(Event {
                pos: environment.body.end,
                block: marker,
                start: false,
                unit: "",
            });
        }
    }
    for (i, block) in blocks.iter().enumerate() {
        if i < original_blocks.len()
            && configuration_ranges
                .get(configuration_ranges.partition_point(|range| range.end <= block.begin.start))
                .is_some_and(|range| range.start <= block.begin.start && block.end.end <= range.end)
        {
            continue;
        }
        let newline = newlines.partition_point(|&pos| pos < block.body.start);
        if block.body.start >= block.body.end
            || block.kind != BlockKind::Loop
                && newlines
                    .get(newline)
                    .is_none_or(|&pos| pos >= block.body.end)
        {
            continue;
        }
        let unit = indentation_unit(block, config, &spaces)?;
        if unit.is_empty() {
            continue;
        }
        events.push(Event {
            pos: block.body.start,
            block: i,
            start: true,
            unit,
        });
        events.push(Event {
            pos: block.body.end,
            block: i,
            start: false,
            unit: "",
        });
        for (_, branch) in &block.branches {
            branches.insert(branch.start, i);
        }
    }
    events.sort_unstable_by_key(|e| (e.pos, e.start));
    let preamble_end =
        if matches!(kind, FileKind::Tex | FileKind::Tikz) && !config.config.indent.preamble {
            blocks
                .iter()
                .filter(|b| {
                    b.kind == BlockKind::Environment
                        && b.name == "document"
                        && !has_filecontents_ancestor(b, blocks, config)
                })
                .map(|b| b.begin.start)
                .min()
        } else {
            None
        };
    let mut filecontents: Vec<Range<usize>> = blocks
        .iter()
        .filter(|b| {
            b.kind == BlockKind::Environment
                && config
                    .config
                    .protect
                    .file_contents
                    .iter()
                    .any(|name| name == b.name.as_str())
        })
        .map(|b| b.body.clone())
        .collect();
    filecontents.sort_by_key(|r| r.start);
    let mut output = String::with_capacity(source.len() + source.len() / 8);
    let mut active: BTreeMap<usize, &str> = BTreeMap::new();
    let mut event = 0usize;
    let mut offset = 0usize;
    let mut cached_prefix = String::new();
    let mut dirty = true;
    let mut blanks = 0usize;
    for line in source.split_inclusive('\n') {
        let start = line.len() - line.trim_start_matches([' ', '\t']).len();
        let first = offset + start;
        while event < events.len() && events[event].pos <= first {
            let e = &events[event];
            if e.start {
                active.insert(e.block, e.unit);
            } else {
                active.remove(&e.block);
            }
            event += 1;
            dirty = true;
        }
        // Argument expansion owns only its changed groups. Other lines may
        // already carry mathematical continuation or alignment indentation.
        if !arguments.is_empty()
            && arguments
                .get(arguments.partition_point(|argument| argument.end <= first))
                .is_none_or(|argument| first < argument.start)
        {
            output.push_str(line);
            offset += line.len();
            continue;
        }
        let protected_index = protected.partition_point(|r| r.end <= offset);
        let in_protected = protected
            .get(protected_index)
            .filter(|r| r.start <= offset)
            .is_some_and(|r| r.start < offset || source[r.clone()].contains('\n'));
        let in_argument_literal = argument_literals
            .get(argument_literals.partition_point(|r| r.end <= offset))
            .is_some_and(|r| r.start < offset);
        let in_preamble = preamble_end.is_some_and(|end| first < end)
            && !syntax::is_protected(first, &filecontents);
        let preamble_argument = in_preamble
            .then(|| {
                arguments
                    .get(arguments.partition_point(|argument| argument.end <= first))
                    .filter(|argument| argument.contains(&first))
            })
            .flatten();
        let in_configuration = configuration_ranges
            .get(configuration_ranges.partition_point(|range| range.end <= first))
            .is_some_and(|range| range.contains(&first));
        let algorithm_depth = algorithm_lines.get(&first).copied();
        if in_protected && !in_configuration && algorithm_depth.is_none()
            || in_argument_literal
            || in_configuration && line.trim_matches([' ', '\t', '\r', '\n']).is_empty()
        {
            output.push_str(line);
            offset += line.len();
            blanks = 0;
            continue;
        }
        let ending = if line.ends_with("\r\n") {
            "\r\n"
        } else if line.ends_with('\n') {
            "\n"
        } else {
            ""
        };
        let body = &line[..line.len() - ending.len()];
        let trimmed = trailing_content_end(body);
        let content_end = if config.config.whitespace.remove_trailing
            && !syntax::overlaps(&(offset + trimmed..offset + body.len()), protected)
        {
            trimmed
        } else {
            body.len()
        };
        if body.trim_matches([' ', '\t']).is_empty() {
            blanks += 1;
            let maximum = config.config.whitespace.blank_lines.limit();
            if maximum.is_none_or(|maximum| blanks <= maximum) {
                output.push_str(&body[..content_end]);
                output.push_str(ending);
            }
            offset += line.len();
            continue;
        }
        blanks = 0;
        if in_preamble && preamble_argument.is_none() {
            output.push_str(&body[..content_end]);
            output.push_str(ending);
            offset += line.len();
            continue;
        }
        if dirty {
            cached_prefix.clear();
            for value in active.values() {
                cached_prefix.push_str(value);
            }
            dirty = false;
        }
        let branch = branches.get(&first);
        let mut prefix = if let Some(argument) = preamble_argument {
            // Preamble layout outside the edited argument remains preserved.
            // Its original line prefix supplies the base; only the selected
            // argument's internal blocks add new structural indentation.
            let line_start = source[..argument.start].rfind('\n').map_or(0, |i| i + 1);
            let mut prefix: String = source[line_start..argument.start]
                .chars()
                .take_while(|c| matches!(c, ' ' | '\t'))
                .collect();
            for (&block, value) in &active {
                if blocks[block].begin.start >= argument.start
                    && blocks[block].end.end <= argument.end
                    && branch != Some(&block)
                {
                    prefix.push_str(value);
                }
            }
            std::borrow::Cow::Owned(prefix)
        } else if let Some(&i) = branch {
            let mut prefix = String::new();
            for (&block, value) in &active {
                if block != i {
                    prefix.push_str(value);
                }
            }
            std::borrow::Cow::Owned(prefix)
        } else {
            std::borrow::Cow::Borrowed(cached_prefix.as_str())
        };
        if let Some(depth) = algorithm_depth.filter(|&depth| depth > 0) {
            let depth = depth.min(indent.maximum.unwrap_or(usize::MAX));
            prefix.to_mut().push_str(&algorithm_unit.repeat(depth));
        }
        if let Some(maximum) = config.config.indent.maximum {
            let mut width = 0usize;
            let mut length = 0usize;
            for c in prefix.chars() {
                let next = if c == '\t' {
                    config.config.indent.width
                } else {
                    1
                };
                if width + next > maximum {
                    break;
                }
                width += next;
                length += c.len_utf8();
            }
            if length < prefix.len() {
                prefix.to_mut().truncate(length);
            }
        }
        output.push_str(&prefix);
        output.push_str(&body[start..content_end]);
        output.push_str(ending);
        offset += line.len();
    }
    parsing.reuse_indented_snapshot(source, &output);
    Ok(output)
}

fn has_filecontents_ancestor(block: &Block, blocks: &[Block], config: &CompiledConfig) -> bool {
    let mut parent = block.parent;
    while let Some(i) = parent {
        if blocks[i].kind == BlockKind::Environment
            && config
                .config
                .protect
                .file_contents
                .iter()
                .any(|name| name == blocks[i].name.as_str())
        {
            return true;
        }
        parent = blocks[i].parent;
    }
    false
}

#[derive(Debug)]
struct Boundary {
    pos: usize,
    before: BreakMode,
    after: BreakMode,
}

// `o` is a bracketed option, `O` is an option that must be token-adjacent,
// `m` is a mandatory TeX argument, and `p` is a picture dimension pair. Known
// signatures avoid confusing first-cell groups or brackets with arguments.
pub(crate) fn environment_signature(name: &str) -> Option<&'static str> {
    if let Some(signature) = crate::packages::header_signature(name) {
        return Some(signature);
    }
    if crate::config::matrix_environment(name) {
        return Some(if name.ends_with('*') { "o" } else { "" });
    }
    Some(match name {
        "filecontents" | "filecontents*" => "om",
        "minipage" => "ooom",
        "alignat" | "alignat*" | "thebibliography" => "m",
        "alignedat" => "om",
        "IEEEeqnarray" | "IEEEeqnarray*" => "om",
        "list" => "mm",
        "picture" => "pp",
        "align" | "align*" | "gather" | "gather*" | "multline" | "multline*" | "flalign"
        | "flalign*" | "equation" | "equation*" | "eqnarray" | "eqnarray*" | "displaymath"
        | "math" | "split" | "cases" | "cases*" | "dcases" | "dcases*" | "rcases" | "rcases*"
        | "drcases" | "drcases*" | "document" | "abstract" | "quote" | "quotation" | "verse"
        | "center" | "flushleft" | "flushright" | "theindex" => "",
        "table" | "table*" | "figure" | "figure*" | "enumerate" | "enumerate*" | "itemize"
        | "itemize*" | "description" | "compactenum" | "compactitem" | "compactdesc"
        | "tikzpicture" | "scope" | "axis" | "groupplot" | "aligned" | "gathered" => "o",
        _ => return None,
    })
}

fn source_environment_signature(
    name: &str,
    analysis: &SourceAnalysis<'_>,
    begin_end: usize,
) -> Option<&'static str> {
    if crate::packages::scoped_header_signature(name).is_some() {
        return analysis
            .visual_header_active(name, begin_end)
            .then(|| environment_signature(name))
            .flatten();
    }
    if analysis
        .declared_environments()
        .is_ok_and(|names| names.contains(name))
    {
        return None;
    }
    environment_signature(name)
}

fn inline_math_environment(name: &str) -> bool {
    crate::package_environments::is_math(name)
        || crate::config::matrix_environment(name)
        || matches!(
            name,
            "array"
                | "math"
                | "split"
                | "aligned"
                | "alignedat"
                | "gathered"
                | "cases"
                | "cases*"
                | "dcases"
                | "dcases*"
                | "rcases"
                | "rcases*"
                | "drcases"
                | "drcases*"
        )
}

fn box_environment(name: &str) -> bool {
    crate::package_tables::is_box(name)
        || crate::package_environments::is_box(name)
        || matches!(name, "minipage" | "picture" | "tikzpicture")
}

fn environment_can_split_outer_boundaries(
    analysis: &SourceAnalysis<'_>,
    block: &Block,
    blocks: &[Block],
    config: &CompiledConfig,
    declared_environments: &std::collections::HashSet<String>,
) -> bool {
    if source_environment_signature(&block.name, analysis, block.begin.end).is_none()
        && analysis
            .box_arguments_at(&block.name, block.body.start)
            .is_none()
        || declared_environments.contains(block.name.as_str())
        || inline_math_environment(&block.name)
    {
        return false;
    }
    let mut container = false;
    let mut parent = block.parent;
    while let Some(i) = parent {
        let ancestor = &blocks[i];
        if matches!(
            ancestor.kind,
            BlockKind::Group
                | BlockKind::MandatoryArgument
                | BlockKind::OptionalArgument
                | BlockKind::NamedGroup
                | BlockKind::KeyValue
        ) || ancestor.kind == BlockKind::Special
            && matches!(
                ancestor.name.as_str(),
                "inlineMath" | "displayMath" | "displayMathTeX"
            )
            || ancestor.kind == BlockKind::Environment
                && (inline_math_environment(&ancestor.name)
                    || crate::package_tables::is_table(&ancestor.name)
                    || matches!(
                        ancestor.name.as_str(),
                        "align"
                            | "align*"
                            | "gather"
                            | "gather*"
                            | "alignat"
                            | "alignat*"
                            | "flalign"
                            | "flalign*"
                            | "eqnarray"
                            | "eqnarray*"
                    )
                    || config
                        .config
                        .alignment
                        .rules
                        .get(ancestor.name.as_str())
                        .is_some_and(|rule| rule.enabled != Some(false)))
        {
            return false;
        }
        if ancestor.kind == BlockKind::Environment
            && (crate::package_environments::is_block(&ancestor.name)
                || analysis
                    .box_arguments_at(&ancestor.name, ancestor.body.start)
                    .is_some()
                || matches!(
                    ancestor.name.as_str(),
                    "table"
                        | "table*"
                        | "figure"
                        | "figure*"
                        | "center"
                        | "flushleft"
                        | "flushright"
                        | "quote"
                        | "quotation"
                        | "minipage"
                        | "tcolorbox"
                        | "tikzpicture"
                        | "scope"
                ))
        {
            container = true;
        }
        parent = ancestor.parent;
    }
    !box_environment(&block.name) || container
}

fn automatic_environment_begin(
    source: &str,
    block: &Block,
    tokens: &[syntax::Token],
    paragraph_ends: &std::collections::HashSet<usize>,
    split_outer: bool,
) -> BreakMode {
    if !split_outer {
        return BreakMode::Preserve;
    }
    let pos = block.begin.start;
    if pos == 0 || source.as_bytes()[pos - 1].is_ascii_whitespace() {
        return BreakMode::Line;
    }
    let previous = tokens.get(
        tokens
            .partition_point(|token| token.span.end <= pos)
            .wrapping_sub(1),
    );
    if previous.is_some_and(|token| {
        token.kind == syntax::TokenKind::Command
            && token.span.end == pos
            && source[token.span.start + 1..token.span.end]
                .chars()
                .all(|c| c.is_ascii_alphabetic())
    }) {
        // TeX discards a control word's following line ending even if the
        // command expands to visible text. No macro meaning is assumed here.
        return BreakMode::Line;
    }
    if paragraph_ends.contains(&pos) {
        BreakMode::Line
    } else {
        BreakMode::CommentLine
    }
}

fn automatic_environment_after(
    source: &str,
    block: &Block,
    tokens: &[syntax::Token],
    config: &CompiledConfig,
    declared_commands: &std::collections::HashSet<String>,
    split_outer: bool,
) -> BreakMode {
    if !split_outer {
        return BreakMode::Preserve;
    }
    let mut next = block.end.end;
    loop {
        while source
            .as_bytes()
            .get(next)
            .is_some_and(u8::is_ascii_whitespace)
        {
            next += 1;
        }
        if source.as_bytes().get(next) != Some(&b'%') {
            break;
        }
        next = source[next..]
            .find('\n')
            .map_or(source.len(), |offset| next + offset + 1);
    }
    let Some(token) = tokens.get(tokens.partition_point(|token| token.span.start < next)) else {
        return BreakMode::Preserve;
    };
    if token.span.start != next || token.kind != syntax::TokenKind::Command {
        return BreakMode::Preserve;
    }
    let Some((name, _)) = syntax::command_at(source, next) else {
        return BreakMode::Preserve;
    };
    let name = name.trim_end_matches('*');
    if !is_paragraph_caption(name) {
        return BreakMode::Preserve;
    }
    // Standard caption commands end the paragraph and discard trailing glue.
    // A locally redefined caption may print text in the current paragraph.
    let tight = next == block.end.end;
    if tight
        && (declared_commands.contains(name) || config.config.syntax.commands.contains_key(name))
    {
        BreakMode::CommentLine
    } else {
        BreakMode::Line
    }
}

fn is_paragraph_caption(name: &str) -> bool {
    matches!(name, "caption" | "subcaption" | "captionof")
        || crate::packages::equation_floats::bilingual_caption_signature(name).is_some()
}

fn paragraph_command_end(source: &str, block: &Block) -> Option<usize> {
    let signature = match block.name.trim_end_matches('*') {
        "caption" | "subcaption" => "om",
        "captionof" => "mom",
        _ => crate::packages::equation_floats::command_signature(
            source,
            &block.name,
            block.begin.end,
        )?,
    };
    syntax::arguments::signature_end(source, block.begin.end, signature)
}

fn environment_argument_range(
    source: &str,
    cursor: usize,
    limit: usize,
    slot: u8,
) -> Option<Range<usize>> {
    let mut start = cursor;
    if slot == b'O' {
        // A comment removes its line ending. TeX also discards indentation
        // on the next line, so it does not separate a tight option token.
        // Initial spaces and ordinary line endings remain real separators.
        while start < limit && source.as_bytes().get(start) == Some(&b'%') {
            start = source[start..limit]
                .find('\n')
                .map_or(limit, |offset| start + offset + 1);
            while start < limit && matches!(source.as_bytes()[start], b' ' | b'\t') {
                start += 1;
            }
        }
    } else {
        start = crate::trivia::skip_until(source, cursor, limit);
    }
    if start >= limit {
        return None;
    }
    let end = match slot {
        b'o' | b'O' => syntax::balanced_end(source, start, b'[', b']'),
        b'p' => syntax::balanced_end(source, start, b'(', b')'),
        b'd' => syntax::balanced_end(source, start, b'<', b'>'),
        b'm' | b'g' if source.as_bytes()[start] == b'{' => {
            syntax::balanced_end(source, start, b'{', b'}')
        }
        b'l' => syntax::command_at(source, start)
            .filter(|(command, _)| *command == "label")
            .map(|(_, end)| end),
        b'm' if source.as_bytes()[start] == b'\\' => {
            syntax::command_at(source, start).map(|(token, mut end)| {
                // A control word is one mandatory argument token; a
                // following star is a separate token in TeX.
                if token.len() > 1 && token.ends_with('*') {
                    end -= 1;
                }
                end
            })
        }
        b'm' if source.as_bytes()[start] != b'}' => {
            Some(start + source[start..].chars().next()?.len_utf8())
        }
        _ => None,
    };
    end.filter(|end| *end <= limit).map(|end| start..end)
}

pub(crate) fn configured_environment_argument_ranges(
    source: &str,
    body: Range<usize>,
    rule: &crate::config::EnvironmentArguments,
) -> Vec<Range<usize>> {
    if let Some(signature) = rule
        .signature
        .as_deref()
        .filter(|signature| crate::syntax::xparse::extended(signature))
    {
        return crate::syntax::xparse::Signature::parse(signature)
            .and_then(|signature| signature.ranges(source, body))
            .unwrap_or_default();
    }
    signature_argument_ranges(source, body, rule.slots())
}

pub(crate) fn signature_argument_ranges(
    source: &str,
    body: Range<usize>,
    slots: impl Iterator<Item = u8>,
) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut cursor = body.start;
    for slot in slots {
        match environment_argument_range(source, cursor, body.end, slot) {
            Some(range) => {
                cursor = range.end;
                ranges.push(range);
            }
            None if slot == b'm' => break,
            None => (),
        }
    }
    ranges
}

pub(crate) fn environment_header_end(
    source: &str,
    body: Range<usize>,
    name: &str,
    config: &crate::Config,
    analysis: &SourceAnalysis<'_>,
) -> usize {
    let next_argument = |cursor: usize, slot: u8| {
        environment_argument_range(source, cursor, body.end, slot).map(|range| range.end)
    };
    let mut cursor = body.start;
    let configured_rule = config.syntax.environment_arguments.get(name).filter(|_| {
        !config
            .syntax
            .inferred_environment_arguments
            .contains_key(name)
    });
    let source_rule = if configured_rule.is_some() {
        None
    } else {
        analysis.box_arguments_at(name, body.start)
    };
    if let Some(rule) = configured_rule.or(source_rule.as_deref()) {
        if rule
            .signature
            .as_deref()
            .is_some_and(crate::syntax::xparse::extended)
        {
            return configured_environment_argument_ranges(source, body, rule)
                .last()
                .map_or(cursor, |range| range.end);
        }
        for slot in rule.slots() {
            match next_argument(cursor, slot) {
                Some(end) => cursor = end,
                None if slot == b'm' => break,
                None => (),
            }
        }
    } else {
        // Unknown environments may have a leading option. Their mandatory
        // parameters require a configured signature to avoid consuming body.
        let signature =
            source_environment_signature(name, analysis, body.start).and_then(|signature| {
                crate::packages::scoped_invocation_signature(source, body.start, name)
                    .or_else(|| {
                        crate::packages::links_color_columns::invocation_signature(
                            source, body.start, name,
                        )
                    })
                    .or_else(|| {
                        crate::class_environments::invocation_signature(source, body.clone(), name)
                    })
                    .or(Some(signature))
            });
        if signature.is_some() && matches!(name, "tabu" | "longtabu") {
            return crate::package_tables::tabu_column_spec(source, body.clone())
                .map_or(body.start, |range| range.end);
        }
        for slot in signature.unwrap_or("o").bytes() {
            match next_argument(cursor, slot) {
                Some(end) => cursor = end,
                None if slot == b'm' => break,
                None => (),
            }
        }
    }
    cursor
}

// Row modifiers use TeX's whitespace-skipping lookahead. Consume them before
// adding a boundary, including options written after a comment or line ending.
pub(crate) fn row_terminator_end(
    source: &str,
    start: usize,
    limit: usize,
    allow_star: bool,
) -> usize {
    let trivia_end = |mut pos: usize| {
        let mut line_start = false;
        loop {
            while pos < limit {
                match source.as_bytes()[pos] {
                    b' ' | b'\t' | b'\r' => pos += 1,
                    b'\n' if line_start => return pos,
                    b'\n' => {
                        line_start = true;
                        pos += 1;
                    }
                    _ => break,
                }
            }
            if pos >= limit || source.as_bytes()[pos] != b'%' {
                return pos;
            }
            pos = source[pos..limit]
                .find('\n')
                .map_or(limit, |offset| pos + offset + 1);
            line_start = true;
        }
    };
    let mut end = start;
    let next = trivia_end(end);
    if allow_star && next < limit && source.as_bytes()[next] == b'*' {
        end = next + 1;
    }
    let next = trivia_end(end);
    if next < limit
        && source.as_bytes()[next] == b'['
        && let Some(option_end) = syntax::balanced_end(source, next, b'[', b']')
        && option_end <= limit
    {
        end = option_end;
    }
    end
}

fn structural_bracket_events(source: &str, blocks: &[Block]) -> BTreeMap<usize, bool> {
    // Synthetic loop parents can follow their children. Resolve each role
    // once with an iterative ancestor walk, keeping deep inputs linear.
    // 0 = unknown, 1 = visiting, 2 = ordinary, 3 = command option.
    let mut option_scopes = vec![0u8; blocks.len()];
    let mut pending = Vec::new();
    for index in 0..blocks.len() {
        if option_scopes[index] >= 2 {
            continue;
        }
        let mut current = Some(index);
        let actual_option = loop {
            let Some(index) = current.filter(|&index| index < blocks.len()) else {
                break false;
            };
            match option_scopes[index] {
                1 | 2 => break false,
                3 => break true,
                _ => (),
            }
            option_scopes[index] = 1;
            pending.push(index);
            let block = &blocks[index];
            if block.kind == BlockKind::OptionalArgument {
                break block
                    .parent
                    .and_then(|parent| blocks.get(parent))
                    .is_some_and(|parent| {
                        matches!(parent.kind, BlockKind::Command | BlockKind::Heading)
                    });
            }
            current = block.parent;
        };
        for index in pending.drain(..) {
            option_scopes[index] = if actual_option { 3 } else { 2 };
        }
    }
    let mut events = BTreeMap::new();
    for (index, block) in blocks.iter().enumerate() {
        if option_scopes[index] == 3
            && source.as_bytes().get(block.begin.start) == Some(&b'[')
            && source.as_bytes().get(block.end.start) == Some(&b']')
        {
            events.insert(block.begin.start, true);
            events.insert(block.end.start, false);
        }
    }
    events
}

struct BoundaryIndex<'a, 's> {
    config: &'a CompiledConfig,
    analysis: &'a SourceAnalysis<'s>,
    environments: BTreeMap<usize, usize>,
    brackets: BTreeMap<usize, bool>,
    headers: Vec<usize>,
    filecontents_ancestors: Vec<bool>,
}

impl<'a, 's> BoundaryIndex<'a, 's> {
    fn new(
        source: &str,
        blocks: &[Block],
        config: &'a CompiledConfig,
        analysis: &'a SourceAnalysis<'s>,
    ) -> Self {
        Self {
            config,
            analysis,
            environments: environment_index(blocks),
            brackets: structural_bracket_events(source, blocks),
            filecontents_ancestors: filecontents_ancestry(blocks, config),
            headers: blocks
                .iter()
                .map(|block| {
                    if block.kind == BlockKind::Environment {
                        environment_header_end(
                            source,
                            block.body.clone(),
                            &block.name,
                            &config.config,
                            analysis,
                        )
                    } else {
                        block.begin.end
                    }
                })
                .collect(),
        }
    }

    fn environments(&self) -> &BTreeMap<usize, usize> {
        &self.environments
    }

    fn brackets(&self) -> &BTreeMap<usize, bool> {
        &self.brackets
    }

    fn header(&self, index: usize) -> usize {
        self.headers[index]
    }
}

// Resolve each parent link once, including trees whose parents appear later in
// the block array. Boundary decisions then use constant-time ancestry queries.
fn filecontents_ancestry(blocks: &[Block], config: &CompiledConfig) -> Vec<bool> {
    let mut known: Vec<Option<bool>> = blocks
        .iter()
        .map(|block| {
            (block.kind == BlockKind::Environment
                && config
                    .config
                    .protect
                    .file_contents
                    .iter()
                    .any(|name| name == block.name.as_str()))
            .then_some(true)
        })
        .collect();
    if known.iter().all(Option::is_none) {
        return vec![false; blocks.len()];
    }
    let mut path = Vec::new();
    for index in 0..blocks.len() {
        let mut current = Some(index);
        let inside = loop {
            let Some(next) = current else { break false };
            if let Some(inside) = known[next] {
                break inside;
            }
            path.push(next);
            current = blocks[next].parent;
        };
        for next in path.drain(..) {
            known[next] = Some(inside);
        }
    }
    blocks
        .iter()
        .map(|block| {
            block
                .parent
                .is_some_and(|parent| known[parent] == Some(true))
        })
        .collect()
}

fn environment_index(blocks: &[Block]) -> BTreeMap<usize, usize> {
    blocks
        .iter()
        .enumerate()
        .filter(|(_, block)| block.kind == BlockKind::Environment)
        .map(|(index, block)| (block.begin.start, index))
        .collect()
}

fn row_boundaries(
    source: &str,
    tokens: &[syntax::Token],
    blocks: &[Block],
    declared_environments: &std::collections::HashSet<String>,
    index: &BoundaryIndex<'_, '_>,
) -> Vec<(usize, BreakMode)> {
    use syntax::TokenKind;
    let config = index.config;
    let analysis = index.analysis;
    let environments = index.environments();
    let mut stack = Vec::new();
    let brackets = index.brackets();
    let mut depth = 0usize;
    let mut skip_until = 0usize;
    let mut output = Vec::new();
    for token in tokens {
        if token.span.start < skip_until {
            continue;
        }
        match token.kind {
            TokenKind::BeginEnvironment => {
                if let Some(&i) = environments.get(&token.span.start) {
                    let header = index.header(i);
                    stack.push((i, depth));
                    skip_until = header;
                }
            }
            TokenKind::EndEnvironment => {
                if stack
                    .last()
                    .is_some_and(|&(i, _)| blocks[i].end.start == token.span.start)
                {
                    stack.pop();
                }
            }
            TokenKind::OpenBrace => depth += 1,
            TokenKind::CloseBrace => depth = depth.saturating_sub(1),
            TokenKind::OpenBracket | TokenKind::CloseBracket => {
                if let Some(open) = brackets.get(&token.span.start) {
                    if *open {
                        depth += 1;
                    } else {
                        depth = depth.saturating_sub(1);
                    }
                }
            }
            TokenKind::Command
                if matches!(
                    &source[token.span.clone()],
                    "\\\\" | "\\tabularnewline" | "\\tabularnewline*"
                ) =>
            {
                let Some(&(i, baseline)) = stack.last() else {
                    continue;
                };
                if depth != baseline {
                    continue;
                }
                let block = &blocks[i];
                let explicit = config.config.breaks.rules.contains_key(block.name.as_str())
                    || config.config.breaks.categories.contains_key("environment");
                if !explicit && index.filecontents_ancestors[i] {
                    continue;
                }
                let Some(rule) = config.config.breaks.rule("environment", &block.name) else {
                    continue;
                };
                let end = row_terminator_end(
                    source,
                    token.span.end,
                    block.body.end,
                    !source[token.span.clone()].ends_with('*'),
                );
                skip_until = end;
                let adjacency = !explicit
                    && (source_environment_signature(&block.name, analysis, block.begin.end)
                        .is_none()
                        || declared_environments.contains(block.name.as_str()));
                let mode = if adjacency
                    && rule.row == BreakMode::Line
                    && source
                        .as_bytes()
                        .get(end)
                        .is_some_and(|b| !b.is_ascii_whitespace())
                {
                    BreakMode::CommentLine
                } else {
                    rule.row
                };
                output.push((end, mode));
            }
            _ => (),
        }
    }
    output
}

fn table_rule_boundaries(
    source: &str,
    tokens: &[syntax::Token],
    blocks: &[Block],
    declared_environments: &std::collections::HashSet<String>,
    declared_commands: &std::collections::HashSet<String>,
    index: &BoundaryIndex<'_, '_>,
) -> Vec<(usize, BreakMode, bool)> {
    use syntax::TokenKind;
    let config = index.config;
    let environments = index.environments();
    let mut stack = Vec::new();
    let brackets = index.brackets();
    let mut depth = 0usize;
    let mut skip_until = 0;
    let mut output = Vec::new();
    for token in tokens {
        if token.span.start < skip_until {
            continue;
        }
        match token.kind {
            TokenKind::BeginEnvironment => {
                if let Some(&i) = environments.get(&token.span.start) {
                    stack.push((i, depth));
                    skip_until = index.header(i);
                }
            }
            TokenKind::EndEnvironment => {
                if stack
                    .last()
                    .is_some_and(|&(i, _)| blocks[i].end.start == token.span.start)
                {
                    stack.pop();
                }
            }
            TokenKind::OpenBrace => depth += 1,
            TokenKind::CloseBrace => depth = depth.saturating_sub(1),
            TokenKind::OpenBracket | TokenKind::CloseBracket => {
                if let Some(open) = brackets.get(&token.span.start) {
                    if *open {
                        depth += 1;
                    } else {
                        depth = depth.saturating_sub(1);
                    }
                }
            }
            TokenKind::Command => {
                let Some(&(i, baseline)) = stack.last() else {
                    continue;
                };
                let block = &blocks[i];
                if depth != baseline
                    || !(crate::package_tables::is_table(&block.name)
                        || crate::package_environments::column_spec_slot(&block.name).is_some())
                {
                    continue;
                }
                let Some((name, _)) = syntax::command_at(source, token.span.start) else {
                    continue;
                };
                let named = config
                    .config
                    .breaks
                    .rules
                    .get(name)
                    .or_else(|| config.config.breaks.categories.get("table-rule"));
                if named.is_none()
                    && (declared_environments.contains(block.name.as_str())
                        || declared_commands.contains(name)
                        || config.config.syntax.commands.contains_key(name)
                        || index.filecontents_ancestors[i])
                {
                    continue;
                }
                let Some(mut end) = crate::table_rules::rule_end(
                    source,
                    token.span.start,
                    block.body.end,
                    &block.name,
                ) else {
                    continue;
                };
                // When ranges share a physical line, give the group a single
                // pair of boundaries. A named break rule takes precedence.
                let tabularray = crate::package_tables::is_tabularray(&block.name);
                let merge_cmidrules = named.is_none()
                    && !config.config.breaks.split_cmidrules
                    && if tabularray {
                        matches!(name, "cmidrule" | "cmidrulemore")
                    } else {
                        name == "cmidrule"
                    };
                if merge_cmidrules {
                    loop {
                        let rest = source[end..block.body.end].trim_start_matches([' ', '\t']);
                        let next = block.body.end - rest.len();
                        let Some((next_name, _)) = syntax::command_at(source, next) else {
                            break;
                        };
                        let is_cmidrule = if tabularray {
                            matches!(next_name, "cmidrule" | "cmidrulemore")
                        } else {
                            next_name == "cmidrule"
                        };
                        if !is_cmidrule {
                            break;
                        }
                        let Some(close) =
                            crate::table_rules::rule_end(source, next, block.body.end, &block.name)
                        else {
                            break;
                        };
                        end = close;
                    }
                }
                let (before, after) = if let Some(rule) = named {
                    (rule.begin, rule.after)
                } else {
                    let Some(rule) = config.config.breaks.rule("environment", &block.name) else {
                        continue;
                    };
                    (rule.row, rule.row)
                };
                output.push((token.span.start, before, true));
                output.push((end, after, false));
                skip_until = end;
            }
            _ => (),
        }
    }
    output
}

fn apply_breaks(
    source: &str,
    config: &CompiledConfig,
    parsing: &mut ParseContext<'_>,
) -> Result<String, FormatError> {
    if config.config.breaks.rules.is_empty()
        && config.config.breaks.categories.is_empty()
        && !source.contains("\\begin")
        && !source.contains("\\foreach")
        && !source.contains("\\tikz")
        && config.config.syntax.environment_aliases.is_empty()
        && config.config.syntax.inferred_environment_aliases.is_empty()
        && config.config.syntax.environment_alias_events.is_empty()
    {
        return Ok(source.into());
    }
    let parsed = parsing.snapshot(source, true, true)?;
    let protected = &parsed.protected;
    let analysis = &SourceAnalysis::with_memo(
        source,
        &config.config,
        protected,
        std::rc::Rc::clone(&parsed.metadata),
    );
    let tokens = parsed.tokens.as_ref().expect("parsed tokens");
    let mut blocks: Cow<'_, [Block]> =
        Cow::Borrowed(parsed.blocks.as_deref().expect("parsed structure"));
    let verbatim = syntax::verbatim_blocks_with_ranges(source, protected);
    if !verbatim.is_empty() {
        blocks.to_mut().extend(verbatim);
    }
    let declared_environments = analysis.declared_environments()?;
    let declared_commands = analysis.declared_commands()?;
    let boundary_index = BoundaryIndex::new(source, &blocks, config, analysis);
    let paragraph_ends = blocks
        .iter()
        .filter(|block| {
            matches!(block.kind, BlockKind::Command | BlockKind::Heading)
                && is_paragraph_caption(block.name.trim_end_matches('*'))
                && !declared_commands.contains(block.name.trim_end_matches('*'))
                && !config
                    .config
                    .syntax
                    .commands
                    .contains_key(block.name.trim_end_matches('*'))
        })
        .filter_map(|block| paragraph_command_end(source, block))
        .collect();
    let ending = preferred_ending(source, config);
    // New boundaries on a mixed-ending line inherit that line's ending. This
    // also keeps rewrap stable when it rejoins a command split in this stage.
    let local_newlines = (config.config.output.line_ending == LineEnding::Preserve
        && source.contains("\r\n"))
    .then(|| memchr::memchr_iter(b'\n', source.as_bytes()).collect::<Vec<_>>());
    let mut boundaries: BTreeMap<usize, Boundary> = BTreeMap::new();
    let mut preserved_path_begins = std::collections::HashSet::new();
    let mut record = |pos, mode, before| {
        if mode == BreakMode::Preserve {
            return;
        }
        let entry = boundaries.entry(pos).or_insert(Boundary {
            pos,
            before: BreakMode::Preserve,
            after: BreakMode::Preserve,
        });
        if before {
            entry.before = mode;
        } else {
            entry.after = mode;
        }
    };
    for (block_index, block) in blocks.iter().enumerate() {
        if block.kind == BlockKind::TikzPath && block.begin.is_empty() {
            continue;
        }
        let environment_argument = matches!(
            block.kind,
            BlockKind::MandatoryArgument | BlockKind::OptionalArgument
        ) && block
            .parent
            .is_some_and(|parent| blocks[parent].kind == BlockKind::Environment);
        let explicit_rule = (!environment_argument
            && config.config.breaks.rules.contains_key(block.name.as_str()))
            || config
                .config
                .breaks
                .categories
                .contains_key(block.kind.category());
        if !explicit_rule && boundary_index.filecontents_ancestors[block_index] {
            continue;
        }
        // Environment header arguments share the environment's name. Its name
        // rule belongs to the environment boundary; argument categories can
        // still explicitly request changes inside an argument.
        let rule = if environment_argument {
            config.config.breaks.categories.get(block.kind.category())
        } else {
            config
                .config
                .breaks
                .rule(block.kind.category(), &block.name)
        };
        if let Some(rule) = rule {
            let split_outer = block.kind == BlockKind::Environment
                && !explicit_rule
                && environment_can_split_outer_boundaries(
                    analysis,
                    block,
                    &blocks,
                    config,
                    &declared_environments,
                );
            let begin_mode = if block.kind == BlockKind::Environment && !explicit_rule {
                automatic_environment_begin(source, block, tokens, &paragraph_ends, split_outer)
            } else {
                rule.begin
            };
            let begin_mode = if block.kind == BlockKind::Loop
                && !explicit_rule
                && begin_mode == BreakMode::Line
                && block.begin.start > 0
                && !source.as_bytes()[block.begin.start - 1].is_ascii_whitespace()
            {
                BreakMode::CommentLine
            } else {
                begin_mode
            };
            record(block.begin.start, begin_mode, true);
            let body_start = if block.kind == BlockKind::Environment {
                boundary_index.header(block_index)
            } else {
                block.begin.end
            };
            if block.kind == BlockKind::Environment
                && explicit_rule
                && rule.body == BreakMode::Preserve
            {
                preserved_path_begins.insert(crate::trivia::skip_until(
                    source,
                    body_start,
                    block.body.end,
                ));
            }
            let preserve_adjacency = !explicit_rule
                && (block.kind == BlockKind::Loop
                    || block.kind == BlockKind::Environment
                        && (source_environment_signature(&block.name, analysis, block.begin.end)
                            .is_none()
                            || crate::package_environments::preserves_tight_whitespace(
                                &block.name,
                            )
                            || declared_environments.contains(block.name.as_str())));
            let body_mode = if preserve_adjacency
                && rule.body == BreakMode::Line
                && source
                    .as_bytes()
                    .get(body_start)
                    .is_some_and(|byte| !byte.is_ascii_whitespace())
            {
                BreakMode::CommentLine
            } else {
                rule.body
            };
            record(body_start, body_mode, false);
            if !block.end.is_empty() {
                let end_mode = if preserve_adjacency
                    && rule.end == BreakMode::Line
                    && block.end.start > 0
                    && !source.as_bytes()[block.end.start - 1].is_ascii_whitespace()
                {
                    BreakMode::CommentLine
                } else {
                    rule.end
                };
                record(block.end.start, end_mode, true);
                let after_mode = if block.kind == BlockKind::Environment && !explicit_rule {
                    automatic_environment_after(
                        source,
                        block,
                        tokens,
                        config,
                        &declared_commands,
                        split_outer,
                    )
                } else {
                    rule.after
                };
                let after_mode = if block.kind == BlockKind::Loop
                    && !explicit_rule
                    && after_mode == BreakMode::Line
                    && source
                        .as_bytes()
                        .get(block.end.end)
                        .is_some_and(|b| !b.is_ascii_whitespace())
                {
                    BreakMode::CommentLine
                } else {
                    after_mode
                };
                record(block.end.end, after_mode, false);
            } else {
                let after_mode = if block.kind == BlockKind::Loop
                    && !explicit_rule
                    && rule.after == BreakMode::Line
                    && source
                        .as_bytes()
                        .get(block.body.end)
                        .is_some_and(|b| !b.is_ascii_whitespace())
                {
                    BreakMode::CommentLine
                } else {
                    rule.after
                };
                record(block.body.end, after_mode, false);
            }
            for (is_or, branch) in &block.branches {
                record(
                    branch.start,
                    if *is_or {
                        rule.or_begin
                    } else {
                        rule.else_begin
                    },
                    true,
                );
                record(
                    branch.end,
                    if *is_or {
                        rule.or_after
                    } else {
                        rule.else_after
                    },
                    false,
                );
            }
        }
    }
    for (pos, mode) in row_boundaries(
        source,
        tokens,
        &blocks,
        &declared_environments,
        &boundary_index,
    ) {
        record(pos, mode, false);
    }
    for (pos, mode, before) in table_rule_boundaries(
        source,
        tokens,
        &blocks,
        &declared_environments,
        &declared_commands,
        &boundary_index,
    ) {
        record(pos, mode, before);
    }
    for (pos, mode, before) in crate::tikz::boundaries(source, &blocks, config, protected) {
        // Only automatic path boundaries yield to an explicit environment
        // body preserve rule. Explicit path rules are recorded above.
        if before && preserved_path_begins.contains(&pos) {
            continue;
        }
        record(pos, mode, before);
    }
    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    for boundary in boundaries.values() {
        for (before, mode) in [(true, boundary.before), (false, boundary.after)] {
            if mode == BreakMode::Preserve {
                continue;
            }
            let mut start = boundary.pos;
            let mut end = boundary.pos;
            if before {
                while start > 0 && source.as_bytes()[start - 1].is_ascii_whitespace() {
                    start -= 1;
                }
            } else {
                while end < source.len() && source.as_bytes()[end].is_ascii_whitespace() {
                    end += 1;
                }
            }
            let at_protected_start = before
                && protected
                    .get(protected.partition_point(|range| range.start < boundary.pos))
                    .is_some_and(|range| range.start == boundary.pos);
            if syntax::overlaps(&(start..end), protected)
                || syntax::is_protected(boundary.pos, protected) && !at_protected_start
            {
                continue;
            }
            let whitespace = &source[start..end];
            let ending = local_newlines
                .as_ref()
                .and_then(|positions| {
                    memchr::memchr(b'\n', whitespace.as_bytes())
                        .map(|offset| start + offset)
                        .or_else(|| {
                            positions
                                .get(positions.partition_point(|&pos| pos < boundary.pos))
                                .or_else(|| positions.last())
                                .copied()
                        })
                })
                .map_or(ending, |pos| {
                    if pos > 0 && source.as_bytes()[pos - 1] == b'\r' {
                        "\r\n"
                    } else {
                        "\n"
                    }
                });
            let replacement = match mode {
                BreakMode::Preserve => unreachable!(),
                BreakMode::Join => {
                    // A comment immediately before this boundary owns its newline.
                    let line = source[..start].rsplit('\n').next().unwrap_or("");
                    if comment_position(line).is_some() {
                        continue;
                    }
                    if whitespace.is_empty() {
                        String::new()
                    } else {
                        " ".into()
                    }
                }
                BreakMode::Line => {
                    if whitespace.contains('\n') || !before && source[end..].starts_with('%') {
                        continue;
                    } else {
                        ending.into()
                    }
                }
                BreakMode::CommentLine => {
                    if whitespace.contains('\n') || !before && source[end..].starts_with('%') {
                        continue;
                    }
                    format!("%{ending}")
                }
                BreakMode::BlankLine => {
                    if whitespace.matches('\n').count() >= 2 {
                        continue;
                    } else {
                        ending.repeat(2)
                    }
                }
                BreakMode::ResetBlankLine => ending.repeat(2),
            };
            if start == 0 && before && mode != BreakMode::Join {
                continue;
            }
            edits.push((start..end, replacement));
        }
    }
    apply_edits(source, edits)
}

pub(crate) fn comment_position(line: &str) -> Option<usize> {
    let mut pos = 0usize;
    while pos < line.len() {
        match line.as_bytes()[pos] {
            b'\\' => pos = syntax::command_at(line, pos).map_or(pos + 1, |(_, end)| end),
            b'%' => return Some(pos),
            _ => {
                pos += line[pos..]
                    .chars()
                    .next()
                    .expect("valid position")
                    .len_utf8()
            }
        }
    }
    None
}

pub(crate) fn apply_edits(
    source: &str,
    mut edits: Vec<(Range<usize>, String)>,
) -> Result<String, FormatError> {
    edits.sort_by_key(|(range, _)| (range.start, range.end));
    let mut output = String::with_capacity(source.len());
    let mut cursor = 0usize;
    let mut last_empty = None;
    for (range, text) in edits {
        if range.start < cursor {
            continue;
        }
        if range.is_empty() && last_empty == Some(range.start) {
            continue;
        }
        if !source.is_char_boundary(range.start) || !source.is_char_boundary(range.end) {
            return Err(FormatError::Syntax("invalid UTF-8 edit boundary".into()));
        }
        output.push_str(&source[cursor..range.start]);
        output.push_str(&text);
        cursor = range.end;
        last_empty = range.is_empty().then_some(range.start);
    }
    output.push_str(&source[cursor..]);
    Ok(output)
}

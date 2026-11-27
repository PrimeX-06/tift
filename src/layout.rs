use crate::config::{BreakMode, FinalNewline, LineEnding};
use crate::edit_plan::EditOrigin;
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
    details: &mut Option<&mut crate::trace::DetailJournal>,
    config: &CompiledConfig,
    operation: impl FnOnce() -> Result<String, FormatError>,
) -> Result<String, FormatError> {
    stage_recording(name, source, trace, details, config, || {
        Ok((operation()?, None))
    })
}

fn stage_recording(
    name: &'static str,
    source: &str,
    trace: &mut Option<&mut Vec<StageTrace>>,
    details: &mut Option<&mut crate::trace::DetailJournal>,
    config: &CompiledConfig,
    operation: impl FnOnce() -> Result<(String, Option<crate::edit_plan::EditHistory>), FormatError>,
) -> Result<String, FormatError> {
    let started = trace.is_some().then(Instant::now);
    let (text, exact) = operation()?;
    let elapsed = started.map(|started| started.elapsed());
    if let Some(details) = details.as_deref_mut() {
        details.record(name, source, &text, config, exact);
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

fn planned_stage(
    name: &'static str,
    source: &str,
    trace: &mut Option<&mut Vec<StageTrace>>,
    details: &mut Option<&mut crate::trace::DetailJournal>,
    config: &CompiledConfig,
    operation: impl FnOnce(Option<&mut crate::edit_plan::EditHistory>) -> Result<String, FormatError>,
) -> Result<String, FormatError> {
    let detailed = details.is_some();
    stage_recording(name, source, trace, details, config, || {
        let mut history = detailed.then(crate::edit_plan::EditHistory::default);
        let output = operation(history.as_mut())?;
        Ok((output, history))
    })
}

fn optional_stage(
    name: &'static str,
    text: &mut String,
    trace: &mut Option<&mut Vec<StageTrace>>,
    details: &mut Option<&mut crate::trace::DetailJournal>,
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

/// Each rule reads the preceding rule's output. Follow-up passes consult the
/// edit status of their prerequisites, including on the untraced entry point.
#[derive(Clone, Copy)]
enum TexRule {
    Macros,
    Math,
    Arguments,
    ProseSpaces,
    Description,
    TextBefore,
    Alignment,
    TextAfter,
    WhitespaceAfter,
    Code,
    MathFinal,
    MacrosFinal,
    WhitespaceFinal,
    ColumnSpecs,
    Trees,
    PlotTables,
    Foreach,
    Consecutive,
    TrailingComments,
}

impl TexRule {
    const ORDER: &[Self] = &[
        Self::Macros,
        Self::Math,
        Self::Arguments,
        Self::ProseSpaces,
        Self::Description,
        Self::TextBefore,
        Self::Alignment,
        Self::TextAfter,
        Self::WhitespaceAfter,
        Self::Code,
        Self::MathFinal,
        Self::MacrosFinal,
        Self::WhitespaceFinal,
        Self::ColumnSpecs,
        Self::Trees,
        Self::PlotTables,
        Self::Foreach,
        Self::Consecutive,
        Self::TrailingComments,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::Macros => "macro-bodies",
            Self::Math => "math",
            Self::Arguments => "arguments",
            Self::ProseSpaces => "prose-spaces",
            Self::Description => "description",
            Self::TextBefore => "text-before",
            Self::Alignment => "alignment",
            Self::TextAfter => "text-after",
            Self::WhitespaceAfter => "whitespace-after",
            Self::Code => "code",
            Self::MathFinal => "math-final",
            Self::MacrosFinal => "macro-bodies-final",
            Self::WhitespaceFinal => "whitespace-final",
            Self::ColumnSpecs => "column-specs",
            Self::Trees => "trees",
            Self::PlotTables => "pgfplots-tables",
            Self::Foreach => "foreach-lists",
            Self::Consecutive => "consecutive-arguments",
            Self::TrailingComments => "trailing-comments",
        }
    }

    const fn bit(self) -> u32 {
        1 << self as u8
    }

    fn local_edits(
        self,
        source: &str,
        config: &CompiledConfig,
        parsing: &mut ParseContext<'_>,
    ) -> Result<crate::edit_plan::RuleEdits, FormatError> {
        match self {
            Self::ColumnSpecs => crate::alignment::preamble_edits(source, config, parsing),
            Self::Trees => crate::tree_layout::tree_edits(source, config, parsing),
            Self::PlotTables => crate::pgfplots_table::table_edits(source, config, parsing),
            Self::Foreach => crate::foreach::list_edits(source, config, parsing),
            _ => unreachable!("nonlocal rule cannot submit to the local layout batch"),
        }
    }
}

fn format_tex_rules(
    text: &mut String,
    kind: FileKind,
    config: &CompiledConfig,
    parsing: &mut ParseContext<'_>,
    trace: &mut Option<&mut Vec<StageTrace>>,
    details: &mut Option<&mut crate::trace::DetailJournal>,
) -> Result<(), FormatError> {
    use TexRule::*;
    const CLEANUP_INPUTS: u32 =
        Math.bit() | Arguments.bit() | TextBefore.bit() | Alignment.bit() | TextAfter.bit();
    let mut changed = 0;
    let mut code_activity = None;
    let mut math_recheck = false;
    let mut column_edits = None;
    let mut batch = crate::edit_plan::LayoutBatch::default();
    let settings = &config.config;
    for &rule in TexRule::ORDER {
        let local = matches!(rule, ColumnSpecs | Trees | PlotTables | Foreach);
        if !local || batch.requires_snapshot() {
            batch.flush(text)?;
        }
        // The two horizontal alignment passes share token positions. When
        // individual trace stages are requested, materialize each stage for
        // its own timing, coordinates and diff.
        if matches!(rule, Consecutive)
            && settings.arguments.align_consecutive
            && settings.comments.align_trailing
            && trace.is_none()
        {
            column_edits = Some(crate::argument_layout::consecutive::collect_edits(
                text, config, parsing,
            )?);
            continue;
        }
        let enabled = match rule {
            Macros => syntax::macro_layout_active(text, config),
            Math => crate::math_layout::is_active(text, config),
            Arguments => crate::argument_layout::is_active(text, config),
            ProseSpaces => {
                kind == FileKind::Tex
                    && settings.wrap.mode == "preserve"
                    && settings.wrap.collapse_spaces
                    && text.contains("  ")
            }
            Description => settings.wrap.align_description,
            TextBefore => crate::wrap::is_active(config, crate::wrap::Stage::Before),
            Alignment => true,
            TextAfter => crate::wrap::is_active(config, crate::wrap::Stage::After),
            WhitespaceAfter if changed & CLEANUP_INPUTS == 0 => continue,
            WhitespaceAfter => true,
            Code => {
                let activity = crate::code_layout::activity(text, config);
                code_activity = Some(activity);
                activity.active()
            }
            MathFinal => {
                changed & Code.bit() != 0
                    && math_recheck
                    && crate::math_layout::is_active(text, config)
            }
            MacrosFinal => changed & (Code.bit() | CLEANUP_INPUTS) != 0,
            WhitespaceFinal => {
                changed & Macros.bit() != 0 || code_activity.is_some_and(|a| a.active())
            }
            ColumnSpecs => {
                settings.alignment.column_spec != crate::alignment::ColumnSpecLayout::Preserve
                    || settings
                        .alignment
                        .rules
                        .values()
                        .any(|rule| rule.column_spec.is_some())
            }
            Trees => {
                settings.trees.mode != crate::tree_layout::TreeMode::Preserve
                    || settings.trees.dirtree && text.contains(r"\dirtree")
            }
            PlotTables => {
                settings.breaks.enabled
                    && (settings.breaks.pgfplots_tables && text.contains(r"\addplot")
                        || settings.breaks.pgfplots_named_tables
                            && text.contains(r"\pgfplotstableread"))
            }
            Foreach => {
                settings.arguments.foreach_lists
                    != crate::argument_layout::ForeachListMode::Preserve
            }
            Consecutive => settings.arguments.align_consecutive,
            TrailingComments => settings.comments.align_trailing,
        };
        if local && enabled && trace.is_none() {
            let mut proposals = rule.local_edits(text, config, parsing)?;
            if batch.affects(&proposals.reads) {
                batch.flush(text)?;
                proposals = rule.local_edits(text, config, parsing)?;
            }
            batch.queue(text, proposals);
            continue;
        }
        let edited = optional_stage(
            rule.name(),
            text,
            trace,
            details,
            config,
            enabled,
            |source| match rule {
                Macros | MacrosFinal => {
                    syntax::format_macro_definitions_with_context(source, config, parsing)
                }
                Math | MathFinal => {
                    crate::math_layout::format_math_with_context(source, kind, config, parsing)
                }
                Arguments => crate::argument_layout::format_arguments_with_context(
                    source, config, kind, parsing,
                ),
                ProseSpaces => crate::wrap::collapse_prose_spaces(source, config, parsing),
                Description => crate::description::format(source, config, parsing),
                TextBefore => crate::wrap::format_text_with_context(
                    source,
                    config,
                    crate::wrap::Stage::Before,
                    parsing,
                ),
                Alignment => {
                    crate::alignment::format_alignment_after_indent(source, config, parsing)
                }
                TextAfter => crate::wrap::format_text_with_context(
                    source,
                    config,
                    crate::wrap::Stage::After,
                    parsing,
                ),
                WhitespaceAfter | WhitespaceFinal => {
                    clean_whitespace(source, config, parsing, None)
                }
                Code => {
                    let output = crate::code_layout::format_code_with_context(
                        source,
                        kind,
                        config,
                        parsing,
                        code_activity.expect("code eligibility checked"),
                    )?;
                    math_recheck = output.math_recheck;
                    Ok(output.text)
                }
                ColumnSpecs | Trees | PlotTables | Foreach => rule
                    .local_edits(source, config, parsing)?
                    .edits
                    .apply_first_wins(source),
                Consecutive => crate::argument_layout::consecutive::format(source, config, parsing),
                TrailingComments => crate::wrap::align_comments_after_edits(
                    source,
                    config,
                    parsing,
                    column_edits.take().unwrap_or_default(),
                ),
            },
        )?;
        if edited {
            changed |= rule.bit();
        }
    }
    Ok(())
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
    let mut details = crate::trace::DetailJournal::default();
    let output = format_document_inner(source, kind, config, Some(&mut trace), Some(&mut details))?;
    Ok((output, trace, details.records))
}

/// A document terminator inside a macro, example or verbatim block cannot
/// close the root environment. Validate the candidate with the normal parser.
fn document_prefix(
    source: &str,
    config: &CompiledConfig,
) -> Option<(usize, crate::source_model::SourceModel)> {
    if !source.contains(r"\documentclass") {
        return None;
    }
    let content_end = source.trim_end().len();
    let candidate = |(at, _): (usize, &str)| {
        let ("document", false, end) = syntax::env_after_command(source, "end", at + 4)? else {
            return None;
        };
        Some((at, end))
    };
    let candidates = source
        .match_indices(r"\end")
        .filter_map(candidate)
        .filter(|(_, end)| *end < content_end);
    let prefix_end = |end: usize| {
        end + if source[end..].starts_with("\r\n") {
            2
        } else {
            usize::from(source[end..].starts_with('\n'))
        }
    };
    fn root_end(model: &crate::source_model::SourceModel) -> Option<&std::ops::Range<usize>> {
        model.document_class()?;
        if model.has_errors() {
            return None;
        }
        model
            .structure
            .as_ref()?
            .1
            .iter()
            .find(|b| {
                b.kind == BlockKind::Environment && b.name == "document" && b.parent.is_none()
            })
            .map(|b| &b.end)
    }
    let mut candidates = candidates.peekable();
    candidates.peek()?;
    let (_, last) = source.rmatch_indices(r"\end").find_map(candidate)?;
    // Include the remainder of the last candidate's physical line. A short
    // verbatim reader can close immediately after the apparent terminator.
    let limit = if last >= content_end {
        source.len()
    } else {
        syntax::line_end(source, last)
    };
    let model = crate::source_model::SourceModel::analyze(&source[..limit], config);
    if let Some(close) = root_end(&model)
        && source[close.clone()].starts_with(r"\end")
        && matches!(syntax::env_after_command(source, "end", close.start + 4),
            Some(("document", false, end)) if end == close.end)
    {
        let tail = if close.end >= content_end {
            source.len()
        } else {
            prefix_end(close.end)
        };
        if tail == limit {
            return Some((limit, model));
        }
        let prefix = crate::source_model::SourceModel::analyze(&source[..tail], config);
        if root_end(&prefix) == Some(close) {
            return Some((tail, prefix));
        }
    }

    // A real document may be followed by unparseable material. Validate its
    // prefix independently, keeping reader spans from the untruncated line.
    let protected = model.lexical_protection();
    for (at, end) in candidates {
        let i = protected.partition_point(|r| r.end <= at);
        if protected
            .get(i)
            .is_some_and(|r| r.contains(&at) && r.len() < limit)
        {
            continue;
        }
        let tail = prefix_end(end);
        let prefix = crate::source_model::SourceModel::analyze(&source[..tail], config);
        if root_end(&prefix) == Some(&(at..end)) {
            return Some((tail, prefix));
        }
    }
    (limit == source.len()).then_some((limit, model))
}

fn format_document_inner(
    source: &str,
    kind: FileKind,
    config: &CompiledConfig,
    mut trace: Option<&mut Vec<StageTrace>>,
    mut details: Option<&mut crate::trace::DetailJournal>,
) -> Result<FormatOutput, FormatError> {
    let (end, model) = if !config.config.replacement_only
        && matches!(
            kind,
            FileKind::Tex | FileKind::Tikz | FileKind::Style | FileKind::Class
        ) {
        let (end, model) = (kind == FileKind::Tex)
            .then(|| document_prefix(source, config))
            .flatten()
            .unwrap_or_else(|| {
                (
                    source.len(),
                    crate::source_model::SourceModel::analyze(source, config),
                )
            });
        (end, Some(model))
    } else {
        (source.len(), None)
    };
    // A prefix ends at an internal source boundary. Apply the user's
    // final-newline policy only when formatting the physical end of the file.
    let final_newline = if end == source.len() {
        config.config.output.final_newline
    } else {
        FinalNewline::Preserve
    };
    match format_document_stages(
        &source[..end],
        kind,
        config,
        model,
        final_newline,
        trace.as_deref_mut(),
        details.as_deref_mut(),
    ) {
        Err(message @ (FormatError::Syntax(_) | FormatError::StructuralMismatch { .. })) => {
            if let Some(trace) = trace {
                trace.clear();
            }
            if let Some(details) = details {
                details.records.clear();
            }
            Ok(FormatOutput {
                text: source.into(),
                diagnostics: vec![Diagnostic {
                    severity: Severity::Error,
                    message: format!(
                        "Source retained because a formatting stage could not safely analyze an intermediate snapshot: {message}"
                    ),
                    span: 0..end,
                }],
            })
        }
        Ok(mut output) if end < source.len() => {
            if output
                .diagnostics
                .iter()
                .any(|d| d.severity == Severity::Error)
            {
                output.text = source.into();
            } else {
                output.text.push_str(&source[end..]);
                output.diagnostics.push(Diagnostic {
                    severity: Severity::Warning,
                    message: "Nonexecuted source after \\end{document} retained unchanged".into(),
                    span: end..source.len(),
                });
            }
            Ok(output)
        }
        result => result,
    }
}

fn format_document_stages(
    source: &str,
    kind: FileKind,
    config: &CompiledConfig,
    model: Option<crate::source_model::SourceModel>,
    final_newline: FinalNewline,
    mut trace: Option<&mut Vec<StageTrace>>,
    mut details: Option<&mut crate::trace::DetailJournal>,
) -> Result<FormatOutput, FormatError> {
    let original_source = source;
    if let Some(model) = &model
        && model.has_errors()
    {
        let mut diagnostics = Vec::new();
        let text = stage(
            "syntax-recovery",
            source,
            &mut trace,
            &mut details,
            config,
            || {
                if let Some(output) = syntax::recovery::Recovery::format(
                    source,
                    kind,
                    config,
                    model,
                    |masked, model| {
                        format_document_stages(
                            masked,
                            kind,
                            config,
                            Some(model),
                            final_newline,
                            None,
                            None,
                        )
                    },
                ) {
                    diagnostics = output.diagnostics;
                    Ok(output.text)
                } else {
                    diagnostics.extend_from_slice(model.diagnostics());
                    Ok(model.reconstruct())
                }
            },
        )?;
        return Ok(FormatOutput { text, diagnostics });
    }
    let bom = source.starts_with('\u{feff}');
    if bom && let Some(journal) = details.as_deref_mut() {
        journal.history.record(vec![(0..3, 0..0)]);
    }
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let mut diagnostics = model
        .as_ref()
        .map_or_else(Vec::new, |model| model.diagnostics().to_vec());
    let mut parsing = ParseContext::with_model(config, model);
    let mut history = crate::edit_plan::EditHistory::default();
    let record_positions = !config.config.replacement_only
        && matches!(
            kind,
            FileKind::Bibtex | FileKind::Bst | FileKind::Dtx | FileKind::Ins
        );
    let detailed = details.is_some();
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
        embedded = stage_recording(
            "embedded-bibtex",
            source,
            &mut trace,
            &mut details,
            config,
            || {
                let text = crate::bibtex::format_embedded(
                    source,
                    config,
                    &mut parsing,
                    usize::from(bom) * 3,
                    &mut diagnostics,
                    (record_positions || detailed).then_some(&mut history),
                )?;
                Ok((text, detailed.then(|| history.clone())))
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
    let mut text = stage_recording(
        "replacements-before",
        source,
        &mut trace,
        &mut details,
        config,
        || {
            let mut replacements = crate::edit_plan::EditHistory::default();
            let text = crate::replacements::apply_recording(
                source,
                kind,
                config,
                crate::replacements::When::Before,
                (record_positions || detailed).then_some(&mut replacements),
            )?;
            if record_positions {
                history.append(replacements.clone());
            }
            Ok((text, detailed.then_some(replacements)))
        },
    )?;
    let original_diagnostic = |mut diagnostic: Diagnostic| {
        let span = history.original_range(diagnostic.span);
        let start = usize::from(bom) * '\u{feff}'.len_utf8();
        diagnostic.span = span.start + start..span.end + start;
        diagnostic
    };
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
                diagnostics.extend(output.diagnostics.into_iter().map(original_diagnostic));
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
            let mut syntax_error = false;
            text = stage("docstrip", &text, &mut trace, &mut details, config, || {
                let output = match crate::docstrip::format_docstrip(&text, kind, config) {
                    Ok(output) => output,
                    Err(error) => {
                        let Some(at) = error.byte_offset().filter(|&at| text.is_char_boundary(at))
                        else {
                            return Err(error);
                        };
                        let end = at + text[at..].chars().next().map_or(0, char::len_utf8);
                        let source_start = |at| {
                            text.is_char_boundary(at).then(|| {
                                history.original_range(at..at).start
                                    + usize::from(bom) * '\u{feff}'.len_utf8()
                            })
                        };
                        diagnostics.push(original_diagnostic(Diagnostic {
                            severity: Severity::Error,
                            span: at..end,
                            message: error.map_byte_offsets(source_start).to_string(),
                        }));
                        syntax_error = true;
                        return Ok(text.clone());
                    }
                };
                diagnostics.extend(output.diagnostics.into_iter().map(original_diagnostic));
                Ok(output.text)
            })?;
            if syntax_error {
                return Ok(FormatOutput {
                    text: original_source.to_owned(),
                    diagnostics,
                });
            }
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
                text = planned_stage(
                    "whitespace-before",
                    &text,
                    &mut trace,
                    &mut details,
                    config,
                    |history| clean_whitespace(&text, config, &mut parsing, history),
                )?;
            }
            if config.config.breaks.enabled {
                text = planned_stage(
                    "structure",
                    &text,
                    &mut trace,
                    &mut details,
                    config,
                    |history| apply_breaks(&text, config, &mut parsing, history),
                )?;
            }
            text = planned_stage(
                "indentation",
                &text,
                &mut trace,
                &mut details,
                config,
                |history| {
                    indent_with_argument_ranges(&text, kind, config, &mut parsing, &[], history)
                },
            )?;
            format_tex_rules(
                &mut text,
                kind,
                config,
                &mut parsing,
                &mut trace,
                &mut details,
            )?;
        }
    }
    if crate::replacements::is_active(config, crate::replacements::When::After) {
        text = planned_stage(
            "replacements-after",
            &text,
            &mut trace,
            &mut details,
            config,
            |history| {
                crate::replacements::apply_recording(
                    &text,
                    kind,
                    config,
                    crate::replacements::When::After,
                    history,
                )
            },
        )?;
    } else if let Some(trace) = trace.as_deref_mut() {
        trace.push(StageTrace {
            name: "replacements-after",
            changed: false,
            input_bytes: text.len(),
            output_bytes: text.len(),
            elapsed: Duration::ZERO,
        });
    }
    let final_started = trace.is_some().then(Instant::now);
    let before_final = details.is_some().then(|| text.clone());
    let input_bytes = text.len();
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
    let protected = boundary_protection(&text)?;
    let mut edits = crate::edit_plan::EditPlan::new();
    if bom {
        edits.replace(0..0, "\u{feff}");
    }
    let remove_trailing_endings = if final_newline == FinalNewline::Never {
        let mut start = text.len();
        while start > 0 && text.as_bytes()[start - 1] == b'\n' {
            let newline = start - 1;
            if syntax::is_protected(newline, &protected) {
                break;
            }
            start -= usize::from(start >= 2 && text.as_bytes()[start - 2] == b'\r') + 1;
        }
        (start < text.len()).then_some(start..text.len())
    } else {
        None
    };
    if config.config.output.line_ending != LineEnding::Preserve {
        let retained_end = remove_trailing_endings
            .as_ref()
            .map_or(text.len(), |range| range.start);
        let mut offset = 0;
        for line in text.split_inclusive('\n') {
            if line.ends_with('\n') {
                let newline = offset + line.len() - 1;
                if newline < retained_end && !syntax::is_protected(newline, &protected) {
                    let start = if line.ends_with("\r\n") {
                        newline - 1
                    } else {
                        newline
                    };
                    if &text[start..newline + 1] != ending {
                        edits.replace(start..newline + 1, ending);
                    }
                }
            }
            offset += line.len();
        }
    }
    match final_newline {
        FinalNewline::Always
            if !text.is_empty()
                && !text.ends_with('\n')
                && !syntax::is_protected(text.len() - 1, &protected) =>
        {
            edits.replace(text.len()..text.len(), ending);
        }
        FinalNewline::Never => {
            if let Some(range) = remove_trailing_endings {
                edits.replace(range, "");
            }
        }
        _ => (),
    }
    let changed = !edits.is_empty();
    let mut boundaries = crate::edit_plan::EditHistory::default();
    if changed {
        text = edits.apply_recording(&text, detailed.then_some(&mut boundaries))?;
    }
    if let Some(trace) = trace {
        if let Some(details) = details {
            details.record(
                "file-boundaries",
                before_final.as_deref().expect("detailed trace input"),
                &text,
                config,
                Some(boundaries),
            );
        }
        trace.push(StageTrace {
            name: "file-boundaries",
            changed,
            input_bytes,
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

pub(crate) fn clean_whitespace(
    source: &str,
    config: &CompiledConfig,
    parsing: &mut ParseContext<'_>,
    history: Option<&mut crate::edit_plan::EditHistory>,
) -> Result<String, FormatError> {
    // Most layout stages leave trailing whitespace and blank-line runs clean.
    // Check the bytes before rebuilding semantic protection for a new snapshot.
    // Protected text can trigger this conservative check, but cannot hide work.
    if !whitespace_cleanup_needed(source, config) {
        return Ok(source.into());
    }
    clean_whitespace_parsed(source, config, parsing, history)
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
    history: Option<&mut crate::edit_plan::EditHistory>,
) -> Result<String, FormatError> {
    let protected = &parsing.snapshot(source, false, false)?.protected;
    let mut edits = crate::edit_plan::EditPlan::new();
    let mut offset = 0usize;
    let mut blanks = 0usize;
    let maximum = config.config.whitespace.blank_lines.limit();
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
            if maximum.is_some_and(|maximum| blanks > maximum) {
                edits.replace(offset..offset + line.len(), "");
                offset += line.len();
                continue;
            }
        } else {
            blanks = 0;
        }
        let trimmed = trailing_content_end(body);
        if config.config.whitespace.remove_trailing
            && trimmed < body.len()
            && !syntax::overlaps(&(offset + trimmed..offset + body.len()), protected)
        {
            edits.replace(offset + trimmed..offset + body.len(), "");
        }
        offset += line.len();
    }
    let (output, mappings) = edits.apply_ordered_with_mappings(source)?;
    parsing.reuse_whitespace_snapshot(source, &output, &mappings);
    if let Some(history) = history {
        history.record(mappings);
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
    indent_with_argument_ranges(source, kind, config, parsing, arguments, None)
}

fn indent_with_argument_ranges(
    source: &str,
    kind: FileKind,
    config: &CompiledConfig,
    parsing: &mut ParseContext<'_>,
    arguments: &[Range<usize>],
    history: Option<&mut crate::edit_plan::EditHistory>,
) -> Result<String, FormatError> {
    let parsed = parsing.snapshot(source, true, true)?;
    let protected = &parsed.protected;
    let argument_literals = if arguments.is_empty() {
        Vec::new()
    } else {
        let analysis = parsed.analysis(&config.config);
        crate::argument_layout::literal_argument_ranges(
            source,
            config,
            protected,
            analysis.declared_commands(),
            analysis.model().tokens(),
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
    let newlines = parsed.document.newlines();
    let indent = &config.config.indent;
    let algorithm_lines = if indent.algpseudocode && source.contains("algorithmic") {
        let analysis = parsed.analysis(&config.config);
        crate::packages::code_algorithms::indent_lines(source, config, protected, &analysis)
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
        let analysis = parsed.analysis(&config.config);
        let commands = analysis.declared_commands();
        let environments = analysis.declared_environments();
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
        let analysis = parsed.analysis(&config.config);
        for (marker, parent) in
            crate::package_tables::code_after_markers(blocks, &config.config, &analysis)
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
    let mut edits = crate::edit_plan::EditPlan::new();
    let mut active: BTreeMap<usize, &str> = BTreeMap::new();
    let mut event = 0usize;
    let mut cached_prefix = String::new();
    let mut dirty = true;
    let mut blanks = 0usize;
    for (offset, line) in parsed.document.lines() {
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
            if maximum.is_some_and(|maximum| blanks > maximum) {
                edits.replace(offset..offset + line.len(), "");
            } else if content_end < body.len() {
                edits.replace(offset + content_end..offset + body.len(), "");
            }
            continue;
        }
        blanks = 0;
        if in_preamble && preamble_argument.is_none() {
            if content_end < body.len() {
                edits.replace(offset + content_end..offset + body.len(), "");
            }
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
        if prefix.as_ref() != &line[..start] {
            edits.replace(offset..offset + start, prefix.into_owned());
        }
        if content_end < body.len() {
            edits.replace(offset + content_end..offset + body.len(), "");
        }
    }
    let (output, mappings) = edits.apply_ordered_with_mappings(source)?;
    parsing.reuse_whitespace_snapshot(source, &output, &mappings);
    if let Some(history) = history {
        history.record(mappings);
    }
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
    before: (BreakMode, EditOrigin),
    after: (BreakMode, EditOrigin),
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
    if analysis.declared_environments().contains(name) {
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
) -> Vec<(usize, BreakMode, EditOrigin)> {
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
                let origin = if config.config.breaks.rules.contains_key(block.name.as_str()) {
                    EditOrigin::Named
                } else if config.config.breaks.categories.contains_key("environment") {
                    EditOrigin::Category
                } else {
                    EditOrigin::TableRow
                };
                let explicit = origin.priority() > 0;
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
                output.push((end, mode, origin));
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
) -> Vec<(usize, BreakMode, bool, EditOrigin)> {
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
                let origin = if config.config.breaks.rules.contains_key(name) {
                    EditOrigin::Named
                } else if named.is_some() {
                    EditOrigin::Category
                } else if config.config.breaks.rules.contains_key(block.name.as_str()) {
                    EditOrigin::Named
                } else if config.config.breaks.categories.contains_key("environment") {
                    EditOrigin::Category
                } else {
                    EditOrigin::TableRule
                };
                output.push((token.span.start, before, true, origin));
                output.push((end, after, false, origin));
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
    history: Option<&mut crate::edit_plan::EditHistory>,
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
    let analysis = &parsed.analysis(&config.config);
    let tokens = parsed.tokens.as_ref().expect("parsed tokens");
    let mut blocks: Cow<'_, [Block]> =
        Cow::Borrowed(parsed.blocks.as_deref().expect("parsed structure"));
    let verbatim = syntax::verbatim_blocks_with_ranges(source, protected);
    if !verbatim.is_empty() {
        blocks.to_mut().extend(verbatim);
    }
    let declared_environments = analysis.declared_environments();
    let declared_commands = analysis.declared_commands();
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
    .then(|| parsed.document.newlines());
    let mut boundaries: BTreeMap<usize, Boundary> = BTreeMap::new();
    let mut preserved_path_begins = std::collections::HashSet::new();
    let mut record = |pos, mode, before, origin: EditOrigin| {
        if mode == BreakMode::Preserve {
            return;
        }
        let entry = boundaries.entry(pos).or_insert(Boundary {
            pos,
            before: (BreakMode::Preserve, EditOrigin::Layout),
            after: (BreakMode::Preserve, EditOrigin::Layout),
        });
        // A tight TikZ path needs a comment to avoid introducing a TeX space.
        // Its comment line also satisfies a configured line on the other side.
        if origin == EditOrigin::TikzPath
            && before
            && mode == BreakMode::CommentLine
            && entry.after.0 == BreakMode::Line
        {
            entry.after.0 = BreakMode::CommentLine;
        }
        let side = if before {
            &mut entry.before
        } else {
            &mut entry.after
        };
        if origin.priority() >= side.1.priority() {
            *side = (mode, origin);
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
        let origin = if !environment_argument
            && config.config.breaks.rules.contains_key(block.name.as_str())
        {
            EditOrigin::Named
        } else if config
            .config
            .breaks
            .categories
            .contains_key(block.kind.category())
        {
            EditOrigin::Category
        } else {
            EditOrigin::Layout
        };
        let explicit_rule = origin.priority() > 0;
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
            let mut record = |pos, mode, before| record(pos, mode, before, origin);
            let split_outer = block.kind == BlockKind::Environment
                && !explicit_rule
                && environment_can_split_outer_boundaries(
                    analysis,
                    block,
                    &blocks,
                    config,
                    declared_environments,
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
                        declared_commands,
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
    for (pos, mode, origin) in row_boundaries(
        source,
        tokens,
        &blocks,
        declared_environments,
        &boundary_index,
    ) {
        record(pos, mode, false, origin);
    }
    for (pos, mode, before, origin) in table_rule_boundaries(
        source,
        tokens,
        &blocks,
        declared_environments,
        declared_commands,
        &boundary_index,
    ) {
        record(pos, mode, before, origin);
    }
    for (pos, mode, before) in crate::tikz::boundaries(source, &blocks, config, protected) {
        // Only automatic path boundaries yield to an explicit environment
        // body preserve rule. Explicit path rules are recorded above.
        if before && preserved_path_begins.contains(&pos) {
            continue;
        }
        record(pos, mode, before, EditOrigin::TikzPath);
    }
    let mut edits = crate::edit_plan::EditPlan::new();
    for boundary in boundaries.values() {
        for (before, (mode, origin)) in [(true, boundary.before), (false, boundary.after)] {
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
            let already_satisfied = match mode {
                BreakMode::Line | BreakMode::CommentLine => whitespace.contains('\n'),
                BreakMode::BlankLine => whitespace.matches('\n').count() >= 2,
                _ => false,
            };
            if already_satisfied {
                // Keep a configured boundary ahead of conflicting lower-priority
                // edits even if its own replacement would be a no-op.
                if origin.priority() > 0 {
                    edits.replace_from(origin, start..end, whitespace);
                }
                continue;
            }
            let replacement = match mode {
                BreakMode::Preserve => unreachable!(),
                BreakMode::Join => {
                    // A comment immediately before this boundary owns its newline.
                    let line = source[..start].rsplit('\n').next().unwrap_or("");
                    if comment_position(line).is_some() {
                        continue;
                    }
                    if whitespace.is_empty() {
                        "".into()
                    } else {
                        " ".into()
                    }
                }
                BreakMode::Line | BreakMode::CommentLine
                    if !before && source[end..].starts_with('%') =>
                {
                    continue;
                }
                BreakMode::Line => ending.into(),
                BreakMode::CommentLine => Cow::Owned(format!("%{ending}")),
                BreakMode::BlankLine | BreakMode::ResetBlankLine => Cow::Owned(ending.repeat(2)),
            };
            if start == 0 && before && mode != BreakMode::Join {
                continue;
            }
            edits.replace_from(origin, start..end, replacement);
        }
    }
    edits.resolve_priorities().apply_recording(source, history)
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

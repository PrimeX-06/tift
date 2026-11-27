//! Source-preserving alignment of TeX alignment environments.
//!
//! Delimiters are located once per environment. TeX groups, comments, escaped
//! characters, protected spans, and child environments are opaque to that scan.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::{Arc, LazyLock};

use regex::Regex;
use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::{CompiledConfig, FormatError};

#[path = "column_spec.rs"]
mod column_spec;
mod equation_labels;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ColumnSpecLayout {
    #[default]
    Preserve,
    Auto,
    Expand,
}

pub(crate) use column_spec::parts as comma_parts;
pub(crate) use column_spec::preamble_edits;

const DEFAULT_ENVIRONMENTS: &[&str] = &[
    "tabu",
    "matrix",
    "listabla",
    "align",
    "align*",
    "alignat",
    "alignat*",
    "aligned",
    "alignedat",
    "eqnarray",
    "eqnarray*",
    "bmatrix",
    "Bmatrix",
    "cases",
    "flalign",
    "flalign*",
    "pmatrix",
    "split",
    "vmatrix",
    "Vmatrix",
    "cases*",
    "dcases",
    "dcases*",
    "rcases",
    "rcases*",
    "drcases",
    "drcases*",
    "NiceMatrix",
    "pNiceMatrix",
    "bNiceMatrix",
    "BNiceMatrix",
    "vNiceMatrix",
    "VNiceMatrix",
    "NiceArray",
    "pNiceArray",
    "bNiceArray",
    "BNiceArray",
    "vNiceArray",
    "VNiceArray",
    "NiceArrayWithDelims",
    "pNiceArrayC",
    "bNiceArrayC",
    "BNiceArrayC",
    "vNiceArrayC",
    "VNiceArrayC",
    "NiceArrayCwithDelims",
    "pNiceArrayRC",
    "bNiceArrayRC",
    "BNiceArrayRC",
    "vNiceArrayRC",
    "VNiceArrayRC",
    "NiceArrayRCwithDelims",
    "spreadtab",
    "gathered",
    "smallmatrix",
    "smallmatrix*",
    "matrix*",
    "pmatrix*",
    "bmatrix*",
    "Bmatrix*",
    "vmatrix*",
    "Vmatrix*",
];

const DEFAULT_ROW_BREAK: &str = r"\\\\\*?|\\tabularnewline\*?";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Width {
    Graphemes,
    #[default]
    Display,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Justification {
    #[default]
    Left,
    Right,
    Center,
    /// Read horizontal alignment from each literal table column specification.
    Columns,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ShortRows {
    #[default]
    Align,
    Preserve,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RegexEngine {
    Rust,
    #[default]
    Enhanced,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct AlignmentConfig {
    pub enabled: bool,
    pub width: Width,
    pub environments: Vec<String>,
    pub delimiter: String,
    pub row_break: String,
    pub delimiter_group: Option<usize>,
    pub row_break_group: Option<usize>,
    pub regex_engine: RegexEngine,
    pub backtrack_limit: usize,
    pub spaces_before: usize,
    pub spaces_after: usize,
    pub spaces_before_row_break: usize,
    #[serde(rename = "align-row-breaks")]
    pub align_double_backslash: bool,
    #[serde(rename = "align-last-row-break")]
    pub align_final_double_backslash: bool,
    pub align_content_after_row_break: bool,
    pub spaces_after_row_break: usize,
    pub format_children: bool,
    pub multicolumn: bool,
    pub align_decimals: bool,
    pub align_equation_labels: bool,
    pub align_by_section: bool,
    /// Expand multiple arrows attached to one tikz-cd node.
    pub tikzcd_arrows: bool,
    /// Fold table rows wider than this many source columns. Zero disables folding.
    pub row_wrap_columns: Option<usize>,
    pub column_spec: ColumnSpecLayout,
    pub short_rows: ShortRows,
    pub justification: Justification,
    pub delimiter_justification: Justification,
    /// Cells matching these regular expressions do not contribute to widths.
    pub dont_measure: Vec<String>,
    pub dont_measure_rows: Vec<String>,
    pub dont_measure_literals: Vec<String>,
    pub dont_measure_largest: bool,
    pub rules: BTreeMap<String, AlignmentRule>,
}

impl Default for AlignmentConfig {
    fn default() -> Self {
        let mut seen = std::collections::HashSet::new();
        Self {
            enabled: true,
            width: Width::Display,
            environments: DEFAULT_ENVIRONMENTS
                .iter()
                .copied()
                .chain(crate::packages::alignment_environments())
                .filter(|name| seen.insert(*name))
                .map(String::from)
                .collect(),
            delimiter: "&".into(),
            row_break: DEFAULT_ROW_BREAK.into(),
            delimiter_group: None,
            row_break_group: None,
            regex_engine: RegexEngine::Enhanced,
            backtrack_limit: 1_000_000,
            spaces_before: 1,
            spaces_after: 1,
            spaces_before_row_break: 1,
            align_double_backslash: true,
            align_final_double_backslash: true,
            align_content_after_row_break: false,
            spaces_after_row_break: 1,
            format_children: true,
            multicolumn: true,
            align_decimals: true,
            align_equation_labels: true,
            align_by_section: false,
            tikzcd_arrows: false,
            row_wrap_columns: None,
            column_spec: ColumnSpecLayout::Preserve,
            short_rows: ShortRows::Align,
            justification: Justification::Columns,
            delimiter_justification: Justification::Left,
            dont_measure: Vec::new(),
            dont_measure_rows: Vec::new(),
            dont_measure_literals: Vec::new(),
            dont_measure_largest: false,
            rules: BTreeMap::new(),
        }
    }
}

/// Partial, per-environment overrides. An entry also enables a custom name.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct AlignmentRule {
    pub enabled: Option<bool>,
    pub width: Option<Width>,
    pub delimiter: Option<String>,
    pub row_break: Option<String>,
    pub delimiter_group: Option<usize>,
    pub row_break_group: Option<usize>,
    pub regex_engine: Option<RegexEngine>,
    pub backtrack_limit: Option<usize>,
    pub spaces_before: Option<usize>,
    pub spaces_after: Option<usize>,
    pub spaces_before_row_break: Option<usize>,
    #[serde(rename = "align-row-breaks")]
    pub align_double_backslash: Option<bool>,
    #[serde(rename = "align-last-row-break")]
    pub align_final_double_backslash: Option<bool>,
    pub align_content_after_row_break: Option<bool>,
    pub spaces_after_row_break: Option<usize>,
    pub format_children: Option<bool>,
    pub multicolumn: Option<bool>,
    pub align_decimals: Option<bool>,
    pub align_equation_labels: Option<bool>,
    pub align_by_section: Option<bool>,
    pub tikzcd_arrows: Option<bool>,
    pub row_wrap_columns: Option<usize>,
    pub column_spec: Option<ColumnSpecLayout>,
    pub short_rows: Option<ShortRows>,
    pub justification: Option<Justification>,
    pub delimiter_justification: Option<Justification>,
    pub dont_measure: Option<Vec<String>>,
    pub dont_measure_rows: Option<Vec<String>>,
    pub dont_measure_literals: Option<Vec<String>>,
    pub dont_measure_largest: Option<bool>,
}

#[derive(Debug)]
pub struct CompiledAlignment {
    rules: BTreeMap<String, Arc<CompiledRule>>,
}

#[derive(Debug)]
struct CompiledRule {
    settings: AlignmentConfig,
    delimiter: Matcher,
    row_break: Matcher,
    dont_measure: Vec<Matcher>,
    dont_measure_rows: Vec<Matcher>,
}

pub fn compile(config: &AlignmentConfig) -> Result<CompiledAlignment, String> {
    let mut rules = BTreeMap::new();
    let mut base = config.clone();
    base.environments.clear();
    base.rules.clear();
    let default = Arc::new(compile_rule(base.clone(), "default")?);
    for name in &config.environments {
        let package_default = crate::packages::tikz_diagrams::default_alignment_delimiter(name);
        if config.delimiter == "&"
            && let Some(delimiter) = package_default
        {
            let mut settings = base.clone();
            settings.delimiter = delimiter.into();
            rules.insert(name.clone(), Arc::new(compile_rule(settings, name)?));
        } else {
            rules.insert(name.clone(), Arc::clone(&default));
        }
    }
    // These displays normally have no source-column layout of their own.
    if config.align_equation_labels
        && config.environments == AlignmentConfig::default().environments
    {
        for name in equation_labels::ADDITIONAL_ENVIRONMENTS {
            rules.insert((*name).into(), Arc::clone(&default));
        }
    }
    for (name, rule) in &config.rules {
        let mut settings = rules
            .get(name)
            .map_or_else(|| base.clone(), |rule| rule.settings.clone());
        macro_rules! override_value {
                ($($field:ident),+ $(,)?) => { $(if let Some(value) = &rule.$field { settings.$field = value.clone(); })+ };
            }
        override_value!(
            enabled,
            width,
            delimiter,
            row_break,
            regex_engine,
            backtrack_limit,
            spaces_before,
            spaces_after,
            spaces_before_row_break,
            align_double_backslash,
            align_final_double_backslash,
            align_content_after_row_break,
            spaces_after_row_break,
            format_children,
            multicolumn,
            align_decimals,
            align_equation_labels,
            align_by_section,
            tikzcd_arrows,
            column_spec,
            short_rows,
            justification,
            delimiter_justification,
            dont_measure,
            dont_measure_rows,
            dont_measure_literals,
            dont_measure_largest
        );
        if let Some(group) = rule.delimiter_group {
            settings.delimiter_group = Some(group);
        }
        if rule.row_wrap_columns.is_some() {
            settings.row_wrap_columns = rule.row_wrap_columns;
        }
        if let Some(group) = rule.row_break_group {
            settings.row_break_group = Some(group);
        }
        rules.insert(name.clone(), Arc::new(compile_rule(settings, name)?));
    }
    Ok(CompiledAlignment { rules })
}

fn compile_rule(settings: AlignmentConfig, name: &str) -> Result<CompiledRule, String> {
    if settings.row_wrap_columns.is_some_and(|n| n > 1_000_000) {
        return Err(format!(
            "alignment {name:?}: row-wrap-columns exceeds 1000000"
        ));
    }
    if settings.delimiter_justification == Justification::Columns {
        return Err(format!(
            "alignment {name:?}: delimiter-justification must be left, right, or center"
        ));
    }
    for (key, value) in [
        ("spaces-before", settings.spaces_before),
        ("spaces-after", settings.spaces_after),
        ("spaces-before-row-break", settings.spaces_before_row_break),
        ("spaces-after-row-break", settings.spaces_after_row_break),
    ] {
        if value > 256 {
            return Err(format!("alignment {name:?}: {key} exceeds 256"));
        }
    }
    if settings.backtrack_limit == 0 {
        return Err(format!(
            "alignment {name:?}: backtrack-limit must be positive"
        ));
    }
    let delimiter = compile_pattern(&settings.delimiter, name, "delimiter", &settings, true)?;
    let row_break = compile_pattern(&settings.row_break, name, "row-break", &settings, true)?;
    for (field, group, matcher) in [
        ("delimiter-group", settings.delimiter_group, &delimiter),
        ("row-break-group", settings.row_break_group, &row_break),
    ] {
        if group.is_some_and(|group| group >= matcher.captures_len()) {
            return Err(format!(
                "alignment {name:?}: {field} exceeds available capture groups"
            ));
        }
    }
    let dont_measure = settings
        .dont_measure
        .iter()
        .map(|pattern| compile_pattern(pattern, name, "dont-measure", &settings, false))
        .collect::<Result<_, _>>()?;
    let dont_measure_rows = settings
        .dont_measure_rows
        .iter()
        .map(|pattern| compile_pattern(pattern, name, "dont-measure-rows", &settings, false))
        .collect::<Result<_, _>>()?;
    Ok(CompiledRule {
        settings,
        delimiter,
        row_break,
        dont_measure,
        dont_measure_rows,
    })
}

fn compile_pattern(
    pattern: &str,
    name: &str,
    field: &str,
    settings: &AlignmentConfig,
    delimiter: bool,
) -> Result<Matcher, String> {
    let regex = match Regex::new(pattern) {
        Ok(regex) => Matcher::Rust(regex),
        Err(error) if settings.regex_engine == RegexEngine::Rust => {
            return Err(format!("alignment {name:?}: {field}: {error}"));
        }
        Err(_) => Matcher::Enhanced(
            fancy_regex::RegexBuilder::new(pattern)
                .backtrack_limit(settings.backtrack_limit)
                .build()
                .map_err(|e| format!("alignment {name:?}: {field}: {e}"))?,
        ),
    };
    if delimiter && regex.is_match("").map_err(|e| e.to_string())? {
        return Err(format!(
            "alignment {name:?}: {field} must consume characters"
        ));
    }
    Ok(regex)
}

#[derive(Debug)]
enum Matcher {
    Rust(Regex),
    Enhanced(fancy_regex::Regex),
}

impl Matcher {
    fn captures_len(&self) -> usize {
        match self {
            Self::Rust(regex) => regex.captures_len(),
            Self::Enhanced(regex) => regex.captures_len(),
        }
    }
    fn is_match(&self, source: &str) -> Result<bool, FormatError> {
        match self {
            Self::Rust(regex) => Ok(regex.is_match(source)),
            Self::Enhanced(regex) => regex
                .is_match(source)
                .map_err(|e| FormatError::Syntax(format!("alignment regex: {e}"))),
        }
    }
    fn find_iter<'r, 's>(&'r self, source: &'s str, group: Option<usize>) -> Matches<'r, 's> {
        match (self, group) {
            (Self::Rust(regex), None) => Matches::Rust(regex.find_iter(source)),
            (Self::Enhanced(regex), None) => Matches::Enhanced(regex.find_iter(source)),
            (Self::Rust(regex), Some(group)) => {
                Matches::RustCapture(regex.captures_iter(source), group)
            }
            (Self::Enhanced(regex), Some(group)) => {
                Matches::EnhancedCapture(regex.captures_iter(source), group)
            }
        }
    }
}

enum Matches<'r, 's> {
    Rust(regex::Matches<'r, 's>),
    Enhanced(fancy_regex::Matches<'r, 's>),
    RustCapture(regex::CaptureMatches<'r, 's>, usize),
    EnhancedCapture(fancy_regex::CaptureMatches<'r, 's>, usize),
}
impl Iterator for Matches<'_, '_> {
    type Item = Result<Range<usize>, FormatError>;
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Rust(matches) => matches.next().map(|m| Ok(m.range())),
            Self::Enhanced(matches) => matches.next().map(|result| {
                result
                    .map(|m| m.start()..m.end())
                    .map_err(|e| FormatError::Syntax(format!("alignment regex: {e}")))
            }),
            Self::RustCapture(matches, group) => loop {
                let captures = matches.next()?;
                if let Some(matched) = captures.get(*group) {
                    return Some(Ok(matched.range()));
                }
            },
            Self::EnhancedCapture(matches, group) => loop {
                match matches.next()? {
                    Ok(captures) => {
                        if let Some(matched) = captures.get(*group) {
                            return Some(Ok(matched.start()..matched.end()));
                        }
                    }
                    Err(error) => {
                        return Some(Err(FormatError::Syntax(format!(
                            "alignment regex: {error}"
                        ))));
                    }
                }
            },
        }
    }
}

fn any_match(patterns: &[Matcher], source: &str) -> Result<bool, FormatError> {
    for pattern in patterns {
        if pattern.is_match(source)? {
            return Ok(true);
        }
    }
    Ok(false)
}

#[derive(Debug)]
struct Block {
    begin: usize,
    body: Range<usize>,
    end: usize,
    name: String,
    children: Vec<usize>,
    environment: bool,
    markup_indent: Option<String>,
}

/// Format selected alignment environments, retaining their original boundaries.
pub fn format_alignment(source: &str, config: &CompiledConfig) -> Result<String, FormatError> {
    format_alignment_impl(source, config, false)
}

pub(crate) fn format_alignment_after_indent(
    source: &str,
    config: &CompiledConfig,
    parsing: &mut crate::parse_context::ParseContext<'_>,
) -> Result<String, FormatError> {
    format_alignment_with_context(source, config, true, parsing)
}

fn format_alignment_impl(
    source: &str,
    config: &CompiledConfig,
    freshly_indented: bool,
) -> Result<String, FormatError> {
    let mut parsing = crate::parse_context::ParseContext::new(config);
    format_alignment_with_context(source, config, freshly_indented, &mut parsing)
}

fn format_alignment_with_context(
    source: &str,
    config: &CompiledConfig,
    freshly_indented: bool,
    parsing: &mut crate::parse_context::ParseContext<'_>,
) -> Result<String, FormatError> {
    if !config.config.alignment.enabled {
        return Ok(source.into());
    }
    let advanced = !config.config.alignment.rules.is_empty();
    let matrix_command = source.match_indices("\\matrix").any(|(start, _)| {
        !crate::syntax::is_escaped(source, start)
            && crate::syntax::command_at(source, start).is_some_and(|(name, _)| name == "matrix")
    });
    let tikz_matrix = source.contains("\\node") && source.contains("matrix");
    let matrix_enabled = config
        .alignment
        .rules
        .get("matrix")
        .is_some_and(|rule| rule.settings.enabled);
    let markup = source.contains("%*");
    // Ordinary documents without an alignment block need no extra token pass.
    if !advanced
        && !matrix_command
        && !tikz_matrix
        && !markup
        && config.config.syntax.environment_pattern.is_none()
        && config.config.syntax.environment_aliases.is_empty()
        && config.config.syntax.inferred_environment_aliases.is_empty()
        && config.config.syntax.environment_alias_events.is_empty()
        && !source.match_indices("\\begin").any(|(start, _)| {
            crate::syntax::command_at(source, start)
                .and_then(|(command, end)| crate::syntax::env_after_command(source, command, end))
                .is_some_and(|(name, begin, _)| {
                    begin
                        && config
                            .alignment
                            .rules
                            .get(name)
                            .is_some_and(|rule| rule.settings.enabled)
                })
        })
    {
        return Ok(source.into());
    }
    let has_code_after = source.contains(r"\CodeAfter");
    let structure_needed =
        advanced || matrix_command || tikz_matrix || source.contains('[') || has_code_after;
    let parsed = parsing.snapshot(source, true, structure_needed)?;
    let protected = &parsed.protected;
    let analysis = parsed.analysis(&config.config);
    let mut data_ends = std::collections::HashMap::new();
    if has_code_after {
        let structure = parsed.blocks.as_deref().expect("parsed structure");
        for (marker, parent) in
            crate::package_tables::code_after_markers(structure, &config.config, &analysis)
        {
            let pos = structure[marker].begin.start;
            let line = source[..pos].rfind('\n').map_or(0, |i| i + 1);
            let end = if source[line..pos].trim_matches([' ', '\t']).is_empty() {
                line
            } else {
                pos
            };
            data_ends
                .entry(structure[parent].begin.start)
                .or_insert(end);
        }
    }
    let cached_tokens = parsed.tokens.as_deref().expect("parsed tokens");
    let declared_column_types = if config
        .alignment
        .rules
        .values()
        .any(|rule| rule.settings.justification == Justification::Columns)
    {
        column_spec::declared_types(source, cached_tokens)
    } else {
        column_spec::DeclaredTypes::default()
    };
    let environments = parsed.environment_ranges(&config.config)?;
    let known_package_environment = crate::packages::is_alignment_environment;
    let check_declarations = environments.iter().any(|environment| {
        known_package_environment(&environment.name)
            || equation_labels::environment(&environment.name)
                && config
                    .alignment
                    .rules
                    .get(&environment.name)
                    .is_some_and(|r| r.settings.align_equation_labels)
    });
    let declared_default =
        |name: &str| check_declarations && analysis.declared_environments().contains(name);
    let declared_ray_commands = if environments
        .iter()
        .any(|environment| crate::package_tables::is_tabularray(&environment.name))
    {
        analysis
            .declared_commands()
            .iter()
            .any(|name| column_spec::TABULARRAY_STYLE_COMMANDS.contains(&name.as_str()))
            || column_spec::TABULARRAY_STYLE_COMMANDS
                .iter()
                .any(|name| config.config.syntax.commands.contains_key(*name))
    } else {
        false
    };
    let mut all_environment_ranges = environments
        .iter()
        .map(|e| e.begin.start..e.end.end)
        .collect::<Vec<_>>();
    let mut blocks = environments
        .iter()
        .filter(|environment| {
            config
                .alignment
                .rules
                .get(&environment.name)
                .is_some_and(|r| r.settings.enabled)
                && !crate::syntax::is_protected(environment.begin.start, protected)
                && (!known_package_environment(&environment.name)
                    || !declared_default(&environment.name)
                    || config
                        .config
                        .alignment
                        .rules
                        .contains_key(&environment.name))
                && (environment.name != "deptext"
                    || config.config.alignment.rules.contains_key("deptext")
                    || config.config.alignment.delimiter != "&"
                    || crate::packages::tikz_diagrams::default_separator_safe(
                        &source[environment.body.start
                            ..crate::layout::environment_header_end(
                                source,
                                environment.body.clone(),
                                &environment.name,
                                &config.config,
                                &analysis,
                            )],
                    ))
        })
        .map(|environment| Block {
            begin: environment.begin.start,
            body: environment.body.clone(),
            end: environment.end.end,
            name: environment.name.clone(),
            children: Vec::new(),
            environment: true,
            markup_indent: None,
        })
        .collect::<Vec<_>>();
    if structure_needed {
        use crate::syntax::BlockKind;
        let tokens = cached_tokens;
        let structural = parsed.blocks.as_deref().expect("parsed structure");
        // A bracket argument belongs to its command. Parent alignments must
        // not see its column/row tokens; nested environments still receive
        // their own alignment blocks and format independently.
        for block in structural.iter().filter(|block| {
            block.kind == BlockKind::OptionalArgument
                && block.parent.is_some_and(|parent| {
                    matches!(
                        structural[parent].kind,
                        BlockKind::Command | BlockKind::Heading
                    )
                })
        }) {
            all_environment_ranges.push(block.begin.start..block.end.end);
        }
        let explicit_matrix = config.config.alignment.rules.contains_key("matrix");
        let matrix_redefined = !explicit_matrix
            && matrix_command
            && (config.config.syntax.commands.contains_key("matrix")
                || analysis.declared_commands().contains("matrix"));
        if matrix_command && !matrix_redefined {
            for token in tokens.iter() {
                if token.kind != crate::syntax::TokenKind::Command
                    || &source[token.span.clone()] != "\\matrix"
                {
                    continue;
                }
                let Some((body, end)) = matrix_command_body(source, token.span.end) else {
                    continue;
                };
                all_environment_ranges.push(token.span.start..end);
                if body.is_empty() || !matrix_enabled {
                    continue;
                }
                blocks.push(Block {
                    begin: token.span.start,
                    body,
                    end,
                    name: "matrix".into(),
                    children: Vec::new(),
                    environment: false,
                    markup_indent: None,
                });
            }
        }
        for block in structural
            .iter()
            .filter(|block| block.kind == BlockKind::TikzPath && block.name == "tikz-matrix-body")
        {
            all_environment_ranges.push(block.begin.start..block.end.end);
            if !block.body.is_empty() && matrix_enabled {
                blocks.push(Block {
                    begin: block.begin.start,
                    body: block.body.clone(),
                    end: block.end.end,
                    name: "matrix".into(),
                    children: Vec::new(),
                    environment: false,
                    markup_indent: None,
                });
            }
        }
        let mut seen_blocks: std::collections::HashSet<_> = blocks
            .iter()
            .map(|block| (block.name.clone(), block.body.start, block.body.end))
            .collect();
        for block in structural.iter().filter(|_| advanced) {
            if matches!(
                block.kind,
                BlockKind::MandatoryArgument | BlockKind::OptionalArgument
            ) && block.parent.is_some_and(|parent| {
                structural[parent].kind == BlockKind::Command && structural[parent].name == "matrix"
            }) {
                // The dedicated command parser selects the matrix body.
                // Options and following groups never become matrix cells.
                continue;
            }
            let allowed = matches!(
                block.kind,
                BlockKind::Special | BlockKind::NamedGroup | BlockKind::KeyValue
            ) || matches!(
                block.kind,
                BlockKind::MandatoryArgument | BlockKind::OptionalArgument
            ) && block.parent.is_some_and(|parent| {
                matches!(
                    structural[parent].kind,
                    BlockKind::Command | BlockKind::Heading
                )
            });
            if !allowed
                || block.body.is_empty()
                || crate::syntax::is_protected(block.begin.start, protected)
            {
                continue;
            }
            let name: &str = if config
                .config
                .alignment
                .rules
                .contains_key(block.name.as_str())
            {
                &block.name
            } else {
                block.name.trim_end_matches('*')
            };
            if !config.config.alignment.rules.contains_key(name)
                || !config
                    .alignment
                    .rules
                    .get(name)
                    .is_some_and(|r| r.settings.enabled)
            {
                continue;
            }
            if !seen_blocks.insert((name.to_owned(), block.body.start, block.body.end)) {
                continue;
            }
            all_environment_ranges.push(block.begin.start..block.end.end);
            blocks.push(Block {
                begin: block.begin.start,
                body: block.body.clone(),
                end: block.end.end,
                name: name.into(),
                children: Vec::new(),
                environment: false,
                markup_indent: None,
            });
        }
    }
    if markup {
        for block in markup_blocks(source, config, protected)? {
            all_environment_ranges.push(block.begin..block.end);
            blocks.push(block);
        }
    }
    all_environment_ranges.sort_by_key(|r| (r.start, std::cmp::Reverse(r.end)));
    blocks.sort_by_key(|b| (b.begin, std::cmp::Reverse(b.end)));
    if blocks.is_empty() {
        return Ok(source.into());
    }
    let uncovered_environment = next_uncovered_ranges(&all_environment_ranges);
    let mut roots = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    for index in 0..blocks.len() {
        while stack
            .last()
            .is_some_and(|&parent| blocks[parent].end <= blocks[index].begin)
        {
            stack.pop();
        }
        if let Some(&parent) = stack.last() {
            if blocks[index].end <= blocks[parent].end {
                blocks[parent].children.push(index);
            } else {
                roots.push(index);
            }
        } else {
            roots.push(index);
        }
        stack.push(index);
    }
    let mut rendered = vec![String::new(); blocks.len()];
    let fold_newline = std::cell::OnceCell::new();
    for index in (0..blocks.len()).rev() {
        let block = &blocks[index];
        let rule = &config.alignment.rules[&block.name];
        let children = if rule.settings.format_children {
            block
                .children
                .iter()
                .map(|&child| (blocks[child].body.clone(), rendered[child].as_str()))
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let content_start = if block.environment {
            crate::layout::environment_header_end(
                source,
                block.body.clone(),
                &block.name,
                &config.config,
                &analysis,
            )
        } else {
            block.body.start
        };
        let opaque = alignment_opaque_ranges(
            content_start..block.body.end,
            &all_environment_ranges,
            &uncovered_environment,
            protected,
        );
        let diagram_separator = (block.name == "tikzcd"
            && rule.settings.delimiter == "&"
            && config
                .config
                .alignment
                .rules
                .get("tikzcd")
                .is_none_or(|rule| rule.delimiter.is_none()))
        .then(|| {
            let header = source[block.body.start..content_start].trim();
            let options = header.strip_prefix('[')?.strip_suffix(']')?;
            crate::packages::keys::definition_entries(options)
                .into_iter()
                .filter_map(|range| options[range].trim_end_matches(',').split_once('='))
                .filter(|(_, value)| !value.contains('%'))
                .rfind(|(key, _)| key.trim() == "ampersand replacement")
                .map(|(_, value)| {
                    let value = value.trim();
                    value
                        .strip_prefix('{')
                        .and_then(|s| s.strip_suffix('}'))
                        .unwrap_or(value)
                        .trim()
                })
        })
        .flatten()
        .filter(|value| {
            crate::syntax::command_at(value, 0).is_some_and(|(_, end)| end == value.len())
        });
        let diagram_rule = if let Some(separator) = diagram_separator {
            let mut settings = rule.settings.clone();
            settings.delimiter = regex::escape(separator);
            Some(compile_rule(settings, "tikzcd").map_err(FormatError::Syntax)?)
        } else {
            None
        };
        let rule = diagram_rule.as_ref().unwrap_or(rule);
        let label_layout = (rule.settings.align_equation_labels
            && block.environment
            && equation_labels::supports_header(
                &block.name,
                &source[block.body.start..content_start],
            )
            && !declared_default(&block.name)
            && !config
                .config
                .syntax
                .environment_arguments
                .contains_key(&block.name)
            && source[content_start..block.body.end].contains(r"\label")
            && !config.config.syntax.commands.contains_key("label")
            && !analysis.declared_commands().contains("label"))
        .then_some(equation_labels::Layout {
            limit: config.config.wrap.columns,
            opaque: &opaque,
        });
        let labels_only = equation_labels::ADDITIONAL_ENVIRONMENTS.contains(&block.name.as_str())
            && !config.config.alignment.environments.contains(&block.name)
            && !config.config.alignment.rules.contains_key(&block.name);
        if labels_only && label_layout.is_none() {
            rendered[index] = apply_edits(source, block.body.clone(), &children);
            continue;
        }
        let columns = if rule.settings.justification == Justification::Columns
            && block.environment
            && !declared_default(&block.name)
            && !config
                .config
                .syntax
                .environment_arguments
                .contains_key(&block.name)
            && (!crate::package_tables::is_tabularray(&block.name) || !declared_ray_commands)
        {
            column_spec::for_environment(
                &block.name,
                &source[block.body.start..content_start],
                &declared_column_types,
            )
        } else {
            None
        };
        let diagram_arrows = block.name == "tikzcd"
            && rule.settings.tikzcd_arrows
            && !declared_default(&block.name)
            && !config
                .config
                .syntax
                .environment_arguments
                .contains_key(&block.name)
            && !["arrow", "ar"]
                .iter()
                .any(|n| config.config.syntax.commands.contains_key(*n));
        let fold = rule
            .settings
            .row_wrap_columns
            .filter(|&n| n > 0)
            .filter(|_| {
                block.environment
                    && !declared_default(&block.name)
                    && (crate::package_tables::is_table(&block.name)
                        || (block.name.contains("NiceArray")
                            && crate::package_environments::column_spec_slot(&block.name)
                                .is_some())
                        || block.name == "spreadtab"
                        || config
                            .config
                            .alignment
                            .rules
                            .get(&block.name)
                            .is_some_and(|rule| rule.row_wrap_columns.is_some()))
            })
            .or_else(|| diagram_arrows.then_some(usize::MAX))
            .map(|limit| RowFold {
                diagram_arrows,
                limit,
                environment_indent: source[..block.begin]
                    .rsplit('\n')
                    .next()
                    .unwrap_or("")
                    .chars()
                    .take_while(|c| matches!(c, ' ' | '\t'))
                    .collect(),
                unit: config.config.indent.unit("environment", &block.name),
                newline: fold_newline.get_or_init(|| {
                    if source.contains("\r\n") {
                        "\r\n"
                    } else {
                        "\n"
                    }
                }),
            });
        let data_end = data_ends
            .get(&block.begin)
            .copied()
            .unwrap_or(block.body.end)
            .clamp(content_start, block.body.end);
        let rows = split_rows(
            source,
            content_start..data_end,
            rule,
            &opaque,
            &block.name,
            fold.is_some(),
            label_layout.is_some(),
        )?;
        let mut formatted = format_rows(
            source,
            rows,
            rule,
            &children,
            &opaque,
            RowLayoutContext {
                freshly_indented,
                tab_width: config.config.indent.width,
                column_spec: columns.as_ref(),
                column_environment: block.environment.then_some(block.name.as_str()),
                label_layout,
                declared_column_types: &declared_column_types,
                fold: fold.as_ref(),
                protected,
            },
        )?;
        if data_end < block.body.end {
            formatted.push_str(&edited_source(source, data_end..block.body.end, &children));
        }
        let formatted = if let Some(indent) = &block.markup_indent {
            indent_markup_body(&formatted, indent, config)?
        } else {
            formatted
        };
        let body = if block.body.start == content_start {
            formatted
        } else {
            let mut body =
                String::with_capacity(content_start - block.body.start + formatted.len());
            body.push_str(&source[block.body.start..content_start]);
            body.push_str(&formatted);
            body
        };
        rendered[index] = body;
    }
    let mut edits = crate::edit_plan::EditPlan::new();
    let mut cursor = 0;
    for root in roots {
        let body = blocks[root].body.clone();
        if body.start < cursor {
            continue;
        }
        cursor = body.end;
        if source[body.clone()] != rendered[root] {
            edits.replace(body, std::mem::take(&mut rendered[root]));
        }
    }
    edits.apply_ordered(source)
}

// Ranges are ordered by start, with the longest first for equal starts. The
// first later range with a larger end is the next one not contained by this
// range. Build these jumps once so parent blocks can skip covered descendants.
fn next_uncovered_ranges(ranges: &[Range<usize>]) -> Vec<usize> {
    let mut next = vec![ranges.len(); ranges.len()];
    let mut stack: Vec<usize> = Vec::new();
    for index in (0..ranges.len()).rev() {
        while stack
            .last()
            .is_some_and(|&later| ranges[later].end <= ranges[index].end)
        {
            stack.pop();
        }
        if let Some(&later) = stack.last() {
            next[index] = later;
        }
        stack.push(index);
    }
    next
}

fn alignment_opaque_ranges(
    body: Range<usize>,
    environments: &[Range<usize>],
    uncovered: &[usize],
    protected: &[Range<usize>],
) -> Vec<Range<usize>> {
    let mut environment_index = environments.partition_point(|range| range.start < body.start);
    let mut environment_ranges = std::iter::from_fn(|| {
        while let Some(range) = environments.get(environment_index) {
            if range.start >= body.end {
                return None;
            }
            if range.end <= body.end {
                let selected = range.clone();
                environment_index = uncovered[environment_index];
                return Some(selected);
            }
            // A crossing range cannot hide a contained child of the current
            // body. Examine its following ranges until one is eligible.
            environment_index += 1;
        }
        None
    })
    .peekable();
    let protected_start = protected.partition_point(|range| range.end <= body.start);
    let mut protected_ranges = protected[protected_start..]
        .iter()
        .take_while(|range| range.start < body.end)
        .map(|range| range.start.max(body.start)..range.end.min(body.end))
        .peekable();
    let mut merged: Vec<Range<usize>> = Vec::new();
    loop {
        let next = match (environment_ranges.peek(), protected_ranges.peek()) {
            (Some(environment), Some(protected)) if environment.start <= protected.start => {
                environment_ranges.next()
            }
            (_, Some(_)) => protected_ranges.next(),
            (Some(_), None) => environment_ranges.next(),
            (None, None) => break,
        }
        .expect("available opaque range");
        if let Some(previous) = merged.last_mut().filter(|range| range.end >= next.start) {
            previous.end = previous.end.max(next.end);
        } else {
            merged.push(next);
        }
    }
    merged
}

// TikZ matrix headers may contain options, a node name, and a position.
// Only the final brace body is aligned; option values remain opaque.
fn matrix_command_body(source: &str, mut cursor: usize) -> Option<(Range<usize>, usize)> {
    loop {
        cursor = crate::trivia::skip(source, cursor);
        match source.as_bytes().get(cursor)? {
            b'[' => cursor = crate::syntax::balanced_end(source, cursor, b'[', b']')?,
            b'(' => cursor = crate::syntax::balanced_end(source, cursor, b'(', b')')?,
            b'{' => {
                let end = crate::syntax::balanced_end(source, cursor, b'{', b'}')?;
                return Some((cursor + 1..end - 1, end));
            }
            b'a' if source[cursor..].starts_with("at")
                && source
                    .as_bytes()
                    .get(cursor + 2)
                    .is_some_and(|c| c.is_ascii_whitespace() || *c == b'(') =>
            {
                cursor += 2;
                while source
                    .as_bytes()
                    .get(cursor)
                    .is_some_and(u8::is_ascii_whitespace)
                {
                    cursor += 1;
                }
                if source.as_bytes().get(cursor) != Some(&b'(') {
                    return None;
                }
            }
            _ => return None,
        }
    }
}

static MARKUP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^[ \t]*%\*[ \t]*\\(begin|end)\{([^{}\r\n]+)\}[^\r\n]*(?:\r?\n|$)")
        .expect("static markup pattern")
});

fn markup_blocks(
    source: &str,
    config: &CompiledConfig,
    protected: &[Range<usize>],
) -> Result<Vec<Block>, FormatError> {
    let mut stack: Vec<(&str, usize, usize, String)> = Vec::new();
    let mut blocks = Vec::new();
    for captures in MARKUP.captures_iter(source) {
        let marker = captures.get(0).unwrap();
        if crate::syntax::is_protected(marker.start(), protected) {
            continue;
        }
        let name = captures.get(2).unwrap().as_str();
        if captures.get(1).unwrap().as_str() == "begin" {
            let line = &source[marker.range()];
            let leading = &line[..line.len() - line.trim_start_matches([' ', '\t']).len()];
            let base = stack.last().map_or(leading, |parent| parent.3.as_str());
            let indent = format!("{base}{}", config.config.indent.unit("environment", name));
            stack.push((name, marker.start(), marker.end(), indent));
        } else if let Some((opening, begin, body_start, indent)) = stack.pop() {
            if opening != name {
                return Err(FormatError::Syntax(format!(
                    "mismatched alignment markup {opening:?} and {name:?}"
                )));
            }
            if config
                .alignment
                .rules
                .get(name)
                .is_some_and(|r| r.settings.enabled)
            {
                blocks.push(Block {
                    begin,
                    body: body_start..marker.start(),
                    end: marker.end(),
                    name: name.into(),
                    children: Vec::new(),
                    environment: false,
                    markup_indent: Some(indent),
                });
            }
        } else if config
            .alignment
            .rules
            .get(name)
            .is_some_and(|r| r.settings.enabled)
        {
            return Err(FormatError::Syntax(format!(
                "unexpected alignment markup end {name:?}"
            )));
        }
    }
    if let Some((name, _, _, _)) = stack.iter().find(|(name, _, _, _)| {
        config
            .alignment
            .rules
            .get(*name)
            .is_some_and(|r| r.settings.enabled)
    }) {
        return Err(FormatError::Syntax(format!(
            "unclosed alignment markup {name:?}"
        )));
    }
    Ok(blocks)
}

fn indent_markup_body(
    source: &str,
    indent: &str,
    config: &CompiledConfig,
) -> Result<String, FormatError> {
    let protected = crate::syntax::protected_ranges(source, &config.config)?;
    // Child markup bodies already received their absolute indentation in the
    // bottom-up alignment pass. Parent passes adjust the marker lines only.
    let mut children = Vec::new();
    let mut stack: Vec<(&str, usize)> = Vec::new();
    for captures in MARKUP.captures_iter(source) {
        let marker = captures.get(0).unwrap();
        if crate::syntax::is_protected(marker.start(), &protected) {
            continue;
        }
        let name = captures.get(2).unwrap().as_str();
        if captures.get(1).unwrap().as_str() == "begin" {
            stack.push((name, marker.end()));
        } else if let Some((opening, start)) = stack.pop()
            && opening == name
            && config
                .alignment
                .rules
                .get(name)
                .is_some_and(|r| r.settings.enabled)
        {
            children.push(start..marker.start());
        }
    }
    children.sort_by_key(|r| (r.start, std::cmp::Reverse(r.end)));
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(children.len());
    for range in children {
        if let Some(previous) = merged.last_mut().filter(|r| r.end >= range.start) {
            previous.end = previous.end.max(range.end);
        } else {
            merged.push(range);
        }
    }
    let children = merged;
    let mut offset = 0;
    let mut base = usize::MAX;
    for line in source.split_inclusive('\n') {
        if !line.trim().is_empty()
            && !crate::syntax::is_protected(offset, &protected)
            && !crate::syntax::is_protected(offset, &children)
        {
            base = base.min(line.len() - line.trim_start_matches([' ', '\t']).len());
        }
        offset += line.len();
    }
    let base = if base == usize::MAX { 0 } else { base };
    let mut edits = crate::edit_plan::EditPlan::new();
    offset = 0;
    for line in source.split_inclusive('\n') {
        if line.trim().is_empty()
            || crate::syntax::is_protected(offset, &protected)
            || crate::syntax::is_protected(offset, &children)
        {
            offset += line.len();
        } else {
            let leading = line.len() - line.trim_start_matches([' ', '\t']).len();
            let mut replacement = indent.to_owned();
            if !line[leading..].starts_with("%*") {
                replacement.push_str(&line[base.min(leading)..leading]);
            }
            if replacement != line[..leading] {
                edits.replace(offset..offset + leading, replacement);
            }
            offset += line.len();
        }
    }
    edits.apply_ordered(source)
}

fn apply_edits(source: &str, range: Range<usize>, edits: &[(Range<usize>, &str)]) -> String {
    edited_source(source, range, edits).into_owned()
}

fn edited_source<'a>(
    source: &'a str,
    range: Range<usize>,
    edits: &[(Range<usize>, &str)],
) -> Cow<'a, str> {
    let first = edits.partition_point(|(edit, _)| edit.end <= range.start);
    let relevant = edits[first..]
        .iter()
        .take_while(|(edit, _)| edit.start < range.end);
    if !relevant
        .clone()
        .any(|(edit, _)| edit.start >= range.start && edit.end <= range.end)
    {
        return Cow::Borrowed(&source[range]);
    }
    let mut output = String::with_capacity(range.len());
    let mut cursor = range.start;
    for (edit, text) in relevant {
        if edit.start >= cursor && edit.end <= range.end {
            output.push_str(&source[cursor..edit.start]);
            output.push_str(text);
            cursor = edit.end;
        }
    }
    output.push_str(&source[cursor..range.end]);
    Cow::Owned(output)
}

fn line_end(source: &str, cursor: usize, end: usize) -> usize {
    source[cursor..end]
        .find('\n')
        .map_or(end, |n| cursor + n + 1)
}

#[derive(Debug)]
struct Row {
    cells: Vec<Range<usize>>,
    delimiters: Vec<Range<usize>>,
    tail: Range<usize>,
    full: Range<usize>,
    terminator_length: usize,
    standalone: bool,
}

fn split_rows(
    source: &str,
    range: Range<usize>,
    rule: &CompiledRule,
    opaque: &[Range<usize>],
    environment: &str,
    fold_rows: bool,
    equation_labels: bool,
) -> Result<Vec<Row>, FormatError> {
    let mut candidates = Vec::new();
    let mut delimiter_matches = rule
        .delimiter
        .find_iter(&source[range.clone()], rule.settings.delimiter_group);
    let mut row_matches = rule
        .row_break
        .find_iter(&source[range.clone()], rule.settings.row_break_group);
    let mut delimiter_match = delimiter_matches.next().transpose()?;
    let mut row_match = row_matches.next().transpose()?;
    loop {
        let row_break = match (&delimiter_match, &row_match) {
            (Some(delimiter), Some(row)) => row.start <= delimiter.start,
            (None, Some(_)) => true,
            (Some(_), None) => false,
            (None, None) => break,
        };
        let matched = if row_break {
            let matched = row_match.take().unwrap();
            row_match = row_matches.next().transpose()?;
            matched
        } else {
            let matched = delimiter_match.take().unwrap();
            delimiter_match = delimiter_matches.next().transpose()?;
            matched
        };
        if matched.is_empty() {
            return Err(FormatError::Syntax(
                "alignment delimiter matched an empty string".into(),
            ));
        }
        let start = range.start + matched.start;
        if row_break
            && rule.settings.row_break == DEFAULT_ROW_BREAK
            && source[start..].starts_with("\\tabularnewline")
            && crate::syntax::command_at(source, start)
                .is_none_or(|(name, _)| name.trim_end_matches('*') != "tabularnewline")
        {
            continue;
        }
        candidates.push((start, range.start + matched.end, row_break));
    }
    let mut opaque_index = opaque.partition_point(|r| r.end <= range.start);
    let mut candidate_index = 0;
    let mut rows = Vec::new();
    let mut cells = Vec::new();
    let mut delimiters = Vec::new();
    let mut row_start = range.start;
    let mut cell_start = range.start;
    let mut cursor = range.start;
    let mut next_syntax = range.start;
    let mut depth = 0usize;
    let bytes = source.as_bytes();
    while cursor < range.end {
        while opaque.get(opaque_index).is_some_and(|r| r.end <= cursor) {
            opaque_index += 1;
        }
        if let Some(protected) = opaque
            .get(opaque_index)
            .filter(|r| r.start <= cursor && cursor < r.end)
        {
            // A standalone child block belongs between alignment rows. Keep it
            // separate from the next first cell so its indentation and markup
            // never contribute to that cell's measured width.
            let standalone_end =
                if source.as_bytes().get(protected.end.wrapping_sub(1)) == Some(&b'\n') {
                    Some(protected.end)
                } else {
                    let mut after = protected.end;
                    while after < range.end && matches!(bytes[after], b' ' | b'\t' | b'\r') {
                        after += 1;
                    }
                    (bytes.get(after) == Some(&b'\n')).then_some(after + 1)
                };
            if depth == 0
                && cells.is_empty()
                && source[row_start..protected.start].trim().is_empty()
                && let Some(end) = standalone_end
                && !(fold_rows && delimiters_follow(source, end, range.end, &candidates))
            {
                rows.push(Row {
                    cells: std::iter::once(row_start..end).collect(),
                    delimiters: Vec::new(),
                    tail: end..end,
                    full: row_start..end,
                    terminator_length: 0,
                    standalone: true,
                });
                row_start = end;
                cell_start = end;
                cursor = end;
                continue;
            }
            cursor = protected.end;
            continue;
        }
        if bytes[cursor] == b'%' {
            cursor = line_end(source, cursor, range.end);
            continue;
        }
        if depth == 0
            && cells.is_empty()
            && bytes[cursor] == b'\\'
            && let Some(end) =
                standalone_rule_end(source, cursor, range.end, environment).or_else(|| {
                    equation_labels
                        .then(|| equation_labels::intertext_end(source, cursor, range.end))
                        .flatten()
                })
            && (source[row_start..cursor].trim().is_empty()
                || (rule.settings.align_by_section
                    || crate::package_tables::is_table(environment)
                        && source[cursor..].starts_with(r"\cmidrule"))
                    && crate::trivia::skip_until(source, row_start, cursor) == cursor)
        {
            rows.push(Row {
                cells: std::iter::once(row_start..end).collect(),
                delimiters: Vec::new(),
                tail: end..end,
                full: row_start..end,
                terminator_length: 0,
                standalone: true,
            });
            row_start = end;
            cell_start = end;
            cursor = end;
            continue;
        }
        if environment == "tikzcd"
            && depth == 0
            && bytes[cursor] == b'\\'
            && let Some(newline) = source[cell_start..cursor].rfind('\n')
            && source[cell_start + newline + 1..cursor].trim().is_empty()
            && trailing_diagram_arrows(source, cursor, range.end, rule.settings.tikzcd_arrows)
        {
            let end = cell_start + newline + 1;
            cells.push(cell_start..end);
            rows.push(Row {
                cells,
                delimiters,
                tail: range.end..range.end,
                full: row_start..end,
                terminator_length: 0,
                standalone: false,
            });
            rows.push(Row {
                cells: std::iter::once(end..range.end).collect(),
                delimiters: Vec::new(),
                tail: range.end..range.end,
                full: end..range.end,
                terminator_length: 0,
                standalone: true,
            });
            return Ok(rows);
        }
        while candidates
            .get(candidate_index)
            .is_some_and(|c| c.0 < cursor)
        {
            candidate_index += 1;
        }
        if depth == 0
            && let Some(&(_, end, row_break)) =
                candidates.get(candidate_index).filter(|c| c.0 == cursor)
        {
            cells.push(cell_start..cursor);
            if row_break {
                let (terminator_end, mut tail_end) = row_suffix_end(source, end, range.end);
                if rule.settings.align_content_after_row_break {
                    let physical_end = line_end(source, terminator_end, range.end);
                    if !source[terminator_end..physical_end]
                        .trim_matches([' ', '\t', '\r', '\n'])
                        .is_empty()
                    {
                        tail_end = physical_end;
                    }
                }
                rows.push(Row {
                    cells: std::mem::take(&mut cells),
                    delimiters: std::mem::take(&mut delimiters),
                    tail: cursor..tail_end,
                    full: row_start..tail_end,
                    terminator_length: terminator_end - cursor,
                    standalone: false,
                });
                row_start = tail_end;
                cell_start = tail_end;
                cursor = tail_end;
            } else {
                delimiters.push(cursor..end);
                cell_start = end;
                cursor = end;
            }
            continue;
        }
        match bytes[cursor] {
            b'\\' => {
                cursor += 1;
                if cursor < range.end {
                    if bytes[cursor].is_ascii_alphabetic() || bytes[cursor] == b'@' {
                        while cursor < range.end
                            && (bytes[cursor].is_ascii_alphabetic() || bytes[cursor] == b'@')
                        {
                            cursor += 1;
                        }
                    } else {
                        cursor += source[cursor..].chars().next().unwrap().len_utf8();
                    }
                }
            }
            b'{' => {
                depth += 1;
                cursor += 1;
            }
            b'}' => {
                depth = depth.saturating_sub(1);
                cursor += 1;
            }
            _ => {
                cursor += source[cursor..].chars().next().unwrap().len_utf8();
                // Ordinary cell bytes do not change nesting. Stop at the next
                // delimiter candidate, protected span, or TeX syntax byte.
                while candidates
                    .get(candidate_index)
                    .is_some_and(|candidate| candidate.0 < cursor)
                {
                    candidate_index += 1;
                }
                let stop = candidates
                    .get(candidate_index)
                    .map_or(range.end, |candidate| candidate.0)
                    .min(
                        opaque
                            .get(opaque_index)
                            .map_or(range.end, |opaque| opaque.start),
                    );
                // Keep the first search result across closing braces so a
                // run of `}` does not rescan the rest of the cell each time.
                if next_syntax < cursor {
                    next_syntax = memchr::memchr3(b'\\', b'%', b'{', &bytes[cursor..stop])
                        .map_or(stop, |at| cursor + at);
                }
                let stop = stop.min(next_syntax);
                cursor = memchr::memchr(b'}', &bytes[cursor..stop]).map_or(stop, |at| cursor + at);
            }
        }
    }
    if cell_start < range.end || !cells.is_empty() {
        cells.push(cell_start..range.end);
        rows.push(Row {
            cells,
            delimiters,
            tail: range.end..range.end,
            full: row_start..range.end,
            terminator_length: 0,
            standalone: false,
        });
    }
    Ok(rows)
}

fn delimiters_follow(
    source: &str,
    start: usize,
    end: usize,
    candidates: &[(usize, usize, bool)],
) -> bool {
    let next = crate::trivia::skip_until(source, start, end);
    candidates
        .get(candidates.partition_point(|c| c.0 < next))
        .is_some_and(|c| c.0 == next && !c.2)
}

/// A run of arrows belongs to a node when every remaining outer token is
/// part of an arrow invocation. Braced labels can themselves contain commands.
fn diagram_arrows(cell: &str, base: &str, index: usize, layout: &RowFold) -> Option<String> {
    if !cell.contains(r"\ar") || cell.contains('%') {
        return None;
    }
    let mut cursor = 0;
    let mut arrows = Vec::new();
    while cursor < cell.len() {
        match cell.as_bytes()[cursor] {
            b'{' if arrows.is_empty() => {
                cursor = crate::syntax::balanced_end(cell, cursor, b'{', b'}')?
            }
            b'\\' => {
                let (name, end) = crate::syntax::command_at(cell, cursor)?;
                if !matches!(name, "ar" | "arrow") {
                    if !arrows.is_empty() || matches!(name, "verb" | "verb*" | "begin") {
                        return None;
                    }
                    cursor = end;
                    continue;
                }
                let start = cursor;
                cursor = crate::trivia::skip(cell, end);
                let args = cursor;
                // New brackets and legacy [options]{direction}[options]{label}.
                while let Some(open @ (b'[' | b'{')) = cell.as_bytes().get(cursor).copied() {
                    let open_at = cursor;
                    cursor = crate::syntax::balanced_end(
                        cell,
                        cursor,
                        open,
                        if open == b'[' { b']' } else { b'}' },
                    )?;
                    if open == b'[' && explicit_arrow_source(&cell[open_at + 1..cursor - 1]) {
                        return None;
                    }
                    let next = crate::trivia::skip(cell, cursor);
                    if !matches!(cell.as_bytes().get(next), Some(b'[' | b'{')) {
                        break;
                    }
                    cursor = next;
                }
                if args == cursor {
                    return None;
                }
                arrows.push(start..cursor);
            }
            byte if arrows.is_empty() || byte.is_ascii_whitespace() => cursor += 1,
            _ => return None,
        }
    }
    if arrows.len() < 2 {
        return None;
    }
    let first = arrows[0].start;
    let node = cell[..first].trim_end_matches([' ', '\t', '\r', '\n']);
    let mut out = node.to_owned();
    for (i, arrow) in arrows.into_iter().enumerate() {
        // A tight first arrow in a text-mode node must not acquire a space.
        if i > 0 || first > node.len() {
            out.push_str(layout.newline);
            out.push_str(base);
            if index > 0 {
                out.push_str(&layout.unit);
            }
            out.push_str(&layout.unit);
        }
        out.push_str(&cell[arrow]);
    }
    Some(out)
}

fn explicit_arrow_source(options: &str) -> bool {
    crate::packages::keys::definition_entries(options)
        .iter()
        .any(|range| {
            options[range.clone()]
                .split_once('=')
                .is_some_and(|(key, _)| key.trim() == "from")
        })
}

fn trailing_diagram_arrows(
    source: &str,
    mut cursor: usize,
    end: usize,
    explicit_only: bool,
) -> bool {
    while let Some((name, after)) = crate::syntax::command_at(source, cursor) {
        if !matches!(name, "arrow" | "ar") {
            return false;
        }
        let arguments =
            crate::layout::signature_argument_ranges(source, after..end, b"o".iter().copied());
        let Some(argument) = arguments.last() else {
            return false;
        };
        if explicit_only && !explicit_arrow_source(&source[argument.start + 1..argument.end - 1]) {
            return false;
        }
        let next = crate::trivia::skip_until(source, argument.end, end);
        if next == end {
            return true;
        }
        if !source[argument.end..next].contains('\n') {
            return false;
        }
        cursor = next;
    }
    false
}

/// Standalone table rules sit between logical rows. Their source stays outside
/// the following first cell, including rule parameters and any trailing comment.
fn standalone_rule_end(source: &str, start: usize, end: usize, environment: &str) -> Option<usize> {
    if matches!(
        environment,
        "longtable" | "xltabular" | "tabularx" | "longtabu"
    ) && let Some((name, after)) = crate::syntax::command_at(source, start)
        && let Some(shape) = match name.trim_end_matches('*') {
            "caption" | "captionabove" | "captionbelow" => Some("om"),
            "bicaption" => crate::packages::equation_floats::command_signature(source, name, after),
            _ => None,
        }
        && let Some(caption_end) =
            crate::syntax::arguments::signature_end(&source[..end], after, shape)
    {
        let label = crate::trivia::skip_until(source, caption_end, end);
        let after = if let Some(("label", after)) = crate::syntax::command_at(source, label) {
            crate::layout::signature_argument_ranges(source, after..end, b"m".iter().copied())
                .last()
                .map_or(caption_end, |argument| argument.end)
        } else {
            caption_end
        };
        let row_break = crate::trivia::skip_until(source, after, end);
        if let Some((r"\" | "tabularnewline", after)) = crate::syntax::command_at(source, row_break)
        {
            return Some(row_suffix_end(source, after, end).1);
        }
    }
    let mut cursor = crate::table_rules::rule_end(source, start, end, environment)?;
    while cursor < end {
        match source.as_bytes()[cursor] {
            b' ' | b'\t' | b'\r' => cursor += 1,
            b'\n' => return Some(cursor + 1),
            b'%' => return Some(line_end(source, cursor, end)),
            b'\\' => {
                cursor = crate::table_rules::rule_end(source, cursor, end, environment)?;
            }
            _ => return None,
        }
    }
    Some(end)
}

fn row_suffix_end(source: &str, cursor: usize, end: usize) -> (usize, usize) {
    let bytes = source.as_bytes();
    // The row regex may have already consumed the star. Only a raw double
    // backslash can still accept one; a second star belongs to the next row.
    let allow_star =
        source[..cursor].ends_with("\\\\") || source[..cursor].ends_with("\\tabularnewline");
    let mut cursor = crate::layout::row_terminator_end(source, cursor, end, allow_star);
    let after_option = cursor;
    while cursor < end && matches!(bytes[cursor], b' ' | b'\t') {
        cursor += 1;
    }
    if bytes.get(cursor) == Some(&b'%') {
        return (after_option, line_end(source, cursor, end));
    }
    (after_option, after_option)
}

struct MeasuredCell<'a> {
    source: Range<usize>,
    text: Cow<'a, str>,
    span: usize,
    width: usize,
    measure: bool,
    decimal_at: Option<usize>,
    justification: Option<Justification>,
}

struct MeasuredRow<'a> {
    equation_label: Option<&'a str>,
    folded: bool,
    ends_section: bool,
    original: Range<usize>,
    prefix: Cow<'a, str>,
    starts_physical_line: bool,
    suffix: &'a str,
    has_row_break: bool,
    terminator_length: usize,
    cells: Vec<MeasuredCell<'a>>,
    delimiters: Vec<&'a str>,
    columns: usize,
    eligible: bool,
    data_row: bool,
}

struct RowLayoutContext<'a> {
    label_layout: Option<equation_labels::Layout<'a>>,
    protected: &'a [Range<usize>],
    fold: Option<&'a RowFold>,
    freshly_indented: bool,
    tab_width: usize,
    column_spec: Option<&'a column_spec::TableSpec>,
    column_environment: Option<&'a str>,
    declared_column_types: &'a column_spec::DeclaredTypes,
}

struct RowFold {
    diagram_arrows: bool,
    limit: usize,
    environment_indent: String,
    unit: String,
    newline: &'static str,
}

fn apply_column_spec(
    source: &str,
    opaque: &[Range<usize>],
    rows: &mut [MeasuredRow<'_>],
    spec: &column_spec::TableSpec,
    settings: &AlignmentConfig,
) {
    let row_count = rows.iter().filter(|row| row.data_row).count();
    let column_count = rows
        .iter()
        .filter(|row| row.data_row)
        .map(|row| row.columns)
        .max()
        .unwrap_or(0)
        .max(spec.column_count());
    let mut omitted: Vec<(usize, usize, usize, usize)> = Vec::new();
    let body_styles = if spec.is_tabularray() {
        let mut row_number = 0;
        let cells = rows.iter().filter(|row| row.data_row).flat_map(|row| {
            row_number += 1;
            let mut column_number = 0;
            row.cells.iter().map(move |cell| {
                column_number += cell.span;
                (row_number, column_number, cell.source.clone())
            })
        });
        let Ok(styles) = spec.body_styles(source, opaque, cells) else {
            return;
        };
        Some(styles)
    } else {
        None
    };
    let mut logical_row = 0;
    for row in rows {
        if !row.data_row {
            continue;
        }
        logical_row += 1;
        if !row.eligible {
            continue;
        }
        if body_styles
            .as_ref()
            .is_some_and(|styles| !styles.row_supported(logical_row))
        {
            row.eligible = false;
            continue;
        }
        // Obsolete multicolumn calls in tabularray do not share SetCell's
        // physical-placeholder grammar. Retain the earlier conservative path.
        if spec.is_tabularray() && row.cells.iter().any(|cell| cell.span != 1) {
            continue;
        }
        let mut index = 0;
        let mut column = 1;
        while index < row.cells.len() {
            let covered =
                omitted
                    .iter()
                    .any(|&(first_row, last_row, first_column, last_column)| {
                        (first_row..=last_row).contains(&logical_row)
                            && (first_column..=last_column).contains(&column)
                            && (logical_row != first_row || column != first_column)
                    });
            if covered {
                row.cells[index].measure = false;
                row.cells[index].justification = Some(Justification::Left);
                column += row.cells[index].span;
                index += 1;
                continue;
            }
            let mut style = spec.style(logical_row, column, row_count, column_count);
            if let Some(styles) = &body_styles {
                let local = styles.style(logical_row, column);
                if local.justification.is_some() {
                    style.decimal = local.decimal;
                }
                style.justification = local.justification.or(style.justification);
                style.colspan = local.colspan.or(style.colspan);
                style.rowspan = local.rowspan.or(style.rowspan);
            }
            let cell = &mut row.cells[index];
            if settings.align_decimals
                && cell.justification.is_none()
                && cell.span == 1
                && style.colspan.unwrap_or(1) == 1
            {
                cell.decimal_at = style
                    .decimal
                    .and_then(|marker| decimal_anchor(&cell.text, marker));
            }
            cell.justification = cell.justification.or(style.justification);
            let span = if spec.is_tabularray() {
                style.colspan.unwrap_or(1)
            } else {
                row.cells[index].span
            };
            if spec.is_tabularray() && span > 1 {
                if span > row.cells.len() - index
                    || row.cells[index + 1..index + span]
                        .iter()
                        .any(|cell| !cell.text.is_empty())
                {
                    row.eligible = false;
                    break;
                }
                let mut text = row.cells[index].text.to_string();
                for offset in 0..span - 1 {
                    text.extend(std::iter::repeat_n(' ', settings.spaces_before));
                    text.push_str(row.delimiters[index + offset]);
                    text.extend(std::iter::repeat_n(' ', settings.spaces_after));
                }
                row.cells[index].width = measure_width(&text, settings.width);
                row.cells[index].text = Cow::Owned(text);
                row.cells[index].span = span;
                row.cells.drain(index + 1..index + span);
                row.delimiters.drain(index..index + span - 1);
            }
            if spec.is_tabularray() && (span > 1 || style.rowspan.unwrap_or(1) > 1) {
                omitted.push((
                    logical_row,
                    logical_row
                        .saturating_add(style.rowspan.unwrap_or(1) - 1)
                        .min(row_count),
                    column,
                    column + span - 1,
                ));
            }
            column += row.cells[index].span;
            index += 1;
        }
    }
}

fn format_rows(
    source: &str,
    rows: Vec<Row>,
    rule: &CompiledRule,
    children: &[(Range<usize>, &str)],
    opaque: &[Range<usize>],
    context: RowLayoutContext<'_>,
) -> Result<String, FormatError> {
    let RowLayoutContext {
        label_layout,
        protected,
        fold,
        freshly_indented,
        tab_width,
        column_spec,
        column_environment,
        declared_column_types,
    } = context;
    let settings = &rule.settings;
    let mut measured = rows
        .into_iter()
        .map(|row| {
            let measure_row = !any_match(&rule.dont_measure_rows, &source[row.full.clone()])?;
            let first = row.cells.first().map_or("", |r| &source[r.clone()]);
            let prefix_len = row.cells.first().map_or(0, |range| {
                alignment_trivia_end(source, range.clone(), opaque) - range.start
            });
            let prefix = &first[..prefix_len];
            // A standalone rule or child block may already own the preceding
            // newline, leaving the next physical row with only horizontal indent.
            let starts_physical_line = prefix.contains('\n')
                || row.full.start == 0
                || source.as_bytes().get(row.full.start - 1) == Some(&b'\n');
            let final_suffix_start = row
                .cells
                .last()
                .filter(|_| row.tail.is_empty())
                .map(|range| final_cell_suffix_start(source, range.clone(), opaque));
            let unsafe_comment = row.cells.iter().enumerate().any(|(i, cell)| {
                let start = cell.start + if i == 0 { prefix_len } else { 0 };
                let end = if i + 1 == row.cells.len() {
                    final_suffix_start.unwrap_or(cell.end)
                } else {
                    cell.end
                };
                contains_comment(source, start.min(end)..end, opaque)
            });
            let cells = row
                .cells
                .iter()
                .enumerate()
                .map(|(i, range)| {
                    let start = range.start + if i == 0 { prefix_len } else { 0 };
                    let end = if i + 1 == row.cells.len() {
                        final_suffix_start.unwrap_or(range.end)
                    } else {
                        range.end
                    };
                    let body_source = start.min(end)..end;
                    let text = edited_source(source, body_source.clone(), children);
                    let left = text.len() - text.trim_start_matches([' ', '\t', '\r', '\n']).len();
                    let end = trim_cell_end(&text).len();
                    let text = match text {
                        Cow::Borrowed(value) => Cow::Borrowed(&value[left.min(end)..end]),
                        Cow::Owned(mut value) => {
                            value.truncate(end);
                            if left > 0 {
                                value.drain(..left.min(end));
                            }
                            Cow::Owned(value)
                        }
                    };
                    let override_column = (settings.justification == Justification::Columns)
                        .then(|| {
                            column_spec::multicolumn(
                                &text,
                                declared_column_types,
                                column_environment,
                            )
                        })
                        .flatten();
                    let span = if let Some((span, _)) = override_column {
                        span
                    } else if settings.multicolumn
                        || settings.justification == Justification::Columns
                    {
                        multicolumn_span(&text).unwrap_or(1)
                    } else {
                        1
                    };
                    let width = measure_width(&text, settings.width);
                    let measure = measure_row
                        && !settings
                            .dont_measure_literals
                            .iter()
                            .any(|literal| literal == text.as_ref())
                        && !any_match(&rule.dont_measure, &text)?;
                    Ok::<_, FormatError>(MeasuredCell {
                        decimal_at: None,
                        source: body_source,
                        text,
                        span,
                        width,
                        measure,
                        justification: override_column.map(|(_, align)| align),
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            let columns = cells.iter().map(|c| c.span).sum();
            let has_row_break = !row.tail.is_empty();
            let suffix = if has_row_break {
                &source[row.tail.clone()]
            } else {
                let suffix = row.cells.last().map_or("", |range| {
                    &source[final_suffix_start.unwrap_or(range.end)..range.end]
                });
                // An empty final cell already receives the configured space
                // after its delimiter. Retain its closing line boundary once.
                if cells.last().is_some_and(|cell| cell.text.is_empty()) {
                    suffix.trim_start_matches([' ', '\t'])
                } else {
                    suffix
                }
            };
            let data_row = !row.standalone
                && (has_row_break
                    || !row.delimiters.is_empty()
                    || cells.iter().any(|cell| !cell.text.is_empty()));
            // Joining physical lines can change line-scoped readers such as
            // ltxdoc short verbatim and TeX character translations. Keep such
            // a row in place when either boundary would move across a line.
            let content = row.full.start + prefix_len..row.tail.start.min(row.full.end);
            let first_reader = protected.partition_point(|r| r.end <= content.start);
            let reader_spans_lines = protected[first_reader..]
                .iter()
                .take_while(|r| r.start < content.end)
                .any(|r| {
                    let literal = &source[r.clone()];
                    literal.contains("^^")
                        || literal.chars().next().is_some_and(|c| {
                            !c.is_alphanumeric()
                                && !c.is_whitespace()
                                && !matches!(c, '\\' | '{' | '[' | '%')
                                && literal.len() > c.len_utf8()
                                && literal.ends_with(c)
                        })
                })
                && source[content.clone()]
                    .trim_end_matches(['\r', '\n', ' ', '\t'])
                    .contains('\n');
            let eligible = data_row && !unsafe_comment && !reader_spans_lines;
            Ok::<_, FormatError>(MeasuredRow {
                equation_label: None,
                folded: false,
                ends_section: label_layout.is_some() && row.standalone
                    || settings.align_by_section
                        && row.standalone
                        && column_environment.is_some_and(|env| {
                            section_boundary(source, row.full.clone(), opaque, env)
                        }),
                original: row.full,
                prefix: prefix.into(),
                starts_physical_line,
                suffix,
                has_row_break,
                terminator_length: row.terminator_length,
                cells,
                delimiters: row.delimiters.iter().map(|r| &source[r.clone()]).collect(),
                columns,
                eligible,
                data_row,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if settings.justification == Justification::Columns
        && let Some(spec) = column_spec
    {
        apply_column_spec(source, opaque, &mut measured, spec, settings);
    }
    if let Some(layout) = fold.filter(|f| f.diagram_arrows) {
        for row in measured
            .iter_mut()
            .filter(|r| r.eligible && r.starts_physical_line)
        {
            let base = &row.prefix[row.prefix.trim_end_matches([' ', '\t']).len()..];
            for (index, cell) in row.cells.iter_mut().enumerate() {
                if !crate::syntax::overlaps(&cell.source, protected)
                    && let Some(text) = diagram_arrows(&cell.text, base, index, layout)
                {
                    cell.text = text.into();
                    row.folded = true;
                }
            }
        }
    }
    let last_data = measured
        .iter()
        .rfind(|row| row.eligible)
        .map(|row| row.original.start);
    let mut output = String::with_capacity(measured.iter().map(|r| r.original.len()).sum());
    for section in measured.split_inclusive_mut(|row| row.ends_section) {
        write_section(
            source,
            section,
            settings,
            children,
            freshly_indented,
            tab_width,
            last_data,
            fold,
            label_layout,
            &mut output,
        );
    }
    Ok(output)
}

fn section_boundary(source: &str, range: Range<usize>, opaque: &[Range<usize>], env: &str) -> bool {
    let mut at = crate::trivia::skip_until(source, range.start, range.end);
    while at < range.end && !crate::syntax::is_protected(at, opaque) {
        let Some(end) = crate::table_rules::rule_end(source, at, range.end, env) else {
            break;
        };
        if crate::syntax::command_at(source, at).is_some_and(|(name, _)| {
            matches!(
                name,
                "midrule" | "endfirsthead" | "endhead" | "endfoot" | "endlastfoot"
            )
        }) {
            return true;
        }
        at = crate::trivia::skip_until(source, end, range.end);
    }
    false
}

#[allow(clippy::too_many_arguments)]
fn write_section(
    source: &str,
    measured: &mut [MeasuredRow<'_>],
    settings: &AlignmentConfig,
    children: &[(Range<usize>, &str)],
    freshly_indented: bool,
    tab_width: usize,
    last_data: Option<usize>,
    fold: Option<&RowFold>,
    label_layout: Option<equation_labels::Layout<'_>>,
    output: &mut String,
) {
    let columns = measured
        .iter()
        .filter(|r| r.eligible)
        .map(|r| r.columns)
        .max()
        .unwrap_or(0);
    if columns == 0 {
        for row in measured {
            output.push_str(&apply_edits(source, row.original.clone(), children));
        }
        return;
    }
    let contains_last_row = measured
        .last()
        .is_some_and(|row| last_data.is_some_and(|pos| pos < row.original.end));
    if settings.short_rows == ShortRows::Preserve {
        for row in measured.iter_mut() {
            if row.columns < columns {
                row.eligible = false;
            }
        }
    }
    if let Some(fold) = fold
        && measured
            .iter()
            .any(|r| r.eligible && !r.starts_physical_line)
    {
        let indent = format!("{}{}", fold.environment_indent, fold.unit);
        let width = trailing_indent_width(&indent, tab_width);
        if measured.iter().any(|row| {
            row.eligible
                && !row.starts_physical_line
                && row.natural_width(settings, width) > fold.limit
        }) {
            for row in measured.iter_mut().filter(|row| {
                row.eligible && !row.starts_physical_line && row.prefix.trim().is_empty()
            }) {
                row.prefix = format!("{}{indent}", fold.newline).into();
                row.starts_physical_line = true;
            }
        }
    }
    let justification = |_column: usize, cell: &MeasuredCell<'_>| {
        if settings.justification == Justification::Columns {
            cell.justification.unwrap_or(Justification::Left)
        } else {
            settings.justification
        }
    };
    let last_data = contains_last_row
        .then(|| measured.iter().rfind(|row| row.eligible))
        .flatten()
        .map(|row| row.original.start);
    let first_column_has_leading_padding = measured
        .iter()
        .filter(|row| row.eligible)
        .filter_map(|row| row.cells.first())
        .any(|cell| {
            matches!(
                justification(0, cell),
                Justification::Right | Justification::Center
            )
        });
    // Leading whitespace in an empty first cell includes alignment padding.
    // Same-line logical rows have no indentation prefix. Physical rows recover
    // their indentation from a row whose first cell contains actual content.
    let content_indent = measured
        .iter()
        .filter(|row| {
            row.eligible
                && row.starts_physical_line
                && row.cells.first().is_some_and(|cell| !cell.text.is_empty())
        })
        .map(|row| trailing_indent_width(&row.prefix, tab_width))
        .min();
    // A first row beside the opening token establishes a hanging column for
    // subsequent physical rows. Other same-line logical rows keep their layout.
    let hanging_indent = measured
        .iter()
        .find(|row| row.eligible)
        .filter(|row| {
            !row.starts_physical_line
                && row.cells.first().is_some_and(|cell| {
                    !cell.text.contains('\n')
                        && (!cell.text.is_empty() || !row.delimiters.is_empty())
                })
                && measured
                    .iter()
                    .any(|row| row.eligible && row.starts_physical_line)
        })
        .map(|row| {
            let prefix = if first_column_has_leading_padding
                || row.cells.first().is_some_and(|cell| cell.text.is_empty())
            {
                0
            } else {
                row.prefix.len()
            };
            let end = row.original.start + prefix;
            let line = source[..end].rsplit('\n').next().unwrap_or("");
            measure_width(&line.replace('\t', &" ".repeat(tab_width)), settings.width)
        });
    let base_indent = hanging_indent.or(content_indent).or_else(|| {
        if freshly_indented {
            measured
                .iter()
                .filter(|row| row.eligible && row.starts_physical_line)
                .map(|row| trailing_indent_width(&row.prefix, tab_width))
                .min()
        } else {
            None
        }
    });
    for row in measured
        .iter_mut()
        .filter(|row| row.eligible && row.cells.first().is_some_and(|cell| cell.text.is_empty()))
    {
        if row.starts_physical_line {
            if let Some(base) = base_indent {
                set_trailing_indent(&mut row.prefix, base, tab_width);
            }
        } else {
            let end = row.prefix.trim_end_matches([' ', '\t']).len();
            if end < row.prefix.len() {
                row.prefix.to_mut().truncate(end);
            }
        }
    }
    if let Some(base) = hanging_indent {
        for row in measured
            .iter_mut()
            .filter(|row| row.eligible && row.starts_physical_line)
        {
            set_trailing_indent(&mut row.prefix, base, tab_width);
        }
    }
    // Right padding in the first column shares the physical indentation prefix.
    // Recover the common base indentation so reformatting does not accumulate it.
    if first_column_has_leading_padding {
        let base = measured
            .iter()
            .filter(|r| r.eligible && r.starts_physical_line)
            .map(|r| trailing_indent_width(&r.prefix, tab_width))
            .min()
            .unwrap_or(0);
        for row in measured.iter_mut().filter(|r| r.eligible) {
            if row.starts_physical_line {
                set_trailing_indent(&mut row.prefix, base, tab_width);
            } else {
                let end = row.prefix.trim_end_matches([' ', '\t']).len();
                // Whitespace separates a bracketed first cell from a tight
                // trailing header option. Keep that token boundary while
                // discarding any padding left by a previous alignment pass.
                let separator = row.prefix.len() > end
                    && row
                        .cells
                        .first()
                        .is_some_and(|cell| cell.text.starts_with('['));
                if end < row.prefix.len() {
                    row.prefix.to_mut().truncate(end);
                }
                if separator {
                    row.prefix.to_mut().push(' ');
                }
            }
        }
    }
    if let Some(fold) = fold {
        for row in measured
            .iter_mut()
            .filter(|row| row.eligible && row.starts_physical_line)
        {
            let indent = trailing_indent_width(&row.prefix, tab_width);
            row.folded |= row.natural_width(settings, indent) > fold.limit;
        }
    }
    let mut labels =
        label_layout.and_then(|layout| layout.extract(measured, source, settings.width));
    let ordinary = |row: &&MeasuredRow<'_>| row.eligible && !row.folded;
    let columns = if fold.is_some() {
        measured
            .iter()
            .filter(ordinary)
            .map(|row| row.columns)
            .max()
            .unwrap_or(0)
    } else {
        columns
    };
    let mut widths = vec![0usize; columns];
    let delimiter_width = measured
        .iter()
        .filter(|row| !row.folded)
        .flat_map(|r| &r.delimiters)
        .map(|d| measure_width(d, settings.width))
        .max()
        .unwrap_or(1);
    let gap = settings.spaces_before + delimiter_width + settings.spaces_after;
    for row in measured.iter().filter(ordinary) {
        let mut column = 0;
        for cell in &row.cells {
            if cell.span == 1 && cell.measure {
                widths[column] = widths[column].max(cell.width);
            }
            column += cell.span;
        }
    }
    if settings.dont_measure_largest {
        let maxima = widths.clone();
        widths.fill(0);
        for row in measured.iter().filter(ordinary) {
            let mut column = 0;
            for cell in &row.cells {
                if cell.span == 1 && cell.measure && cell.width < maxima[column] {
                    widths[column] = widths[column].max(cell.width);
                }
                column += cell.span;
            }
        }
    }
    let mut decimal_widths = Vec::new();
    if settings.align_decimals {
        decimal_widths.resize(columns, (0, 0));
        for row in measured.iter().filter(ordinary) {
            let mut column = 0;
            for cell in &row.cells {
                if let Some(left) = cell.decimal_at
                    && cell.measure
                    && (!settings.dont_measure_largest || cell.width <= widths[column])
                {
                    let (before, after) = &mut decimal_widths[column];
                    *before = (*before).max(left);
                    *after = (*after).max(cell.width - left);
                }
                column += cell.span;
            }
        }
        for (width, (left, right)) in widths.iter_mut().zip(&decimal_widths) {
            *width = (*width).max(left + right);
        }
    }
    for row in measured.iter().filter(ordinary) {
        let mut column = 0;
        for cell in &row.cells {
            let span = cell.span;
            if span > 1 && cell.measure {
                let available =
                    widths[column..column + span].iter().sum::<usize>() + gap * (span - 1);
                if cell.width > available {
                    widths[column + span - 1] += cell.width - available;
                }
            }
            column += span;
        }
    }
    let aligns_break = |row: &MeasuredRow<'_>| {
        row.has_row_break
            && (settings.align_content_after_row_break
                || settings.align_double_backslash
                    && (Some(row.original.start) != last_data
                        || settings.align_final_double_backslash))
    };
    let break_target = fold.and_then(|_| {
        measured
            .iter()
            .filter(ordinary)
            .find(|row| aligns_break(row) && row.starts_physical_line)
            .map(|row| {
                trailing_indent_width(&row.prefix, tab_width)
                    + widths.iter().sum::<usize>()
                    + gap * columns.saturating_sub(1)
                    + settings.spaces_before_row_break
            })
    });
    for row in measured.iter() {
        if !row.eligible {
            output.push_str(&apply_edits(source, row.original.clone(), children));
            continue;
        }
        output.push_str(&row.prefix);
        let align_row_break = aligns_break(row);
        let mut formula_end = None;
        if let Some(fold) = fold.filter(|_| row.folded) {
            fold.write(
                row,
                settings,
                tab_width,
                break_target.filter(|_| align_row_break),
                output,
            );
        } else {
            let mut column = 0;
            for (i, cell) in row.cells.iter().enumerate() {
                let span = cell.span;
                // Without a nonempty physical first cell, its indentation cannot
                // be separated from existing column padding. Keep that prefix and
                // avoid generating more first-column padding on each pass.
                let preserve_empty_prefix = i == 0
                    && row.starts_physical_line
                    && cell.text.is_empty()
                    && base_indent.is_none();
                let target = widths[column..column + span].iter().sum::<usize>() + gap * (span - 1);
                let cell_justification = justification(column, cell);
                let pad_cell = !preserve_empty_prefix
                    && (i < row.delimiters.len()
                        || align_row_break
                        || settings.justification == Justification::Columns
                            && cell_justification != Justification::Left);
                let pad = target.saturating_sub(cell.width);
                let (before_pad, after_pad) = if let Some(left) = cell.decimal_at {
                    let before = decimal_widths[column].0.saturating_sub(left).min(pad);
                    (before, pad - before)
                } else {
                    match cell_justification {
                        Justification::Right => (pad, 0),
                        Justification::Center => (pad / 2, pad - pad / 2),
                        _ => (0, pad),
                    }
                };
                if pad_cell {
                    output.extend(std::iter::repeat_n(' ', before_pad));
                }
                output.push_str(&cell.text);
                if labels.is_some() && i + 1 == row.cells.len() {
                    formula_end = Some(output.len());
                }
                // A final cell without an aligned row terminator has no following
                // delimiter to position. Preserve its existing suffix without
                // adding center padding that would grow again on the next pass.
                if pad_cell && (i < row.delimiters.len() || align_row_break) {
                    output.extend(std::iter::repeat_n(' ', after_pad));
                }
                if let Some(delimiter) = row.delimiters.get(i) {
                    let before = if preserve_empty_prefix {
                        settings
                            .spaces_before
                            .saturating_sub(trailing_indent_width(&row.prefix, tab_width))
                    } else {
                        settings.spaces_before
                    };
                    output.extend(std::iter::repeat_n(' ', before));
                    let delimiter_pad =
                        delimiter_width.saturating_sub(measure_width(delimiter, settings.width));
                    let (before_pad, after_pad) = match settings.delimiter_justification {
                        Justification::Right => (delimiter_pad, 0),
                        Justification::Center => {
                            (delimiter_pad / 2, delimiter_pad - delimiter_pad / 2)
                        }
                        _ => (0, delimiter_pad),
                    };
                    output.extend(std::iter::repeat_n(' ', before_pad));
                    output.push_str(delimiter);
                    output.extend(std::iter::repeat_n(' ', after_pad));
                    output.extend(std::iter::repeat_n(' ', settings.spaces_after));
                } else if row.has_row_break {
                    if align_row_break && row.columns < columns {
                        let missing_width = widths[row.columns..].iter().sum::<usize>()
                            + gap * (columns - row.columns);
                        output.extend(std::iter::repeat_n(' ', missing_width));
                    }
                    output.extend(std::iter::repeat_n(' ', settings.spaces_before_row_break));
                }
                column += span;
            }
        }
        if let (Some(labels), Some(end)) = (&mut labels, formula_end) {
            labels.record(output, end, row, align_row_break, settings, tab_width);
        }
        if settings.align_content_after_row_break && row.has_row_break {
            let (terminator, tail) = row.suffix.split_at(row.terminator_length);
            output.push_str(terminator);
            let content = tail.trim_start_matches([' ', '\t']);
            if !content.is_empty() && !content.starts_with(['\r', '\n']) {
                output.extend(std::iter::repeat_n(' ', settings.spaces_after_row_break));
                output.push_str(content);
            } else {
                output.push_str(tail);
            }
        } else {
            output.push_str(row.suffix);
        }
    }
    if let Some(labels) = labels {
        labels.write(output, settings);
    }
}

impl MeasuredRow<'_> {
    fn natural_width(&self, settings: &AlignmentConfig, indent: usize) -> usize {
        let mut col = indent;
        let mut widest = 0;
        for (i, cell) in self.cells.iter().enumerate() {
            if let Some((first, rest)) = cell.text.split_once('\n') {
                widest = widest.max(col + measure_width(first, settings.width));
                for line in rest.split('\n') {
                    col = measure_width(line, settings.width);
                    widest = widest.max(col);
                }
            } else {
                col += cell.width;
            }
            if let Some(delimiter) = self.delimiters.get(i) {
                col += settings.spaces_before
                    + measure_width(delimiter, settings.width)
                    + settings.spaces_after;
            }
        }
        if self.has_row_break {
            col += settings.spaces_before_row_break
                + measure_width(&self.suffix[..self.terminator_length], settings.width);
        }
        widest.max(col)
    }
}

impl RowFold {
    fn write(
        &self,
        row: &MeasuredRow<'_>,
        settings: &AlignmentConfig,
        tab_width: usize,
        target: Option<usize>,
        output: &mut String,
    ) {
        let indent = &row.prefix[row.prefix.trim_end_matches([' ', '\t']).len()..];
        output.push_str(&row.cells[0].text);
        for (i, (delimiter, cell)) in row.delimiters.iter().zip(&row.cells[1..]).enumerate() {
            if i != 0 || !row.cells[0].text.is_empty() {
                output.push_str(self.newline);
                output.push_str(indent);
                output.push_str(&self.unit);
            }
            output.push_str(delimiter);
            if !cell.text.is_empty() {
                output.extend(std::iter::repeat_n(' ', settings.spaces_after));
                output.push_str(&cell.text);
            }
        }
        if row.has_row_break {
            let line = output.rsplit('\n').next().unwrap_or("");
            let current = measure_width(line, settings.width)
                + line.bytes().filter(|&b| b == b'\t').count() * tab_width.saturating_sub(1);
            let suffix = row.suffix.lines().next().unwrap_or("");
            let pad = target
                .filter(|t| t + measure_width(suffix, settings.width) <= self.limit)
                .map_or(0, |t| t.saturating_sub(current))
                .max(settings.spaces_before_row_break);
            output.extend(std::iter::repeat_n(' ', pad));
        }
    }
}

fn decimal_anchor(text: &str, marker: &[char]) -> Option<usize> {
    let unsigned = text.strip_prefix(['+', '-']).unwrap_or(text);
    let mut mantissa = unsigned;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if let Some((base, exponent)) = mantissa.split_once(['e', 'E']) {
        if !digits(exponent.strip_prefix(['+', '-']).unwrap_or(exponent)) {
            return None;
        }
        mantissa = base;
    }
    if let Some((base, uncertainty)) = mantissa.split_once('(') {
        if !digits(uncertainty.strip_suffix(')')?) {
            return None;
        }
        mantissa = base;
    }
    let (whole, fraction) = mantissa.split_once(marker).unwrap_or((mantissa, ""));
    if (whole.is_empty() && fraction.is_empty())
        || !whole
            .bytes()
            .chain(fraction.bytes())
            .all(|b| b.is_ascii_digit())
    {
        return None;
    }
    Some(text.len() - unsigned.len() + whole.len())
}

fn measure_width(cell: &str, width: Width) -> usize {
    // A delimiter follows the final physical line of a multiline cell.
    let final_line = cell
        .rsplit('\n')
        .next()
        .unwrap_or(cell)
        .trim_end_matches('\r');
    if width == Width::Graphemes && final_line.is_ascii() {
        return final_line.len();
    }
    match width {
        Width::Graphemes => final_line.graphemes(true).count(),
        Width::Display => UnicodeWidthStr::width(final_line),
    }
}

fn trailing_indent_width(prefix: &str, tab_width: usize) -> usize {
    prefix[prefix.trim_end_matches([' ', '\t']).len()..]
        .chars()
        .map(|character| if character == '\t' { tab_width } else { 1 })
        .sum()
}

fn set_trailing_indent(prefix: &mut Cow<'_, str>, target: usize, tab_width: usize) {
    if trailing_indent_width(prefix, tab_width) == target {
        return;
    }
    let start = prefix.trim_end_matches([' ', '\t']).len();
    let mut width = 0;
    let mut keep = 0;
    for character in prefix[start..].chars() {
        let next = if character == '\t' { tab_width } else { 1 };
        if width + next > target {
            break;
        }
        width += next;
        keep += character.len_utf8();
    }
    let prefix = prefix.to_mut();
    prefix.truncate(start + keep);
    prefix.extend(std::iter::repeat_n(' ', target - width));
}

fn contains_comment(text: &str, range: Range<usize>, opaque: &[Range<usize>]) -> bool {
    memchr::memchr_iter(b'%', &text.as_bytes()[range.clone()]).any(|relative| {
        let offset = range.start + relative;
        !crate::syntax::is_protected(offset, opaque) && !crate::syntax::is_escaped(text, offset)
    })
}

// Empty comments at the opening boundary are TeX trivia. Keep their complete
// lines in the row prefix so they neither add width nor lose their newline.
fn alignment_trivia_end(text: &str, range: Range<usize>, opaque: &[Range<usize>]) -> usize {
    let mut cursor = range.start;
    loop {
        while cursor < range.end && matches!(text.as_bytes()[cursor], b' ' | b'\t' | b'\r' | b'\n')
        {
            cursor += 1;
        }
        if cursor >= range.end
            || text.as_bytes()[cursor] != b'%'
            || crate::syntax::is_protected(cursor, opaque)
            || crate::syntax::is_escaped(text, cursor)
        {
            return cursor;
        }
        let Some(newline) = text[cursor + 1..range.end].find('\n') else {
            return cursor;
        };
        let end = cursor + 1 + newline;
        if !text[cursor + 1..end]
            .bytes()
            .all(|byte| matches!(byte, b' ' | b'\t' | b'\r'))
        {
            return cursor;
        }
        cursor = end + 1;
    }
}

// An empty closing-boundary comment is emitted as a suffix of the final cell.
// Other cell comments remain protected: trimming their line ending could move
// an alignment delimiter into the comment.
fn final_cell_suffix_start(text: &str, range: Range<usize>, opaque: &[Range<usize>]) -> usize {
    let mut end = range.end;
    loop {
        let content_end = range.start + trim_cell_end(&text[range.start..end]).len();
        let Some(percent) = content_end
            .checked_sub(1)
            .filter(|offset| *offset >= range.start)
        else {
            return content_end;
        };
        if text.as_bytes()[percent] != b'%'
            || !text[content_end..end].contains('\n')
            || crate::syntax::is_protected(percent, opaque)
            || crate::syntax::is_escaped(text, percent)
        {
            return content_end;
        }
        let line_start = text[..percent].rfind('\n').map_or(0, |offset| offset + 1);
        if contains_comment(text, line_start..percent, opaque) {
            return content_end;
        }
        end = percent;
    }
}

fn trim_cell_end(text: &str) -> &str {
    let bytes = text.as_bytes();
    let mut end = text.len();
    while end > 0 && matches!(bytes[end - 1], b' ' | b'\t' | b'\r' | b'\n') {
        let mut slash_start = end - 1;
        while slash_start > 0 && bytes[slash_start - 1] == b'\\' {
            slash_start -= 1;
        }
        if (end - 1 - slash_start) % 2 == 1 {
            break;
        }
        end -= 1;
    }
    &text[..end]
}

fn multicolumn_span(text: &str) -> Option<usize> {
    let rest = text.strip_prefix("\\multicolumn")?;
    let rest = rest.trim_start();
    let body = rest.strip_prefix('{')?;
    let end = body.find('}')?;
    let span = body[..end].trim().parse::<usize>().ok()?;
    (1..=256).contains(&span).then_some(span)
}

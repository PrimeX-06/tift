//! Paragraph layout, comment wrapping and configurable sentence boundaries.
//!
//! Prose layout keeps argument, mathematics and literal spans atomic.
//! Standard text arguments have a separate whitespace-safe wrap pass.
//! Multi-line atomic spans form paragraph boundaries.

use crate::edit_plan::EditPlan;
use std::borrow::Cow;
use std::collections::HashSet;
use std::ops::Range;

use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Stage {
    #[default]
    Before,
    After,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct WrapConfig {
    /// Align the body text and its continuation lines in description lists.
    pub align_description: bool,
    /// `preserve`, `wrap`, `unwrap`, or `rewrap` (join then wrap).
    pub mode: String,
    pub columns: usize,
    /// Prefer nearby sentence endings, then Chinese clause punctuation, on overflow.
    pub prefer_sentence_ends: bool,
    /// Keep figure and table names with a following literal reference.
    pub keep_reference_names: bool,
    /// Keep a plain number and a recognized unit symbol on the same source line.
    pub keep_number_units: bool,
    /// Keep the last two English words together when they fit on a continuation.
    pub avoid_lone_last_word: bool,
    #[serde(rename = "stage")]
    pub when: Stage,
    /// Additional control-word names allowed to begin prose lines.
    pub prose_commands: Vec<String>,
    /// Standard commands whose ordinary text argument may wrap at existing
    /// ASCII whitespace. None selects the safe defaults in wrap mode;
    /// an explicit empty list disables argument wrapping.
    pub text_commands: Option<Vec<String>>,
    /// Additional plain-text characters after which wrapping may break.
    pub break_chars: Vec<char>,
    /// Collapse repeated spaces in prose, including when preserving line breaks.
    pub collapse_spaces: bool,
    /// Regex matching the beginning of an eligible prose block.
    pub begin_pattern: Option<String>,
    /// Regex before which a prose block ends; the remaining source line is kept.
    pub end_before_pattern: Option<String>,
    /// Regex matching the end of the source line preceding a new prose block.
    pub follow_pattern: Option<String>,
    /// Inserted wrap separator. Empty chooses the source line ending.
    pub separator: String,
    /// `overflow` keeps words intact; `break` splits plain text at graphemes.
    #[serde(rename = "long-words")]
    pub huge: String,
}

impl Default for WrapConfig {
    fn default() -> Self {
        Self {
            align_description: false,
            mode: "preserve".into(),
            columns: 80,
            prefer_sentence_ends: true,
            keep_reference_names: true,
            keep_number_units: true,
            avoid_lone_last_word: false,
            when: Stage::Before,
            prose_commands: Vec::new(),
            text_commands: None,
            break_chars: Vec::new(),
            collapse_spaces: true,
            begin_pattern: None,
            end_before_pattern: None,
            follow_pattern: None,
            separator: String::new(),
            huge: "overflow".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct SentenceConfig {
    pub enabled: bool,
    /// Keep existing physical line boundaries inside a sentence.
    pub preserve_line_breaks: bool,
    #[serde(rename = "stage")]
    pub when: Stage,
    pub abbreviations: Vec<String>,
    /// Optional regex matching the beginning of the following sentence.
    pub begin_pattern: Option<String>,
    /// Optional regex matching a sentence ending, including its punctuation.
    pub end_pattern: Option<String>,
    /// Additional indentation on newly separated sentence lines.
    pub indent: String,
    /// Prose blocks containing this regex are excluded from sentence changes.
    pub exclude_pattern: Option<String>,
}

impl Default for SentenceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            preserve_line_breaks: true,
            when: Stage::Before,
            abbreviations: [
                "Mr.", "Mrs.", "Ms.", "Dr.", "Prof.", "Sr.", "Jr.", "e.g.", "i.e.", "et al.",
                "vs.", "cf.", "Fig.", "Figs.", "Eq.", "Eqs.", "Sec.", "Secs.", "No.",
            ]
            .map(String::from)
            .to_vec(),
            begin_pattern: None,
            end_pattern: None,
            indent: String::new(),
            exclude_pattern: None,
        }
    }
}

#[derive(Debug)]
enum Pattern {
    Linear(regex::Regex),
    Advanced(fancy_regex::Regex),
}

impl Pattern {
    fn compile(pattern: &str) -> Result<Self, String> {
        match regex::Regex::new(pattern) {
            Ok(regex) => Ok(Self::Linear(regex)),
            Err(_) => fancy_regex::RegexBuilder::new(pattern)
                .backtrack_limit(1_000_000)
                .build()
                .map(Self::Advanced)
                .map_err(|error| error.to_string()),
        }
    }

    fn find(&self, text: &str) -> Result<Option<Range<usize>>, String> {
        match self {
            Self::Linear(regex) => Ok(regex.find(text).map(|m| m.range())),
            Self::Advanced(regex) => regex
                .find(text)
                .map(|m| m.map(|m| m.range()))
                .map_err(|error| error.to_string()),
        }
    }

    fn matches(&self, text: &str) -> Result<Vec<Range<usize>>, String> {
        match self {
            Self::Linear(regex) => Ok(regex.find_iter(text).map(|m| m.range()).collect()),
            Self::Advanced(regex) => regex
                .find_iter(text)
                .map(|m| m.map(|m| m.range()).map_err(|error| error.to_string()))
                .collect(),
        }
    }
}

#[derive(Debug)]
pub struct CompiledText {
    sentence_begin: Option<Pattern>,
    sentence_end: Pattern,
    default_ending: bool,
    abbreviations: Vec<String>,
    abbreviation_context: usize,
    block_begin: Option<Pattern>,
    block_end: Option<Pattern>,
    block_follow: Option<Pattern>,
    sentence_exclude: Option<Pattern>,
    prose_commands: HashSet<String>,
    text_commands: HashSet<String>,
    break_chars: HashSet<char>,
}

pub fn compile(wrap: &WrapConfig, sentences: &SentenceConfig) -> Result<CompiledText, String> {
    if !matches!(
        wrap.mode.as_str(),
        "preserve" | "wrap" | "stable" | "unwrap" | "rewrap"
    ) {
        return Err("wrap.mode must be preserve, wrap, stable, unwrap, or rewrap".into());
    }
    if wrap.columns == 0 || wrap.columns > 1_000_000 {
        return Err("wrap.columns must be between 1 and 1000000".into());
    }
    if !matches!(wrap.huge.as_str(), "overflow" | "break") {
        return Err("wrap.long-words must be overflow or break".into());
    }
    let mut prose_commands = HashSet::new();
    for name in &wrap.prose_commands {
        let name = name.strip_prefix('\\').unwrap_or(name);
        let name = name.strip_suffix('*').unwrap_or(name);
        if name.is_empty()
            || !name
                .chars()
                .all(|character| character.is_ascii_alphabetic() || "@_:".contains(character))
        {
            return Err("wrap.prose-commands must contain control-word names".into());
        }
        prose_commands.insert(name.to_owned());
    }
    let mut text_commands = HashSet::new();
    let selected = wrap.text_commands.as_deref().unwrap_or_else(|| {
        if matches!(wrap.mode.as_str(), "wrap" | "stable" | "rewrap") {
            default_text_commands()
        } else {
            &[]
        }
    });
    for entry in selected {
        let name = entry.strip_prefix('\\').unwrap_or(entry);
        if text_argument_signature(name).is_none() {
            return Err(format!(
                "wrap.text-commands requires a supported ordinary-text command: {entry:?}"
            ));
        }
        text_commands.insert(name.to_owned());
    }
    if wrap.break_chars.iter().any(|character| {
        character.is_whitespace()
            || character.is_control()
            || "\\{}[]$%~".contains(*character)
            || matches!(*character, '\u{a0}' | '\u{202f}' | '\u{2060}')
    }) {
        return Err("wrap.break-chars must contain plain-text non-whitespace characters".into());
    }
    if !matches!(
        wrap.separator.as_str(),
        "" | "\n" | "\r\n" | "%\n" | "%\r\n"
    ) {
        return Err("wrap.separator must be empty, LF, CRLF, % + LF, or % + CRLF".into());
    }
    if !sentences.indent.chars().all(|c| c == ' ' || c == '\t') {
        return Err("sentences.indent must contain only spaces or tabs".into());
    }
    let sentence_end =
        Pattern::compile(sentences.end_pattern.as_deref().unwrap_or(r"[.!?。！？]+"))
            .map_err(|error| format!("sentences.end-pattern: {error}"))?;
    if sentence_end.find("")?.is_some() {
        return Err("sentences.end-pattern must not match empty text".into());
    }
    let sentence_begin = sentences
        .begin_pattern
        .as_deref()
        .map(|pattern| Pattern::compile(&format!(r"\A(?:{pattern})")))
        .transpose()
        .map_err(|error| format!("sentences.begin-pattern: {error}"))?;
    let optional = |value: &Option<String>, name: &str, start: bool, end: bool| {
        value
            .as_ref()
            .map(|pattern| {
                let pattern = if start {
                    format!(r"\A(?:{pattern})")
                } else if end {
                    format!(r"(?:{pattern})\z")
                } else {
                    pattern.clone()
                };
                Pattern::compile(&pattern).map_err(|error| format!("{name}: {error}"))
            })
            .transpose()
    };
    let block_end = optional(
        &wrap.end_before_pattern,
        "wrap.end-before-pattern",
        false,
        false,
    )?;
    if block_end
        .as_ref()
        .map(|pattern| pattern.find(""))
        .transpose()?
        .flatten()
        .is_some()
    {
        return Err("wrap.end-before-pattern must not match empty text".into());
    }
    Ok(CompiledText {
        sentence_begin,
        sentence_end,
        default_ending: sentences.end_pattern.is_none(),
        abbreviation_context: sentences
            .abbreviations
            .iter()
            .map(|entry| entry.chars().count())
            .max()
            .unwrap_or(0)
            + 1,
        abbreviations: sentences
            .abbreviations
            .iter()
            .filter(|entry| !entry.is_empty())
            .map(|entry| entry.to_lowercase())
            .collect(),
        block_begin: optional(&wrap.begin_pattern, "wrap.begin-pattern", true, false)?,
        block_end,
        block_follow: optional(&wrap.follow_pattern, "wrap.follow-pattern", false, true)?,
        sentence_exclude: optional(
            &sentences.exclude_pattern,
            "sentences.exclude-pattern",
            false,
            false,
        )?,
        prose_commands,
        text_commands,
        break_chars: wrap.break_chars.iter().copied().collect(),
    })
}

#[derive(Debug)]
struct Paragraph {
    span: Range<usize>,
    regions: Vec<Region>,
    prefix: String,
    continuation: String,
    ending: String,
    newline: &'static str,
    comment: bool,
    join_with_space: bool,
}

#[derive(Debug)]
struct Region {
    range: Range<usize>,
    space_before: bool,
    newline_before: Option<Cow<'static, str>>,
}

#[derive(Debug)]
struct Canonical {
    text: String,
    opaque: Vec<Range<usize>>,
    quantities: Vec<Range<usize>>,
    hard_breaks: Vec<HardBreak>,
}

#[derive(Debug)]
struct HardBreak {
    before: usize,
    after: usize,
    newline: Cow<'static, str>,
}

/// Apply active text transformations at the requested pipeline stage.
pub(crate) fn is_active(config: &crate::CompiledConfig, stage: Stage) -> bool {
    config.config.wrap.mode != "preserve" && config.config.wrap.when == stage
        || config.config.sentences.enabled && config.config.sentences.when == stage
}

pub(crate) fn collapse_prose_spaces(
    source: &str,
    config: &crate::CompiledConfig,
    parsing: &mut crate::parse_context::ParseContext<'_>,
) -> Result<String, crate::FormatError> {
    let grammar = &config.config.syntax;
    if grammar.command_pattern.is_some()
        || grammar.environment_pattern.is_some()
        || grammar.condition_pattern.is_some()
        || grammar.item_pattern.is_some()
        || grammar.argument_between.is_some()
        || !grammar.special.is_empty()
        || !config.config.protect.dynamic_catcodes
    {
        return Ok(source.into());
    }

    let needs_environments = source.contains("\\begin") || source.contains("\\end");
    let parsed = parsing.snapshot(source, needs_environments, false)?;
    let analysis = parsed.analysis(&config.config);
    let declared_commands = analysis.declared_commands();
    let declared_environments = analysis.declared_environments();
    let mut opaque = parsed.protected.clone();
    let mut body = 0..source.len();
    if needs_environments {
        let Ok(environments) = parsed.environment_ranges(&config.config) else {
            return Ok(source.into());
        };
        for environment in environments {
            if environment.name == "document" {
                body = environment.body.clone();
            }
            if is_non_prose_environment(&environment.name, config)
                || crate::package_tables::is_table(&environment.name)
                || declared_environments.contains(&environment.name)
                || grammar
                    .inferred_environment_arguments
                    .contains_key(&environment.name)
            {
                opaque.push(environment.begin.start..environment.end.end);
            }
        }
    }
    merge_ranges(&mut opaque);
    let candidates = prose_space_candidates(source, body, &opaque);
    if candidates.is_empty() {
        return Ok(source.into());
    }
    add_inline_atoms_to_merged(source, &mut opaque, &config.config, declared_commands, true);

    let mut edits = EditPlan::new();
    for line in candidates.chunk_by(|left, right| left.0 == right.0) {
        let end = line.last().unwrap().1.end;
        let end_prose = comment_start(source, line[0].0..end, &opaque).unwrap_or(end);
        for (_, spaces) in line
            .iter()
            .take_while(|(_, spaces)| spaces.end <= end_prose)
        {
            if !intersects(spaces.clone(), &opaque) {
                edits.replace(spaces.start + 1..spaces.end, "");
            }
        }
    }
    if !edits.is_empty() && has_deferred_space_setter(source, &parsed.protected) {
        return Ok(source.into());
    }
    edits.apply_ordered(source)
}

fn has_deferred_space_setter(source: &str, protected: &[Range<usize>]) -> bool {
    // A setter stored in a macro, alias or hook can execute after its source
    // range ends. Only use a setter's existing protection when the scanner
    // identified it as the operation opening that range.
    source.match_indices('\\').any(|(at, _)| {
        crate::syntax::command_at(source, at).is_some_and(|(name, _)| {
            matches!(
                name,
                "catcode" | "obeyspaces" | "obeylines" | "scantokens" | "endlinechar"
            ) || crate::packages::code::catcode_value(name).is_some()
        }) && !crate::syntax::is_escaped(source, at)
            && containing(at, protected).is_none_or(|range| {
                let prefix = source[range.start..at].trim();
                let standalone_group = matches!(prefix, "{" | "\\begingroup" | "\\bgroup")
                    && source[..range.start]
                        .rsplit('\n')
                        .next()
                        .is_some_and(|line| line.trim().is_empty());
                !prefix.is_empty() && !standalone_group
            })
            && {
                let line_start = source[..at].rfind('\n').map_or(0, |index| index + 1);
                comment_start(source, line_start..at, protected).is_none()
            }
    })
}

fn prose_space_candidates(
    source: &str,
    body: Range<usize>,
    opaque: &[Range<usize>],
) -> Vec<(usize, Range<usize>)> {
    let mut candidates = Vec::new();
    let mut start = body.start;
    for line in source[body].split_inclusive('\n') {
        start += line.len();
        let content = line.trim_start_matches([' ', '\t']);
        let start_content = start - content.len();
        let content = content.trim_end_matches([' ', '\t', '\r', '\n']);
        if containing(start_content, opaque)
            .is_some_and(|range| range.end >= start_content + content.len())
        {
            continue;
        }
        let mut spaces = crate::trivia::repeated_spaces(content)
            .map(|range| start_content + range.start..start_content + range.end)
            .filter(|range| !intersects(range.clone(), opaque))
            .peekable();
        if spaces.peek().is_some() && !contains_definition(content) {
            candidates.extend(spaces.map(|range| (start_content, range)));
        }
    }
    candidates
}

pub fn format_text(
    source: &str,
    config: &crate::CompiledConfig,
    stage: Stage,
) -> Result<String, crate::FormatError> {
    format_text_with_context(
        source,
        config,
        stage,
        &mut crate::parse_context::ParseContext::new(config),
    )
}

pub(crate) fn format_text_with_context(
    source: &str,
    config: &crate::CompiledConfig,
    stage: Stage,
    parsing: &mut crate::parse_context::ParseContext<'_>,
) -> Result<String, crate::FormatError> {
    format_text_with_arguments(source, config, stage, parsing, true)
}

fn format_text_with_arguments(
    source: &str,
    config: &crate::CompiledConfig,
    stage: Stage,
    parsing: &mut crate::parse_context::ParseContext<'_>,
    arguments: bool,
) -> Result<String, crate::FormatError> {
    let wrap_here = config.config.wrap.mode != "preserve" && config.config.wrap.when == stage;
    let sentences_here = config.config.sentences.enabled && config.config.sentences.when == stage;
    if !wrap_here && !sentences_here {
        return Ok(source.to_owned());
    }
    // The final text stage retains the layout requested by the earlier stage.
    let wrap_active = wrap_here
        || (stage == Stage::After
            && sentences_here
            && config.config.wrap.when == Stage::Before
            && config.config.wrap.mode != "preserve");
    let sentences_active = sentences_here
        || (stage == Stage::After
            && wrap_here
            && config.config.sentences.when == Stage::Before
            && config.config.sentences.enabled);
    let preserve_lines = wrap_active
        && matches!(config.config.wrap.mode.as_str(), "wrap" | "stable")
        || sentences_active
            && config.config.sentences.preserve_line_breaks
            && !(wrap_active && config.config.wrap.mode == "rewrap");
    let needs_environments = config.config.syntax.environment_pattern.is_some()
        || source.contains("\\begin")
        || source.contains("\\end");
    let inline_paths = (config.config.breaks.tikz_path_columns.is_some()
        || config.config.breaks.pgfplots_coordinates)
        && source.contains("\\tikz");
    let parsed = parsing.snapshot(source, needs_environments || inline_paths, inline_paths)?;
    let mut opaque = parsed.protected.clone();
    let environments = if needs_environments {
        parsed.environment_ranges(&config.config)?
    } else {
        &[]
    };
    let analysis = parsed.analysis(&config.config);
    let declared_commands = analysis.declared_commands();
    if inline_paths && let Some(blocks) = &parsed.blocks {
        opaque.extend(
            blocks
                .iter()
                .filter(|block| {
                    block.kind == crate::syntax::BlockKind::TikzPath
                        && !block.begin.is_empty()
                        && matches!(block.name.as_str(), "tikz-inline" | "tikz-scope")
                })
                .map(|block| block.begin.start..block.end.end),
        );
    }
    merge_ranges(&mut opaque);
    add_inline_atoms_to_merged(
        source,
        &mut opaque,
        &config.config,
        declared_commands,
        false,
    );
    for environment in environments {
        if is_non_prose_environment(&environment.name, config) {
            opaque.push(environment.begin.start..environment.end.end);
        }
    }
    merge_ranges(&mut opaque);
    let blocked: Vec<_> = opaque
        .iter()
        .filter(|range| source[(*range).clone()].contains('\n'))
        .cloned()
        .collect();
    let mut edits = EditPlan::new();
    let mut paragraph: Option<Paragraph> = None;
    let mut blocked_item: Option<(usize, String)> = None;
    let mut offset = 0;
    let mut previous_content = "";
    for line in source.split_inclusive('\n') {
        let start = offset;
        offset += line.len();
        let content_end = start + line.trim_end_matches(['\r', '\n']).len();
        let content = &source[start..content_end];
        let ending = &source[content_end..offset];
        let indent_len = content.len() - content.trim_start_matches([' ', '\t']).len();
        let body_start = start + indent_len;
        let body = &source[body_start..content_end];
        let exterior_continuation = if blocked_item.as_ref().is_some_and(|(end, _)| *end <= start) {
            blocked_item.take().map(|(_, prefix)| prefix)
        } else {
            None
        };
        let follows = config
            .text
            .block_follow
            .as_ref()
            .map(|pattern| {
                pattern
                    .find(previous_content)
                    .map(|matched| matched.is_some())
            })
            .transpose()
            .map_err(crate::FormatError::Syntax)?
            .unwrap_or(true);
        previous_content = content;
        if config.text.block_follow.is_some() && follows {
            flush(
                &mut paragraph,
                source,
                &opaque,
                config,
                wrap_active,
                sentences_active,
                &mut edits,
            )?;
        }
        let comment = comment_prefix(body);
        let blocked_line = intersects(start..offset, &blocked)
            || containing(start, &opaque).is_some_and(|range| range.end >= content_end)
            || body.is_empty();
        if blocked_line {
            if wrap_active
                && let Some(length) = item_body(body, &config.config.syntax.items)
                && !contains_definition(body)
                && let Some(atom) = blocked
                    .get(blocked.partition_point(|range| range.end <= body_start))
                    .filter(|range| body_start < range.start && range.start < content_end)
            {
                // The inline atom can span several lines, while the following
                // prose still belongs to the item's hanging paragraph.
                blocked_item = Some((
                    atom.end,
                    item_continuation(
                        &source[start..body_start],
                        &body[..length],
                        config.config.indent.width,
                    ),
                ));
            }
            let continuation = if let Some(current) = paragraph.as_ref().filter(|paragraph| {
                !paragraph.comment && paragraph.prefix != paragraph.continuation
            }) {
                let first = blocked.partition_point(|range| range.start <= body_start);
                let atom = blocked[first..]
                    .iter()
                    .take_while(|range| range.start < content_end)
                    .find(|range| {
                        range.start > body_start
                            && range.start < content_end
                            && range.end >= offset
                            && !has_comment(source, body_start..range.start, &opaque)
                    });
                if atom.is_some()
                    && !contains_definition(body)
                    && allows_prose_start(body, &config.text).map_err(crate::FormatError::Syntax)?
                {
                    blocked_continuation(
                        current,
                        source,
                        body_start..content_end,
                        &opaque,
                        config,
                        sentences_active,
                    )?
                } else {
                    None
                }
            } else {
                None
            };
            flush(
                &mut paragraph,
                source,
                &opaque,
                config,
                wrap_active,
                sentences_active,
                &mut edits,
            )?;
            if let Some(prefix) = continuation {
                // Only the indentation before the multiline atom is adjusted;
                // its contents and the blocked-line protection stay intact.
                if prefix != source[start..body_start] {
                    edits.replace(start..body_start, prefix);
                }
            }
            continue;
        }
        if let Some(prefix_len) = comment {
            let comment_text = &body[prefix_len..];
            if !wrap_active
                || !config.config.comments.wrap
                || comment_text.trim().is_empty()
                || comment_text.starts_with("arara:")
            {
                flush(
                    &mut paragraph,
                    source,
                    &opaque,
                    config,
                    wrap_active,
                    sentences_active,
                    &mut edits,
                )?;
                continue;
            }
            let base_prefix = format!(
                "{}{} ",
                &content[..indent_len],
                body[..prefix_len].trim_end()
            );
            let raw_prefix = &content[..indent_len + prefix_len];
            let list_marker = comment_list_marker(&body[prefix_len..]);
            if paragraph.as_ref().is_some_and(|p| {
                !p.comment
                    || list_marker.is_some()
                    || (p.continuation != base_prefix && p.continuation != raw_prefix)
            }) {
                flush(
                    &mut paragraph,
                    source,
                    &opaque,
                    config,
                    wrap_active,
                    sentences_active,
                    &mut edits,
                )?;
            }
            let paragraph = paragraph.get_or_insert_with(|| {
                let (prefix, continuation) = match list_marker {
                    Some(marker) => (
                        format!("{raw_prefix}{marker}"),
                        comment_item_continuation(raw_prefix, marker, config.config.indent.width),
                    ),
                    None => (base_prefix.clone(), base_prefix),
                };
                Paragraph {
                    span: start..offset,
                    regions: Vec::new(),
                    continuation,
                    prefix,
                    ending: String::new(),
                    newline: if ending == "\r\n" { "\r\n" } else { "\n" },
                    comment: true,
                    join_with_space: true,
                }
            });
            paragraph.regions.push(Region {
                range: body_start + prefix_len + list_marker.map_or(0, str::len)..content_end,
                space_before: true,
                newline_before: paragraph_boundary(&paragraph.ending),
            });
            paragraph.span.end = offset;
            paragraph.ending = ending.into();
            continue;
        }
        if wrap_active
            && config.config.comments.wrap
            && let Some(comment_start) = comment_start(source, body_start..content_end, &opaque)
        {
            let prefix_len =
                comment_prefix(&source[comment_start..content_end]).expect("comment marker");
            let text_start = comment_start + prefix_len;
            let marker = source[comment_start..text_start].trim_end_matches([' ', '\t']);
            let next_content = source[offset..]
                .split('\n')
                .next()
                .unwrap_or("")
                .trim_end_matches('\r');
            let next_body = next_content.trim_start_matches([' ', '\t']);
            let continues = comment_prefix(next_body).is_some_and(|prefix_len| {
                !next_body[prefix_len..].trim().is_empty()
                    && !next_body[prefix_len..].starts_with("arara:")
                    && next_content[..next_content.len() - next_body.len()] == content[..indent_len]
                    && next_body[..prefix_len].trim_end_matches([' ', '\t']) == marker
            });
            if !source[text_start..content_end].trim().is_empty()
                && (matches!(config.config.wrap.mode.as_str(), "unwrap" | "rewrap")
                    || continues
                    || display_width(content, 0, config.config.indent.width)
                        > config.config.wrap.columns)
            {
                flush(
                    &mut paragraph,
                    source,
                    &opaque,
                    config,
                    wrap_active,
                    sentences_active,
                    &mut edits,
                )?;
                let list_marker = comment_list_marker(&source[text_start..content_end]);
                let body_start = text_start + list_marker.map_or(0, str::len);
                let continuation = if let Some(item) = list_marker {
                    let base = format!(
                        "{}{}",
                        &content[..indent_len],
                        &source[comment_start..text_start]
                    );
                    comment_item_continuation(&base, item, config.config.indent.width)
                } else {
                    format!("{}{} ", &content[..indent_len], marker)
                };
                paragraph = Some(Paragraph {
                    span: start..offset,
                    regions: vec![Region {
                        range: body_start..content_end,
                        space_before: true,
                        newline_before: None,
                    }],
                    prefix: source[start..body_start].to_owned(),
                    continuation,
                    ending: ending.into(),
                    newline: if ending == "\r\n" { "\r\n" } else { "\n" },
                    comment: true,
                    join_with_space: true,
                });
                continue;
            }
        }
        let item = item_body(body, &config.config.syntax.items);
        // Empty trailing comments suppress TeX's newline space. Unwrap can
        // remove that physical boundary while retaining any space before '%'.
        let generated_break =
            comment_start(source, body_start..content_end, &opaque).and_then(|marker| {
                let prefix = &source[body_start..marker];
                let custom_break = prefix
                    .chars()
                    .next_back()
                    .is_some_and(|character| config.text.break_chars.contains(&character));
                if ending.is_empty()
                    || !(wrap_active
                        || sentences_active && config.config.sentences.preserve_line_breaks)
                    || !source[marker + 1..content_end]
                        .trim_matches([' ', '\t'])
                        .is_empty()
                    || ends_control_word(prefix)
                    || !(preserve_lines
                        || matches!(config.config.wrap.mode.as_str(), "unwrap" | "rewrap")
                        || config.config.wrap.separator.starts_with('%')
                        || config.config.wrap.huge == "break"
                        || custom_break)
                    || (!preserve_lines
                        && !matches!(config.config.wrap.mode.as_str(), "unwrap" | "rewrap")
                        && !prefix.ends_with([' ', '\t'])
                        && config.config.wrap.huge != "break"
                        && !custom_break)
                {
                    None
                } else {
                    Some((marker, prefix.ends_with([' ', '\t'])))
                }
            });
        let prose_end = generated_break.map_or(content_end, |(marker, _)| marker);
        let unsafe_line = has_comment(source, body_start..prose_end, &opaque)
            || (body.starts_with('\\')
                && item.is_none()
                && !allows_prose_start(body, &config.text).map_err(crate::FormatError::Syntax)?
                && containing(body_start, &opaque).is_none())
            || contains_definition(body);
        if unsafe_line {
            flush(
                &mut paragraph,
                source,
                &opaque,
                config,
                wrap_active,
                sentences_active,
                &mut edits,
            )?;
            continue;
        }
        if item.is_some() || paragraph.as_ref().is_some_and(|p| p.comment) {
            flush(
                &mut paragraph,
                source,
                &opaque,
                config,
                wrap_active,
                sentences_active,
                &mut edits,
            )?;
        }
        let text_start = body_start + item.unwrap_or(0);
        let begins = config
            .text
            .block_begin
            .as_ref()
            .map(|pattern| {
                pattern
                    .find(&source[text_start..prose_end])
                    .map(|matched| matched.is_some())
            })
            .transpose()
            .map_err(crate::FormatError::Syntax)?
            .unwrap_or(true);
        if paragraph.is_none() && (!begins || !follows) {
            continue;
        }
        let end_before = config
            .text
            .block_end
            .as_ref()
            .map(|pattern| pattern.matches(&source[text_start..prose_end]))
            .transpose()
            .map_err(crate::FormatError::Syntax)?
            .and_then(|matches| {
                matches.into_iter().find(|range| {
                    !intersects(text_start + range.start..text_start + range.end, &opaque)
                })
            })
            .map(|range| text_start + range.start);
        if end_before == Some(text_start) {
            flush(
                &mut paragraph,
                source,
                &opaque,
                config,
                wrap_active,
                sentences_active,
                &mut edits,
            )?;
            continue;
        }
        let exterior_continuation = exterior_continuation.filter(|_| item.is_none());
        let prefix = exterior_continuation
            .as_deref()
            .unwrap_or(&source[start..text_start]);
        let continuation = if let Some(item_length) = item {
            item_continuation(
                &source[start..body_start],
                &body[..item_length],
                config.config.indent.width,
            )
        } else if let Some(prefix) = &exterior_continuation {
            prefix.clone()
        } else {
            source[start..body_start].to_owned()
        };
        let current = paragraph.get_or_insert_with(|| Paragraph {
            span: start..offset,
            regions: Vec::new(),
            prefix: prefix.into(),
            continuation,
            ending: String::new(),
            newline: if ending == "\r\n" { "\r\n" } else { "\n" },
            comment: false,
            join_with_space: true,
        });
        current.regions.push(Region {
            range: text_start..end_before.unwrap_or(prose_end),
            space_before: current.join_with_space,
            newline_before: paragraph_boundary(&current.ending),
        });
        current.span.end = offset;
        current.join_with_space = generated_break.is_none_or(|(_, space)| space);
        current.ending = match generated_break {
            Some((marker, space)) => format!(
                "{}{}",
                if space { " " } else { "" },
                &source[marker..offset]
            ),
            None => ending.into(),
        };
        if let Some(boundary) = end_before {
            // Keep the whitespace at the exclusion boundary and all following
            // source bytes. No synthetic newline is inserted before the suffix.
            let whitespace = &source[text_start..boundary];
            let retained_start =
                boundary - (whitespace.len() - whitespace.trim_end_matches([' ', '\t']).len());
            current.span.end = retained_start;
            current.ending.clear();
            flush(
                &mut paragraph,
                source,
                &opaque,
                config,
                wrap_active,
                sentences_active,
                &mut edits,
            )?;
        }
    }
    flush(
        &mut paragraph,
        source,
        &opaque,
        config,
        wrap_active,
        sentences_active,
        &mut edits,
    )?;
    let output = edits.apply_ordered(source)?;
    if arguments
        && wrap_active
        && matches!(
            config.config.wrap.mode.as_str(),
            "wrap" | "stable" | "rewrap"
        )
        && !config.text.text_commands.is_empty()
    {
        let (formatted, unblocked) = format_text_arguments(&output, config, parsing)?;
        if unblocked && config.config.wrap.mode == "rewrap" {
            // A formerly multiline inline atom no longer blocks its paragraph.
            // Reflow that newly available prose with the final argument text;
            // argument layout is already complete for this stage.
            format_text_with_arguments(&formatted, config, stage, parsing, false)
        } else {
            Ok(formatted)
        }
    } else {
        Ok(output)
    }
}

fn blocked_continuation(
    paragraph: &Paragraph,
    source: &str,
    range: Range<usize>,
    opaque: &[Range<usize>],
    config: &crate::CompiledConfig,
    sentences_active: bool,
) -> Result<Option<String>, crate::FormatError> {
    if item_body(&source[range.clone()], &config.config.syntax.items).is_some() {
        return Ok(None);
    }
    let ends_before = config
        .text
        .block_end
        .as_ref()
        .map(|pattern| pattern.matches(&source[range.clone()]))
        .transpose()
        .map_err(crate::FormatError::Syntax)?
        .is_some_and(|matches| {
            matches.into_iter().any(|boundary| {
                !intersects(
                    range.start + boundary.start..range.start + boundary.end,
                    opaque,
                )
            })
        });
    if ends_before {
        return Ok(None);
    }
    let mut prefix = paragraph.continuation.clone();
    if !sentences_active || config.config.sentences.indent.is_empty() {
        return Ok(Some(prefix));
    }
    let mut regions = Vec::with_capacity(paragraph.regions.len() + 1);
    regions.extend(paragraph.regions.iter().map(|region| Region {
        range: region.range.clone(),
        space_before: region.space_before,
        newline_before: region.newline_before.clone(),
    }));
    regions.push(Region {
        range,
        space_before: paragraph.join_with_space,
        newline_before: paragraph_boundary(&paragraph.ending),
    });
    let canonical = canonicalize(source, &regions, opaque, config.config.wrap.collapse_spaces);
    let excluded = config
        .text
        .sentence_exclude
        .as_ref()
        .map(|pattern| pattern.find(&canonical.text).map(|value| value.is_some()))
        .transpose()
        .map_err(crate::FormatError::Syntax)?
        .unwrap_or(false);
    if !excluded
        && sentence_ranges(&canonical, &config.text)
            .map_err(crate::FormatError::Syntax)?
            .len()
            > 1
    {
        prefix.push_str(&config.config.sentences.indent);
    }
    Ok(Some(prefix))
}

fn line_ending(text: &str) -> Option<&'static str> {
    if text.ends_with("\r\n") {
        Some("\r\n")
    } else if text.ends_with('\n') {
        Some("\n")
    } else {
        None
    }
}

fn paragraph_boundary(text: &str) -> Option<Cow<'static, str>> {
    line_ending(text).map(|newline| {
        if text.contains('%') {
            Cow::Owned(text.to_owned())
        } else {
            Cow::Borrowed(newline)
        }
    })
}

/// Each segment ends with a printable, braced argument. Previous slots
/// describe its optional short entry, colour, or float type.
fn text_argument_signature(name: &str) -> Option<&'static [&'static str]> {
    Some(match name {
        "caption" | "caption*" | "footnote" | "footnotetext" => &["om"],
        "captionof" | "captionof*" => &["mom"],
        "textcolor" => &["omm"],
        "bicaption" | "bisubcaption" | "subbicaption" => &["om", "om"],
        "bicaption*" | "bisubcaption*" | "subbicaption*" => &["m", "m"],
        "emph" | "textbf" | "textit" | "textsf" | "textrm" | "textsc" | "textnormal" | "textup"
        | "textsl" | "underline" => &["m"],
        _ => return None,
    })
}

fn default_text_commands() -> &'static [String] {
    static COMMANDS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    COMMANDS.get_or_init(|| {
        [
            "caption",
            "caption*",
            "captionof",
            "captionof*",
            "bicaption",
            "bicaption*",
            "bisubcaption",
            "bisubcaption*",
            "subbicaption",
            "subbicaption*",
            "footnote",
            "footnotetext",
            "emph",
            "textbf",
            "textit",
            "textsf",
            "textrm",
            "textsc",
            "textnormal",
            "textup",
            "textsl",
            "underline",
            "textcolor",
        ]
        .map(String::from)
        .to_vec()
    })
}

fn standard_arguments(
    source: &str,
    mut cursor: usize,
    name: &str,
    protected: &[Range<usize>],
) -> Option<Vec<Range<usize>>> {
    let mut segments = text_argument_signature(name)?;
    if matches!(segments, ["om", "om"]) {
        let star = crate::trivia::skip(source, cursor);
        if source.as_bytes().get(star) == Some(&b'*') {
            cursor = star + 1;
            segments = &["m", "m"];
        }
    }
    let mut bodies = Vec::with_capacity(segments.len());
    for segment in segments {
        let mut start = cursor;
        for slot in segment.bytes() {
            start = crate::trivia::skip(source, cursor);
            let (open, close) = if slot == b'o' {
                (b'[', b']')
            } else {
                (b'{', b'}')
            };
            if slot == b'o' && source.as_bytes().get(start) != Some(&open) {
                continue;
            }
            if source.as_bytes().get(start) != Some(&open) || containing(start, protected).is_some()
            {
                return None;
            }
            cursor = balanced_end(source, start, open, close, protected)?;
        }
        bodies.push(start + 1..cursor - 1);
    }
    Some(bodies)
}

fn safe_text_body(
    source: &str,
    range: Range<usize>,
    declared: &HashSet<String>,
    config: &crate::CompiledConfig,
) -> bool {
    let mut cursor = range.start;
    while cursor < range.end {
        match source.as_bytes()[cursor] {
            b'%' => return false,
            b'\\' => {
                if source[cursor..].starts_with("\\(") {
                    let end = math_end(source, cursor + 2, "\\)", &[]);
                    if end > range.end {
                        return false;
                    }
                    cursor = end;
                    continue;
                }
                let Some((name, end)) = crate::syntax::command_at(source, cursor) else {
                    return false;
                };
                let base = name.trim_end_matches('*');
                if declared.contains(base) || config.config.syntax.commands.contains_key(base) {
                    return false;
                }
                if !matches!(
                    name,
                    "LaTeX"
                        | "LaTeXe"
                        | "TeX"
                        | "XeLaTeX"
                        | "XeTeX"
                        | "LuaLaTeX"
                        | "ldots"
                        | "dots"
                        | "textellipsis"
                        | "textbackslash"
                        | "ref"
                        | "eqref"
                        | "pageref"
                        | "autoref"
                        | "cref"
                        | "Cref"
                        | "label"
                        | "cite"
                        | "citep"
                        | "citet"
                        | "parencite"
                        | "textcite"
                        | "url"
                        | "path"
                        | "href"
                        | "hyperlink"
                        | "hypertarget"
                        | "$"
                        | "%"
                        | "{"
                        | "}"
                        | "_"
                        | "&"
                        | "#"
                        | " "
                ) && text_argument_signature(name).is_none()
                {
                    return false;
                }
                cursor = end;
            }
            b'$' => {
                // Display mathematics has its own layout. Only a complete
                // inline span may be retained as an atom inside prose.
                if source[cursor..].starts_with("$$") {
                    return false;
                }
                let end = math_end(source, cursor + 1, "$", &[]);
                if end > range.end {
                    return false;
                }
                cursor = end;
            }
            _ => {
                cursor += source[cursor..]
                    .chars()
                    .next()
                    .expect("character")
                    .len_utf8()
            }
        }
    }
    true
}

fn format_text_arguments(
    source: &str,
    config: &crate::CompiledConfig,
    parsing: &mut crate::parse_context::ParseContext<'_>,
) -> Result<(String, bool), crate::FormatError> {
    // Custom grammar can assign these tokens another role or alter the group
    // ownership used for indentation. Preserve text arguments in that case.
    let grammar = &config.config.syntax;
    if grammar.command_pattern.is_some()
        || grammar.environment_pattern.is_some()
        || grammar.condition_pattern.is_some()
        || grammar.item_pattern.is_some()
        || grammar.argument_between.is_some()
        || !grammar.special.is_empty()
        || !config.config.protect.dynamic_catcodes
    {
        return Ok((source.to_owned(), false));
    }
    if !memchr::memchr_iter(b'\\', source.as_bytes()).any(|offset| {
        crate::syntax::command_at(source, offset)
            .is_some_and(|(name, _)| config.text.text_commands.contains(name))
    }) {
        return Ok((source.to_owned(), false));
    }
    let mut protected = parsing.snapshot(source, false, false)?.protected.clone();
    let Ok(parsed) = parsing.snapshot(source, true, true) else {
        return Ok((source.to_owned(), false));
    };
    let blocks = parsed.blocks.as_deref().expect("text argument structure");
    for block in blocks {
        if block.kind == crate::syntax::BlockKind::Environment
            && is_non_prose_environment(&block.name, config)
        {
            protected.push(block.begin.start..block.end.end);
        }
    }
    merge_ranges(&mut protected);
    let declared = if crate::syntax::has_analysis_declaration_tokens(source) {
        let definitions = crate::syntax::macro_definitions_with_ranges(source, &protected);
        crate::syntax::declared_command_names_with_ranges(source, &protected, &definitions)
    } else {
        HashSet::new()
    };
    let mut edits = EditPlan::new();
    let mut previous_line = String::new();
    let mut copied = 0;
    let mut cursor = 0;
    let mut unblocked = false;
    while cursor < source.len() {
        if let Some(range) = containing(cursor, &protected) {
            cursor = range.end;
            continue;
        }
        match source.as_bytes()[cursor] {
            b'%' => {
                cursor = source[cursor..]
                    .find('\n')
                    .map_or(source.len(), |offset| cursor + offset + 1);
            }
            b'{' | b'[' => {
                let open = source.as_bytes()[cursor];
                cursor = balanced_end(
                    source,
                    cursor,
                    open,
                    if open == b'{' { b'}' } else { b']' },
                    &protected,
                )
                .unwrap_or(source.len());
            }
            b'\\' => {
                if source[cursor..].starts_with("\\(") || source[cursor..].starts_with("\\[") {
                    let delimiter = if source[cursor..].starts_with("\\(") {
                        "\\)"
                    } else {
                        "\\]"
                    };
                    cursor = math_end(source, cursor + 2, delimiter, &protected);
                    continue;
                }
                let Some((name, end)) = crate::syntax::command_at(source, cursor) else {
                    cursor += 1;
                    continue;
                };
                cursor = end;
                let base = name.trim_end_matches('*');
                if !config.text.text_commands.contains(name)
                    || declared.contains(base)
                    || config.config.syntax.commands.contains_key(base)
                    || grammar.headings.contains_key(base)
                    || grammar.items.iter().any(|name| name == base)
                    || grammar.conditions.iter().any(|name| name == base)
                {
                    continue;
                }
                let Some(bodies) = standard_arguments(source, cursor, name, &protected) else {
                    continue;
                };
                for body in bodies {
                    cursor = body.end + 1;
                    if intersects(body.clone(), &protected)
                        || !safe_text_body(source, body.clone(), &declared, config)
                    {
                        continue;
                    }
                    let pending = &source[copied..body.start];
                    let header = if let Some(newline) = pending.rfind('\n') {
                        Cow::Borrowed(&pending[newline + 1..])
                    } else {
                        Cow::Owned(format!("{}{}", previous_line, pending))
                    };
                    let continuation = argument_continuation(source, body.start, blocks, config);
                    let replacement = wrap_text_argument(
                        source,
                        body.clone(),
                        &header,
                        &continuation,
                        config,
                        &declared,
                    );
                    if replacement != source[body.clone()] {
                        unblocked |=
                            source[body.clone()].contains('\n') && !replacement.contains('\n');
                        previous_line = if let Some((_, line)) = replacement.rsplit_once('\n') {
                            line.to_owned()
                        } else {
                            let mut line = header.into_owned();
                            line.push_str(&replacement);
                            line
                        };
                        copied = body.end;
                        edits.replace(body, replacement);
                    }
                }
            }
            b'$' => {
                let delimiter = if source[cursor..].starts_with("$$") {
                    "$$"
                } else {
                    "$"
                };
                cursor = math_end(source, cursor + delimiter.len(), delimiter, &protected);
            }
            _ => {
                cursor += source[cursor..]
                    .chars()
                    .next()
                    .expect("character")
                    .len_utf8()
            }
        }
    }
    Ok((edits.apply_ordered(source)?, unblocked))
}

fn argument_continuation(
    source: &str,
    position: usize,
    blocks: &[crate::syntax::SharedBlock],
    config: &crate::CompiledConfig,
) -> String {
    use crate::syntax::BlockKind;
    let preamble_end = blocks
        .iter()
        .filter(|block| block.kind == BlockKind::Environment && block.name == "document")
        .map(|block| block.begin.start)
        .min();
    let mut prefix = String::new();
    if !config.config.indent.preamble && preamble_end.is_some_and(|end| position < end) {
        let start = source[..position]
            .rfind('\n')
            .map_or(0, |offset| offset + 1);
        let line = &source[start..position];
        prefix.push_str(&line[..line.len() - line.trim_start_matches([' ', '\t']).len()]);
    } else {
        for block in blocks {
            if block.body.start <= position && position < block.body.end {
                if block.kind == BlockKind::Heading
                    && !config
                        .config
                        .syntax
                        .headings
                        .get(block.name.trim_end_matches('*'))
                        .is_some_and(|rule| rule.indent)
                    || block.name == "foreach-body"
                        && matches!(block.kind, BlockKind::Group | BlockKind::MandatoryArgument)
                {
                    continue;
                }
                prefix.push_str(
                    &config
                        .config
                        .indent
                        .unit(block.kind.category(), &block.name),
                );
            }
        }
    }
    if let Some(maximum) = config.config.indent.maximum {
        let mut width = 0;
        if let Some(end) = prefix.char_indices().find_map(|(index, character)| {
            width += if character == '\t' {
                config.config.indent.width
            } else {
                1
            };
            (width > maximum).then_some(index)
        }) {
            prefix.truncate(end);
        }
    }
    prefix
}

fn wrap_text_argument(
    source: &str,
    body: Range<usize>,
    header: &str,
    continuation: &str,
    config: &crate::CompiledConfig,
    declared: &HashSet<String>,
) -> String {
    let original = &source[body.clone()];
    let mut atoms = Vec::new();
    add_inline_atoms_to_merged(original, &mut atoms, &config.config, declared, false);
    if atoms
        .iter()
        .any(|range| original[range.clone()].contains('\n'))
    {
        return original.to_owned();
    }
    let fallback_newline = if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    if config.config.wrap.mode == "rewrap" {
        return rewrap_text_argument(
            original,
            &atoms,
            header,
            continuation,
            fallback_newline,
            config,
        );
    }
    let mut output = String::with_capacity(original.len());
    let mut offset = 0;
    for (index, line) in original.split_inclusive('\n').enumerate() {
        let ending = line_ending(line).unwrap_or("");
        let content = &line[..line.len() - ending.len()];
        let leading = content.len() - content.trim_start_matches([' ', '\t']).len();
        let trailing = content.trim_end_matches([' ', '\t']).len();
        if leading >= trailing {
            output.push_str(line);
            offset += line.len();
            continue;
        }
        let region = Region {
            range: offset + leading..offset + trailing,
            space_before: false,
            newline_before: None,
        };
        let mut canonical = canonicalize(original, &[region], &atoms, false);
        // CJK and configured punctuation can create no-space wrap points in
        // ordinary prose. Text arguments only replace existing ASCII spaces.
        let mut word_start = 0;
        for (position, character) in canonical
            .text
            .char_indices()
            .chain(std::iter::once((canonical.text.len(), ' ')))
        {
            if matches!(character, ' ' | '\t') {
                if word_start < position && !canonical.text[word_start..position].is_ascii() {
                    canonical.opaque.push(word_start..position);
                }
                word_start = position + character.len_utf8();
            }
        }
        merge_ranges(&mut canonical.opaque);
        let external = if index == 0 { header } else { "" };
        let prefix = format!("{}{}", external, &content[..leading]);
        let mut formatted = String::new();
        // The specialized emitter ignores no-space and huge-word break modes.
        emit_argument_layout(
            &canonical,
            &prefix,
            continuation,
            if ending.is_empty() {
                fallback_newline
            } else {
                ending
            },
            config,
            &mut formatted,
        );
        output.push_str(&formatted[external.len()..]);
        output.push_str(&content[trailing..]);
        output.push_str(ending);
        offset += line.len();
    }
    output
}

fn rewrap_text_argument(
    original: &str,
    atoms: &[Range<usize>],
    header: &str,
    continuation: &str,
    newline: &str,
    config: &crate::CompiledConfig,
) -> String {
    let mut output = String::with_capacity(original.len());
    let mut lines = original.split_inclusive('\n').peekable();
    let mut offset = 0;
    while let Some(line) = lines.next() {
        let content = line.trim_end_matches(['\r', '\n']);
        if content.trim_matches([' ', '\t']).is_empty() {
            output.push_str(line);
            offset += line.len();
            continue;
        }
        let first = offset;
        let leading = content.len() - content.trim_start_matches([' ', '\t']).len();
        let external = if first == 0 { header } else { "" };
        let prefix = format!("{}{}", external, &content[..leading]);
        let mut regions = Vec::new();
        let mut current = line;
        let (trailing, ending) = loop {
            let content = current.trim_end_matches(['\r', '\n']);
            let leading = content.len() - content.trim_start_matches([' ', '\t']).len();
            let end = content.trim_end_matches([' ', '\t']).len();
            regions.push(Region {
                range: offset + leading..offset + end,
                space_before: !regions.is_empty(),
                newline_before: None,
            });
            offset += current.len();
            if lines
                .peek()
                .is_some_and(|next| !next.trim_matches([' ', '\t', '\r', '\n']).is_empty())
            {
                current = lines.next().expect("peeked argument line");
            } else {
                break (&content[end..], line_ending(current).unwrap_or(""));
            }
        };
        let mut canonical = canonicalize(
            original,
            &regions,
            atoms,
            config.config.wrap.collapse_spaces,
        );
        // Argument layout changes existing whitespace only; non-ASCII words
        // remain indivisible even when prose can break at a CJK boundary.
        let mut word_start = 0;
        for (position, character) in canonical
            .text
            .char_indices()
            .chain(std::iter::once((canonical.text.len(), ' ')))
        {
            if matches!(character, ' ' | '\t') {
                if word_start < position && !canonical.text[word_start..position].is_ascii() {
                    canonical.opaque.push(word_start..position);
                }
                word_start = position + character.len_utf8();
            }
        }
        merge_ranges(&mut canonical.opaque);
        let mut formatted = String::new();
        emit_argument_layout(
            &canonical,
            &prefix,
            continuation,
            newline,
            config,
            &mut formatted,
        );
        output.push_str(&formatted[external.len()..]);
        output.push_str(trailing);
        output.push_str(ending);
    }
    output
}

fn emit_argument_layout(
    text: &Canonical,
    prefix: &str,
    continuation: &str,
    newline: &str,
    config: &crate::CompiledConfig,
    output: &mut String,
) {
    output.push_str(prefix);
    let mut width = display_width(prefix, 0, config.config.indent.width);
    let continuation_width = display_width(continuation, 0, config.config.indent.width);
    let mut has_text = false;
    visit_atoms(text, 0..text.text.len(), &HashSet::new(), |atom| {
        let separator = &text.text[atom.separator.clone()];
        let value = &text.text[atom.range.clone()];
        let separator_width = display_width(separator, width, config.config.indent.width);
        let mut value_width =
            display_width(value, width + separator_width, config.config.indent.width);
        if has_text
            && !separator.is_empty()
            && width + separator_width + value_width > config.config.wrap.columns
        {
            output.push_str(newline);
            output.push_str(continuation);
            width = continuation_width;
            value_width = display_width(value, width, config.config.indent.width);
        } else {
            output.push_str(separator);
            width += separator_width;
        }
        output.push_str(value);
        width += value_width;
        has_text = true;
    });
}

#[allow(clippy::too_many_arguments)]
fn flush(
    paragraph: &mut Option<Paragraph>,
    source: &str,
    opaque: &[Range<usize>],
    config: &crate::CompiledConfig,
    wrap_active: bool,
    sentences_active: bool,
    edits: &mut EditPlan<'_>,
) -> Result<(), crate::FormatError> {
    let Some(paragraph) = paragraph.take() else {
        return Ok(());
    };
    let mut canonical = canonicalize(
        source,
        &paragraph.regions,
        opaque,
        config.config.wrap.collapse_spaces,
    );
    if wrap_active
        && !paragraph.comment
        && matches!(
            config.config.wrap.mode.as_str(),
            "wrap" | "stable" | "rewrap"
        )
    {
        keep_prose_pairs(&mut canonical, &paragraph.continuation, config);
    }
    let sentence_excluded = config
        .text
        .sentence_exclude
        .as_ref()
        .map(|pattern| {
            pattern
                .find(&canonical.text)
                .map(|matched| matched.is_some())
        })
        .transpose()
        .map_err(crate::FormatError::Syntax)?
        .unwrap_or(false);
    let split_sentences = sentences_active && !paragraph.comment && !sentence_excluded;
    let prefer_endings = wrap_active
        && config.config.wrap.prefer_sentence_ends
        && matches!(
            config.config.wrap.mode.as_str(),
            "wrap" | "rewrap" | "stable"
        )
        && !paragraph.comment
        && !sentence_excluded;
    let detected = if split_sentences || prefer_endings {
        sentence_ranges(&canonical, &config.text).map_err(crate::FormatError::Syntax)?
    } else {
        Vec::new()
    };
    let paragraph_range = 0..canonical.text.len();
    let sentences = if split_sentences {
        detected.as_slice()
    } else {
        std::slice::from_ref(&paragraph_range)
    };
    let preserve_lines = wrap_active
        && matches!(config.config.wrap.mode.as_str(), "wrap" | "stable")
        || sentences_active
            && !paragraph.comment
            && !sentence_excluded
            && config.config.sentences.preserve_line_breaks
            && !(wrap_active && config.config.wrap.mode == "rewrap");
    let sentence_prefix = (sentences.len() > 1).then(|| {
        format!(
            "{}{}",
            paragraph.continuation, config.config.sentences.indent
        )
    });
    let mut output = String::with_capacity(paragraph.span.len());
    for (index, sentence) in sentences.iter().enumerate() {
        if index > 0 {
            output.push_str(
                preserve_lines
                    .then(|| {
                        canonical.hard_breaks.get(
                            canonical
                                .hard_breaks
                                .partition_point(|boundary| boundary.after < sentence.start),
                        )
                    })
                    .flatten()
                    .filter(|boundary| boundary.after == sentence.start)
                    .map_or(paragraph.newline, |boundary| boundary.newline.as_ref()),
            );
        }
        let prefix = if index == 0 {
            paragraph.prefix.as_str()
        } else {
            sentence_prefix
                .as_deref()
                .expect("sentence continuation prefix")
        };
        let continuation = if index == 0 {
            paragraph.continuation.as_str()
        } else {
            prefix
        };
        emit_lines(
            &canonical,
            sentence.clone(),
            prefer_endings.then_some(detected.as_slice()),
            prefix,
            continuation,
            if config.config.wrap.separator.is_empty() {
                paragraph.newline
            } else if paragraph.comment {
                config.config.wrap.separator.trim_start_matches('%')
            } else {
                &config.config.wrap.separator
            },
            wrap_active
                && matches!(
                    config.config.wrap.mode.as_str(),
                    "wrap" | "stable" | "rewrap"
                ),
            preserve_lines,
            paragraph.comment,
            config,
            &mut output,
        )?;
    }
    output.push_str(&paragraph.ending);
    if output != source[paragraph.span.clone()] {
        edits.replace(paragraph.span, output);
    }
    Ok(())
}

fn keep_prose_pairs(text: &mut Canonical, continuation: &str, config: &crate::CompiledConfig) {
    let wrap = &config.config.wrap;
    let mut pairs = Vec::new();
    if wrap.keep_number_units {
        text.quantities = quantity_ranges(text);
    }
    if wrap.keep_reference_names && text.text.contains("ref") {
        static REFERENCE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
            regex::Regex::new(concat!(
                r"\b(?:",
                r"(?i:figures?|figs?\.|tables?|tabs?\.|sections?|secs?\.|chapters?|chaps?\.|ch\.|",
                r"appendix|appendices|apps?\.|theorems?|thms?\.|lemmas?|lem\.|propositions?|props?\.|",
                r"corollary|corollaries|cor\.|algorithms?|algs?\.|listings?|definitions?|defs?\.|examples?)[ \t]+\\ref|",
                r"(?i:equations?|eqs?\.)[ \t]+\\(?:eqref|ref)|(?i:pages?|pp?\.)[ \t]+\\pageref",
                r")\*?\{[^{}\\%\s]+\}",
            ))
            .unwrap()
        });
        let tab_width = config.config.indent.width;
        let indent = display_width(continuation, 0, tab_width);
        for reference in REFERENCE.find_iter(&text.text) {
            if containing(reference.start(), &text.opaque).is_none()
                && !text.text[..reference.start()].ends_with('\\')
                && indent + display_width(reference.as_str(), indent, tab_width) <= wrap.columns
            {
                pairs.push(reference.range());
            }
        }
    }
    if wrap.avoid_lone_last_word {
        let tab_width = config.config.indent.width;
        if let Some((before_last, last)) = text.text.rsplit_once([' ', '\t'])
            && last
                .trim_end_matches(['.', ',', ';', ':', '!', '?', '\'', '"', ')'])
                .bytes()
                .all(|b| b.is_ascii_alphabetic())
            && last.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
            && let Some((before_pair, penultimate)) = before_last
                .trim_end_matches([' ', '\t'])
                .rsplit_once([' ', '\t'])
            && !penultimate.is_empty()
            && penultimate.bytes().all(|b| b.is_ascii_alphabetic())
        {
            let pair = before_pair.len() + 1..text.text.len();
            let indent = display_width(continuation, 0, tab_width);
            if !intersects(pair.clone(), &text.opaque)
                && !intersects(pair.clone(), &text.quantities)
                && indent + display_width(&text.text[pair.clone()], indent, tab_width)
                    <= wrap.columns
            {
                pairs.push(pair);
            }
        }
    }
    if !pairs.is_empty() {
        text.opaque.extend(pairs);
        merge_ranges(&mut text.opaque);
    }
}

fn quantity_ranges(text: &Canonical) -> Vec<Range<usize>> {
    static QUANTITY: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(concat!(
            r"[+−-]?(?:[0-9]+(?:\.[0-9]+)?|\.[0-9]+)(?:[eE][+-]?[0-9]+)?[ \t]+",
            r"(?:°[CF]|min|mol|[kMGT]i?B|[kMG]?Hz|[kMh]?Pa|[μµnm]s|[μµncmk]m|",
            r"[μµmk]g|[μµm]L|[μµmk][AVW]|kJ|[KshmgLAVWBJ])"
        ))
        .unwrap()
    });
    let word_char = |c: char| c == '_' || (c.is_alphanumeric() && !is_cjk(c));
    QUANTITY
        .find_iter(&text.text)
        .map(|m| m.range())
        .filter(|r| {
            !intersects(r.clone(), &text.opaque)
                && !text.text[..r.start]
                    .chars()
                    .next_back()
                    .is_some_and(|c| word_char(c) || matches!(c, '\\' | '.'))
                && !text.text[r.end..].chars().next().is_some_and(word_char)
        })
        .collect()
}

fn canonicalize(
    source: &str,
    regions: &[Region],
    opaque: &[Range<usize>],
    collapse: bool,
) -> Canonical {
    let capacity = regions
        .iter()
        .map(|region| region.range.len())
        .sum::<usize>();
    let mut result = Canonical {
        text: String::with_capacity(capacity),
        opaque: Vec::new(),
        quantities: Vec::new(),
        hard_breaks: Vec::new(),
    };
    for region in regions {
        let text = &source[region.range.clone()];
        let start = region.range.start + text.len() - text.trim_start_matches([' ', '\t']).len();
        let mut end = region.range.end - text.len() + text.trim_end_matches([' ', '\t']).len();
        if end < region.range.end && crate::syntax::is_escaped(source, end) {
            // The first trailing whitespace belongs to a control-space token.
            end += 1;
        }
        if start >= end {
            continue;
        }
        let before = result.text.len();
        if before > 0 {
            if region.space_before {
                result.text.push(' ');
            }
            if let Some(newline) = &region.newline_before {
                result.hard_breaks.push(HardBreak {
                    before,
                    after: result.text.len(),
                    newline: newline.clone(),
                });
            }
        }
        if !intersects(start..end, opaque) {
            if collapse {
                for (index, word) in source[start..end].split_ascii_whitespace().enumerate() {
                    if index > 0 {
                        result.text.push(' ');
                    }
                    result.text.push_str(word);
                }
            } else {
                result.text.push_str(&source[start..end]);
            }
            continue;
        }
        let mut position = start;
        let mut opaque_index = opaque.partition_point(|range| range.end <= start);
        while position < end {
            while opaque
                .get(opaque_index)
                .is_some_and(|range| range.end <= position)
            {
                opaque_index += 1;
            }
            if let Some(range) = opaque
                .get(opaque_index)
                .filter(|range| range.start <= position)
            {
                let stop = range.end.min(end);
                let destination = result.text.len();
                result.text.push_str(&source[position..stop]);
                result.opaque.push(destination..result.text.len());
                position = stop;
            } else {
                let stop = opaque
                    .get(opaque_index)
                    .map_or(end, |range| range.start.min(end));
                if !collapse {
                    result.text.push_str(&source[position..stop]);
                    position = stop;
                    continue;
                }
                while position < stop {
                    let plain_end =
                        memchr::memchr2(b' ', b'\t', &source.as_bytes()[position..stop])
                            .map_or(stop, |offset| position + offset);
                    result.text.push_str(&source[position..plain_end]);
                    position = plain_end;
                    if position < stop {
                        result.text.push(' ');
                        while position < stop && matches!(source.as_bytes()[position], b' ' | b'\t')
                        {
                            position += 1;
                        }
                    }
                }
            }
        }
    }
    result
}

fn sentence_ranges(text: &Canonical, config: &CompiledText) -> Result<Vec<Range<usize>>, String> {
    let mut result = Vec::new();
    let mut start = 0;
    for boundary in config.sentence_end.matches(&text.text)? {
        if boundary.end <= start
            || intersects(boundary.clone(), &text.opaque)
            || containing(boundary.start, &text.opaque).is_some()
        {
            continue;
        }
        let ending = &text.text[boundary.clone()];
        if config.default_ending {
            let next = text.text[boundary.end..].chars().next();
            if ending.ends_with('.') && next.is_some_and(|c| c.is_alphanumeric()) {
                continue;
            }
            let context_start = text.text[..boundary.end]
                .char_indices()
                .rev()
                .nth(config.abbreviation_context)
                .map_or(0, |(index, _)| index);
            let prefix = text.text[context_start..boundary.end].to_lowercase();
            if ending.ends_with('.')
                && config.abbreviations.iter().any(|abbreviation| {
                    prefix.strip_suffix(abbreviation).is_some_and(|before| {
                        before
                            .chars()
                            .next_back()
                            .is_none_or(|c| !c.is_alphanumeric())
                    })
                })
            {
                continue;
            }
        }
        let mut end = boundary.end;
        while let Some(character) = text.text[end..].chars().next() {
            if !matches!(character, '\'' | '"' | '’' | '”' | '»' | ')' | ']') {
                break;
            }
            end += character.len_utf8();
        }
        let rest = &text.text[end..];
        let whitespace = rest.len() - rest.trim_start_matches([' ', '\t']).len();
        let next_start = end + whitespace;
        if next_start >= text.text.len() {
            continue;
        }
        if config.default_ending && whitespace == 0 && !ending.chars().any(is_cjk_punctuation) {
            continue;
        }
        // Every generated line must remain an eligible paragraph continuation
        // on the next pass. Unregistered command starts are preserved in place
        // so structural indentation cannot drop an item hanging prefix later.
        if containing(next_start, &text.opaque).is_none()
            && !allows_prose_start(&text.text[next_start..], config)?
        {
            continue;
        }
        if let Some(begin) = &config.sentence_begin
            && begin
                .find(&text.text[next_start..])?
                .is_none_or(|range| range.start != 0)
        {
            continue;
        }
        result.push(start..end);
        start = next_start;
    }
    if start < text.text.len() {
        result.push(start..text.text.len());
    }
    Ok(result)
}

#[derive(Debug)]
struct Atom {
    range: Range<usize>,
    separator: Range<usize>,
    cjk: bool,
}

fn visit_atoms(
    text: &Canonical,
    sentence: Range<usize>,
    break_chars: &HashSet<char>,
    mut visit: impl FnMut(Atom),
) {
    let mut position = sentence.start;
    let mut previous_end = position;
    let mut opaque_index = text.opaque.partition_point(|range| range.end <= position);
    while position < sentence.end {
        while position < sentence.end && matches!(text.text.as_bytes()[position], b' ' | b'\t') {
            position += 1;
        }
        if position >= sentence.end {
            break;
        }
        let word_start = position;
        let mut atomic = false;
        while position < sentence.end {
            while text
                .opaque
                .get(opaque_index)
                .is_some_and(|range| range.end <= position)
            {
                opaque_index += 1;
            }
            if let Some(range) = text
                .opaque
                .get(opaque_index)
                .filter(|range| range.start <= position)
            {
                atomic = true;
                position = range.end.min(sentence.end);
            } else {
                let stop = text
                    .opaque
                    .get(opaque_index)
                    .map_or(sentence.end, |range| range.start.min(sentence.end));
                if let Some(offset) =
                    memchr::memchr2(b' ', b'\t', &text.text.as_bytes()[position..stop])
                {
                    position += offset;
                    break;
                }
                position = stop;
            }
        }
        if atomic || (text.text[word_start..position].is_ascii() && break_chars.is_empty()) {
            visit(Atom {
                range: word_start..position,
                separator: previous_end..word_start,
                cjk: false,
            });
        } else {
            let mut run_start = word_start;
            for (relative, grapheme) in text.text[word_start..position].grapheme_indices(true) {
                let offset = word_start + relative;
                if grapheme.chars().next().is_some_and(is_cjk) {
                    if run_start < offset {
                        visit(Atom {
                            range: run_start..offset,
                            separator: previous_end..run_start,
                            cjk: false,
                        });
                        previous_end = offset;
                    }
                    let end = offset + grapheme.len();
                    visit(Atom {
                        range: offset..end,
                        separator: previous_end..offset,
                        cjk: true,
                    });
                    previous_end = end;
                    run_start = end;
                } else if grapheme.chars().next().is_some_and(|character| {
                    character.len_utf8() == grapheme.len() && break_chars.contains(&character)
                }) {
                    let end = offset + grapheme.len();
                    visit(Atom {
                        range: run_start..end,
                        separator: previous_end..run_start,
                        cjk: false,
                    });
                    previous_end = end;
                    run_start = end;
                }
            }
            if run_start < position {
                visit(Atom {
                    range: run_start..position,
                    separator: previous_end..run_start,
                    cjk: false,
                });
            }
        }
        previous_end = position;
    }
}

fn visit_prose_atoms(
    text: &Canonical,
    sentence: Range<usize>,
    break_chars: &HashSet<char>,
    mut visit: impl FnMut(Atom),
) {
    let first = text.quantities.partition_point(|r| r.end <= sentence.start);
    if text
        .quantities
        .get(first)
        .is_none_or(|r| r.start >= sentence.end)
    {
        return visit_atoms(text, sentence, break_chars, visit);
    }
    // Join only atoms crossed by a quantity. Adjacent CJK characters retain
    // their own break points, including in text with no surrounding spaces.
    let mut quantities = text.quantities[first..].iter().peekable();
    let mut held: Option<Atom> = None;
    visit_atoms(text, sentence, break_chars, |atom| {
        let end = atom.range.end;
        let group = held.get_or_insert(atom);
        group.range.end = end;
        while quantities.next_if(|r| r.end <= end).is_some() {}
        if quantities.peek().is_none_or(|r| r.start >= end) {
            visit(held.take().expect("current atom"));
        }
    });
    if let Some(group) = held {
        visit(group);
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_lines(
    text: &Canonical,
    sentence: Range<usize>,
    sentence_boundaries: Option<&[Range<usize>]>,
    prefix: &str,
    continuation: &str,
    newline: &str,
    wrap: bool,
    preserve_lines: bool,
    comment: bool,
    config: &crate::CompiledConfig,
    output: &mut String,
) -> Result<(), crate::FormatError> {
    let mut start = sentence.start;
    let mut current_prefix = prefix;
    if preserve_lines {
        let first = text
            .hard_breaks
            .partition_point(|boundary| boundary.before <= sentence.start);
        for boundary in text.hard_breaks[first..]
            .iter()
            .take_while(|boundary| boundary.after < sentence.end)
        {
            let carry = emit_layout(
                text,
                start..boundary.before,
                sentence_boundaries,
                current_prefix,
                continuation,
                newline,
                wrap,
                comment,
                config,
                output,
            )?;
            if wrap
                && config.config.wrap.mode == "stable"
                && !boundary.newline.contains('%')
                && let Some((source_start, output_start)) = carry
            {
                // Refill only the new continuation with the following source line.
                output.truncate(output_start);
                start = source_start;
                current_prefix = continuation;
                continue;
            }
            output.push_str(&boundary.newline);
            start = boundary.after;
            current_prefix = continuation;
        }
    }
    emit_layout(
        text,
        start..sentence.end,
        sentence_boundaries,
        current_prefix,
        continuation,
        newline,
        wrap,
        comment,
        config,
        output,
    )
    .map(|_| ())
}

#[allow(clippy::too_many_arguments)]
fn emit_layout(
    text: &Canonical,
    sentence: Range<usize>,
    sentence_boundaries: Option<&[Range<usize>]>,
    prefix: &str,
    continuation: &str,
    newline: &str,
    wrap: bool,
    comment: bool,
    config: &crate::CompiledConfig,
    output: &mut String,
) -> Result<Option<(usize, usize)>, crate::FormatError> {
    output.push_str(prefix);
    if !wrap {
        output.push_str(&text.text[sentence]);
        return Ok(None);
    }
    let columns = config.config.wrap.columns;
    let tab_width = config.config.indent.width;
    let break_huge = config.config.wrap.huge == "break";
    let mut width = display_width(prefix, 0, tab_width);
    let continuation_width = display_width(continuation, 0, tab_width);
    let mut line_has_text = false;
    let mut previous: Option<Atom> = None;
    let mut carry = None;
    let mut error = None;
    let mut preferred: Option<PreferredBreak> = None;
    visit_prose_atoms(text, sentence, &config.text.break_chars, |atom| {
        if error.is_some() {
            return;
        }
        let value = &text.text[atom.range.clone()];
        let separator = &text.text[atom.separator.clone()];
        // A generated line must remain eligible prose on the next formatting
        // pass. Keep unfamiliar commands attached to the preceding source word.
        let prose_start = if comment || containing(atom.range.start, &text.opaque).is_some() {
            true
        } else {
            match allows_prose_start(value, &config.text) {
                Ok(eligible) => eligible,
                Err(message) => {
                    error = Some(crate::FormatError::Syntax(message));
                    return;
                }
            }
        };
        let custom_break = previous.as_ref().is_some_and(|previous| {
            text.text[previous.range.clone()]
                .chars()
                .next_back()
                .is_some_and(|character| config.text.break_chars.contains(&character))
        });
        let can_break = prose_start
            && (!separator.is_empty()
                || custom_break
                || previous.as_ref().is_some_and(|previous| {
                    (previous.cjk || atom.cjk)
                        && !value.chars().next().is_some_and(is_closing_punctuation)
                        && !text.text[previous.range.clone()]
                            .chars()
                            .next_back()
                            .is_some_and(is_opening_punctuation)
                }));
        let mut separator_width = display_width(separator, width, tab_width);
        let mut value_width = display_width(value, width + separator_width, tab_width);
        if let Some(boundaries) = sentence_boundaries
            && line_has_text
            && can_break
            && width <= columns
            && columns - width <= 12.min(columns / 2)
        {
            let sentence_end = boundaries
                .binary_search_by_key(&atom.range.start, |s| s.start)
                .is_ok();
            let clause_end = text.text[..atom.separator.start].ends_with(['，', '；', '：']);
            if sentence_end || (clause_end && !preferred.as_ref().is_some_and(|p| p.sentence_end)) {
                preferred = Some(PreferredBreak {
                    separator: atom.separator.clone(),
                    output_at: output.len(),
                    sentence_end,
                });
            }
        }
        if line_has_text
            && can_break
            && width + separator_width + value_width > columns
            && let Some(point) = preferred.take().filter(|p| p.output_at < output.len())
        {
            // Move the already emitted start of the next sentence onto its continuation.
            let suffix = output.split_off(point.output_at + point.separator.len());
            output.truncate(point.output_at);
            push_wrap_break(
                output,
                &text.text[point.separator.clone()],
                newline,
                comment,
            );
            carry = Some((point.separator.end, output.len()));
            output.push_str(continuation);
            output.push_str(&suffix);
            width = continuation_width + display_width(&suffix, continuation_width, tab_width);
            separator_width = display_width(separator, width, tab_width);
            value_width = display_width(value, width + separator_width, tab_width);
        }
        if line_has_text && can_break && width + separator_width + value_width > columns {
            push_wrap_break(output, separator, newline, comment);
            carry = Some((atom.range.start, output.len()));
            output.push_str(continuation);
            width = continuation_width;
            value_width = display_width(value, width, tab_width);
        } else {
            output.push_str(separator);
            width += separator_width;
        }
        if break_huge
            && width + value_width > columns
            && columns > continuation_width
            && !intersects(atom.range.clone(), &text.opaque)
            && !intersects(atom.range.clone(), &text.quantities)
            && !value.contains(['\\', '~', '\u{a0}', '\u{202f}', '\u{2060}'])
        {
            let mut emitted = false;
            for grapheme in value.graphemes(true) {
                let mut grapheme_width = display_width(grapheme, width, tab_width);
                if emitted && width + grapheme_width > columns {
                    // A hard break inside a source word suppresses TeX's newline
                    // space. Grapheme boundaries preserve combining sequences.
                    if !comment {
                        output.push('%');
                    }
                    output.push_str(newline.trim_start_matches('%'));
                    carry = None;
                    preferred = None;
                    output.push_str(continuation);
                    width = continuation_width;
                    grapheme_width = display_width(grapheme, width, tab_width);
                }
                output.push_str(grapheme);
                width += grapheme_width;
                emitted = true;
            }
        } else {
            output.push_str(value);
            width += value_width;
        }
        line_has_text = true;
        previous = Some(atom);
    });
    error.map_or(Ok(carry), Err)
}

struct PreferredBreak {
    separator: Range<usize>,
    output_at: usize,
    sentence_end: bool,
}

fn push_wrap_break(output: &mut String, separator: &str, newline: &str, comment: bool) {
    if comment {
        output.push_str(newline.trim_start_matches('%'));
    } else if separator.is_empty() {
        // A tight CJK or custom boundary must not introduce a TeX space.
        output.push('%');
        output.push_str(newline.trim_start_matches('%'));
    } else {
        if newline.starts_with('%') {
            output.push_str(separator);
        }
        output.push_str(newline);
    }
}

pub(crate) fn display_width(text: &str, start: usize, tab_width: usize) -> usize {
    if text.bytes().all(|byte| (32..127).contains(&byte)) {
        return text.len();
    }
    let mut column = start;
    for grapheme in text.graphemes(true) {
        if grapheme == "\t" {
            column += tab_width - column % tab_width;
        } else {
            column += UnicodeWidthStr::width(grapheme);
        }
    }
    column - start
}

fn is_cjk(character: char) -> bool {
    matches!(character as u32, 0x2e80..=0x9fff | 0xac00..=0xd7af | 0xf900..=0xfaff | 0x20000..=0x3134f)
        || is_cjk_punctuation(character)
}

fn is_cjk_punctuation(character: char) -> bool {
    matches!(
        character,
        '。' | '！'
            | '？'
            | '，'
            | '、'
            | '；'
            | '：'
            | '（'
            | '）'
            | '【'
            | '】'
            | '《'
            | '》'
            | '「'
            | '」'
            | '『'
            | '』'
    )
}

fn is_closing_punctuation(character: char) -> bool {
    matches!(
        character,
        '。' | '！'
            | '？'
            | '，'
            | '、'
            | '；'
            | '：'
            | '）'
            | '】'
            | '》'
            | '〉'
            | '」'
            | '』'
            | '〕'
            | '〗'
    )
}

fn is_opening_punctuation(character: char) -> bool {
    matches!(
        character,
        '（' | '【' | '《' | '〈' | '「' | '『' | '〔' | '〖'
    )
}

fn comment_prefix(body: &str) -> Option<usize> {
    if !body.starts_with('%') {
        return None;
    }
    let marks = body.bytes().take_while(|&byte| byte == b'%').count();
    let spaces = body[marks..].len() - body[marks..].trim_start_matches([' ', '\t']).len();
    Some(marks + spaces)
}

fn comment_list_marker(body: &str) -> Option<&str> {
    let digits = body.bytes().take_while(u8::is_ascii_digit).count();
    let end = match (digits, *body.as_bytes().get(digits)?) {
        (0, b'-' | b'*' | b'+') => 1,
        (1.., b'.' | b')') => digits + 1,
        _ => return None,
    };
    let content = body[end..].trim_start_matches([' ', '\t']);
    (!content.is_empty() && content.len() < body.len() - end)
        .then(|| &body[..body.len() - content.len()])
}

fn comment_item_continuation(prefix: &str, marker: &str, tab_width: usize) -> String {
    let width = display_width(marker, display_width(prefix, 0, tab_width), tab_width);
    format!("{prefix}{}", " ".repeat(width))
}

fn item_continuation(prefix: &str, marker: &str, tab_width: usize) -> String {
    format!(
        "{prefix}{}",
        " ".repeat(display_width(marker, 0, tab_width))
    )
}

fn item_body(body: &str, items: &[String]) -> Option<usize> {
    if !body.starts_with('\\') {
        return None;
    }
    let name_end = 1 + body[1..]
        .bytes()
        .take_while(u8::is_ascii_alphabetic)
        .count();
    if !items.iter().any(|item| item == &body[1..name_end]) {
        return None;
    }
    let mut end = name_end;
    if body.as_bytes().get(end) == Some(&b'*') {
        end += 1;
    }
    end += body[end..].len() - body[end..].trim_start_matches([' ', '\t']).len();
    if body.as_bytes().get(end) == Some(&b'[') {
        let close = balanced_end(body, end, b'[', b']', &[]);
        end = close?;
        end += body[end..].len() - body[end..].trim_start_matches([' ', '\t']).len();
    }
    (end < body.len()).then_some(end)
}

fn contains_definition(body: &str) -> bool {
    if !body.contains('\\') {
        return false;
    }
    [
        "\\def",
        "\\gdef",
        "\\edef",
        "\\xdef",
        "\\newcommand",
        "\\renewcommand",
        "\\providecommand",
        "\\newenvironment",
        "\\renewenvironment",
        "\\Declare",
        "\\let",
        "\\obeyspaces",
        "\\obeylines",
    ]
    .iter()
    .any(|command| body.contains(command))
}

pub(crate) fn allows_prose_start(body: &str, config: &CompiledText) -> Result<bool, String> {
    if !body.starts_with('\\') {
        return Ok(true);
    }
    let Some((name, _)) = crate::syntax::command_at(body, 0) else {
        return Ok(false);
    };
    let name = name.strip_suffix('*').unwrap_or(name);
    if builtin_prose_command(name) || config.prose_commands.contains(name) {
        return Ok(true);
    }
    config
        .block_begin
        .as_ref()
        .map(|pattern| pattern.find(body).map(|matched| matched.is_some()))
        .transpose()
        .map(|matched| matched.unwrap_or(false))
}

fn builtin_prose_command(name: &str) -> bool {
    matches!(
        name,
        "emph"
            | "textbf"
            | "textit"
            | "textsf"
            | "textrm"
            | "texttt"
            | "textsc"
            | "textnormal"
            | "textup"
            | "textsl"
            | "underline"
            | "mbox"
            | "fbox"
            | "textcolor"
            | "label"
            | "ref"
            | "eqref"
            | "pageref"
            | "autoref"
            | "cref"
            | "Cref"
            | "cite"
            | "citep"
            | "citet"
            | "parencite"
            | "textcite"
            | "autocite"
            | "footnote"
            | "url"
            | "path"
            | "href"
            | "hyperlink"
            | "hypertarget"
            | "SI"
            | "si"
            | "SIrange"
            | "SIlist"
            | "num"
            | "numrange"
            | "numlist"
            | "qty"
            | "qtyrange"
            | "qtylist"
            | "unit"
            | "LaTeX"
            | "LaTeXe"
            | "TeX"
            | "XeLaTeX"
            | "LuaLaTeX"
    )
}

fn is_non_prose_environment(name: &str, config: &crate::CompiledConfig) -> bool {
    crate::packages::visual_languages::is_non_prose(name)
        || crate::packages::circuits_timing::is_non_prose(name)
        || crate::packages::tikz_diagrams::is_non_prose(name)
        || crate::tikz_scan::environment(name)
        || matches!(
            name.trim_end_matches('*'),
            "math"
                | "displaymath"
                | "IPA"
                | "equation"
                | "align"
                | "alignat"
                | "aligned"
                | "alignedat"
                | "flalign"
                | "gather"
                | "gathered"
                | "multline"
                | "split"
                | "array"
                | "matrix"
                | "pmatrix"
                | "bmatrix"
                | "Bmatrix"
                | "vmatrix"
                | "Vmatrix"
                | "smallmatrix"
                | "cases"
                | "tabular"
                | "tabularx"
                | "tabularray"
                | "tblr"
                | "longtable"
                | "tabbing"
                | "tikzpicture"
                | "pgfpicture"
        )
        || config
            .config
            .alignment
            .environments
            .iter()
            .any(|environment| environment == name)
        || config.config.alignment.rules.contains_key(name)
}

fn containing(offset: usize, ranges: &[Range<usize>]) -> Option<&Range<usize>> {
    let index = ranges.partition_point(|range| range.end <= offset);
    ranges.get(index).filter(|range| range.start <= offset)
}

fn intersects(range: Range<usize>, ranges: &[Range<usize>]) -> bool {
    let index = ranges.partition_point(|candidate| candidate.end <= range.start);
    ranges
        .get(index)
        .is_some_and(|candidate| candidate.start < range.end)
}

fn merge_ranges(ranges: &mut Vec<Range<usize>>) {
    ranges.sort_unstable_by_key(|range| range.start);
    let mut result: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
    for range in ranges.drain(..) {
        if let Some(previous) = result
            .last_mut()
            .filter(|previous| previous.end >= range.start)
        {
            previous.end = previous.end.max(range.end);
        } else {
            result.push(range);
        }
    }
    *ranges = result;
}

fn has_comment(source: &str, range: Range<usize>, opaque: &[Range<usize>]) -> bool {
    comment_start(source, range, opaque).is_some()
}

pub(crate) fn align_comments_after_edits(
    source: &str,
    config: &crate::CompiledConfig,
    parsing: &mut crate::parse_context::ParseContext<'_>,
    mut edits: crate::edit_plan::EditPlan<'_>,
) -> Result<String, crate::FormatError> {
    if !source.contains('%') {
        return edits.apply_ordered(source);
    }
    let parsed = parsing.snapshot(source, true, false)?;
    let mut groups = Vec::new();
    for token in parsed.tokens.as_deref().expect("comment tokens") {
        if token.kind != crate::syntax::TokenKind::Comment {
            continue;
        }
        let marker = token.span.start;
        let line = source[..marker].rfind('\n').map_or(0, |at| at + 1);
        let prefix = &source[line..marker];
        let code = prefix.trim_end_matches([' ', '\t']);
        let gap = line + code.len()..marker;
        if code.is_empty()
            || gap.is_empty()
            || crate::syntax::is_escaped(source, gap.start)
            || crate::syntax::overlaps(&gap, &parsed.protected)
            || source[token.span.clone()]
                .trim_start_matches('%')
                .trim()
                .is_empty()
        {
            continue;
        }
        let indent = &code[..code.len() - code.trim_start_matches([' ', '\t']).len()];
        let next_line = source[marker..]
            .find('\n')
            .map_or(source.len(), |n| marker + n + 1);
        let projected = edits.projected_span(source, line..gap.start);
        let width = display_width(&projected, 0, config.config.indent.width);
        groups.push((line, next_line, indent, gap, width));
    }
    collect_comment_edits(source, &groups, &mut edits);
    edits.apply_first_wins(source)
}

pub(crate) fn align_comment_groups(
    source: &str,
    groups: &[(usize, usize, &str, Range<usize>, usize)],
) -> Result<String, crate::FormatError> {
    let mut edits = crate::edit_plan::EditPlan::new();
    collect_comment_edits(source, groups, &mut edits);
    edits.apply_ordered(source)
}

fn collect_comment_edits(
    source: &str,
    groups: &[(usize, usize, &str, Range<usize>, usize)],
    edits: &mut crate::edit_plan::EditPlan<'_>,
) {
    for group in groups.chunk_by(|a, b| a.1 == b.0 && a.2 == b.2) {
        if group.len() < 2 {
            continue;
        }
        let target = group.iter().map(|v| v.4).max().unwrap() + 1;
        for (_, _, _, gap, width) in group {
            let pad = target - width;
            if gap.len() != pad || source[gap.clone()].contains('\t') {
                edits.replace_from(
                    crate::edit_plan::EditOrigin::TrailingComment,
                    gap.clone(),
                    " ".repeat(pad),
                );
            }
        }
    }
}

fn ends_control_word(text: &str) -> bool {
    let word_start = text.len()
        - text
            .trim_end_matches(|c: char| c.is_ascii_alphabetic() || c == '@')
            .len();
    if word_start == 0 {
        return false;
    }
    let prefix = &text[..text.len() - word_start];
    let slashes = prefix
        .bytes()
        .rev()
        .take_while(|byte| *byte == b'\\')
        .count();
    slashes % 2 == 1
}

fn comment_start(source: &str, range: Range<usize>, opaque: &[Range<usize>]) -> Option<usize> {
    if !source[range.clone()].contains('%') {
        return None;
    }
    let mut position = range.start;
    while position < range.end {
        if let Some(atom) = containing(position, opaque) {
            position = atom.end;
            continue;
        }
        match source.as_bytes()[position] {
            b'\\' => {
                position += 1;
                if position < range.end {
                    position += source[position..].chars().next().unwrap().len_utf8();
                }
            }
            b'%' => return Some(position),
            _ => {
                position = memchr::memchr2(b'\\', b'%', &source.as_bytes()[position..range.end])
                    .map_or(range.end, |next| position + next);
            }
        }
    }
    None
}

fn balanced_end(
    source: &str,
    start: usize,
    open: u8,
    close: u8,
    protected: &[Range<usize>],
) -> Option<usize> {
    let mut depth = 1usize;
    let mut position = start + 1;
    while position < source.len() {
        if let Some(range) = containing(position, protected) {
            position = range.end;
            continue;
        }
        match source.as_bytes()[position] {
            b'\\' => {
                position += 1;
                if position < source.len() {
                    position += source[position..].chars().next().unwrap().len_utf8();
                }
            }
            b'%' => {
                position = source[position..]
                    .find('\n')
                    .map_or(source.len(), |offset| position + offset + 1)
            }
            b'{' if open == b'[' => {
                position = balanced_end(source, position, b'{', b'}', protected)?
            }
            byte if byte == open => {
                depth += 1;
                position += 1;
            }
            byte if byte == close => {
                depth -= 1;
                position += 1;
                if depth == 0 {
                    return Some(position);
                }
            }
            _ => position += source[position..].chars().next().unwrap().len_utf8(),
        }
    }
    None
}

fn math_end(source: &str, start: usize, delimiter: &str, protected: &[Range<usize>]) -> usize {
    let mut position = start;
    while position < source.len() {
        if let Some(range) = containing(position, protected) {
            position = range.end;
            continue;
        }
        if source[position..].starts_with(delimiter) {
            return position + delimiter.len();
        }
        match source.as_bytes()[position] {
            b'\\' => {
                position += 1;
                if position < source.len() {
                    position += source[position..].chars().next().unwrap().len_utf8();
                }
            }
            b'%' => {
                position = source[position..]
                    .find('\n')
                    .map_or(source.len(), |offset| position + offset + 1)
            }
            _ => position += source[position..].chars().next().unwrap().len_utf8(),
        }
    }
    source.len()
}

fn add_inline_atoms_to_merged(
    source: &str,
    ranges: &mut Vec<Range<usize>>,
    config: &crate::Config,
    declared_commands: &std::collections::HashSet<String>,
    expose_text_arguments: bool,
) {
    let protected = ranges.clone();
    let mut position = 0;
    let mut optional_argument = false;
    let mut text_openings = HashSet::new();
    while position < source.len() {
        if let Some(range) = containing(position, &protected) {
            position = range.end;
            optional_argument = false;
            continue;
        }
        let start = position;
        match source.as_bytes()[position] {
            b'%' => {
                position = source[position..]
                    .find('\n')
                    .map_or(source.len(), |offset| position + offset + 1)
            }
            b'\\'
                if source[position..].starts_with("\\(")
                    || source[position..].starts_with("\\[") =>
            {
                let close = if source.as_bytes()[position + 1] == b'(' {
                    "\\)"
                } else {
                    "\\]"
                };
                position = math_end(source, position + 2, close, &protected);
                ranges.push(start..position);
                optional_argument = false;
            }
            b'\\' => {
                if let Some((name, end)) = crate::syntax::command_at(source, position) {
                    position = end;
                    let base = name.trim_end_matches('*');
                    if expose_text_arguments
                        && !declared_commands.contains(base)
                        && !config.syntax.commands.contains_key(base)
                        && let Some(bodies) = standard_arguments(source, end, name, &protected)
                    {
                        let mut prefix = start;
                        for body in &bodies {
                            ranges.push(prefix..body.start);
                            if prefix != start {
                                text_openings.insert(body.start - 1);
                            }
                            prefix = body.end;
                        }
                        position = bodies[0].start;
                        optional_argument = false;
                        continue;
                    }
                    if name.chars().next().is_some_and(|symbol| {
                        symbol.is_ascii_alphabetic() || "@_:".contains(symbol)
                    }) {
                        optional_argument = config
                            .syntax
                            .commands
                            .get(name.trim_end_matches('*'))
                            .or_else(|| {
                                crate::package_commands::builtin_command_rule(name).filter(|_| {
                                    !declared_commands.contains(name.trim_end_matches('*'))
                                })
                            })
                            .is_none_or(|rule| rule.optional_arguments);
                    } else {
                        optional_argument = false;
                        ranges.push(start..position);
                    }
                } else {
                    position += 1;
                    optional_argument = false;
                }
            }
            b'{' if text_openings.remove(&position) => {
                position += 1;
                optional_argument = false;
            }
            b'{' | b'[' if source.as_bytes()[position] == b'{' || optional_argument => {
                let open = source.as_bytes()[position];
                position = balanced_end(
                    source,
                    position,
                    open,
                    if open == b'{' { b'}' } else { b']' },
                    &protected,
                )
                .unwrap_or(source.len());
                ranges.push(start..position);
            }
            b'[' => {
                position += 1;
                optional_argument = false;
            }
            b'$' => {
                let delimiter = if source[position..].starts_with("$$") {
                    "$$"
                } else {
                    "$"
                };
                position = math_end(source, position + delimiter.len(), delimiter, &protected);
                ranges.push(start..position);
                optional_argument = false;
            }
            _ => {
                let end = position
                    + source.as_bytes()[position..]
                        .iter()
                        .position(|byte| matches!(byte, b'\\' | b'%' | b'{' | b'[' | b'$'))
                        .unwrap_or(source.len() - position);
                if !source[position..end]
                    .trim_matches([' ', '\t', '\r', '\n'])
                    .is_empty()
                {
                    optional_argument = false;
                }
                position = end;
            }
        }
    }
    merge_ranges(ranges);
}

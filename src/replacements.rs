use std::borrow::Cow;

use crate::edit_plan::EditPlan;
use crate::syntax::protected_ranges;
use crate::{CompiledConfig, FileKind, FormatError};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum When {
    #[default]
    Before,
    After,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Scope {
    #[default]
    Unprotected,
    All,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum MatchMode {
    #[default]
    Literal,
    Regex,
    Enhanced,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct ReplacementConfig {
    pub find: String,
    pub replace: String,
    #[serde(rename = "match")]
    pub match_mode: MatchMode,
    #[serde(rename = "stage")]
    pub when: When,
    pub scope: Scope,
    pub enabled: bool,
    pub count: Option<usize>,
    pub backtrack_limit: usize,
}
impl Default for ReplacementConfig {
    fn default() -> Self {
        Self {
            find: String::new(),
            replace: String::new(),
            match_mode: MatchMode::Literal,
            when: When::Before,
            scope: Scope::Unprotected,
            enabled: true,
            count: None,
            backtrack_limit: 1_000_000,
        }
    }
}

#[derive(Debug)]
enum Pattern {
    Literal(String),
    Regex(regex::Regex),
    Enhanced(fancy_regex::Regex),
}
#[derive(Debug)]
pub struct CompiledReplacement {
    config: ReplacementConfig,
    pattern: Pattern,
}

pub fn compile(rules: &[ReplacementConfig]) -> Result<Vec<CompiledReplacement>, String> {
    rules
        .iter()
        .filter(|r| r.enabled)
        .map(|rule| {
            if rule.find.is_empty() {
                return Err("replacement.find must not be empty".into());
            }
            let pattern = match rule.match_mode {
                MatchMode::Literal => Pattern::Literal(rule.find.clone()),
                MatchMode::Regex => {
                    Pattern::Regex(regex::Regex::new(&rule.find).map_err(|e| e.to_string())?)
                }
                MatchMode::Enhanced => Pattern::Enhanced(
                    fancy_regex::RegexBuilder::new(&rule.find)
                        .backtrack_limit(rule.backtrack_limit)
                        .build()
                        .map_err(|e| e.to_string())?,
                ),
            };
            Ok(CompiledReplacement {
                config: rule.clone(),
                pattern,
            })
        })
        .collect()
}

impl CompiledReplacement {
    fn propose_edits<'a>(
        &'a self,
        source: &str,
        offset: usize,
        remaining: &mut usize,
        edits: &mut EditPlan<'a>,
    ) -> Result<(), FormatError> {
        if *remaining == 0 {
            return Ok(());
        }
        match &self.pattern {
            Pattern::Literal(pattern) => {
                for (pos, matched) in source.match_indices(pattern) {
                    if *remaining == 0 {
                        break;
                    }
                    Self::propose_match(
                        source,
                        offset,
                        pos..pos + matched.len(),
                        self.config.replace.as_str(),
                        edits,
                    );
                    *remaining -= 1;
                }
            }
            Pattern::Regex(regex) => {
                for capture in regex.captures_iter(source) {
                    if *remaining == 0 {
                        break;
                    }
                    let matched = capture.get(0).expect("capture zero");
                    let mut replacement = String::new();
                    capture.expand(&self.config.replace, &mut replacement);
                    Self::propose_match(
                        source,
                        offset,
                        matched.start()..matched.end(),
                        replacement,
                        edits,
                    );
                    *remaining -= 1;
                }
            }
            Pattern::Enhanced(regex) => {
                for capture in regex.captures_iter(source) {
                    if *remaining == 0 {
                        break;
                    }
                    let capture = capture.map_err(|e| FormatError::Replacement(e.to_string()))?;
                    let matched = capture.get(0).expect("capture zero");
                    let mut replacement = String::new();
                    capture.expand(&self.config.replace, &mut replacement);
                    Self::propose_match(
                        source,
                        offset,
                        matched.start()..matched.end(),
                        replacement,
                        edits,
                    );
                    *remaining -= 1;
                }
            }
        }
        Ok(())
    }

    fn propose_match<'a>(
        source: &str,
        offset: usize,
        range: std::ops::Range<usize>,
        replacement: impl Into<Cow<'a, str>>,
        edits: &mut EditPlan<'a>,
    ) {
        let replacement = replacement.into();
        if &source[range.clone()] != replacement.as_ref() {
            edits.replace(offset + range.start..offset + range.end, replacement);
        }
    }
}

pub(crate) fn is_active(config: &CompiledConfig, when: When) -> bool {
    config
        .replacements
        .iter()
        .any(|rule| rule.config.when == when)
}

pub fn apply(source: &str, config: &CompiledConfig, when: When) -> Result<String, FormatError> {
    apply_for_kind(source, FileKind::Tex, config, when)
}

pub fn apply_for_kind(
    source: &str,
    kind: FileKind,
    config: &CompiledConfig,
    when: When,
) -> Result<String, FormatError> {
    apply_recording(source, kind, config, when, None)
}

pub(crate) fn apply_recording(
    source: &str,
    kind: FileKind,
    config: &CompiledConfig,
    when: When,
    mut history: Option<&mut crate::edit_plan::EditHistory>,
) -> Result<String, FormatError> {
    let mut text = source.to_owned();
    for rule in config.replacements.iter().filter(|r| r.config.when == when) {
        let mut remaining = rule.config.count.unwrap_or(usize::MAX);
        let mut edits = EditPlan::new();
        if rule.config.scope == Scope::All {
            rule.propose_edits(&text, 0, &mut remaining, &mut edits)?;
        } else {
            let ranges = if kind == FileKind::Bibtex {
                crate::bibtex::protected_ranges(&text)?
            } else if kind == FileKind::Bst {
                crate::bst::protected_ranges(&text, true)
            } else {
                protected_ranges(&text, &config.config)?
            };
            let mut pos = 0usize;
            for range in ranges {
                rule.propose_edits(&text[pos..range.start], pos, &mut remaining, &mut edits)?;
                pos = range.end;
            }
            rule.propose_edits(&text[pos..], pos, &mut remaining, &mut edits)?;
        }
        if !edits.is_empty() {
            text = edits.apply_recording(&text, history.as_deref_mut())?;
        }
    }
    Ok(text)
}

#[path = "passive_runs.rs"]
mod passive_runs;

use super::{
    CompiledSyntax, Config, FormatError, ReaderScan, conditional_candidate_at,
    conditional_environment_spellings, environment_definition_spellings, is_protected,
    may_reference_raw_reader, protected_ranges_indexed,
};
use crate::config::ProtectConfig;
use crate::lexical_index::{RawTokenStream, SourceToken, SourceTokenKind, SourceTokens};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    ops::Range,
    sync::{Mutex, OnceLock},
};

#[derive(Default)]
pub(super) struct RedefinedCommands {
    names: HashSet<String>,
    filter: crate::name_filter::NameFilter,
}

#[derive(Default)]
pub(super) struct ScopedEnvironments<'source> {
    slots: Vec<Option<(&'source str, Option<&'source str>)>>,
}

impl<'source> ScopedEnvironments<'source> {
    pub(super) fn get(&mut self, name: &'source str) -> Option<&'source str> {
        if self.slots.is_empty() {
            self.slots.resize(256, None);
        }
        let bytes = name.as_bytes();
        let index = (bytes.len().wrapping_mul(31)
            ^ usize::from(bytes.first().copied().unwrap_or(0))
            ^ (usize::from(bytes.last().copied().unwrap_or(0)) * 7))
            & 255;
        let slot = &mut self.slots[index];
        if let Some((key, environment)) = *slot
            && key == name
        {
            return environment;
        }
        let environment = crate::packages::scoped_environment_for_command(name);
        *slot = Some((name, environment));
        environment
    }
}

impl RedefinedCommands {
    pub(super) fn contains(&self, name: &str) -> bool {
        self.filter.may_contain(name) && self.names.contains(name)
    }

    pub(super) fn insert(&mut self, name: String) {
        self.filter.insert(&name);
        self.names.insert(name);
    }

    pub(super) fn remove(&mut self, name: &str) {
        self.names.remove(name);
    }
}

pub struct ProtectionTape {
    text: String,
    candidates: OnceLock<Option<Candidates>>,
    facts: OnceLock<TapeFacts>,
}

impl ProtectionTape {
    pub fn new(text: String) -> Self {
        Self {
            text,
            candidates: OnceLock::new(),
            facts: OnceLock::new(),
        }
    }

    pub fn prefix(&self, end: usize) -> Option<&str> {
        self.text.get(..end)
    }

    pub fn ranges(
        &self,
        end: usize,
        config: &Config,
        compiled: Option<&CompiledSyntax>,
    ) -> Option<Result<Vec<Range<usize>>, FormatError>> {
        let source = self.prefix(end)?;
        let candidates = self.candidates.get_or_init(|| Candidates::scan(&self.text));
        let facts =
            (end >= 32 * 1024).then(|| self.facts.get_or_init(|| TapeFacts::new(&self.text)));
        let tokens = facts
            .filter(|_| !crate::source_model::SourceModel::has_anchored_loader_events(config))
            .map(|facts| {
                let count = facts.raw.partition_point(|token| token.span.end <= end);
                RefCell::new(RawTokenStream::buffered(source, &facts.raw[..count]))
            });
        Some(protected_ranges_indexed(
            source,
            config,
            compiled,
            Some(&mut Vec::new()),
            ReaderScan {
                raw: tokens.as_ref().map(|tokens| super::RawReaderFacts {
                    tokens,
                    loaders: None,
                }),
                tape: facts,
                ..Default::default()
            },
            candidates.as_ref(),
        ))
    }
}

pub(super) struct TapeFacts {
    pub(super) passive: passive_runs::PassiveRuns,
    raw: Vec<SourceToken>,
    bodies: HashMap<usize, OnceLock<Range<usize>>>,
    groups: OnceLock<HashMap<usize, usize>>,
}

impl TapeFacts {
    fn new(text: &str) -> Self {
        let mut raw = Vec::new();
        let mut bodies = HashMap::new();
        for token in SourceTokens::new(text, &[]) {
            if let Some((name, end)) = token.control(text)
                && matches!(
                    super::unstarred(name),
                    "def"
                        | "gdef"
                        | "edef"
                        | "xdef"
                        | "newcommand"
                        | "renewcommand"
                        | "providecommand"
                )
            {
                bodies.insert(end, OnceLock::new());
            }
            if matches!(
                token.kind,
                SourceTokenKind::Control | SourceTokenKind::OpenGroup | SourceTokenKind::CloseGroup
            ) {
                raw.push(token);
            }
        }
        Self {
            passive: passive_runs::PassiveRuns::new(text),
            raw,
            bodies,
            groups: OnceLock::new(),
        }
    }

    pub(super) fn body(&self, source: &str, name: &str, end: usize) -> Option<Range<usize>> {
        let Some(slot) = self.bodies.get(&end) else {
            return super::macro_body(source, name, end);
        };
        if let Some(range) = slot.get() {
            return (range.end <= source.len()).then(|| range.clone());
        }
        let range = super::macro_body(source, name, end)?;
        let _ = slot.set(range.clone());
        Some(range)
    }

    pub(super) fn group_end(&self, start: usize, limit: usize) -> Option<usize> {
        self.groups
            .get_or_init(|| {
                crate::source_model::SourceModel::literal_group_ends(self.raw.iter().cloned())
            })
            .get(&start)
            .copied()
            .filter(|&end| end <= limit)
    }
}

const MAX_MARKER: usize = r"\ProvideDocumentEnvironment".len();

#[derive(Clone, Default)]
pub(super) struct Candidates {
    pub(super) closing: super::protection_branch::ClosingIndex,
    definitions: Vec<Range<usize>>,
    branches: Vec<Range<usize>>,
    expl3_end: Option<usize>,
    first_begin: Option<usize>,
    raw_reader: RawReaderPrefix,
}

#[derive(Default)]
struct RawReaderPrefix(Mutex<Option<(ProtectConfig, Option<usize>)>>);

impl Clone for RawReaderPrefix {
    fn clone(&self) -> Self {
        Self(Mutex::new(self.0.lock().unwrap().clone()))
    }
}

impl RawReaderPrefix {
    fn may_reference(&self, source: &str, config: &Config) -> bool {
        if source.len() < 32 * 1024 || !source.ends_with('\n') {
            return true;
        }
        let mut memo = self.0.lock().unwrap();
        let (protect, end) = memo.get_or_insert_with(|| (config.protect.clone(), Some(0)));
        if *protect != config.protect {
            return true;
        }
        let Some(suffix) = end.and_then(|end| source.get(end..)) else {
            return true;
        };
        if may_reference_raw_reader(suffix, config) {
            *end = None;
            return true;
        }
        *end = Some(source.len());
        false
    }
}

impl Candidates {
    fn scan(text: &str) -> Option<Self> {
        let mut candidates = Self::default();
        candidates.extend(text, 0)?;
        Some(candidates)
    }

    fn extend(&mut self, text: &str, previous: usize) -> Option<()> {
        let restart = text.floor_char_boundary(previous.saturating_sub(MAX_MARKER - 1));
        let tail = &text[restart..];
        self.closing.extend(text, restart);
        if self.first_begin.is_none() {
            self.first_begin = tail.find(r"\begin").map(|at| restart + at);
        }
        for (pattern, ranges) in [
            (environment_definition_spellings(), &mut self.definitions),
            (conditional_environment_spellings(), &mut self.branches),
        ] {
            for marker in pattern.find_iter(tail) {
                if restart + marker.end() <= previous {
                    continue;
                }
                if ranges.len() == 2048 {
                    return None;
                }
                ranges.push(restart + marker.start()..restart + marker.end());
            }
        }
        if self.expl3_end.is_none() {
            self.expl3_end = tail
                .find(r"\ExplSyntaxOn")
                .map(|at| restart + at + r"\ExplSyntaxOn".len());
        }
        Some(())
    }

    pub(super) fn has_definition(
        &self,
        source: &str,
        config: &Config,
        ranges: &[Range<usize>],
    ) -> bool {
        self.definitions
            .iter()
            .take_while(|marker| marker.end <= source.len())
            .any(|marker| {
                !is_protected(marker.start, ranges)
                    || config.protect.expl3 && self.expl3_end.is_some_and(|end| end <= source.len())
            })
            && self.raw_reader.may_reference(source, config)
    }

    pub(super) fn has_branch(&self, source: &str, ranges: &[Range<usize>]) -> bool {
        self.branches
            .iter()
            .take_while(|marker| marker.end <= source.len())
            .any(|marker| conditional_candidate_at(source, ranges, marker.clone()))
    }

    pub(super) fn branch_limit(&self, source: &str, grammar: &CompiledSyntax) -> usize {
        let limit = self.closing_limit(source, grammar);
        if grammar.specials.is_empty()
            && !source.ends_with('\\')
            && self.first_begin.is_none_or(|start| start >= limit)
        {
            return 0;
        }
        limit
    }

    fn closing_limit(&self, source: &str, grammar: &CompiledSyntax) -> usize {
        if source.ends_with('\n') {
            return self.closing.token_start_limit(source, grammar);
        }
        if source.ends_with('\\') {
            return source.len();
        }
        let Some(line) = memchr::memrchr(b'\n', source.as_bytes()).map(|at| at + 1) else {
            return source.len();
        };
        let limit = self.closing.token_start_limit(&source[..line], grammar);
        let tail = &source[line..];
        if limit == line
            || tail.contains(r"\fi")
            || tail.contains(r"\MH_fi:")
            || tail.contains(r"\csname")
        {
            return source.len();
        }
        limit
    }
}

#[derive(Clone, Default)]
pub(super) struct MarkerCursor {
    end: usize,
    positions: Option<Candidates>,
}

impl MarkerCursor {
    pub(super) fn update(
        &mut self,
        source: &str,
        verified_prefix: Option<usize>,
    ) -> Option<&Candidates> {
        if self.end == 0 || verified_prefix.is_none_or(|verified| self.end > verified) {
            self.positions = Candidates::scan(source);
        } else if source.len() != self.end
            && let Some(positions) = &mut self.positions
            && positions.extend(source, self.end).is_none()
        {
            self.positions = None;
        }
        self.end = source.len();
        self.positions.as_ref()
    }
}

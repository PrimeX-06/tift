use super::{
    CompiledSyntax, Config, FormatError, ReaderScan, conditional_candidate_at,
    conditional_environment_spellings, environment_definition_spellings, is_protected,
    may_reference_raw_reader, protected_ranges_indexed,
};
use crate::config::ProtectConfig;
use std::{
    ops::Range,
    sync::{Mutex, OnceLock},
};

pub struct ProtectionTape {
    text: String,
    candidates: OnceLock<Option<Candidates>>,
}

impl ProtectionTape {
    pub fn new(text: String) -> Self {
        Self {
            text,
            candidates: OnceLock::new(),
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
        Some(protected_ranges_indexed(
            source,
            config,
            compiled,
            Some(&mut Vec::new()),
            ReaderScan::default(),
            candidates.as_ref(),
        ))
    }
}

const MAX_MARKER: usize = r"\ProvideDocumentEnvironment".len();

#[derive(Clone, Default)]
pub(super) struct Candidates {
    pub(super) closing: super::protection_branch::ClosingIndex,
    definitions: Vec<Range<usize>>,
    branches: Vec<Range<usize>>,
    expl3_end: Option<usize>,
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
        if self.end == 0 || Some(self.end) != verified_prefix {
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

use super::{Config, ImportSource, Input, Sources, StoredSource, syntax};
use std::ops::Range;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

const MAX_TAPE: usize = 4 * 1024 * 1024;
const MAX_SCAN: usize = 24 * 1024 * 1024;

struct Cut {
    end: usize,
    ranges: OnceLock<Option<Vec<Range<usize>>>>,
}

pub(super) struct ProtectionRun {
    tape: syntax::ProtectionTape,
    cuts: Vec<Cut>,
    config: Config,
    compiled: Option<syntax::CompiledSyntax>,
    cancelled: AtomicBool,
}

impl ProtectionRun {
    pub(super) fn from_cached_leaves(
        prefix: &str,
        parent: &ImportSource,
        mut cursor: usize,
        inputs: &[Input],
        path: &Path,
        sources: &Sources,
        config: &Config,
    ) -> Option<Arc<Self>> {
        if prefix.len() > MAX_TAPE || inputs.len() < 3 {
            return None;
        }
        let directory = path.parent()?;
        let mut tape = String::new();
        let mut cuts = Vec::new();
        let mut scan = 0;
        for input in inputs.iter().take(16) {
            if !input
                .name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_./".contains(&byte))
            {
                break;
            }
            let mut child = directory.join(&input.name);
            if child.extension().is_none() {
                child.set_extension("tex");
            }
            let file = sources.files.lock().unwrap().get(&child).cloned();
            let Some(Ok(StoredSource::Decoded(leaf))) = file.as_deref().and_then(|file| file.get())
            else {
                break;
            };
            let segment = parent.text.get(cursor..input.span.end)?;
            let start = if cuts.is_empty() {
                prefix.len()
            } else {
                tape.len()
            };
            let end = start
                .checked_add(segment.len())?
                .checked_add(leaf.text.len())?
                .checked_add(3)?;
            if end > MAX_TAPE
                || scan + end > MAX_SCAN
                || super::input_candidates(&leaf.text).next().is_some()
            {
                break;
            }
            if cuts.is_empty() {
                tape.push_str(prefix);
            }
            tape.push_str(segment);
            tape.push('\n');
            tape.push_str(&leaf.text);
            tape.push_str("\n\n");
            cuts.push(Cut {
                end,
                ranges: OnceLock::new(),
            });
            scan += end;
            cursor = input.span.end;
        }
        (cuts.len() >= 3).then(|| {
            Arc::new(Self {
                tape: syntax::ProtectionTape::new(tape),
                cuts,
                compiled: syntax::compile(&config.syntax).ok(),
                config: config.clone(),
                cancelled: AtomicBool::new(false),
            })
        })
    }

    fn ranges<'cut>(&self, cut: &'cut Cut) -> Option<&'cut [Range<usize>]> {
        cut.ranges
            .get_or_init(|| {
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    self.tape
                        .ranges(cut.end, &self.config, self.compiled.as_ref())
                }))
                .ok()??
                .ok()
            })
            .as_deref()
    }

    pub(super) fn at(&self, source: &str) -> Option<&[Range<usize>]> {
        if self.cancelled.load(Ordering::Relaxed) {
            return None;
        }
        let index = self
            .cuts
            .binary_search_by_key(&source.len(), |cut| cut.end)
            .ok()?;
        if self.tape.prefix(source.len()) != Some(source) {
            self.cancel();
            return None;
        }
        self.ranges(&self.cuts[index])
    }

    pub(super) fn fill(&self) {
        for cut in &self.cuts {
            if self.cancelled.load(Ordering::Relaxed) {
                break;
            }
            self.ranges(cut);
        }
    }

    pub(super) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    pub(super) fn scanned(&self) -> (usize, usize) {
        self.cuts
            .iter()
            .filter(|cut| cut.ranges.get().is_some())
            .fold((0, 0), |(cuts, bytes), cut| (cuts + 1, bytes + cut.end))
    }
}

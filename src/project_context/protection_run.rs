use super::{Config, ImportSource, Input, ProjectContext, Sources, StoredSource, syntax};
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

const MAX_TAPE: usize = 4 * 1024 * 1024;
const MAX_SCAN: usize = 24 * 1024 * 1024;

struct Cut {
    end: usize,
    prefetch: bool,
    infer: bool,
    ranges: OnceLock<Option<Arc<Vec<Range<usize>>>>>,
    inference: OnceLock<Option<PredictionState>>,
    exports: OnceLock<super::DeclarationSnapshot>,
}

struct LoaderProbe {
    file: Arc<ImportSource>,
    span: Range<usize>,
    result: OnceLock<(Arc<Config>, Option<Option<usize>>)>,
}

pub(super) struct PredictionState {
    pub(super) options: Config,
    pub(super) independent: bool,
    pub(super) boundary: syntax::DeclarationReuseBoundary,
    pub(super) reader: syntax::DeclarationIndex,
    pub(super) exports: Option<super::DeclarationSnapshot>,
}

pub(super) struct ProtectionRun {
    tape: syntax::ProtectionTape,
    cuts: Vec<Cut>,
    config: Config,
    compiled: Option<syntax::CompiledSyntax>,
    cancelled: AtomicBool,
    loaders: HashMap<usize, LoaderProbe>,
}

impl ProtectionRun {
    pub(super) fn predict(
        path: &Path,
        file: &Arc<ImportSource>,
        selected: &HashSet<PathBuf>,
        config: &Config,
        context: &mut ProjectContext,
    ) -> Option<Arc<Self>> {
        if file.candidates.len() < 8 || file.text.len() > MAX_TAPE {
            return None;
        }
        let compiled = syntax::compile(&config.syntax).ok()?;
        let mut plan = Prediction {
            root: path,
            selected,
            config,
            compiled: &compiled,
            context,
            active: HashSet::new(),
            tape: String::new(),
            cuts: Vec::new(),
            visits: 0,
            loaders: HashMap::new(),
        };
        plan.visit(path, file)?;
        let Prediction {
            tape,
            mut cuts,
            loaders,
            ..
        } = plan;
        cuts.sort_unstable_by_key(|&(end, prefetch, infer)| (end, !infer, !prefetch));
        cuts.dedup_by_key(|cut| cut.0);
        let mut scan = 0;
        (cuts.len() >= 5).then(|| {
            Arc::new(Self {
                tape: syntax::ProtectionTape::new(tape),
                cuts: cuts
                    .into_iter()
                    .map(|(end, prefetch, infer)| {
                        if prefetch {
                            scan += end;
                        }
                        Cut {
                            end,
                            prefetch: prefetch && scan <= MAX_SCAN,
                            infer,
                            ranges: OnceLock::new(),
                            inference: OnceLock::new(),
                            exports: OnceLock::new(),
                        }
                    })
                    .collect(),
                config: config.clone(),
                compiled: Some(compiled),
                cancelled: AtomicBool::new(false),
                loaders,
            })
        })
    }

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
                prefetch: true,
                infer: false,
                ranges: OnceLock::new(),
                inference: OnceLock::new(),
                exports: OnceLock::new(),
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
                loaders: HashMap::new(),
            })
        })
    }

    fn ranges<'cut>(&self, cut: &'cut Cut) -> Option<&'cut Arc<Vec<Range<usize>>>> {
        cut.ranges
            .get_or_init(|| {
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    self.tape
                        .ranges(cut.end, &self.config, self.compiled.as_ref())
                }))
                .ok()??
                .ok()
                .map(Arc::new)
            })
            .as_ref()
    }

    pub(super) fn at(&self, source: &str) -> Option<&[Range<usize>]> {
        self.shared_at(source).map(|ranges| ranges.as_slice())
    }

    pub(super) fn shared_at(&self, source: &str) -> Option<&Arc<Vec<Range<usize>>>> {
        self.ranges(self.cut_at(source)?)
    }

    fn cut_at(&self, source: &str) -> Option<&Cut> {
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
        Some(&self.cuts[index])
    }

    pub(super) fn inference(&self, source: &str) -> Option<&PredictionState> {
        let cut = self.cut_at(source)?;
        if !cut.infer || !cut.prefetch {
            return None;
        }
        if self.dense() {
            return cut.inference.get()?.as_ref();
        }
        cut.inference
            .get_or_init(|| self.read_state(cut, &mut Default::default(), &mut Default::default()))
            .as_ref()
    }

    pub(super) fn infer_at(
        &self,
        source: &str,
        reader: &syntax::DeclarationIndex,
        exports: &mut tift::source_model::ProjectExportIndex,
    ) -> Option<&PredictionState> {
        let cut = self.cut_at(source)?;
        if !cut.infer || !cut.prefetch {
            return None;
        }
        cut.inference
            .get_or_init(|| self.read_state(cut, &mut reader.clone(), exports))
            .as_ref()
    }

    pub(super) fn suffix_state(
        &self,
        source: &str,
        suffix: &str,
        newline: bool,
    ) -> Option<&PredictionState> {
        let length = source
            .len()
            .checked_add(suffix.len())?
            .checked_add(usize::from(newline))?;
        let index = self
            .cuts
            .binary_search_by_key(&length, |cut| cut.end)
            .ok()?;
        let state = self.cuts[index].inference.get()?.as_ref()?;
        let text = self.tape.prefix(length)?;
        let text = if newline {
            text.strip_suffix('\n')?
        } else {
            text
        };
        (text.strip_prefix(source) == Some(suffix)).then_some(state)
    }

    fn read_state(
        &self,
        cut: &Cut,
        reader: &mut syntax::DeclarationIndex,
        exports: &mut tift::source_model::ProjectExportIndex,
    ) -> Option<PredictionState> {
        let ranges = self.ranges(cut)?;
        let source = self.tape.prefix(cut.end)?;
        reader.supply_protection(source, &self.config, Arc::clone(ranges));
        let mut snapshot = None;
        let (options, independent, boundary) = reader
            .infer_with_export_index(source, &self.config, exports, &mut snapshot)
            .ok()?;
        Some(PredictionState {
            options,
            independent,
            boundary,
            reader: reader.clone(),
            exports: snapshot,
        })
    }

    pub(super) fn exports(
        &self,
        source: &str,
        reader: &syntax::DeclarationIndex,
        exports: &mut tift::source_model::ProjectExportIndex,
    ) -> Option<&super::DeclarationSnapshot> {
        let cut = self.cut_at(source)?;
        if cut.infer && cut.prefetch {
            return self.infer_at(source, reader, exports)?.exports.as_ref();
        }
        let ranges = self.ranges(cut)?;
        Some(
            cut.exports
                .get_or_init(|| exports.snapshot(source, &self.config, Some(ranges))),
        )
    }

    pub(super) fn loader(
        &self,
        end: usize,
        file: &Arc<ImportSource>,
        input: &Input,
        options: &Config,
    ) -> Option<Option<Option<usize>>> {
        let probe = self.loaders.get(&end)?;
        let (state, result) = probe.result.get()?;
        (Arc::ptr_eq(file, &probe.file)
            && probe.span == input.span
            && state.syntax == options.syntax
            && state.protect == options.protect)
            .then_some(*result)
    }

    pub(super) fn fill(&self) {
        if self.dense() {
            let workers = rayon::current_num_threads().saturating_sub(1).max(1);
            let mut cost = 0usize;
            let cumulative: Vec<_> = self
                .cuts
                .iter()
                .map(|cut| {
                    cost += if cut.prefetch { cut.end } else { 0 };
                    cost
                })
                .collect();
            let first = cost / (2 * (workers + 1));
            let boundary = |part| {
                cumulative.partition_point(|&sum| sum <= first + (cost - first) * part / workers)
            };
            (0..workers).into_par_iter().for_each(|worker| {
                let start = boundary(worker);
                let end = boundary(worker + 1);
                let _ =
                    super::super::recover_panic(|| self.fill_chunk(self.cuts[start..end].iter()));
            });
            return;
        }
        let next = AtomicUsize::new(0);
        let workers = rayon::current_num_threads().saturating_sub(1).max(1);
        (0..workers).into_par_iter().for_each(|_| {
            let cuts = std::iter::from_fn(|| self.cuts.get(next.fetch_add(1, Ordering::Relaxed)));
            let _ = super::super::recover_panic(|| self.fill_chunk(cuts));
        });
    }

    pub(super) fn dense(&self) -> bool {
        self.cuts.len() > 40 && self.cuts.last().is_some_and(|cut| cut.end >= 256 * 1024)
    }

    fn fill_chunk<'cut>(&self, cuts: impl Iterator<Item = &'cut Cut>) {
        let mut reader = syntax::DeclarationIndex::default();
        let mut exports = tift::source_model::ProjectExportIndex::default();
        let mut prior_options = None;
        for cut in cuts {
            if self.cancelled.load(Ordering::Relaxed) {
                break;
            }
            if cut.prefetch
                && let Some(ranges) = self.ranges(cut)
            {
                let source = self.tape.prefix(cut.end).unwrap();
                if !cut.infer {
                    let _ = cut
                        .exports
                        .get_or_init(|| exports.snapshot(source, &self.config, Some(ranges)));
                    continue;
                }
                let _ = cut
                    .inference
                    .get_or_init(|| self.read_state(cut, &mut reader, &mut exports));
                prior_options = cut
                    .inference
                    .get()
                    .and_then(|state| state.as_ref())
                    .map(|state| Arc::new(state.options.clone()));
            }
            if let Some(probe) = self.loaders.get(&cut.end)
                && let Some(options) = &prior_options
            {
                let _ = probe.result.get_or_init(|| {
                    let result = super::literal_inputs(&probe.file.text[..probe.span.end], options)
                        .ok()
                        .and_then(|parsed| {
                            parsed
                                .inputs
                                .iter()
                                .any(|input| input.span == probe.span)
                                .then_some(parsed.document_start)
                        });
                    (Arc::clone(options), result)
                });
            }
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

struct Prediction<'a> {
    root: &'a Path,
    selected: &'a HashSet<PathBuf>,
    config: &'a Config,
    compiled: &'a syntax::CompiledSyntax,
    context: &'a mut ProjectContext,
    active: HashSet<PathBuf>,
    tape: String,
    cuts: Vec<(usize, bool, bool)>,
    visits: usize,
    loaders: HashMap<usize, LoaderProbe>,
}

impl Prediction<'_> {
    fn cut(&mut self, prefetch: bool, infer: bool) {
        if self.tape.len() >= 4096 {
            self.cuts.push((self.tape.len(), prefetch, infer));
        }
    }

    fn visit(&mut self, path: &Path, file: &Arc<ImportSource>) -> Option<()> {
        self.visits += 1;
        if self.visits > 128
            || self.cuts.len() > 512
            || self.active.len() >= 24
            || self.tape.len().checked_add(file.text.len())? > MAX_TAPE
        {
            return None;
        }
        self.active.insert(path.to_owned());
        if self.selected.contains(path) {
            self.cut(false, false);
        }
        let mut cursor = 0;
        if !file.candidates.is_empty() {
            let structural = file
                .candidates
                .iter()
                .any(|input| input.name.contains('\\'))
                || matches!(
                    path.extension().and_then(|extension| extension.to_str()),
                    Some("cls" | "sty")
                );
            let parsed = if structural {
                Some(
                    super::literal_inputs_with_syntax(&file.text, self.config, self.compiled)
                        .ok()?,
                )
            } else {
                None
            };
            for input in &file.candidates {
                let prefix = &file.text[..input.span.start];
                let line = prefix.rsplit('\n').next().unwrap_or(prefix);
                if line.contains('%')
                    || parsed.as_ref().is_some_and(|parsed| {
                        !parsed.inputs.iter().any(|active| active.span == input.span)
                    }) && !(input.name.contains('\\')
                        && super::literal_inputs_with_syntax(
                            &file.text[..input.span.end],
                            self.config,
                            self.compiled,
                        )
                        .is_ok_and(|prefix| {
                            prefix.inputs.iter().any(|active| active.span == input.span)
                        }))
                {
                    continue;
                }
                let name = syntax::command_at(&file.text, input.span.start)?.0;
                if super::ignored_resource_input(input, name, &self.config.syntax) {
                    continue;
                }
                let dynamic = input.name.contains('\\');
                let paths = if dynamic {
                    Vec::new()
                } else {
                    self.context
                        .loader_paths(&input.name, name, path, self.root, &self.config.syntax)
                        .ok()?
                };
                if paths.is_empty() && !dynamic {
                    continue;
                }
                if input.span.end >= 4096 && self.tape.len() >= super::LARGE_PROJECT_PREFIX {
                    self.loaders.insert(
                        self.tape.len(),
                        LoaderProbe {
                            file: Arc::clone(file),
                            span: input.span.clone(),
                            result: OnceLock::new(),
                        },
                    );
                }
                self.tape.push_str(&file.text[cursor..input.span.start]);
                let preview = !file.text[cursor..input.span.start].trim().is_empty();
                self.cut(preview, preview && dynamic);
                self.tape.push_str(&file.text[input.span.clone()]);
                self.tape.push('\n');
                let child_selected = paths
                    .last()
                    .is_some_and(|child| self.selected.contains(child));
                for child in paths {
                    if self.active.contains(&child) {
                        continue;
                    }
                    if !child.starts_with(self.root.parent()?) {
                        return None;
                    }
                    if !self.context.sources.contains(&child)
                        && std::fs::metadata(&child).ok()?.len() > MAX_TAPE as u64
                    {
                        return None;
                    }
                    let child_file = self.context.sources.import(&child, None).ok()?;
                    self.visit(&child, &child_file)?;
                }
                self.tape.push('\n');
                if self.selected.contains(path) {
                    self.cut(!child_selected, !child_selected);
                }
                cursor = input.span.end;
            }
        }
        self.tape.push_str(&file.text[cursor..]);
        self.tape.push('\n');
        self.cut(self.selected.contains(path), self.selected.contains(path));
        self.active.remove(path);
        Some(())
    }
}

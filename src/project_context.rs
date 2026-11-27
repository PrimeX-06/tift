use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use ignore::WalkBuilder;
use rayon::prelude::*;
use tift::config::{ProtectConfig, SyntaxConfig};
use tift::syntax::{self, TokenKind};
use tift::{CompiledConfig, Config, FileKind};

use super::config_io::absolute;
use super::project_cache::ProjectCache;
use super::source_encoding::SourceFile;
#[path = "project_telemetry.rs"]
mod telemetry;
use telemetry::{ProjectMetrics, ProjectReports, ProjectTrace};

#[path = "project_context/protection_run.rs"]
mod protection_run;
use protection_run::ProtectionRun;

const LARGE_PROJECT_PREFIX: usize = 32 * 1024;
const MAX_INPUT_DEPTH: usize = 128;

enum StoredSource {
    Decoded(Arc<SourceFile>),
    Undecodable { error: String, bytes: Vec<u8> },
}

impl StoredSource {
    fn load(path: &Path) -> Result<Self, String> {
        let bytes = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
        Ok(match SourceFile::decode_snapshot(bytes, None) {
            Ok(source) => Self::Decoded(Arc::new(source)),
            Err((error, bytes)) => Self::Undecodable {
                error: format!("{}: {error}", path.display()),
                bytes,
            },
        })
    }
}

type SourceSlot = OnceLock<Result<StoredSource, String>>;
type ImportKey = (PathBuf, Option<String>);

/// A selected file and all of its importers see the same input, even when
/// different workers reach it at the same time.
#[derive(Default)]
struct Sources {
    files: Mutex<HashMap<PathBuf, Arc<SourceSlot>>>,
    imports: Mutex<HashMap<ImportKey, Arc<ImportSlot>>>,
}

type ImportSlot = OnceLock<Result<Arc<ImportSource>, String>>;

/// These inputs depend on the decoded bytes only. Executed declarations and
/// active loader decisions still belong to the ordered root expansion.
struct ImportSource {
    text: Arc<String>,
    candidates: Vec<Input>,
    loaders: OnceLock<HashMap<usize, (String, usize)>>,
}

impl ImportSource {
    fn loader_at(&self, start: usize) -> Option<&(String, usize)> {
        self.loaders
            .get_or_init(|| tift::source_model::SourceModel::loader_occurrences(&self.text))
            .get(&start)
    }
}

impl Sources {
    fn slot(&self, path: &Path) -> Arc<SourceSlot> {
        let mut slots = self.files.lock().unwrap();
        if let Some(slot) = slots.get(path) {
            return Arc::clone(slot);
        }
        Arc::clone(slots.entry(path.to_owned()).or_default())
    }

    fn read(&self, path: &Path) -> Result<Arc<SourceFile>, String> {
        let slot = self.slot(path);
        match slot.get_or_init(|| StoredSource::load(path)).as_ref()? {
            StoredSource::Decoded(source) => Ok(Arc::clone(source)),
            StoredSource::Undecodable { error, .. } => Err(error.clone()),
        }
    }

    fn read_with_codec(&self, path: &Path, label: &str) -> Result<Arc<String>, String> {
        let slot = self.slot(path);
        let bytes = match slot.get_or_init(|| StoredSource::load(path)).as_ref()? {
            StoredSource::Decoded(source) => source.original_bytes(),
            StoredSource::Undecodable { bytes, .. } => bytes.as_slice(),
        };
        SourceFile::decode_with_default(bytes.to_vec(), Some(label))
            .map(|input| input.text)
            .map_err(|error| format!("{}: {error}", path.display()))
    }

    fn import(&self, path: &Path, codec: Option<&str>) -> Result<Arc<ImportSource>, String> {
        let key = (path.to_path_buf(), codec.map(str::to_owned));
        let slot = Arc::clone(self.imports.lock().unwrap().entry(key).or_default());
        slot.get_or_init(|| {
            let text = match codec {
                Some(label) => self.read_with_codec(path, label)?,
                None => Arc::clone(&self.read(path)?.text),
            };
            let candidates: Vec<_> = input_candidates(&text).collect();
            Ok(Arc::new(ImportSource {
                text,
                candidates,
                loaders: OnceLock::new(),
            }))
        })
        .clone()
    }

    fn contains(&self, path: &Path) -> bool {
        self.files
            .lock()
            .unwrap()
            .get(path)
            .and_then(|slot| slot.get())
            .is_some_and(Result::is_ok)
    }
}

/// One dependency is built before its consumer documents enter the worker queue.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum ProjectDependency {
    Standalone,
    Root(PathBuf),
    Siblings(PathBuf),
}

#[derive(Default)]
pub(super) struct ProjectSnapshot {
    sources: Arc<Sources>,
    cache_enabled: bool,
    reports: Option<Box<ProjectReports>>,
}

impl ProjectSnapshot {
    pub(super) fn new(cache_enabled: bool, trace: bool) -> Self {
        Self {
            cache_enabled,
            reports: trace.then(Box::default),
            ..Self::default()
        }
    }
    pub(super) fn report(&self) {
        if let Some(reports) = &self.reports {
            reports.report();
        }
    }
    pub(super) fn read(&self, path: &Path) -> Result<Arc<SourceFile>, String> {
        self.sources.read(path)
    }

    fn context(&self) -> ProjectContext {
        ProjectContext {
            sources: Arc::clone(&self.sources),
            ..Default::default()
        }
    }

    pub(super) fn dependency(
        &self,
        path: &Path,
        source: &str,
        config: &CompiledConfig,
        inputs: Option<ProjectInputs>,
    ) -> Result<ProjectDependency, String> {
        let Some(inputs) = inputs else {
            return Ok(ProjectDependency::Standalone);
        };
        let mut resolver = self.context();
        let root = resolver.resolve_root(path, inputs.root.as_deref(), config)?;
        if inputs.root.is_some() || resolver.has_inputs(path, &root, source, config, &inputs) {
            // Selected chapters can prepare their lexical inputs concurrently,
            // before their common root interprets the ordered declarations.
            self.sources.import(path, None)?;
            return Ok(ProjectDependency::Root(root));
        }
        if config.config.syntax.infer_list_environments
            && (source.contains("\\begin")
                || source.contains("\\csname")
                || config.config.syntax.environment_pattern.is_some()
                || !config.config.syntax.special.is_empty())
        {
            return Ok(ProjectDependency::Siblings(
                path.parent().expect("absolute input").to_path_buf(),
            ));
        }
        Ok(ProjectDependency::Standalone)
    }

    pub(super) fn build(
        &self,
        dependency: &ProjectDependency,
        base: Arc<CompiledConfig>,
        selected: HashSet<PathBuf>,
    ) -> Result<PreparedProject, String> {
        self.build_inner(dependency, base, selected, None)
    }

    pub(super) fn build_streaming(
        &self,
        dependency: &ProjectDependency,
        base: Arc<CompiledConfig>,
        selected: HashSet<PathBuf>,
        ready: &mut dyn FnMut(&Path, Arc<CompiledConfig>),
    ) -> Result<PreparedProject, String> {
        self.build_inner(dependency, base, selected, Some(ready))
    }

    fn build_inner(
        &self,
        dependency: &ProjectDependency,
        mut base: Arc<CompiledConfig>,
        selected: HashSet<PathBuf>,
        mut ready: Option<&mut FileReady<'_>>,
    ) -> Result<PreparedProject, String> {
        let mut context = self.context();
        let compiled_states = CompiledStates::default();
        let graph = match dependency {
            ProjectDependency::Root(root) => {
                let mut graph = InputGraph {
                    selected,
                    metrics: self.reports.as_ref().map(|_| ProjectMetrics::default()),
                    trace: self.reports.as_ref().map(|_| ProjectTrace::new()),
                    ..Default::default()
                };
                if self.cache_enabled && base.config.cache.enabled {
                    graph.cache_root = Some(root.clone());
                }
                let source = self.sources.import(root, None)?;
                let expand = |schedule: Option<&dyn Fn(Arc<ProtectionRun>)>| {
                    let result = context.expand_inputs(
                        root,
                        source,
                        &base.config,
                        &mut graph,
                        schedule,
                        &mut |path, state| {
                            if let Some(ready) = ready.as_mut()
                                && let Some(options) = &state.options
                                && let Ok(config) = compiled_states.resolve(
                                    &base,
                                    options.syntax.clone(),
                                    options.protect.clone(),
                                    Some(state),
                                )
                            {
                                ready(path, config);
                            }
                        },
                    );
                    if let Some(run) = &graph.protection_run {
                        run.cancel();
                    }
                    result
                };
                if rayon::current_thread_index().is_some() && rayon::current_num_threads() >= 3 {
                    rayon::in_place_scope_fifo(|scope| {
                        expand(Some(&|run| scope.spawn_fifo(move |_| run.fill())))
                    })?;
                } else {
                    expand(None)?;
                }
                let fingerprint = (self.cache_enabled && base.config.cache.enabled)
                    .then(|| graph.full_hash.of(&graph.source));
                if let Some(cache) = graph.cache.take() {
                    cache.flush();
                }
                if let Some(reports) = &self.reports {
                    reports.finish(root, &mut graph);
                }
                Some(PreparedGraph {
                    source: graph.source,
                    files: graph.files,
                    preamble_end: graph.preamble_end,
                    fingerprint,
                })
            }
            ProjectDependency::Siblings(directory) => {
                base = context.sibling_lists(directory, base)?;
                None
            }
            ProjectDependency::Standalone => None,
        };
        Ok(PreparedProject {
            base,
            graph,
            compiled_states,
            trace: self.reports.is_some(),
        })
    }

    pub(super) fn prepare_buffer(
        &self,
        path: &Path,
        input: &Arc<SourceFile>,
        config: Arc<CompiledConfig>,
        inputs: Option<ProjectInputs>,
    ) -> Result<PreparedSource, String> {
        let cwd = std::env::current_dir().map_err(|error| error.to_string())?;
        let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
        let path = absolute(&canonical, &cwd);
        let _ = self
            .sources
            .slot(&path)
            .set(Ok(StoredSource::Decoded(Arc::clone(input))));
        let dependency = self.dependency(&path, &input.text, &config, inputs)?;
        let prepared = self.build(&dependency, config, HashSet::from([path.clone()]))?;
        self.report();
        prepared.for_source(&path, &input.text)
    }
}

type CompiledStateSlot = Arc<OnceLock<Result<Arc<CompiledConfig>, String>>>;
type FileReady<'a> = dyn FnMut(&Path, Arc<CompiledConfig>) + 'a;

#[derive(Default)]
struct CompiledStates(Mutex<HashMap<(SyntaxConfig, ProtectConfig), CompiledStateSlot>>);

impl CompiledStates {
    fn resolve(
        &self,
        base: &Arc<CompiledConfig>,
        mut syntax: SyntaxConfig,
        protect: ProtectConfig,
        file: Option<&FileState>,
    ) -> Result<Arc<CompiledConfig>, String> {
        if let Some(file) = file {
            file.apply_imports(&mut syntax, &base.config.syntax);
        }
        if syntax == base.config.syntax && protect == base.config.protect {
            return Ok(Arc::clone(base));
        }
        let key = (syntax.clone(), protect.clone());
        let slot = Arc::clone(self.0.lock().unwrap().entry(key).or_default());
        slot.get_or_init(|| {
            base.with_project_syntax(syntax, protect)
                .map(Arc::new)
                .map_err(|error| error.to_string())
        })
        .clone()
    }
}

pub(super) struct PreparedProject {
    base: Arc<CompiledConfig>,
    graph: Option<PreparedGraph>,
    compiled_states: CompiledStates,
    trace: bool,
}

struct PreparedGraph {
    source: SourceTape,
    files: HashMap<PathBuf, FileState>,
    preamble_end: Option<usize>,
    fingerprint: Option<blake3::Hash>,
}

#[derive(Default)]
struct ProjectContext {
    sources: Arc<Sources>,
    /// Canonical identities are stable for this invocation's source snapshot.
    /// Relative requests retain their working directory in the cache key.
    normalized_paths: HashMap<(PathBuf, PathBuf), PathBuf>,
    /// Dependency probes share this invocation's immutable source snapshot.
    /// Missing candidates are rechecked by the next ProjectContext.
    input_paths: HashMap<PathBuf, Option<PathBuf>>,
}

#[derive(Clone)]
pub struct PreparedSource {
    pub config: Arc<CompiledConfig>,
    pub input_graph: Option<blake3::Hash>,
}

/// A literal XeTeX assignment directly preceding a loader selects the codec
/// of the child. Blank lines and TeX comments can separate the two commands.
fn preceding_xetex_encoding(prefix: &str) -> Option<&str> {
    let line = prefix.lines().rev().find_map(|line| {
        let line = line.split('%').next()?.trim();
        (!line.is_empty()).then_some(line)
    })?;
    let value = line.strip_prefix(r"\XeTeXdefaultencoding")?.trim();
    let label = value.strip_prefix('"')?.strip_suffix('"')?;
    (!label.is_empty()
        && label
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_-.".contains(&c)))
    .then_some(label)
}

#[derive(Default)]
struct SourceTape {
    full: String,
    compact: Option<String>,
    compacted: Vec<(usize, usize)>,
}

impl std::ops::Deref for SourceTape {
    type Target = str;

    fn deref(&self) -> &str {
        &self.full
    }
}

impl SourceTape {
    fn inference(&self) -> &str {
        self.compact.as_deref().unwrap_or(&self.full)
    }

    fn inference_mut(&mut self) -> &mut String {
        self.compact.as_mut().unwrap_or(&mut self.full)
    }

    fn append(&mut self, source: &str) {
        self.full.push_str(source);
        if let Some(compact) = &mut self.compact {
            compact.push_str(source);
        }
    }

    fn append_compacted(&mut self, source: &str) {
        let compact = self.compact.get_or_insert_with(|| self.full.clone());
        compact.push('\n');
        self.full.push_str(source);
        self.compacted.push((self.full.len(), compact.len()));
    }

    fn inference_boundary(&self, end: usize) -> usize {
        let count = self.compacted.partition_point(|(full, _)| *full <= end);
        match self.compacted[..count].last() {
            Some(&(full, compact)) => compact + (end - full),
            None => end,
        }
    }
}

#[derive(Default)]
struct InputGraph {
    selected: HashSet<PathBuf>,
    trace: Option<Box<ProjectTrace>>,
    cache_root: Option<PathBuf>,
    cache: Option<ProjectCache>,
    full_hash: PrefixHash,
    inference_hash: PrefixHash,
    index_history: blake3::Hasher,
    deferred_scans: Vec<usize>,
    source: SourceTape,
    declaration_index: syntax::DeclarationIndex,
    alias_index: tift::source_model::ProjectAliasIndex,
    protection_run: Option<Arc<ProtectionRun>>,
    export_index: tift::source_model::ProjectExportIndex,
    inferred_exports: Option<(usize, DeclarationSnapshot)>,
    // Literal binding targets, including definitions with unknown signatures.
    // Their calls may invalidate later dynamic names and must stay in history.
    binding_targets: HashSet<String>,
    literal_inputs_index: Option<LiteralInputState>,
    declarations: Option<Arc<DeclarationSnapshot>>,
    pending_snapshot: Option<(usize, Arc<DeclarationSnapshot>)>,
    export_boundary_end: usize,

    files: HashMap<PathBuf, FileState>,
    preamble_end: Option<usize>,
    revision: u64,
    last_options: Option<DeclarationMemo>,
    // This graph belongs to one immutable explicit base configuration. A
    // historical policy refusal can retain the conservative false answer;
    // incomplete source must keep checking when later inputs complete it.
    boundary_policy_refused: bool,
    metrics: Option<ProjectMetrics>,
}

struct FileState {
    end: usize,
    options: Option<Arc<Config>>,
    entry: Arc<DeclarationSnapshot>,
    imports: Vec<LoaderChange>,
}

struct LoaderChange {
    at: usize,
    loader: Option<(String, usize)>,
    delta: DeclarationSnapshot,
}

fn changed_bindings<T: Clone + PartialEq>(
    before: &BTreeMap<String, Option<T>>,
    after: &BTreeMap<String, Option<T>>,
) -> BTreeMap<String, Option<T>> {
    after
        .iter()
        .filter(|(name, value)| before.get(*name) != Some(value))
        .map(|(name, value)| (name.clone(), value.clone()))
        .chain(
            before
                .keys()
                .filter(|name| !after.contains_key(*name))
                .map(|name| (name.clone(), None)),
        )
        .collect()
}

impl FileState {
    fn apply_imports(&self, syntax: &mut SyntaxConfig, base: &SyntaxConfig) {
        syntax.commands.clone_from(&base.commands);
        syntax.inferred_environment_aliases.clear();
        syntax.environment_alias_events.clear();
        syntax.environment_argument_events.clear();
        syntax.package_load_events.clear();
        syntax.command_import_events.clear();
        let updates = self
            .imports
            .iter()
            .map(|change| (change.at, change.loader.clone(), &change.delta));
        for (at, loader, delta) in std::iter::once((0, None, self.entry.as_ref())).chain(updates) {
            syntax
                .environment_alias_events
                .push(tift::config::EnvironmentAliasEvent {
                    at,
                    loader: loader.clone(),
                    aliases: delta.0.clone(),
                });
            syntax
                .environment_argument_events
                .push(tift::config::EnvironmentArgumentEvent {
                    at,
                    loader: loader.clone(),
                    arguments: delta.1.clone(),
                });
            syntax
                .package_load_events
                .push(tift::config::PackageLoadEvent {
                    at,
                    loader: loader.clone(),
                    packages: delta.2.clone(),
                });
            syntax
                .command_import_events
                .push(tift::config::CommandImportEvent {
                    at,
                    loader,
                    commands: delta.3.clone(),
                });
        }
    }
}

#[derive(Default)]
struct PrefixHash {
    end: usize,
    hash: blake3::Hasher,
}

impl PrefixHash {
    fn of(&mut self, source: &str) -> blake3::Hash {
        self.hash.update(&source.as_bytes()[self.end..]);
        self.end = source.len();
        self.hash.finalize()
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct InferredSyntax {
    syntax: SyntaxConfig,
    protect: ProtectConfig,
    aliases: std::collections::BTreeMap<String, tift::config::EnvironmentAlias>,
    arguments: std::collections::BTreeMap<String, tift::config::EnvironmentArguments>,
}

impl InferredSyntax {
    fn from_config(options: &Config) -> Self {
        Self {
            syntax: options.syntax.clone(),
            protect: options.protect.clone(),
            aliases: options.syntax.inferred_environment_aliases.clone(),
            arguments: options.syntax.inferred_environment_arguments.clone(),
        }
    }

    fn apply(self, base: &Config) -> Config {
        let mut config = base.clone();
        config.syntax = self.syntax;
        config.syntax.inferred_environment_aliases = self.aliases;
        config.syntax.inferred_environment_arguments = self.arguments;
        config.protect = self.protect;
        config
    }
}

struct DeclarationMemo {
    revision: u64,
    options: Arc<Config>,
    safe: bool,
    independent_append_safe: bool,
    reader_settled: bool,
    compiled: Option<Arc<syntax::CompiledSyntax>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DeclarationBoundary {
    Current,
    BeforeSeparator,
    RootEnd,
}

struct LiteralInputState {
    end: usize,
    options: Arc<Config>,
    index: tift::literal_expansion::LiteralExpansionIndex,
}

impl DeclarationMemo {
    fn neutral(&self, source: &str) -> bool {
        (self.safe || self.independent_append_safe || self.reader_settled)
            && self.compiled.as_ref().is_some_and(|compiled| {
                syntax::declaration_neutral_segment_with_syntax(source, &self.options, compiled)
            })
    }

    fn whitespace(&self, source: &str, follows_newline: bool) -> bool {
        // With the standard grammar, whitespace after a newline adds no
        // operand, declaration or scope token. The prefix can remain unsafe
        // for subsequent content while retaining its current inferred options.
        follows_newline
            && source.bytes().all(|byte| byte.is_ascii_whitespace())
            && self.compiled.as_ref().is_some_and(|compiled| {
                syntax::declaration_neutral_segment_with_syntax(source, &self.options, compiled)
            })
    }
}

struct Input {
    name: String,
    span: Range<usize>,
}

impl Input {
    fn plain_file(&self, source: &str, command: &str) -> bool {
        if !matches!(command, "input" | "include")
            || !self
                .name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_./".contains(&byte))
        {
            return false;
        }
        let operand = source[self.span.start + 1 + command.len()..self.span.end].trim_start();
        operand
            .strip_prefix('{')
            .and_then(|value| value.strip_suffix('}'))
            == Some(&self.name)
            || command == "input" && operand == self.name
    }
}

struct ParsedInputs {
    inputs: Vec<Input>,
    document_start: Option<usize>,
    protected_end: usize,
}

type ChildInput = (PathBuf, Result<Arc<ImportSource>, String>);

struct PendingLoader {
    children: std::vec::IntoIter<ChildInput>,
    before: Option<Arc<DeclarationSnapshot>>,
}

struct InputFrame {
    path: PathBuf,
    file: Arc<ImportSource>,
    capture: bool,
    cursor: usize,
    next: usize,
    loaders: LoaderRun,
    pending: Option<PendingLoader>,
    entry: Arc<DeclarationSnapshot>,
    imports: Vec<LoaderChange>,
    before_separator: bool,
}

#[derive(Default)]
struct LoaderRun {
    options: Option<Arc<Config>>,
    end: usize,
    verified: bool,
    document_start: Option<usize>,
    ahead: bool,
}

impl LoaderRun {
    fn supports_batch(options: &Config) -> bool {
        options.syntax.special.is_empty()
            && options.syntax.command_pattern.is_none()
            && options.syntax.environment_pattern.is_none()
            && options.syntax.condition_pattern.is_none()
            && options.syntax.item_pattern.is_none()
            && options.syntax.argument_between.is_none()
    }

    fn verify(candidates: &[Input], parsed: ParsedInputs) -> Option<Option<usize>> {
        (parsed.protected_end <= candidates.first()?.span.start
            && parsed
                .inputs
                .iter()
                .rev()
                .take(candidates.len())
                .map(|item| &item.span)
                .eq(candidates.iter().rev().map(|item| &item.span)))
        .then_some(parsed.document_start)
    }

    fn same_options(&self, options: &Arc<Config>) -> bool {
        self.options
            .as_ref()
            .is_some_and(|prior| Arc::ptr_eq(prior, options))
    }

    fn snapshot_before_next(
        &mut self,
        file: &ImportSource,
        next: usize,
        graph: &mut InputGraph,
        config: &Config,
    ) -> Arc<DeclarationSnapshot> {
        let Some(memo) = &graph.last_options else {
            return graph.declaration_snapshot(config);
        };
        if next >= self.end
            || memo.revision != graph.revision
            || self.same_options(&memo.options)
            || graph.declarations.is_some()
            || graph
                .pending_snapshot
                .as_ref()
                .is_some_and(|(end, _)| *end == graph.source.len())
            || !Self::supports_batch(&memo.options)
            || rayon::current_thread_index().is_none()
            || rayon::current_num_threads() < 2
        {
            return graph.declaration_snapshot(config);
        }
        let Some(compiled) = memo.compiled.clone() else {
            return graph.declaration_snapshot(config);
        };
        let options = Arc::clone(&memo.options);
        let candidates = &file.candidates[next..self.end];
        let prefix = &file.text[..file.candidates[self.end - 1].span.end];
        let (snapshot, verified) = rayon::join(
            || graph.declaration_snapshot(config),
            || {
                Self::verify(
                    candidates,
                    literal_inputs_with_syntax(prefix, &options, &compiled).ok()?,
                )
            },
        );
        if let Some(document_start) = verified {
            self.options = Some(options);
            self.document_start = document_start;
            self.verified = true;
            self.ahead = true;
        }
        snapshot
    }

    fn locate(
        &mut self,
        graph: &mut InputGraph,
        file: &ImportSource,
        index: usize,
        options: &Arc<Config>,
    ) -> Option<Option<usize>> {
        let input = &file.candidates[index];
        let source = file.text.as_str();
        let cached = index < self.end && self.same_options(options);
        if !cached {
            self.ahead = false;
            let mut end = index;
            let mut cursor = input.span.start;
            for candidate in &file.candidates[index..] {
                if candidate.span.start < cursor
                    || !source[cursor..candidate.span.start]
                        .split_inclusive('\n')
                        .all(|line| {
                            let (text, finished) = line
                                .strip_suffix('\n')
                                .map_or((line, false), |text| (text, true));
                            let text = text.trim_start_matches(|ch: char| ch.is_ascii_whitespace());
                            text.is_empty()
                                || finished
                                    && text.starts_with('%')
                                    && !text.contains("tift:")
                                    && !text.contains("tex-fmt:")
                        })
                    || !candidate
                        .plain_file(source, syntax::command_at(source, candidate.span.start)?.0)
                {
                    break;
                }
                cursor = candidate.span.end;
                end += 1;
            }
            self.end = end;
            self.options = Some(Arc::clone(options));
            self.verified = false;
            if end > index + 1
                && Self::supports_batch(options)
                && let Ok(parsed) = graph.literal_inputs(&source[..cursor])
                && let Some(document_start) = Self::verify(&file.candidates[index..end], parsed)
            {
                self.verified = true;
                self.document_start = document_start;
            }
        }
        if self.verified {
            if std::mem::take(&mut self.ahead)
                && let Some(metrics) = &mut graph.metrics
            {
                metrics.loader_ahead_hits += 1;
            }
            return Some(self.document_start);
        }
        let parsed = graph.literal_inputs(&source[..input.span.end]).ok()?;
        parsed
            .inputs
            .iter()
            .any(|candidate| candidate.span == input.span)
            .then_some(parsed.document_start)
    }
}

pub(super) struct ProjectInputs {
    root: Option<String>,
    candidates: Vec<Input>,
}

impl ProjectInputs {
    /// Scan immutable source independently of the shared dependency graph.
    pub(super) fn scan(
        path: &Path,
        source: &str,
        config: &CompiledConfig,
    ) -> Result<Option<Self>, String> {
        if config.config.replacement_only
            || matches!(
                FileKind::from_path(path),
                Some(FileKind::Bibtex | FileKind::Bst | FileKind::Dtx | FileKind::Ins)
            )
        {
            return Ok(None);
        }
        let root =
            root_marker(source, config).map_err(|error| format!("{}: {error}", path.display()))?;
        let candidates = if root.is_some() || input_candidates(source).next().is_none() {
            Vec::new()
        } else {
            literal_inputs_with_syntax(source, &config.config, config.compiled_syntax())
                .map(|parsed| parsed.inputs)
                .unwrap_or_else(|_| input_candidates(source).collect())
        };
        Ok(Some(Self { root, candidates }))
    }
}

type DeclarationSnapshot = tift::source_model::ProjectExports;

impl InputGraph {
    fn checkpoint(&mut self, kind: u8, suffix: &str, config: &Config) -> Option<String> {
        if self.source.len() < LARGE_PROJECT_PREFIX {
            return None;
        }
        if let Some(root) = self.cache_root.take() {
            self.cache = ProjectCache::open(&root, config);
        }
        let cache = self.cache.as_mut()?;
        let full = self.full_hash.of(&self.source);
        if kind == 0 {
            return Some(cache.key(kind, &[full]));
        }
        let inferred = self
            .source
            .compact
            .as_ref()
            .map_or(full, |source| self.inference_hash.of(source));
        Some(cache.key(
            kind,
            &[
                full,
                inferred,
                self.index_history.finalize(),
                blake3::hash(suffix.as_bytes()),
            ],
        ))
    }

    fn restore_index(&mut self, config: &Config) {
        for end in std::mem::take(&mut self.deferred_scans) {
            let started = self.metrics.as_ref().map(|_| Instant::now());
            let _ = syntax::infer_declaration_config_indexed(
                &self.source.inference()[..end],
                config,
                &mut self.declaration_index,
            );
            let work = self.declaration_index.scan_work();
            let protection_bytes = self.declaration_index.protection_scan_bytes();
            self.record_declaration_work(
                end,
                started.map(|start| start.elapsed()),
                protection_bytes,
                work,
            );
        }
    }

    fn declaration_snapshot(&mut self, config: &Config) -> Arc<DeclarationSnapshot> {
        if let Some(snapshot) = &self.declarations {
            return Arc::clone(snapshot);
        }
        if let Some((end, snapshot)) = &self.pending_snapshot
            && *end == self.source.len()
        {
            return Arc::clone(snapshot);
        }
        let key = self.checkpoint(0, "", config);
        let cached = key.as_ref().and_then(|key| self.cache.as_ref()?.get(key));
        if cached.is_some()
            && let Some(metrics) = &mut self.metrics
        {
            metrics.export_cache_hits += 1;
        }
        let (snapshot, boundary) = if let Some(state) = cached {
            state
        } else {
            let started = self.metrics.as_ref().map(|_| Instant::now());
            let exports = match self.inferred_exports.take() {
                Some((end, exports)) if end == self.source.len() => exports,
                _ => {
                    let protection = if self.source.compact.is_none() {
                        self.declaration_index.protected_prefix(&self.source)
                    } else {
                        None
                    };
                    let ahead = protection
                        .is_none()
                        .then(|| self.protection_run.as_ref()?.at(&self.source))
                        .flatten();
                    let protection = protection.or(ahead);
                    let exports = self.export_index.snapshot(&self.source, config, protection);
                    if let (Some(metrics), Some(started)) = (&mut self.metrics, started) {
                        metrics.export_calls += 1;
                        metrics.export_ahead_hits += usize::from(ahead.is_some());
                        metrics.export_input_bytes += self.source.len();
                        metrics.export_lexed_bytes += self.export_index.last_lexed_bytes();
                        if protection.is_none() {
                            metrics.export_protection_input_bytes += self.source.len();
                        }
                        metrics.export_elapsed += started.elapsed();
                        eprintln!(
                            "trace: project-export: end={}, raw-read={}, protection={}, elapsed={:.3} ms",
                            self.source.len(),
                            self.export_index.last_lexed_bytes(),
                            if protection.is_some() {
                                "shared"
                            } else {
                                "full"
                            },
                            started.elapsed().as_secs_f64() * 1000.0
                        );
                    }
                    exports
                }
            };
            let state = (
                exports,
                syntax::project_export_boundary(&self.source[self.export_boundary_end..]),
            );
            if let Some(key) = key {
                self.cache.as_mut().unwrap().put(key, &state);
            }
            state
        };
        let snapshot = Arc::new(snapshot);
        if boundary {
            if self.source.ends_with('\n') {
                self.export_boundary_end = self.source.len();
            }
            self.pending_snapshot = None;
            self.declarations = Some(Arc::clone(&snapshot));
        } else {
            self.pending_snapshot = Some((self.source.len(), Arc::clone(&snapshot)));
        }
        snapshot
    }
    fn expand_input_name(
        &mut self,
        expression: &str,
        prefix: &str,
        base: &Config,
        options: &Arc<Config>,
    ) -> Option<String> {
        let original_length = self.source.len();
        self.source.full.push_str(prefix);
        let end = self.source.len();
        let previous = self.literal_inputs_index.take();
        let result = (|| {
            let index = if let Some(mut previous) = previous.filter(|previous| {
                previous.end <= end
                    && previous.options.syntax == options.syntax
                    && previous.options.protect == options.protect
            }) {
                let suffix = &self.source[previous.end..];
                let ranges = syntax::protected_ranges(suffix, options).ok();
                if ranges.as_ref().is_some_and(|ranges| {
                    previous
                        .index
                        .append_segment(suffix, base, ranges, previous.end)
                }) {
                    previous.index
                } else {
                    let ranges = syntax::protected_ranges(&self.source, options).ok()?;
                    tift::literal_expansion::LiteralExpansionIndex::build(
                        &self.source,
                        base,
                        &ranges,
                    )
                }
            } else {
                let ranges = syntax::protected_ranges(&self.source, options).ok()?;
                tift::literal_expansion::LiteralExpansionIndex::build(&self.source, base, &ranges)
            };
            let expanded = index.expand(expression, end);
            self.literal_inputs_index = Some(LiteralInputState {
                end,
                options: Arc::clone(options),
                index,
            });
            expanded
        })();
        self.source.full.truncate(original_length);
        result.filter(|name| !name.is_empty() && !name.contains(['\\', '{', '}', '%', '#']))
    }

    fn append(&mut self, source: &str) {
        if let Some(metrics) = &mut self.metrics {
            metrics.append_calls += 1;
            metrics.appended_bytes += source.len();
        }
        self.record_binding_targets(source);
        self.source.append(source);
        self.revision = self.revision.wrapping_add(1);
    }

    fn record_declaration_work(
        &mut self,
        input_bytes: usize,
        elapsed: Option<Duration>,
        protection_scan_bytes: usize,
        (scan_bytes, incremental): (usize, bool),
    ) {
        let (Some(metrics), Some(elapsed)) = (&mut self.metrics, elapsed) else {
            return;
        };
        metrics.declaration_calls += 1;
        metrics.declaration_input_bytes += input_bytes;
        metrics.declaration_scan_bytes += scan_bytes;
        metrics.protection_scan_bytes += protection_scan_bytes;
        metrics.declaration_incremental_calls += usize::from(incremental);
        metrics.declaration_full_scans += usize::from(!incremental && scan_bytes > 0);
        metrics.declaration_elapsed += elapsed;
        eprintln!(
            "trace: project-declarations: end={input_bytes}, reader-start={}, protection-bytes={protection_scan_bytes}, elapsed={:.3} ms",
            input_bytes - scan_bytes.min(input_bytes),
            elapsed.as_secs_f64() * 1000.0
        );
    }

    fn append_segment(&mut self, source: &str, neutral: bool) {
        if self.declarations.is_none() {
            self.append(source);
            return;
        }
        let whitespace = source.bytes().all(|byte| byte.is_ascii_whitespace());
        let stable = neutral && (whitespace || self.compaction_preserves_bindings(source));
        if !source.is_empty() && !stable {
            self.declarations = None;
        }
        if stable && source.len() >= 256 {
            if let Some(metrics) = &mut self.metrics {
                metrics.append_calls += 1;
                metrics.appended_bytes += source.len();
            }
            self.source.append_compacted(source);
            self.revision = self.revision.wrapping_add(1);
        } else {
            self.append(source);
        }
    }

    fn record_binding_targets(&mut self, source: &str) {
        for (start, _) in source.match_indices('\\') {
            let Some((name, end)) = syntax::command_at(source, start) else {
                continue;
            };
            let name = name.trim_end_matches('*');
            if !scalar_binding_command(name)
                && !matches!(
                    name,
                    "def"
                        | "gdef"
                        | "edef"
                        | "xdef"
                        | "let"
                        | "futurelet"
                        | "newcommand"
                        | "renewcommand"
                        | "providecommand"
                        | "NewDocumentCommand"
                        | "RenewDocumentCommand"
                        | "ProvideDocumentCommand"
                        | "DeclareDocumentCommand"
                        | "DeclareRobustCommand"
                        | "DeclareMathOperator"
                        | "newenvironment"
                        | "renewenvironment"
                        | "NewDocumentEnvironment"
                        | "RenewDocumentEnvironment"
                        | "ProvideDocumentEnvironment"
                        | "DeclareDocumentEnvironment"
                )
            {
                continue;
            }
            let mut target = trivia_end(source, end);
            if source.as_bytes().get(target) == Some(&b'{') {
                target = trivia_end(source, target + 1);
                if name.ends_with("environment") || name.ends_with("Environment") {
                    if let Some(end) = source[target..].find('}') {
                        let name = &source[target..target + end];
                        if !name.is_empty() && !name.contains(['\\', '{', '%']) {
                            self.binding_targets.insert(name.to_owned());
                        }
                    }
                    continue;
                }
            }
            if let Some((target, _)) = syntax::command_at(source, target) {
                self.binding_targets.insert(target.to_owned());
                if name == "newif"
                    && let Some(flag) = target.strip_prefix("if")
                {
                    self.binding_targets.insert(format!("{flag}true"));
                    self.binding_targets.insert(format!("{flag}false"));
                }
            }
        }
    }

    fn compaction_preserves_bindings(&self, source: &str) -> bool {
        let Some(memo) = &self.last_options else {
            return false;
        };
        if source.contains("^^") {
            return false;
        }
        for (start, _) in source.match_indices('\\') {
            let Some((name, end)) = syntax::command_at(source, start) else {
                return false;
            };
            let name = name.trim_end_matches('*');
            if self.binding_targets.contains(name)
                || memo.options.syntax.commands.contains_key(name)
                || memo.options.syntax.environment_aliases.contains_key(name)
                || self
                    .declarations
                    .as_ref()
                    .is_some_and(|state| state.3.contains_key(name))
                || syntax::project_export_reader(name)
                || scalar_binding_command(name)
                || matches!(
                    name,
                    "count"
                        | "dimen"
                        | "skip"
                        | "muskip"
                        | "toks"
                        | "globaldefs"
                        | "advance"
                        | "multiply"
                        | "divide"
                )
                || matches!(
                    name,
                    "expandafter"
                        | "csdef"
                        | "csgdef"
                        | "csedef"
                        | "csxdef"
                        | "cslet"
                        | "undef"
                        | "csundef"
                )
            {
                return false;
            }
            if matches!(name, "begin" | "end") {
                let argument = trivia_end(source, end);
                let Some(finish) = syntax::balanced_end(source, argument, b'{', b'}') else {
                    return false;
                };
                let environment = source[argument + 1..finish - 1].trim();
                if self.binding_targets.contains(environment)
                    || self
                        .declarations
                        .as_ref()
                        .is_some_and(|state| state.1.contains_key(environment))
                    || memo
                        .options
                        .syntax
                        .environment_arguments
                        .contains_key(environment)
                {
                    return false;
                }
            }
        }
        true
    }

    fn append_line(&mut self, source: &str) {
        let neutral = self
            .last_options
            .as_ref()
            .is_some_and(|memo| memo.revision == self.revision && memo.neutral(source));
        let reusable = neutral
            || self.last_options.as_ref().is_some_and(|memo| {
                memo.revision == self.revision
                    && memo.whitespace(source, self.source.ends_with('\n'))
            });
        self.append_segment(source, reusable);
        self.append("\n");
        if reusable {
            self.last_options
                .as_mut()
                .expect("reusable options")
                .revision = self.revision;
        }
    }

    fn append_prepared_input(&mut self, input: &str, reusable: bool, preserves_exports: bool) {
        self.append(input);
        self.append("\n");
        if !preserves_exports {
            self.declarations = None;
        }
        if reusable {
            self.last_options
                .as_mut()
                .expect("reusable options")
                .revision = self.revision;
        }
    }

    fn infer(&mut self, path: &Path, config: &Config) -> Result<Arc<Config>, String> {
        self.infer_boundary(path, config, DeclarationBoundary::Current)
    }

    fn infer_boundary(
        &mut self,
        path: &Path,
        config: &Config,
        location: DeclarationBoundary,
    ) -> Result<Arc<Config>, String> {
        if let Some(memo) = &self.last_options
            && memo.revision == self.revision
        {
            return Ok(Arc::clone(&memo.options));
        }

        let key_kind = match (location, self.boundary_policy_refused) {
            (DeclarationBoundary::RootEnd, false) => 4,
            (DeclarationBoundary::RootEnd, true) => 5,
            (_, false) => 1,
            (_, true) => 2,
        };
        let key = self.checkpoint(key_kind, "", config);
        if self.cache.is_some() || self.cache_root.is_some() {
            self.index_history
                .update(&(self.source.inference().len() as u64).to_le_bytes());
        }
        let cached: Option<(InferredSyntax, bool, bool, bool, bool)> =
            key.as_ref().and_then(|key| self.cache.as_ref()?.get(key));
        if cached.is_some()
            && let Some(metrics) = &mut self.metrics
        {
            metrics.declaration_cache_hits += 1;
        }
        let (inferred, independent_append_safe, safe, refused, reader_settled) =
            if let Some((options, append, safe, refused, reader)) = cached {
                self.deferred_scans.push(self.source.inference().len());
                (options.apply(config), append, safe, refused, reader)
            } else {
                self.restore_index(config);
                let started = self.metrics.as_ref().map(|_| Instant::now());
                let mut reader_elapsed = None;
                let result = if location == DeclarationBoundary::RootEnd {
                    self.declaration_index
                        .infer_without_exported_aliases(self.source.inference(), config)
                } else if location == DeclarationBoundary::BeforeSeparator
                    && self.declarations.is_none()
                    && self.source.len() >= LARGE_PROJECT_PREFIX
                    && rayon::current_thread_index().is_some()
                    && rayon::current_num_threads() > 1
                    && !self
                        .cache
                        .as_ref()
                        .is_some_and(ProjectCache::has_saved_entries)
                {
                    let end = self.source.len();
                    self.source.full.push('\n');
                    let inferred_source = self
                        .source
                        .compact
                        .as_deref()
                        .unwrap_or(&self.source[..end]);
                    let (result, (exports, export_elapsed, ahead)) = rayon::join(
                        || {
                            let result = self.declaration_index.infer_with_alias_index(
                                inferred_source,
                                config,
                                &mut self.alias_index,
                            );
                            reader_elapsed = started.map(|start| start.elapsed());
                            result
                        },
                        || {
                            let started = self.metrics.as_ref().map(|_| Instant::now());
                            let protection = self
                                .protection_run
                                .as_ref()
                                .and_then(|run| run.at(&self.source));
                            let exports =
                                self.export_index.snapshot(&self.source, config, protection);
                            (
                                exports,
                                started.map(|start| start.elapsed()),
                                protection.is_some(),
                            )
                        },
                    );
                    self.inferred_exports = Some((self.source.len(), exports));
                    if let (Some(metrics), Some(elapsed)) = (&mut self.metrics, export_elapsed) {
                        metrics.export_calls += 1;
                        metrics.export_input_bytes += self.source.len();
                        metrics.export_ahead_hits += usize::from(ahead);
                        metrics.export_protection_input_bytes +=
                            usize::from(!ahead) * self.source.len();
                        metrics.export_lexed_bytes += self.export_index.last_lexed_bytes();
                        metrics.export_elapsed += elapsed;
                    }
                    self.source.full.truncate(end);
                    result
                } else if self.source.compact.is_none() {
                    let mut exports = None;
                    let result = self.declaration_index.infer_with_export_index(
                        &self.source,
                        config,
                        &mut self.export_index,
                        &mut exports,
                    );
                    if let Some(exports) = exports {
                        self.inferred_exports = Some((self.source.len(), exports));
                        if let Some(metrics) = &mut self.metrics {
                            metrics.export_lexed_bytes += self.export_index.last_lexed_bytes();
                        }
                    }
                    result
                } else {
                    syntax::infer_declaration_config_indexed(
                        self.source.inference(),
                        config,
                        &mut self.declaration_index,
                    )
                };
                let work = self.declaration_index.scan_work();
                let protection_bytes = self.declaration_index.protection_scan_bytes();
                self.record_declaration_work(
                    self.source.inference().len(),
                    reader_elapsed.or_else(|| started.map(|start| start.elapsed())),
                    protection_bytes,
                    work,
                );
                // Re-run failures against the full source so error positions and the
                // established diagnostic remain exact after neutral text compaction.
                let (inferred, independent_append_safe, inferred_boundary) = result
                    .or_else(|error| {
                        if self.source.compact.is_some() {
                            syntax::infer_declaration_config_with_reuse(&self.source, config)
                        } else {
                            Err(error)
                        }
                    })
                    .map_err(|error| {
                        format!("{}: project declarations: {error}", path.display())
                    })?;

                let boundary = if self.boundary_policy_refused {
                    syntax::DeclarationReuseBoundary::PolicyRefused
                } else if self.source.compact.is_none() {
                    inferred_boundary
                } else {
                    syntax::declaration_reuse_boundary_status(&self.source, config)
                };

                let refused = boundary == syntax::DeclarationReuseBoundary::PolicyRefused;
                let safe = boundary == syntax::DeclarationReuseBoundary::Safe;
                let reader = self.declaration_index.reader_settled();
                if let Some(key) = key {
                    self.cache.as_mut().unwrap().put(
                        key,
                        &(
                            InferredSyntax::from_config(&inferred),
                            independent_append_safe,
                            safe,
                            refused,
                            reader,
                        ),
                    );
                }
                (inferred, independent_append_safe, safe, refused, reader)
            };
        self.boundary_policy_refused |= refused;
        let options = self
            .last_options
            .as_ref()
            .filter(|memo| {
                memo.options.syntax == inferred.syntax && memo.options.protect == inferred.protect
            })
            .map_or_else(|| Arc::new(inferred), |memo| Arc::clone(&memo.options));
        let compiled = self
            .last_options
            .as_ref()
            .filter(|memo| memo.options.syntax == options.syntax)
            .and_then(|memo| memo.compiled.as_ref().map(Arc::clone))
            .or_else(|| syntax::compile(&options.syntax).ok().map(Arc::new));
        self.last_options = Some(DeclarationMemo {
            revision: self.revision,
            options: Arc::clone(&options),
            safe,
            independent_append_safe,
            reader_settled,
            compiled,
        });
        Ok(options)
    }

    fn prepare_prefix(
        &mut self,
        suffix: &str,
        path: &Path,
        config: &Config,
    ) -> Result<(Arc<Config>, bool), String> {
        if let Some(metrics) = &mut self.metrics {
            metrics.prefix_calls += 1;
        }
        if let Some(memo) = &self.last_options
            && memo.revision == self.revision
        {
            let neutral = memo.neutral(suffix);
            if suffix.is_empty() || neutral || memo.whitespace(suffix, self.source.ends_with('\n'))
            {
                return Ok((Arc::clone(&memo.options), neutral));
            }
        }
        let preview_started = self.metrics.as_ref().map(|_| Instant::now());
        if let Some(metrics) = &mut self.metrics {
            metrics.prefix_preview_calls += 1;
            metrics.prefix_input_bytes += suffix.len();
        }
        // Preview the next input's prefix without copying the cumulative graph.
        // The memo is untouched, and truncation restores its immutable revision.
        let key = self.checkpoint(3, suffix, config);
        if let Some(options) = key
            .as_ref()
            .and_then(|key| self.cache.as_ref()?.get::<InferredSyntax>(key))
        {
            if let (Some(started), Some(metrics)) = (preview_started, &mut self.metrics) {
                metrics.prefix_elapsed += started.elapsed();
            }
            return Ok((Arc::new(options.apply(config)), false));
        }
        self.restore_index(config);
        let length = self.source.inference().len();
        self.source.inference_mut().push_str(suffix);

        let mut preview_index = self.declaration_index.clone();
        let inferred = syntax::infer_declaration_config_indexed(
            self.source.inference(),
            config,
            &mut preview_index,
        )
        .map(|(options, _, _)| options)
        .or_else(|error| {
            if self.source.compact.is_some() {
                let full_length = self.source.len();
                self.source.full.push_str(suffix);
                let result = syntax::infer_declaration_config(&self.source, config);
                self.source.full.truncate(full_length);
                result
            } else {
                Err(error)
            }
        });

        self.source.inference_mut().truncate(length);
        if let (Some(key), Ok(options)) = (key, &inferred) {
            self.cache
                .as_mut()
                .unwrap()
                .put(key, &InferredSyntax::from_config(options));
        }
        if let (Some(started), Some(metrics)) = (preview_started, &mut self.metrics) {
            metrics.prefix_elapsed += started.elapsed();
        }
        inferred
            .map(|options| (Arc::new(options), false))
            .map_err(|error| format!("{}: project declarations: {error}", path.display()))
    }

    fn literal_inputs(&self, source: &str) -> Result<ParsedInputs, String> {
        let memo = self.last_options.as_ref().expect("input options inferred");
        debug_assert_eq!(memo.revision, self.revision);

        if let Some(compiled) = &memo.compiled {
            literal_inputs_with_syntax(source, &memo.options, compiled)
        } else {
            literal_inputs(source, &memo.options)
        }
    }
}

fn scalar_binding_command(name: &str) -> bool {
    matches!(
        name,
        "newif"
            | "chardef"
            | "mathchardef"
            | "Umathchardef"
            | "countdef"
            | "dimendef"
            | "skipdef"
            | "muskipdef"
            | "toksdef"
            | "attributedef"
            | "integerdef"
            | "newcount"
            | "newdimen"
            | "newskip"
            | "newmuskip"
            | "newtoks"
            | "newbox"
            | "newread"
            | "newwrite"
            | "newfam"
            | "newinsert"
            | "newmarks"
    )
}

impl ProjectContext {
    fn normalized_path(&mut self, path: &Path) -> Result<PathBuf, String> {
        let cwd = if path.is_absolute() {
            PathBuf::new()
        } else {
            std::env::current_dir().map_err(|error| error.to_string())?
        };
        Ok(self.normalized_path_from(path, &cwd))
    }

    fn normalized_path_from(&mut self, path: &Path, cwd: &Path) -> PathBuf {
        let key = (path.to_path_buf(), cwd.to_path_buf());
        if let Some(cached) = self.normalized_paths.get(&key) {
            return cached.clone();
        }
        // Keep parent components until canonicalization so symlink/../file
        // requests preserve their filesystem meaning.
        let requested = if path.is_absolute() {
            path.to_path_buf()
        } else {
            cwd.join(path)
        };

        let resolved = absolute(
            &fs::canonicalize(&requested).unwrap_or_else(|_| requested.clone()),
            cwd,
        );
        self.normalized_paths.insert(key, resolved.clone());
        self.normalized_paths
            .entry((resolved.clone(), PathBuf::new()))
            .or_insert_with(|| resolved.clone());
        resolved
    }

    fn read_normalized_source(&mut self, path: &Path) -> Result<Arc<String>, String> {
        self.sources
            .read(path)
            .map(|source| Arc::clone(&source.text))
    }

    fn has_inputs(
        &mut self,
        path: &Path,
        root: &Path,
        source: &str,
        config: &CompiledConfig,
        inputs: &ProjectInputs,
    ) -> bool {
        inputs.candidates.iter().any(|input| {
            let loader = syntax::command_at(source, input.span.start)
                .expect("input candidate")
                .0;
            !config.config.syntax.commands.contains_key(loader)
                && !ignored_resource_input(input, loader, &config.config.syntax)
                && (matches!(loader, "input" | "include")
                    || input
                        .name
                        .split(',')
                        .any(|name| self.input_path(name, path, root).ok().flatten().is_some()))
        })
    }

    fn resolve_root(
        &mut self,
        path: &Path,
        marker: Option<&str>,
        config: &CompiledConfig,
    ) -> Result<PathBuf, String> {
        let mut current = path.to_path_buf();
        let mut marker = marker.map(str::to_owned);
        let mut visited = HashSet::from([current.clone()]);
        while let Some(name) = marker {
            let parent = current.parent().expect("an absolute file has a parent");
            let candidate = self.normalized_path(&parent.join(name))?;
            if candidate == current {
                break;
            }
            if !visited.insert(candidate.clone()) {
                return Err(format!(
                    "{}: TEX root comment cycle at {}",
                    path.display(),
                    candidate.display()
                ));
            }
            let source = self.read_normalized_source(&candidate).map_err(|error| {
                format!("{}: cannot read explicit TEX root: {error}", path.display())
            })?;
            current = candidate;
            marker = root_marker(&source, config)
                .map_err(|error| format!("{}: {error}", current.display()))?;
        }
        Ok(current)
    }

    fn expand_inputs(
        &mut self,
        root: &Path,
        input_source: Arc<ImportSource>,
        config: &Config,
        graph: &mut InputGraph,
        schedule: Option<&dyn Fn(Arc<ProtectionRun>)>,
        ready: &mut dyn FnMut(&Path, &FileState),
    ) -> Result<(), String> {
        let mut active = HashSet::from([root.to_path_buf()]);
        let mut stack = vec![InputFrame::new(
            root.to_path_buf(),
            input_source,
            false,
            config,
            graph,
        )];
        while let Some(frame) = stack.last_mut() {
            if let Some(pending) = &mut frame.pending {
                if let Some((child, input)) = pending.children.next() {
                    let before_separator = frame.capture && pending.children.len() == 0;
                    active.insert(child.clone());
                    stack.push(InputFrame::new(
                        child,
                        input?,
                        before_separator,
                        config,
                        graph,
                    ));
                } else {
                    let before = frame.pending.take().unwrap().before;
                    frame.finish_loader(before, config, graph);
                }
            } else if let Some(pending) =
                self.next_loader(frame, root, config, &active, graph, schedule)?
            {
                frame.pending = Some(pending);
            } else {
                let frame = stack.pop().unwrap();
                active.remove(&frame.path);
                frame.finish(root, config, graph, ready)?;
            }
        }
        Ok(())
    }
}

impl InputFrame {
    fn new(
        path: PathBuf,
        file: Arc<ImportSource>,
        before_separator: bool,
        config: &Config,
        graph: &mut InputGraph,
    ) -> Self {
        graph.trace_enter();
        let capture = graph.selected.contains(&path) && !graph.files.contains_key(&path);
        let entry = if capture || graph.source.len() < LARGE_PROJECT_PREFIX {
            graph.declaration_snapshot(config)
        } else {
            Arc::default()
        };
        Self {
            path,
            file,
            capture,
            cursor: 0,
            next: 0,
            loaders: LoaderRun::default(),
            pending: None,
            entry,
            imports: Vec::new(),
            before_separator,
        }
    }
}

impl ProjectContext {
    fn next_loader(
        &mut self,
        frame: &mut InputFrame,
        root: &Path,
        config: &Config,
        active: &HashSet<PathBuf>,
        graph: &mut InputGraph,
        schedule: Option<&dyn Fn(Arc<ProtectionRun>)>,
    ) -> Result<Option<PendingLoader>, String> {
        let path = frame.path.as_path();
        let input_source = &frame.file;
        let source = input_source.text.as_str();
        let cursor = frame.cursor;
        while let Some(input) = input_source.candidates.get(frame.next) {
            let index = frame.next;
            frame.next += 1;
            let command = syntax::command_at(source, input.span.start)
                .expect("input command")
                .0;
            if config.syntax.commands.contains_key(command)
                || ignored_resource_input(input, command, &config.syntax)
            {
                continue;
            }
            // A literal loader without a local target contributes only its
            // original source text. Leave it in the pending suffix instead of
            // creating a declaration checkpoint and scanning the cumulative
            // graph twice. Dynamic filenames still need prefix inference.

            let dynamic = input.name.contains('\\');
            let literal_paths = if dynamic {
                Vec::new()
            } else {
                let paths = self.loader_paths(&input.name, command, path, root, &config.syntax)?;
                if paths.is_empty() {
                    continue;
                }
                paths
            };
            // Read only as far as this candidate. Later text can use raw
            // environments whose definitions have not been imported yet.
            let options = graph.infer(path, config)?;
            let plain = input.plain_file(source, command);
            let Some(document_start) = frame.loaders.locate(graph, input_source, index, &options)
            else {
                continue;
            };
            let (inferred, reusable_prefix) =
                graph.prepare_prefix(&source[cursor..input.span.start], path, config)?;
            if inferred.syntax.commands.contains_key(command) {
                continue;
            }
            if let Some(schedule) = schedule
                && path == root
                && frame.capture
                && frame.loaders.verified
                && frame.loaders.end >= index + 3
                && graph.protection_run.is_none()
                && graph.source.len() >= LARGE_PROJECT_PREFIX
                && !graph
                    .cache
                    .as_ref()
                    .is_some_and(ProjectCache::has_saved_entries)
                && let Some(run) = ProtectionRun::from_cached_leaves(
                    &graph.source,
                    input_source,
                    cursor,
                    &input_source.candidates[index..frame.loaders.end],
                    path,
                    &self.sources,
                    config,
                )
            {
                schedule(Arc::clone(&run));
                graph.protection_run = Some(run);
            }
            let children = if dynamic {
                let name = graph.expand_input_name(
                    &input.name,
                    &source[cursor..input.span.start],
                    config,
                    &inferred,
                );
                self.loader_paths(
                    name.as_deref().unwrap_or(""),
                    command,
                    path,
                    root,
                    &config.syntax,
                )?
            } else {
                literal_paths
            };
            if path == root && graph.preamble_end.is_none() {
                graph.preamble_end =
                    document_start.map(|start| graph.source.len() + start.saturating_sub(cursor));
            }
            graph.append_segment(&source[cursor..input.span.start], reusable_prefix);
            let before = if frame.capture || graph.source.len() < LARGE_PROJECT_PREFIX {
                Some(graph.declaration_snapshot(config))
            } else {
                graph.declarations.clone()
            };
            graph.append_prepared_input(
                &source[input.span.clone()],
                reusable_prefix,
                reusable_prefix
                    && plain
                    && (command == "include" || syntax::input_package(&input.name).is_none())
                    && before
                        .as_ref()
                        .is_some_and(|snapshot| !snapshot.3.contains_key(command)),
            );
            let ready = if active.len() < MAX_INPUT_DEPTH {
                let children: Vec<_> = children
                    .into_iter()
                    .filter(|child| !active.contains(child))
                    .collect();
                let codec = preceding_xetex_encoding(&source[..input.span.start]);
                // A multi-file loader exposes all child inputs at once. Only
                // the immutable reads and lexical scans can run independently.
                let read = |child: PathBuf| {
                    let input = self.sources.import(&child, codec).or_else(|error| {
                        if codec.is_none()
                            && let Some(label) = super::source_encoding::inputenc_at_loader(
                                &source[..input.span.start],
                            )
                        {
                            self.sources.import(&child, Some(label))
                        } else {
                            Err(error)
                        }
                    });
                    (child, input)
                };
                if children.len() > 1 && rayon::current_thread_index().is_some() {
                    children.into_par_iter().map(read).collect()
                } else {
                    children.into_iter().map(read).collect()
                }
            } else {
                Vec::new()
            };
            return Ok(Some(PendingLoader {
                children: ready.into_iter(),
                before,
            }));
        }
        Ok(None)
    }
}

impl InputFrame {
    fn finish_loader(
        &mut self,
        before: Option<Arc<DeclarationSnapshot>>,
        config: &Config,
        graph: &mut InputGraph,
    ) {
        let input = &self.file.candidates[self.next - 1];
        graph.append_line("");
        self.cursor = input.span.end;
        if !self.capture && graph.source.len() >= LARGE_PROJECT_PREFIX {
            return;
        }
        let Some(before) = before else {
            return;
        };
        let after = self
            .loaders
            .snapshot_before_next(&self.file, self.next, graph, config);
        if !self.capture || Arc::ptr_eq(&before, &after) {
            return;
        }
        let (before_aliases, before_arguments, before_packages, before_commands) = before.as_ref();
        let (after_aliases, after_arguments, after_packages, after_commands) = after.as_ref();
        let commands = after_commands
            .iter()
            .filter(|(name, command)| before_commands.get(*name) != Some(command))
            .map(|(name, command)| (name.clone(), command.clone()))
            .collect();
        let aliases = changed_bindings(before_aliases, after_aliases);
        let arguments = changed_bindings(before_arguments, after_arguments);
        let packages = after_packages
            .iter()
            .filter(|(name, state)| before_packages.get(*name) != Some(state))
            .map(|(name, state)| (name.clone(), *state))
            .collect();
        self.imports.push(LoaderChange {
            at: input.span.end,
            loader: self.file.loader_at(input.span.start).cloned(),
            delta: (aliases, arguments, packages, commands),
        });
    }

    fn finish(
        self,
        root: &Path,
        config: &Config,
        graph: &mut InputGraph,
        ready: &mut dyn FnMut(&Path, &FileState),
    ) -> Result<(), String> {
        let path = self.path.as_path();
        let source = self.file.text.as_str();
        let cursor = self.cursor;
        if path == root && graph.preamble_end.is_none() && source.contains("document") {
            graph.infer(path, config)?;
            graph.preamble_end = graph
                .literal_inputs(source)
                .ok()
                .and_then(|parsed| parsed.document_start)
                .map(|start| graph.source.len() + start.saturating_sub(cursor));
        }
        graph.append_line(&source[cursor..]);
        if !graph.files.contains_key(path) {
            // Retain the exact first-inclusion state. Incomplete file tails
            // keep lazy restoration; a later source may complete their reader.
            if self.capture || path != root {
                let location = if path == root {
                    DeclarationBoundary::RootEnd
                } else if self.before_separator {
                    DeclarationBoundary::BeforeSeparator
                } else {
                    DeclarationBoundary::Current
                };
                let _ = graph.infer_boundary(path, config, location);
            }
            // Retain only a memo that already describes this exact first
            // inclusion. Invalidated suffixes keep lazy full inference, so a
            // chapter can still complete declarations from preceding inputs.
            let memo = graph
                .last_options
                .as_ref()
                .filter(|memo| memo.revision == graph.revision);
            let options = memo.map(|memo| Arc::clone(&memo.options));
            let state = FileState {
                end: graph.source.len(),
                options,
                entry: self.entry,
                imports: self.imports,
            };
            if self.capture && memo.is_some_and(|memo| memo.reader_settled || memo.safe) {
                ready(path, &state);
            }
            graph.files.insert(path.to_owned(), state);
        }
        graph.trace_exit(path, source.len());
        Ok(())
    }
}

impl ProjectContext {
    fn loader_paths(
        &mut self,
        names: &str,
        command: &str,
        input: &Path,
        root: &Path,
        config: &SyntaxConfig,
    ) -> Result<Vec<PathBuf>, String> {
        let file = matches!(command, "input" | "include");
        // TeX reads a comma in an existing filename literally. When no such
        // file exists, retain per-member lookup for configured input lists.
        if file
            && names.contains(',')
            && let Some(path) = self.input_path(names, input, root)?
        {
            return Ok(if ignored_input_extension(names, config) {
                Vec::new()
            } else {
                vec![path]
            });
        }
        let mut targets = Vec::new();
        for name in names
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
        {
            if (!file || !ignored_input_extension(name, config))
                && let Some(path) = self.input_path(name, input, root)?
            {
                targets.push(path);
            }
        }
        Ok(targets)
    }

    fn input_path(
        &mut self,
        name: &str,
        input: &Path,
        root: &Path,
    ) -> Result<Option<PathBuf>, String> {
        // Standard input paths are normally relative to the main document's
        // working directory. A sibling fallback supports nested local settings.
        let root_directory = root.parent();
        let input_directory = input
            .parent()
            .filter(|directory| Some(*directory) != root_directory);
        for directory in [root_directory, input_directory].into_iter().flatten() {
            let candidate = directory.join(name);
            let tex_candidate = candidate
                .extension()
                .is_none()
                .then(|| candidate.with_extension("tex"));
            for candidate in tex_candidate.into_iter().chain(std::iter::once(candidate)) {
                // An explicitly supplied in-memory source is already part of
                // this invocation's snapshot, even if a prior probe missed it.
                let result = if self.sources.contains(&candidate) {
                    Some(candidate)
                } else if let Some(cached) = self.input_paths.get(&candidate) {
                    cached.clone().or_else(|| {
                        let path = self
                            .normalized_paths
                            .get(&(candidate.clone(), PathBuf::new()))
                            .cloned()
                            .unwrap_or_else(|| absolute(&candidate, Path::new("")));
                        self.sources.contains(&path).then_some(path)
                    })
                } else {
                    let path = self.normalized_path(&candidate)?;
                    let result = (self.sources.contains(&path) || path.is_file()).then_some(path);
                    self.input_paths.insert(candidate, result.clone());
                    result
                };
                if result.is_some() {
                    return Ok(result);
                }
            }
        }
        Ok(None)
    }

    fn sibling_lists(
        &self,
        directory: &Path,
        config: Arc<CompiledConfig>,
    ) -> Result<Arc<CompiledConfig>, String> {
        let paths: Vec<_> = WalkBuilder::new(directory)
            .hidden(false)
            .follow_links(false)
            .max_depth(Some(1))
            .sort_by_file_path(|a, b| a.cmp(b))
            .build()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry.file_type().is_some_and(|kind| kind.is_file())
                    && matches!(
                        FileKind::from_path(entry.path()),
                        Some(FileKind::Tex | FileKind::Tikz | FileKind::Style | FileKind::Class)
                    )
            })
            .map(|entry| entry.into_path())
            .collect();
        let declarations = |path: PathBuf| {
            let input = self.sources.read(&path).ok()?;
            let text = &input.text;
            ((text.contains("environment")
                || text.contains("DocumentEnvironment")
                || text.contains("\\newlist"))
                && [
                    "\\newenvironment",
                    "\\renewenvironment",
                    "\\NewDocumentEnvironment",
                    "\\RenewDocumentEnvironment",
                    "\\ProvideDocumentEnvironment",
                    "\\DeclareDocumentEnvironment",
                    "\\newlist",
                ]
                .iter()
                .any(|word| text.contains(word)))
            .then(|| syntax::ListEnvironmentDependencies::from_source(text, &config))
        };
        let dependencies: Vec<_> = if rayon::current_thread_index().is_some() && paths.len() > 1 {
            paths.into_par_iter().filter_map(declarations).collect()
        } else {
            paths.into_iter().filter_map(declarations).collect()
        };
        let mut known = config.config.syntax.list_environments.clone();
        loop {
            let before = known.len();
            for file in &dependencies {
                known.extend(file.resolve(&known));
            }
            if known.len() == before {
                break;
            }
        }
        if known.len() == config.config.syntax.list_environments.len() {
            return Ok(config);
        }
        let mut syntax = config.config.syntax.clone();
        syntax.list_environments = known;
        config
            .with_project_syntax(syntax, config.config.protect.clone())
            .map(Arc::new)
            .map_err(|error| error.to_string())
    }
}

impl PreparedProject {
    pub(super) fn input_graph(&self) -> Option<blake3::Hash> {
        self.graph.as_ref().and_then(|graph| graph.fingerprint)
    }

    /// Expand imports once in TeX execution order, then use the frozen
    /// declarations of each file independently.
    pub(super) fn for_source(&self, path: &Path, source: &str) -> Result<PreparedSource, String> {
        let started = self.trace.then(Instant::now);
        let result = self.prepare_source(path, source);
        if let Some(started) = started {
            eprintln!(
                "trace: {}: project-config: elapsed={:.3} ms",
                path.display(),
                started.elapsed().as_secs_f64() * 1000.0
            );
        }
        result
    }

    fn prepare_source(&self, path: &Path, source: &str) -> Result<PreparedSource, String> {
        let config = &self.base;
        let Some(graph) = &self.graph else {
            return Ok(PreparedSource {
                config: Arc::clone(config),
                input_graph: None,
            });
        };
        let file = graph.files.get(path);
        let (syntax, protect) = if let Some(options) = file.and_then(|file| file.options.as_ref()) {
            (options.syntax.clone(), options.protect.clone())
        } else if let Some(file) = file {
            let inference_end = graph.source.inference_boundary(file.end);
            let options = syntax::infer_declaration_config(
                &graph.source.inference()[..inference_end],
                &config.config,
            )
            .or_else(|error| {
                if inference_end != file.end {
                    syntax::infer_declaration_config(&graph.source[..file.end], &config.config)
                } else {
                    Err(error)
                }
            })
            .map_err(|error| format!("{}: project declarations: {error}", path.display()))?;
            (options.syntax, options.protect)
        } else {
            // A not-yet-included chapter can use the explicit root's
            // preamble without importing unrelated chapters.
            let end = graph.preamble_end.unwrap_or(graph.source.len());
            let mut declarations = graph.source[..end].to_owned();
            declarations.push('\n');
            declarations.push_str(source);

            let options = syntax::infer_declaration_config(&declarations, &config.config)
                .map_err(|error| format!("{}: project declarations: {error}", path.display()))?;
            (options.syntax, options.protect)
        };
        let config = self
            .compiled_states
            .resolve(config, syntax, protect, file)?;
        Ok(PreparedSource {
            config,
            input_graph: self.input_graph(),
        })
    }
}

fn root_marker(source: &str, config: &CompiledConfig) -> Result<Option<String>, String> {
    static ROOT: OnceLock<regex::Regex> = OnceLock::new();
    let compile = || {
        regex::Regex::new(r"(?im)^[\t \u{feff}]*%[\t ]*![\t ]*TEX[\t ]+root[\t ]*=[\t ]*([^\r\n]*)")
            .expect("valid root comment pattern")
    };
    let regex = ROOT.get_or_init(compile);
    for matched in regex.captures_iter(source) {
        let comment = matched.get(0).expect("whole match");
        let percent = comment.start() + comment.as_str().find('%').expect("root comment");
        // Validate only the prefix: later chapter text may require declarations
        // from this very root before its raw content can be parsed correctly.
        let prefix = &source[..comment.end()];
        let options = macro_protected_config(&config.config);
        let compiled = config.compiled_syntax();
        let Ok(ranges) = syntax::protected_ranges_with_syntax(prefix, &options, compiled) else {
            continue;
        };
        if syntax::is_protected(percent, &ranges) {
            continue;
        }
        let tokens = syntax::lex_with_ranges(prefix, &options, compiled, &ranges)
            .map_err(|error| error.to_string())?;
        let mut depth = 0usize;
        let mut environments = 0usize;
        let mut real_comment = false;
        for token in tokens {
            match token.kind {
                TokenKind::OpenBrace | TokenKind::OpenBracket => depth += 1,
                TokenKind::CloseBrace | TokenKind::CloseBracket => depth = depth.saturating_sub(1),
                TokenKind::BeginEnvironment => environments += 1,
                TokenKind::EndEnvironment => environments = environments.saturating_sub(1),
                TokenKind::Comment if token.span.start == percent => {
                    real_comment = depth == 0 && environments == 0;
                }
                _ => (),
            }
        }
        if !real_comment {
            continue;
        }
        let name = matched[1].trim();
        let name = name
            .strip_prefix('"')
            .and_then(|name| name.strip_suffix('"'))
            .or_else(|| {
                name.strip_prefix('\'')
                    .and_then(|name| name.strip_suffix('\''))
            })
            .unwrap_or(name);
        if name.is_empty() {
            return Err("TEX root comment requires a file path".into());
        }
        return Ok(Some(name.to_owned()));
    }
    Ok(None)
}

fn literal_inputs(source: &str, config: &Config) -> Result<ParsedInputs, String> {
    let compiled = syntax::compile(&config.syntax).map_err(|error| error.to_string())?;
    literal_inputs_with_syntax(source, config, &compiled)
}

fn macro_protected_config(config: &Config) -> Cow<'_, Config> {
    if config.protect.macro_bodies {
        Cow::Borrowed(config)
    } else {
        let mut options = config.clone();
        options.protect.macro_bodies = true;
        Cow::Owned(options)
    }
}

fn literal_inputs_with_syntax(
    source: &str,
    config: &Config,
    compiled: &syntax::CompiledSyntax,
) -> Result<ParsedInputs, String> {
    let options = macro_protected_config(config);
    let protected = syntax::protected_ranges_with_syntax(source, &options, compiled)
        .map_err(|error| error.to_string())?;
    let tokens = {
        syntax::lex_with_ranges(source, &options, compiled, &protected)
            .map_err(|error| error.to_string())?
    };
    let mut inputs = Vec::new();
    let mut depth = 0usize;
    let mut skip_until = 0usize;
    let mut document_start = None;
    let mut environments = 0usize;
    let mut conditions = 0usize;
    for token in tokens {
        if token.span.start < skip_until {
            continue;
        }
        match token.kind {
            TokenKind::OpenBrace | TokenKind::OpenBracket => depth += 1,
            TokenKind::CloseBrace | TokenKind::CloseBracket => depth = depth.saturating_sub(1),
            TokenKind::BeginEnvironment | TokenKind::EndEnvironment => {
                let (_, end) =
                    syntax::command_at(source, token.span.start).expect("environment command");
                let start = trivia_end(source, end);
                let document = syntax::balanced_end(source, start, b'{', b'}')
                    .is_some_and(|end| source[start + 1..end - 1].trim() == "document");
                if document {
                    if token.kind == TokenKind::BeginEnvironment && depth == 0 && conditions == 0 {
                        document_start.get_or_insert(token.span.start);
                    }
                } else if token.kind == TokenKind::BeginEnvironment {
                    environments += 1;
                } else {
                    environments = environments.saturating_sub(1);
                }
            }
            TokenKind::Command => {
                let Some((command, end)) = syntax::command_at(source, token.span.start) else {
                    continue;
                };
                match command {
                    "begingroup" | "bgroup" => depth += 1,
                    "endgroup" | "egroup" => depth = depth.saturating_sub(1),
                    "fi" => conditions = conditions.saturating_sub(1),
                    _ if syntax::is_condition_command(command, &options) => conditions += 1,
                    _ => (),
                }
                if matches!(command, "string" | "noexpand") {
                    let start = trivia_end(source, end);
                    skip_until = syntax::command_at(source, start).map_or(start, |(_, end)| end);
                    continue;
                }
                if conditions != 0
                    || !matches!(
                        command,
                        "input"
                            | "include"
                            | "documentclass"
                            | "LoadClass"
                            | "LoadClassWithOptions"
                            | "usepackage"
                            | "RequirePackage"
                            | "RequirePackageWithOptions"
                    )
                    || options.syntax.commands.contains_key(command)
                {
                    continue;
                }
                if let Some(input) = input_at(source, token.span.start) {
                    skip_until = input.span.end;
                    inputs.push(input);
                }
            }
            _ => (),
        }
    }
    Ok(ParsedInputs {
        inputs,
        document_start,
        protected_end: protected.last().map_or(0, |range| range.end),
    })
}

fn input_candidates(source: &str) -> impl Iterator<Item = Input> + '_ {
    memchr::memchr_iter(b'\\', source.as_bytes())
        .filter(|&start| {
            let rest = &source[start + 1..];
            match rest.as_bytes().first() {
                Some(b'i') => rest.starts_with("input") || rest.starts_with("include"),
                Some(b'd') => rest.starts_with("documentclass"),
                Some(b'L') => rest.starts_with("LoadClass"),
                Some(b'u') => rest.starts_with("usepackage"),
                Some(b'R') => rest.starts_with("RequirePackage"),
                _ => false,
            }
        })
        .filter_map(|start| input_at(source, start))
}

fn ignored_resource_input(input: &Input, command: &str, config: &SyntaxConfig) -> bool {
    matches!(command, "input" | "include")
        && input
            .name
            .split(',')
            .all(|name| ignored_input_extension(name, config))
}

fn ignored_input_extension(name: &str, config: &SyntaxConfig) -> bool {
    const RESOURCES: &[&str] = &[
        "svg", "png", "pdf", "jpg", "jpeg", "gif", "bmp", "webp", "tif", "tiff", "eps", "ps",
        "mp4", "m4v", "mov", "avi", "webm", "mp3", "wav", "ogg",
    ];
    Path::new(name.trim())
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            if let Some(extensions) = &config.input_ignore_extensions {
                extensions.iter().any(|ignored| {
                    extension.eq_ignore_ascii_case(ignored.strip_prefix('.').unwrap_or(ignored))
                })
            } else {
                RESOURCES
                    .iter()
                    .any(|ignored| extension.eq_ignore_ascii_case(ignored))
            }
        })
}

fn input_at(source: &str, start: usize) -> Option<Input> {
    let (command, end) = syntax::command_at(source, start)?;
    if !matches!(
        command,
        "input"
            | "include"
            | "documentclass"
            | "LoadClass"
            | "LoadClassWithOptions"
            | "usepackage"
            | "RequirePackage"
            | "RequirePackageWithOptions"
    ) {
        return None;
    }
    let mut argument = trivia_end(source, end);
    if source.as_bytes().get(argument) == Some(&b'[')
        && !matches!(
            command,
            "input" | "include" | "LoadClassWithOptions" | "RequirePackageWithOptions"
        )
    {
        argument = trivia_end(source, syntax::balanced_end(source, argument, b'[', b']')?);
    }
    let (name, finish) = if source.as_bytes().get(argument) == Some(&b'{') {
        let finish = syntax::balanced_end(source, argument, b'{', b'}')?;
        (source[argument + 1..finish - 1].trim(), finish)
    } else if command == "input" && source.as_bytes().get(argument) == Some(&b'\\') {
        let finish = syntax::command_at(source, argument)?.1;
        (&source[argument..finish], finish)
    } else if command == "input" {
        let finish = source[argument..]
            .find(|character: char| character.is_whitespace() || "\\{}%".contains(character))
            .map_or(source.len(), |offset| argument + offset);
        (&source[argument..finish], finish)
    } else {
        return None;
    };
    let extension = match command {
        "documentclass" | "LoadClass" | "LoadClassWithOptions" => Some("cls"),
        "usepackage" | "RequirePackage" | "RequirePackageWithOptions" => Some("sty"),
        _ => None,
    };
    (!name.is_empty()
        && !name.contains(['{', '}', '%', '#'])
        && (matches!(command, "input" | "include") || !name.contains('\\')))
    .then(|| Input {
        name: extension.map_or_else(
            || name.to_owned(),
            |extension| {
                name.split(',')
                    .map(str::trim)
                    .map(|name| {
                        if Path::new(name).extension().is_some() {
                            name.to_owned()
                        } else {
                            format!("{name}.{extension}")
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(",")
            },
        ),
        span: start..finish,
    })
}

fn trivia_end(source: &str, mut cursor: usize) -> usize {
    loop {
        while source
            .as_bytes()
            .get(cursor)
            .is_some_and(u8::is_ascii_whitespace)
        {
            cursor += 1;
        }
        if source.as_bytes().get(cursor) != Some(&b'%') {
            return cursor;
        }
        cursor = source[cursor..]
            .find('\n')
            .map_or(source.len(), |offset| cursor + offset + 1);
    }
}

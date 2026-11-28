use std::collections::HashMap;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::{CommandFactory, Parser};
use ignore::WalkBuilder;
use rayon::prelude::*;
use similar::TextDiff;
use tift::layout::StageTrace;
use tift::{CompiledConfig, Diagnostic, FileKind, FormatOutput, Severity};

#[path = "config_io.rs"]
mod config_io;

#[path = "git_changes.rs"]
mod git_changes;

#[path = "project_context.rs"]
mod project_context;

#[path = "persistent_cache.rs"]
mod persistent_cache;

#[path = "project_cache.rs"]
mod project_cache;

#[path = "source_encoding.rs"]
mod source_encoding;

#[derive(Debug, Parser)]
#[command(
    name = "tift",
    version,
    about = "Format LaTeX source files",
    after_help = "Examples:\n  tift main.tex chapters/\n  tift --check .\n  tift --diff main.tex\n  tift - --stdin-filename main.tex < main.tex"
)]
struct Args {
    /// Files or directories; use '-' for stdin. Defaults to the current directory.
    paths: Vec<PathBuf>,
    /// Check formatting without writing files; exit 1 when changes are needed.
    #[arg(long, conflicts_with = "print", help_heading = "Input and output")]
    check: bool,
    /// Show a unified diff without writing files; exit 1 when changes are needed.
    #[arg(long, conflicts_with = "print", help_heading = "Input and output")]
    diff: bool,
    /// Print a single formatted file to stdout without writing it.
    #[arg(long, short = 'p', conflicts_with_all = ["check", "diff"], help_heading = "Input and output")]
    print: bool,
    /// Write a single formatted input to this path, preserving the input file.
    #[arg(long, short = 'o', value_name = "PATH", conflicts_with_all = ["check", "diff", "print", "backup"], help_heading = "Input and output")]
    output: Option<PathBuf>,
    /// Keep N rotating backups when rewriting a file (default N: 1).
    #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = "1", value_parser = parse_backup_count, value_name = "N", conflicts_with_all = ["check", "diff", "print", "output"], help_heading = "Input and output")]
    backup: Option<usize>,
    /// Exit 1 when the formatted text differs, including after successful writes.
    #[arg(long, help_heading = "Input and output")]
    fail_on_change: bool,
    /// Print formatting stage changes and timings to stderr.
    #[arg(long, help_heading = "Performance and diagnostics")]
    trace: bool,
    /// Print changed text, source/output lines, structure and rules for each stage.
    #[arg(long, help_heading = "Performance and diagnostics")]
    trace_detail: bool,
    /// Format selected source lines: e.g. 3,7-12,!9. Other lines retain their bytes.
    #[arg(
        long,
        short = 'n',
        value_name = "RANGES",
        help_heading = "Input and output"
    )]
    lines: Option<tift::LineRanges>,
    /// Format added or modified worktree lines since a Git revision, including new non-ignored files.
    #[arg(long, value_name = "REV", conflicts_with_all = ["lines", "show_config"], help_heading = "Input and output")]
    changed_since: Option<String>,
    /// Print the built-in configuration as editable TOML.
    #[arg(long, exclusive = true, help_heading = "Configuration")]
    dump_config: bool,
    /// Print effective configuration for one input without reading or formatting it.
    #[arg(long, conflicts_with_all = ["check", "diff", "print", "output", "backup", "fail_on_change", "trace", "trace_detail", "lines"], help_heading = "Configuration")]
    show_config: bool,
    /// Generate shell completions to stdout.
    #[arg(
        long,
        value_enum,
        value_name = "SHELL",
        exclusive = true,
        help_heading = "Generated help"
    )]
    completion: Option<clap_complete::Shell>,
    /// Generate a roff manual page to stdout.
    #[arg(long, exclusive = true, help_heading = "Generated help")]
    man: bool,
    /// Name used for stdin's file type, configuration and overrides.
    #[arg(long, value_name = "PATH", help_heading = "Input and output")]
    stdin_filename: Option<PathBuf>,
    /// Read this TOML configuration instead of discovering tift.toml.
    #[arg(
        long,
        value_name = "PATH",
        conflicts_with = "no_config",
        help_heading = "Configuration"
    )]
    config: Option<PathBuf>,
    /// Use defaults and command-line options without reading configuration files.
    #[arg(long, help_heading = "Configuration")]
    no_config: bool,
    /// Disable reading and writing persistent formatting cache files.
    #[arg(long, help_heading = "Performance and diagnostics")]
    no_cache: bool,
    /// Directory for persistent formatting results.
    #[arg(
        long,
        value_name = "PATH",
        help_heading = "Performance and diagnostics"
    )]
    cache_dir: Option<PathBuf>,
    /// Number of spaces per indentation level.
    #[arg(long, value_parser = parse_indent_width, value_name = "N", help_heading = "Layout overrides")]
    indent_width: Option<usize>,
    /// Use tabs for indentation.
    #[arg(long, help_heading = "Layout overrides")]
    tabs: bool,
    /// Wrap prose using the configured column width.
    #[arg(long, conflicts_with_all = ["no_wrap", "rewrap"], help_heading = "Layout overrides")]
    wrap: bool,
    /// Join existing prose lines and wrap paragraphs to the configured width.
    #[arg(long, conflicts_with_all = ["wrap", "no_wrap"], help_heading = "Layout overrides")]
    rewrap: bool,
    /// Carry overflowing prose onto the next line while retaining later line breaks.
    #[arg(long, conflicts_with_all = ["wrap", "rewrap", "no_wrap"], help_heading = "Layout overrides")]
    stable_wrap: bool,
    /// Preserve prose line breaks.
    #[arg(long, conflicts_with = "rewrap", help_heading = "Layout overrides")]
    no_wrap: bool,
    /// Target column width for prose wrapping.
    #[arg(long, value_parser = parse_positive, value_name = "N", help_heading = "Layout overrides")]
    wrap_width: Option<usize>,
    /// Enable configured structural line-break rules.
    #[arg(long, help_heading = "Layout overrides")]
    modify_line_breaks: bool,
    /// Apply configured replacements without other formatting.
    #[arg(long, help_heading = "Layout overrides")]
    replacement_only: bool,
    /// Maximum parallel worker count.
    #[arg(long, short = 'j', value_parser = parse_positive, value_name = "N", help_heading = "Performance and diagnostics")]
    threads: Option<usize>,
    /// Suppress summaries and warnings; errors remain visible.
    #[arg(long, short = 'q', help_heading = "Performance and diagnostics")]
    quiet: bool,
}

fn parse_positive(value: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .ok()
        .filter(|n| *n > 0)
        .ok_or_else(|| "expected a positive integer".into())
}

fn parse_indent_width(value: &str) -> Result<usize, String> {
    parse_positive(value).and_then(|n| {
        if n <= 256 {
            Ok(n)
        } else {
            Err("indent width must be between 1 and 256".into())
        }
    })
}

fn parse_backup_count(value: &str) -> Result<usize, String> {
    parse_positive(value).and_then(|count| {
        if count <= 1000 {
            Ok(count)
        } else {
            Err("backup count must be between 1 and 1000".into())
        }
    })
}

impl Args {
    fn overrides(&self) -> toml::Value {
        let mut value = toml::Table::new();
        if self.no_cache {
            value.insert(
                "cache".into(),
                toml::Value::Table(toml::Table::from_iter([(
                    "enabled".into(),
                    toml::Value::Boolean(false),
                )])),
            );
        }
        if let Some(directory) = &self.cache_dir {
            let cache = value
                .entry("cache")
                .or_insert_with(|| toml::Value::Table(toml::Table::new()));
            cache.as_table_mut().expect("cache is a table").insert(
                "directory".into(),
                toml::Value::String(directory.to_string_lossy().into_owned()),
            );
        }
        let mut indent = toml::Table::new();
        if let Some(width) = self.indent_width {
            indent.insert("width".into(), toml::Value::Integer(width as i64));
        }
        if self.tabs {
            indent.insert("style".into(), toml::Value::String("tabs".into()));
        }
        if !indent.is_empty() {
            value.insert("indent".into(), toml::Value::Table(indent));
        }
        let mut wrap = toml::Table::new();
        if self.wrap {
            wrap.insert("mode".into(), toml::Value::String("wrap".into()));
        }
        if self.rewrap {
            wrap.insert("mode".into(), toml::Value::String("rewrap".into()));
        }
        if self.stable_wrap {
            wrap.insert("mode".into(), toml::Value::String("stable".into()));
        }
        if self.no_wrap {
            wrap.insert("mode".into(), toml::Value::String("preserve".into()));
        }
        if let Some(width) = self.wrap_width {
            wrap.insert("columns".into(), toml::Value::Integer(width as i64));
        }
        if !wrap.is_empty() {
            value.insert("wrap".into(), toml::Value::Table(wrap));
        }
        if self.modify_line_breaks {
            value.insert(
                "breaks".into(),
                toml::Value::Table(toml::Table::from_iter([(
                    "enabled".into(),
                    toml::Value::Boolean(true),
                )])),
            );
        }
        if self.replacement_only {
            value.insert("replacement-only".into(), toml::Value::Boolean(true));
        }
        if let Some(threads) = self.threads {
            value.insert(
                "files".into(),
                toml::Value::Table(toml::Table::from_iter([(
                    "threads".into(),
                    toml::Value::Integer(threads as i64),
                )])),
            );
        }
        toml::Value::Table(value)
    }
}

struct Job {
    path: PathBuf,
    ranges: Option<tift::LineRanges>,
    input: Arc<source_encoding::SourceFile>,
    kind: FileKind,
    config: Arc<CompiledConfig>,
    config_sources: Vec<PathBuf>,
    matched_overrides: usize,
    deferred_store: Option<persistent_cache::Slot>,
}

struct InputPath {
    path: PathBuf,
    identity: Option<PathBuf>,
}

struct ProjectFile {
    selected: InputPath,
    identity: PathBuf,
    input: Arc<source_encoding::SourceFile>,
    preparation_started: Option<std::time::Instant>,
    effective: config_io::EffectiveConfig,
    slot: Option<persistent_cache::Slot>,
}

impl ProjectFile {
    fn can_stream(&self) -> bool {
        !self
            .slot
            .as_ref()
            .is_some_and(persistent_cache::Slot::may_hit)
    }

    fn read(
        selected: InputPath,
        effective: config_io::EffectiveConfig,
        project: &project_context::ProjectSnapshot,
        preparation_started: Option<std::time::Instant>,
    ) -> Result<(project_context::ProjectDependency, Self), String> {
        let identity = selected
            .identity
            .clone()
            .or_else(|| fs::canonicalize(&selected.path).ok());
        let identity =
            config_io::absolute(identity.as_deref().unwrap_or(&selected.path), Path::new(""));
        let input = project.read(&identity)?;
        let dependency = recover_panic(|| {
            let inputs =
                project_context::ProjectInputs::scan(&identity, &input.text, &effective.config)?;
            project.dependency(&identity, &input.text, &effective.config, inputs)
        })
        .map_err(|error| format!("{}: {error}", selected.path.display()))??;
        Ok((
            dependency,
            Self {
                selected,
                identity,
                input,
                preparation_started,
                effective,
                slot: None,
            },
        ))
    }
}

struct Work {
    original: Arc<String>,
    formatted: FormatOutput,
    encoded: Option<Vec<u8>>,
    differs: bool,
    trace: Vec<StageTrace>,
    details: Vec<tift::trace::DetailTrace>,
    cache_status: persistent_cache::Status,
}

type FileWork = (
    PathBuf,
    Result<(Job, Work, Option<std::time::Duration>), String>,
);

enum FileConfig<'a> {
    Graph(Result<&'a project_context::PreparedProject, &'a String>),
    Ready(Arc<CompiledConfig>),
}

fn stream_group(
    project: &project_context::ProjectSnapshot,
    dependency: &project_context::ProjectDependency,
    files: Vec<ProjectFile>,
    cache: &persistent_cache::PersistentCache,
    args: &Args,
    process: &(impl Fn(ProjectFile, FileConfig<'_>) -> Result<FileWork, String> + Sync),
) -> Vec<Result<FileWork, String>> {
    let base = Arc::clone(&files[0].effective.config);
    let mut slots: Vec<_> = files
        .iter()
        .map(|file| (file.selected.path.clone(), None))
        .collect();
    let mut pending = HashMap::new();
    let mut delayed = Vec::new();
    let selected = files.iter().map(|file| file.identity.clone()).collect();
    for (file, (_, slot)) in files.into_iter().zip(&mut slots) {
        if !file.can_stream() {
            delayed.push((file, slot));
            continue;
        }
        pending
            .entry(file.identity.clone())
            .or_insert_with(Vec::new)
            .push((file, slot));
    }
    let built = rayon::scope(|scope| {
        let graph = recover_panic(|| {
            project.build_streaming(dependency, base, selected, &mut |path, config| {
                if let Some(entries) = pending.remove(path) {
                    for (file, slot) in entries {
                        let config = Arc::clone(&config);
                        scope
                            .spawn(move |_| *slot = Some(process(file, FileConfig::Ready(config))));
                    }
                }
            })
        })??;
        pending
            .into_par_iter()
            .flat_map_iter(|(_, entries)| entries)
            .chain(delayed)
            .for_each(|(file, slot)| {
                *slot = Some(process(file, FileConfig::Graph(Ok(&graph))));
            });
        Ok::<_, String>(graph.input_graph())
    });
    slots
        .into_par_iter()
        .map(|(path, result)| {
            let fingerprint = match &built {
                Ok(fingerprint) => *fingerprint,
                Err(error) => return Ok((path, Err(error.clone()))),
            };
            let (_, result) = result.expect("each selected file was processed")?;
            let result = result.and_then(|(mut job, mut work, preparation)| {
                if let Some(slot) = job.deferred_store.take() {
                    recover_panic(|| {
                        if let Some(request) =
                            cache.request(slot, &job.input.text, &job.config, fingerprint)
                        {
                            request.store(&work.formatted);
                            work.cache_status = persistent_cache::Status::Miss;
                        }
                        work.finish(&job, args)
                    })
                    .map_err(|error| format!("{}: {error}", path.display()))??;
                }
                Ok((job, work, preparation))
            });
            Ok((path, result))
        })
        .collect()
}

impl Work {
    fn finish(&mut self, job: &Job, args: &Args) -> Result<(), String> {
        let encoded = job
            .input
            .encode(&self.formatted.text)
            .map_err(|error| format!("{}: {error}", job.path.display()))?;
        if !(job.input.is_utf8() || args.check && !args.diff) {
            self.encoded = Some(encoded.into_owned());
        }
        for diagnostic in &mut self.formatted.diagnostics {
            diagnostic.span = job.input.original_span(diagnostic.span.clone());
        }
        for detail in &mut self.details {
            detail.original_bytes = job.input.original_span(detail.original_bytes.clone());
        }
        if args.check && !args.diff {
            self.formatted.text = String::new();
        }
        Ok(())
    }

    fn output_bytes(&self) -> &[u8] {
        self.encoded
            .as_deref()
            .unwrap_or(self.formatted.text.as_bytes())
    }
}

pub fn run() -> i32 {
    let args = match Args::try_parse() {
        Ok(args) => args,
        Err(error) => {
            let code = error.exit_code();
            let _ = error.print();
            return code;
        }
    };
    match recover_panic(|| execute(&args)).and_then(|result| result) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: {error}");
            2
        }
    }
}

thread_local! {
    static RECOVERING_PANIC: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn recover_panic<T>(run: impl FnOnce() -> T) -> Result<T, String> {
    static HOOK: std::sync::Once = std::sync::Once::new();
    HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if !RECOVERING_PANIC
                .try_with(|flag| flag.get())
                .unwrap_or(false)
            {
                previous(info);
            }
        }));
    });
    let previous = RECOVERING_PANIC.with(|flag| flag.replace(true));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run));
    RECOVERING_PANIC.with(|flag| flag.set(previous));
    result.map_err(|payload| {
        let detail = payload
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| payload.downcast_ref::<&str>().copied())
            .unwrap_or("unknown cause");
        format!("internal formatter error: {detail}")
    })
}

fn worker_pool(
    configured: Option<usize>,
    limit: usize,
) -> Result<Option<rayon::ThreadPool>, String> {
    if limit <= 1 || configured == Some(1) {
        return Ok(None);
    }
    let threads = configured
        .unwrap_or_else(|| {
            std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get)
        })
        .min(limit);
    (threads > 1)
        .then(|| rayon::ThreadPoolBuilder::new().num_threads(threads).build())
        .transpose()
        .map_err(|error| format!("cannot create formatting worker pool: {error}"))
}

fn single_input_pool(
    configured: Option<usize>,
    dependency: &project_context::ProjectDependency,
) -> Result<Option<rayon::ThreadPool>, String> {
    match dependency {
        project_context::ProjectDependency::Root(_) => worker_pool(configured, 2),
        _ => Ok(None),
    }
}

fn execute(args: &Args) -> Result<i32, String> {
    if args.dump_config {
        let config = tift::Config::default_toml().map_err(|error| error.to_string())?;
        let stdout = io::stdout();
        let mut output = stdout.lock();
        output
            .write_all(config.as_bytes())
            .map_err(|error| format!("stdout: {error}"))?;
        output.flush().map_err(|error| format!("stdout: {error}"))?;
        return Ok(0);
    }
    if args.completion.is_some() || args.man {
        if !args.paths.is_empty() || args.stdin_filename.is_some() || args.config.is_some() {
            return Err(
                "completion and manual generation do not accept input or configuration paths"
                    .into(),
            );
        }
        let stdout = io::stdout();
        let mut output = stdout.lock();
        if let Some(shell) = args.completion {
            clap_complete::generate(shell, &mut Args::command(), "tift", &mut output);
        } else {
            clap_mangen::Man::new(Args::command())
                .render(&mut output)
                .map_err(|error| format!("stdout: {error}"))?;
        }
        output.flush().map_err(|error| format!("stdout: {error}"))?;
        return Ok(0);
    }
    let stdin_count = args
        .paths
        .iter()
        .filter(|path| path.as_os_str() == "-")
        .count();
    if stdin_count > 0 && (stdin_count != 1 || args.paths.len() != 1) {
        return Err("stdin '-' must be the only input".into());
    }
    if stdin_count == 0 && args.stdin_filename.is_some() {
        return Err("--stdin-filename requires stdin '-'".into());
    }
    let mut loader =
        config_io::Loader::new(args.config.as_deref(), args.no_config, args.overrides())?;
    if args.show_config {
        return show_configuration(args, &mut loader);
    }
    if args.backup.is_some() && stdin_count > 0 {
        return Err("--backup requires an input file".into());
    }
    if stdin_count == 1 {
        if args.changed_since.is_some() {
            return Err("--changed-since requires worktree files, use --lines for stdin".into());
        }
        return stdin(args, &mut loader);
    }
    let target = if let Some(path) = &args.output {
        if args.paths.len() != 1 {
            return Err("--output requires exactly one input file".into());
        }
        let input = &args.paths[0];
        if fs::symlink_metadata(input).is_ok_and(|metadata| !metadata.is_file()) {
            return Err("--output requires a regular input file".into());
        }
        let target = TargetSnapshot::new(path)?;
        let input =
            fs::canonicalize(input).map_err(|error| format!("{}: {error}", input.display()))?;
        if same_path(&input, &target.path) {
            return Err(
                "--output must differ from the input path; omit it to rewrite the input".into(),
            );
        }
        Some(target)
    } else {
        None
    };
    if args.print {
        if args.paths.len() != 1 {
            return Err("--print requires exactly one file".into());
        }
        let metadata = fs::symlink_metadata(&args.paths[0])
            .map_err(|e| format!("{}: {e}", args.paths[0].display()))?;
        if !metadata.is_file() {
            return Err("--print requires a file, not a directory or symbolic link".into());
        }
    }
    let paths = if args.paths.is_empty() {
        vec![PathBuf::from(".")]
    } else {
        args.paths.clone()
    };
    let preparation_started = (args.trace || args.trace_detail).then(std::time::Instant::now);
    let changes = args
        .changed_since
        .as_ref()
        .map(|revision| git_changes::Changes::load(revision, &paths))
        .transpose()?;
    let files = collect_files(&paths)?;
    let mut selected = Vec::with_capacity(files.len());
    let mut preparation_errors = Vec::new();
    for mut input in files {
        if let Some(changes) = &changes {
            if input.identity.is_none() {
                input.identity = Some(
                    fs::canonicalize(&input.path)
                        .map_err(|error| format!("{}: {error}", input.path.display()))?,
                );
            }
            if !changes.contains(input.identity.as_deref().unwrap())
                && !args.print
                && args.output.is_none()
            {
                continue;
            }
        }
        match loader.for_path(&input.path) {
            Ok(effective) if !effective.excluded => selected.push((input, effective)),
            Ok(_) => (),
            Err(error) => preparation_errors.push((input.path, error)),
        }
    }
    drop(loader);
    let threads = args
        .threads
        .or_else(|| selected.first()?.1.config.config.files.threads);
    let pool = worker_pool(threads, selected.len())?;
    if selected.is_empty() && preparation_errors.is_empty() && (args.print || args.output.is_some())
    {
        return Err("the input file is excluded by configuration".into());
    }
    if let Some(started) = preparation_started {
        eprintln!(
            "trace: input-selection: {} file(s), elapsed={:.3} ms",
            selected.len(),
            started.elapsed().as_secs_f64() * 1000.0
        );
    }
    let mut cache = persistent_cache::PersistentCache::default();
    let cache_enabled =
        !args.trace && !args.trace_detail && args.lines.is_none() && changes.is_none();
    let project =
        project_context::ProjectSnapshot::new(cache_enabled, args.trace || args.trace_detail);
    let read = |(selected, effective): (InputPath, config_io::EffectiveConfig)| {
        let path = selected.path.clone();
        let started = (args.trace || args.trace_detail).then(std::time::Instant::now);
        ProjectFile::read(selected, effective, &project, started).map_err(|error| (path, error))
    };
    let files: Vec<_> = match &pool {
        Some(pool) => pool.install(|| selected.into_par_iter().map(read).collect()),
        None => selected.into_iter().map(read).collect(),
    };
    let pool = match files.as_slice() {
        [Ok((project_context::ProjectDependency::Root(root), file))]
            if root == &file.identity && project_context::parallel_prefixes(&file.input.text) =>
        {
            worker_pool(threads, 4)?
        }
        [Ok((dependency, _))] => single_input_pool(threads, dependency)?,
        _ => pool,
    };
    let mut groups = HashMap::new();
    for file in files {
        match file {
            Ok((dependency, mut file)) => {
                if cache_enabled && file.effective.config.config.cache.enabled {
                    let path = &file.selected.path;
                    file.slot = cache.slot(
                        path,
                        FileKind::from_path(path).expect("selected extension"),
                        &file.effective.config,
                    );
                }
                let base = Arc::as_ptr(&file.effective.config) as usize;
                groups
                    .entry((dependency, base))
                    .or_insert_with(Vec::new)
                    .push(file);
            }
            Err(error) => preparation_errors.push(error),
        }
    }
    let process = |file: ProjectFile, config: FileConfig<'_>| {
        let path = file.selected.path.clone();
        let ranges = match (&changes, &file.selected.identity) {
            (Some(changes), Some(canonical)) => {
                recover_panic(|| changes.ranges(canonical, &file.input.text))
                    .map_err(|error| format!("{}: {error}", path.display()))??
            }
            _ => args.lines.clone(),
        };
        let result = recover_panic(|| {
            let kind = FileKind::from_path(&path).expect("selected file has a supported extension");
            let (config, request, deferred_store) = match config {
                FileConfig::Ready(config) => (config, None, file.slot),
                FileConfig::Graph(graph) => {
                    let prepared = graph
                        .map_err(Clone::clone)?
                        .for_source(&file.identity, &file.input.text)?;
                    let request = file.slot.and_then(|slot| {
                        cache.request(
                            slot,
                            &file.input.text,
                            &prepared.config,
                            prepared.input_graph,
                        )
                    });
                    (prepared.config, request, None)
                }
            };
            let job = Job {
                path: file.selected.path,
                ranges,
                kind,
                input: file.input,
                config,
                config_sources: file.effective.sources,
                matched_overrides: file.effective.matched_overrides,
                deferred_store,
            };
            let preparation = file.preparation_started.map(|start| start.elapsed());
            let work = format_job(&job, args, request.as_ref())?;
            Ok((job, work, preparation))
        })
        .map_err(|error| format!("{}: {error}", path.display()))
        .and_then(|result| result);
        Ok::<_, String>((path, result))
    };
    let build = |dependency: &project_context::ProjectDependency, files: &[ProjectFile]| {
        recover_panic(|| {
            project.build(
                dependency,
                Arc::clone(&files[0].effective.config),
                files.iter().map(|file| file.identity.clone()).collect(),
            )
        })
        .and_then(|result| result)
    };
    let mut results: Vec<_> =
        match pool {
            Some(pool) => pool.install(|| {
                groups
                    .into_par_iter()
                    .flat_map(|((dependency, _), files)| {
                        if changes.is_none()
                            && files.len() > 1
                            && matches!(dependency, project_context::ProjectDependency::Root(_))
                            && files.iter().any(ProjectFile::can_stream)
                        {
                            return rayon::iter::Either::Left(
                                stream_group(&project, &dependency, files, &cache, args, &process)
                                    .into_par_iter(),
                            );
                        }
                        let prepared = Arc::new(build(&dependency, &files));
                        rayon::iter::Either::Right(files.into_par_iter().map(move |file| {
                            process(file, FileConfig::Graph((*prepared).as_ref()))
                        }))
                    })
                    .collect::<Result<_, _>>()
            }),
            None => {
                let mut results = Vec::new();
                for ((dependency, _), files) in groups {
                    let prepared = build(&dependency, &files);
                    for file in files {
                        results.push(process(file, FileConfig::Graph(prepared.as_ref()))?);
                    }
                }
                Ok(results)
            }
        }?;
    project.report();
    drop(project);
    cache.flush();
    results.extend(
        preparation_errors
            .into_iter()
            .map(|(path, error)| (path, Err(error))),
    );
    results.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    let checked = results.len();
    let mut changed = 0;
    let mut failed = false;
    let mut cache_hits = 0;
    let mut cache_misses = 0;
    let stdout = io::stdout();
    let mut output = stdout.lock();
    for (path, result) in results {
        let (job, work, preparation) = match result {
            Ok(done) => done,
            Err(error) => {
                eprintln!("error: {error}");
                failed = true;
                continue;
            }
        };
        match work.cache_status {
            persistent_cache::Status::Hit => cache_hits += 1,
            persistent_cache::Status::Miss => cache_misses += 1,
            persistent_cache::Status::Bypassed => {}
        }
        if let Some(elapsed) = preparation {
            eprintln!(
                "trace: {}: project-preparation: {:.3} ms",
                path.display(),
                elapsed.as_secs_f64() * 1000.0
            );
            trace_configuration(&path, &job.config_sources, job.matched_overrides);
            encoding_trace(&path, &job.input);
            print_trace(&path, &work.trace);
            print_detail_trace(&path, &work.details);
        }
        if diagnostics(&path, &work.formatted.diagnostics, args.quiet) {
            failed = true;
            continue;
        }
        let differs = work.differs;
        if let Some(target) = &target {
            let _lock = lock_output(&target.path)?;
            write_target(
                target,
                work.output_bytes(),
                Some((&path, job.input.original_bytes())),
                None,
            )?;
            changed += usize::from(differs);
        } else if args.diff && differs {
            changed += 1;
            print_diff(&mut output, &path, &work.original, &work.formatted.text)?;
        } else if args.print {
            changed += usize::from(differs);
            output
                .write_all(work.output_bytes())
                .map_err(|e| format!("stdout: {e}"))?;
        } else if args.check {
            changed += usize::from(differs);
        } else if !args.diff && differs {
            let written = commit_input(
                &path,
                job.input.original_bytes(),
                work.output_bytes(),
                args.backup,
            );
            match written {
                Ok(written) => changed += usize::from(written),
                Err(error) => {
                    eprintln!("error: {error}");
                    failed = true;
                }
            }
        }
    }
    output.flush().map_err(|e| format!("stdout: {e}"))?;
    if !args.quiet && !args.print && cache_hits + cache_misses > 0 {
        eprintln!("Cache: {cache_hits} hit(s), {cache_misses} miss(es).");
    }
    if !args.quiet && args.output.is_some() && !failed {
        eprintln!(
            "Formatted output: {}",
            target.as_ref().expect("output target").path.display()
        );
    } else if !args.quiet && !args.print {
        if args.check || args.diff {
            eprintln!("{} file(s) would change; {} checked.", changed, checked);
        } else {
            eprintln!("{} file(s) changed; {} checked.", changed, checked);
        }
    }
    Ok(if failed {
        2
    } else if (args.check || args.diff || args.fail_on_change) && changed > 0 {
        1
    } else {
        0
    })
}

fn stdin(args: &Args, loader: &mut config_io::Loader) -> Result<i32, String> {
    let path = args
        .stdin_filename
        .as_deref()
        .unwrap_or_else(|| Path::new("stdin.tex"));
    let kind = FileKind::from_path(path)
        .ok_or_else(|| format!("{}: unsupported file extension", path.display()))?;
    let effective = loader.for_path(path)?;
    let target = args
        .output
        .as_ref()
        .map(|path| TargetSnapshot::new(path))
        .transpose()?;
    let mut bytes = Vec::new();
    io::stdin()
        .read_to_end(&mut bytes)
        .map_err(|e| format!("stdin: {e}"))?;
    let input = Arc::new(
        source_encoding::SourceFile::decode(bytes).map_err(|error| format!("stdin: {error}"))?,
    );
    let source = input.text.as_str();
    let (mut formatted, trace, mut details) = if effective.excluded {
        (
            FormatOutput {
                text: source.to_owned(),
                diagnostics: Vec::new(),
            },
            Vec::new(),
            Vec::new(),
        )
    } else {
        let inputs = project_context::ProjectInputs::scan(path, source, &effective.config)?;
        let cache_enabled = !args.trace && !args.trace_detail && args.lines.is_none();
        let prepared =
            project_context::ProjectSnapshot::new(cache_enabled, args.trace || args.trace_detail)
                .prepare_buffer(path, &input, effective.config, inputs)?;
        let mut cache = persistent_cache::PersistentCache::default();
        let request = if cache_enabled {
            let cwd = std::env::current_dir().map_err(|e| format!("stdin: {e}"))?;
            cache
                .slot(&config_io::absolute(path, &cwd), kind, &prepared.config)
                .and_then(|slot| {
                    cache.request(slot, source, &prepared.config, prepared.input_graph)
                })
        } else {
            None
        };
        let (output, trace, details, _) = format_cached_source(
            source,
            kind,
            &prepared.config,
            args,
            args.lines.as_ref(),
            request.as_ref(),
        )
        .map_err(|e| format!("{}: {e}", path.display()))?;
        cache.flush();
        (output, trace, details)
    };
    let encoded = input.encode(&formatted.text)?;
    for diagnostic in &mut formatted.diagnostics {
        diagnostic.span = input.original_span(diagnostic.span.clone());
    }
    for detail in &mut details {
        detail.original_bytes = input.original_span(detail.original_bytes.clone());
    }
    if args.trace || args.trace_detail {
        trace_configuration(path, &effective.sources, effective.matched_overrides);
        encoding_trace(path, &input);
        if effective.excluded {
            eprintln!("{}: trace: input excluded by configuration", path.display());
        }
        print_trace(path, &trace);
        print_detail_trace(path, &details);
    }
    if diagnostics(path, &formatted.diagnostics, args.quiet) {
        return Ok(2);
    }
    let changed = source != formatted.text;
    let stdout = io::stdout();
    let mut output = stdout.lock();
    if let Some(target) = &target {
        let _lock = lock_output(&target.path)?;
        write_target(target, &encoded, None, None)?;
        if !args.quiet {
            eprintln!("Formatted output: {}", target.path.display());
        }
    } else if args.diff && changed {
        print_diff(&mut output, path, source, &formatted.text)?;
    } else if !args.check && !args.diff {
        output
            .write_all(&encoded)
            .map_err(|e| format!("stdout: {e}"))?;
    }
    output.flush().map_err(|e| format!("stdout: {e}"))?;
    Ok(
        if (args.check || args.diff || args.fail_on_change) && changed {
            1
        } else {
            0
        },
    )
}

fn collect_files(paths: &[PathBuf]) -> Result<Vec<InputPath>, String> {
    let cwd = std::env::current_dir().map_err(|e| format!("cannot read current directory: {e}"))?;
    let mut files = Vec::with_capacity(paths.len());
    #[cfg(windows)]
    let mut identities = std::collections::HashSet::new();
    let mut insert = |path: PathBuf| {
        #[cfg(windows)]
        let identity = fs::canonicalize(&path).ok();
        #[cfg(not(windows))]
        let identity = None;
        #[cfg(windows)]
        if !identities.insert(identity.as_ref().unwrap_or(&path).clone()) {
            return;
        }
        files.push(InputPath { path, identity });
    };
    for path in paths {
        let metadata =
            fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "{}: symbolic links are not supported as explicit inputs",
                path.display()
            ));
        }
        if metadata.is_file() {
            if FileKind::from_path(path).is_none() {
                return Err(format!("{}: unsupported file extension", path.display()));
            }
            insert(config_io::absolute(path, &cwd));
        } else if metadata.is_dir() {
            let mut walker = WalkBuilder::new(path);
            walker
                .hidden(false)
                .follow_links(false)
                .sort_by_file_path(|a, b| a.cmp(b))
                .filter_entry(|entry| entry.file_name() != ".git");
            for entry in walker.build() {
                let entry = entry.map_err(|e| e.to_string())?;
                if entry.file_type().is_some_and(|kind| kind.is_file())
                    && FileKind::from_path(entry.path()).is_some()
                {
                    insert(config_io::absolute(entry.path(), &cwd));
                }
            }
        } else {
            return Err(format!(
                "{}: expected a regular file or directory",
                path.display()
            ));
        }
    }
    files.sort_unstable_by(|left, right| left.path.cmp(&right.path));
    files.dedup_by(|left, right| left.path == right.path);
    Ok(files)
}

fn format_job(
    job: &Job,
    args: &Args,
    request: Option<&persistent_cache::Request>,
) -> Result<Work, String> {
    let metadata =
        fs::symlink_metadata(&job.path).map_err(|e| format!("{}: {e}", job.path.display()))?;
    if !metadata.is_file() {
        return Err(format!(
            "{}: input is no longer a regular file",
            job.path.display()
        ));
    }
    let original = Arc::clone(&job.input.text);
    let (formatted, trace, details, cache_status) = format_cached_source(
        &original,
        job.kind,
        &job.config,
        args,
        job.ranges.as_ref(),
        request,
    )
    .map_err(|e| format!("{}: {e}", job.path.display()))?;
    let differs = original.as_str() != formatted.text;
    let mut work = Work {
        original,
        formatted,
        encoded: None,
        differs,
        trace,
        details,
        cache_status,
    };
    if job.deferred_store.is_none() {
        work.finish(job, args)?;
    }
    Ok(work)
}

type CachedFormat = (
    FormatOutput,
    Vec<StageTrace>,
    Vec<tift::trace::DetailTrace>,
    persistent_cache::Status,
);

fn format_cached_source(
    source: &str,
    kind: FileKind,
    config: &CompiledConfig,
    args: &Args,
    ranges: Option<&tift::LineRanges>,
    cache: Option<&persistent_cache::Request>,
) -> Result<CachedFormat, tift::FormatError> {
    if let Some(output) = cache.and_then(|cache| cache.load(source)) {
        return Ok((
            output,
            Vec::new(),
            Vec::new(),
            persistent_cache::Status::Hit,
        ));
    }
    let (output, trace, details) = format_source(source, kind, config, args, ranges)?;
    if let Some(cache) = cache {
        cache.store(&output);
    }
    let status = if cache.is_some() {
        persistent_cache::Status::Miss
    } else {
        persistent_cache::Status::Bypassed
    };
    Ok((output, trace, details, status))
}

fn format_source(
    source: &str,
    kind: FileKind,
    config: &CompiledConfig,
    args: &Args,
    ranges: Option<&tift::LineRanges>,
) -> Result<(FormatOutput, Vec<StageTrace>, Vec<tift::trace::DetailTrace>), tift::FormatError> {
    let format = |text: &str| {
        if let Some(ranges) = ranges {
            let (output, stages) = if args.trace || args.trace_detail {
                tift::ranges::format_ranges_traced(text, kind, config, ranges)?
            } else {
                (tift::format_ranges(text, kind, config, ranges)?, Vec::new())
            };
            let details = if args.trace_detail {
                tift::trace::stage_changes("selection", text, &output.text, config)
            } else {
                Vec::new()
            };
            Ok((output, stages, details))
        } else if args.trace_detail {
            tift::layout::format_document_detailed(text, kind, config)
        } else if args.trace {
            tift::layout::format_document_traced(text, kind, config)
                .map(|(output, trace)| (output, trace, Vec::new()))
        } else {
            tift::format(text, kind, config).map(|output| (output, Vec::new(), Vec::new()))
        }
    };
    if let Some(source) = source.strip_prefix('\u{feff}') {
        let (mut formatted, mut trace, mut details) = format(source)?;
        formatted.text.insert(0, '\u{feff}');
        for diagnostic in &mut formatted.diagnostics {
            diagnostic.span.start += 3;
            diagnostic.span.end += 3;
        }
        for stage in &mut trace {
            stage.input_bytes += 3;
            stage.output_bytes += 3;
        }
        for detail in &mut details {
            detail.original_bytes.start += 3;
            detail.original_bytes.end += 3;
            detail.input_bytes.start += 3;
            detail.input_bytes.end += 3;
            detail.output_bytes.start += 3;
            detail.output_bytes.end += 3;
        }
        Ok((formatted, trace, details))
    } else {
        format(source)
    }
}

fn show_configuration(args: &Args, loader: &mut config_io::Loader) -> Result<i32, String> {
    if args.paths.len() > 1 {
        return Err("--show-config accepts at most one input path".into());
    }
    let path = match args.paths.first() {
        Some(path) if path.as_os_str() == "-" => args
            .stdin_filename
            .clone()
            .unwrap_or_else(|| "stdin.tex".into()),
        Some(path) if path.is_dir() => path.join("stdin.tex"),
        Some(path) => path.clone(),
        None => PathBuf::from("stdin.tex"),
    };
    let effective = loader.for_path(&path)?;
    let config = effective.config;
    let document = config
        .config
        .document()
        .map_err(|e| format!("configuration: {e}"))?;
    let text =
        toml::to_string_pretty(&document).map_err(|error| format!("configuration: {error}"))?;
    let stdout = io::stdout();
    let mut output = stdout.lock();
    writeln!(
        output,
        "# Effective configuration for {:?}",
        path.display().to_string()
    )
    .map_err(|error| format!("stdout: {error}"))?;
    if effective.sources.is_empty() {
        writeln!(
            output,
            "# Sources: built-in defaults and command-line settings"
        )
        .map_err(|error| format!("stdout: {error}"))?;
    } else {
        for path in effective.sources {
            writeln!(
                output,
                "# Configuration source: {:?}",
                path.display().to_string()
            )
            .map_err(|error| format!("stdout: {error}"))?;
        }
    }
    writeln!(
        output,
        "# Matched overrides: {}; excluded: {}",
        effective.matched_overrides, effective.excluded
    )
    .map_err(|error| format!("stdout: {error}"))?;
    if config.config.syntax.infer_list_environments {
        writeln!(
            output,
            "# List-environment declarations are inferred from source when formatting"
        )
        .map_err(|error| format!("stdout: {error}"))?;
    }
    output
        .write_all(text.as_bytes())
        .map_err(|error| format!("stdout: {error}"))?;
    output.flush().map_err(|error| format!("stdout: {error}"))?;
    Ok(0)
}

fn trace_configuration(path: &Path, sources: &[PathBuf], matched: usize) {
    if sources.is_empty() {
        eprintln!(
            "{}: trace: configuration=defaults, matched-overrides={matched}",
            path.display()
        );
    } else {
        for source in sources {
            eprintln!(
                "{}: trace: configuration={}, matched-overrides={matched}",
                path.display(),
                source.display()
            );
        }
    }
}

fn print_trace(path: &Path, trace: &[StageTrace]) {
    for stage in trace {
        eprintln!(
            "{}: trace: {}: changed={}, bytes={}->{}, elapsed={:.3} ms",
            path.display(),
            stage.name,
            stage.changed,
            stage.input_bytes,
            stage.output_bytes,
            stage.elapsed.as_secs_f64() * 1000.0
        );
    }
}

fn print_detail_trace(path: &Path, details: &[tift::trace::DetailTrace]) {
    for event in details {
        eprintln!(
            "{}: detail: stage={}, action={}, input-lines={}..{}, output-lines={}..{}, input-bytes={}..{}, output-bytes={}..{}, original-bytes={}..{}, context={}, rule={}",
            path.display(),
            event.stage,
            event.action,
            event.input_lines.start,
            event.input_lines.end,
            event.output_lines.start,
            event.output_lines.end,
            event.input_bytes.start,
            event.input_bytes.end,
            event.output_bytes.start,
            event.output_bytes.end,
            event.original_bytes.start,
            event.original_bytes.end,
            event.context.as_deref().unwrap_or("none"),
            event.rule.as_deref().unwrap_or("none")
        );
        eprintln!("  before={:?}", event.before);
        eprintln!("  after={:?}", event.after);
    }
}

fn diagnostics(path: &Path, diagnostics: &[Diagnostic], quiet: bool) -> bool {
    let mut failed = false;
    for diagnostic in diagnostics {
        let severity = match diagnostic.severity {
            Severity::Warning => "warning",
            Severity::Error => {
                failed = true;
                "error"
            }
        };
        if !quiet || diagnostic.severity == Severity::Error {
            eprintln!(
                "{}: {severity}: {} [bytes {}..{}]",
                path.display(),
                diagnostic.message,
                diagnostic.span.start,
                diagnostic.span.end,
            );
        }
    }
    failed
}

fn encoding_trace(path: &Path, input: &source_encoding::SourceFile) {
    if !input.is_utf8() {
        eprintln!(
            "{}: trace: stage byte counts and ranges address decoded UTF-8 text; original-bytes and diagnostics address original file bytes",
            path.display()
        );
    }
}

fn print_diff(
    output: &mut impl Write,
    path: &Path,
    source: &str,
    formatted: &str,
) -> Result<(), String> {
    let name = path.display().to_string();
    let diff = TextDiff::from_lines(source, formatted)
        .unified_diff()
        .context_radius(3)
        .header(&format!("a/{name}"), &format!("b/{name}"))
        .to_string();
    output
        .write_all(diff.as_bytes())
        .map_err(|e| format!("stdout: {e}"))
}

struct TargetSnapshot {
    path: PathBuf,
    contents: Option<Vec<u8>>,
    permissions: Option<fs::Permissions>,
}

impl TargetSnapshot {
    fn new(path: &Path) -> Result<Self, String> {
        let filename = path
            .file_name()
            .ok_or_else(|| format!("{}: expected an output filename", path.display()))?;
        let parent = path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let parent =
            fs::canonicalize(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
        let path = parent.join(filename);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_file() => Ok(Self {
                contents: Some(
                    fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?,
                ),
                permissions: Some(metadata.permissions()),
                path,
            }),
            Ok(_) => Err(format!(
                "{}: output must be a regular file, not a directory or symbolic link",
                path.display()
            )),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self {
                path,
                contents: None,
                permissions: None,
            }),
            Err(error) => Err(format!("{}: {error}", path.display())),
        }
    }

    fn unchanged(&self) -> Result<(), String> {
        let same = match fs::symlink_metadata(&self.path) {
            Ok(metadata) if metadata.is_file() => self.contents.as_ref().is_some_and(|previous| {
                fs::read(&self.path).is_ok_and(|current| &current == previous)
            }),
            Err(error) if error.kind() == io::ErrorKind::NotFound => self.contents.is_none(),
            _ => false,
        };
        if same {
            Ok(())
        } else {
            Err(format!(
                "{}: output changed during formatting; current content was preserved",
                self.path.display()
            ))
        }
    }
}

fn same_path(left: &Path, right: &Path) -> bool {
    #[cfg(windows)]
    {
        left.as_os_str().to_string_lossy().to_lowercase()
            == right.as_os_str().to_string_lossy().to_lowercase()
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

fn source_unchanged(path: &Path, original: &[u8]) -> Result<(), String> {
    if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_file())
        && fs::read(path).is_ok_and(|current| current == original)
    {
        Ok(())
    } else {
        Err(format!(
            "{}: input changed during formatting; current content was preserved",
            path.display()
        ))
    }
}

fn lock_output(path: &Path) -> Result<fs::File, String> {
    let name = path
        .file_name()
        .ok_or_else(|| format!("{}: expected an output filename", path.display()))?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent =
        fs::canonicalize(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
    #[cfg(windows)]
    let lower_name = name.to_string_lossy().to_lowercase();
    #[cfg(windows)]
    let name = std::ffi::OsStr::new(&lower_name);
    let id = blake3::hash(name.as_encoded_bytes()).to_hex();
    let lock_path = parent.join(format!(".tift-lock-{id}"));
    let lock_error = |error: io::Error| {
        format!(
            "{}: cannot lock output ({}): {error}",
            path.display(),
            lock_path.display()
        )
    };
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true);
    let lock = match options.clone().create_new(true).open(&lock_path) {
        Ok(lock) => lock,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            if !fs::symlink_metadata(&lock_path)
                .map_err(lock_error)?
                .is_file()
            {
                return Err(format!(
                    "{}: output lock must be a regular file",
                    lock_path.display()
                ));
            }
            options.open(&lock_path).map_err(lock_error)?
        }
        Err(error) => return Err(lock_error(error)),
    };
    // Keep the sidecar inode stable while any process could be waiting on it.
    fs2::FileExt::lock_exclusive(&lock).map_err(lock_error)?;
    Ok(lock)
}

fn commit_input(
    path: &Path,
    original: &[u8],
    formatted: &[u8],
    backups: Option<usize>,
) -> Result<bool, String> {
    let _lock = lock_output(path)?;
    // Another tift process may already have committed the same result.
    if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_file())
        && fs::read(path).is_ok_and(|current| current == formatted)
    {
        return Ok(false);
    }
    source_unchanged(path, original)?;
    if let Some(count) = backups {
        create_backup(path, original, count)?;
    }
    atomic_write(path, original, formatted)?;
    Ok(true)
}

fn write_target(
    target: &TargetSnapshot,
    formatted: &[u8],
    source: Option<(&Path, &[u8])>,
    fallback_permissions: Option<fs::Permissions>,
) -> Result<(), String> {
    if let Some((path, original)) = source {
        source_unchanged(path, original)?;
    }
    target.unchanged()?;
    if target.contents.as_deref() == Some(formatted) {
        return Ok(());
    }
    let mut temporary =
        tempfile::NamedTempFile::new_in(target.path.parent().expect("resolved output parent"))
            .map_err(|error| {
                format!(
                    "{}: cannot create temporary file: {error}",
                    target.path.display()
                )
            })?;
    temporary
        .write_all(formatted)
        .map_err(|error| format!("{}: {error}", target.path.display()))?;
    temporary
        .flush()
        .map_err(|error| format!("{}: {error}", target.path.display()))?;
    if let Some(permissions) = target.permissions.clone().or(fallback_permissions) {
        temporary
            .as_file()
            .set_permissions(permissions)
            .map_err(|error| format!("{}: {error}", target.path.display()))?;
    }
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| format!("{}: {error}", target.path.display()))?;
    if let Some((path, original)) = source {
        source_unchanged(path, original)?;
    }
    target.unchanged()?;
    if target.contents.is_none() {
        temporary.persist_noclobber(&target.path).map_err(|error| {
            format!(
                "{}: cannot create output: {}",
                target.path.display(),
                error.error
            )
        })?;
    } else {
        temporary.persist(&target.path).map_err(|error| {
            format!(
                "{}: cannot replace output: {}",
                target.path.display(),
                error.error
            )
        })?;
    }
    Ok(())
}

fn create_backup(path: &Path, original: &[u8], count: usize) -> Result<(), String> {
    source_unchanged(path, original)?;
    let permissions = fs::metadata(path)
        .map_err(|error| format!("{}: {error}", path.display()))?
        .permissions();
    let mut slots = Vec::with_capacity(count);
    for index in 0..count {
        let mut name = path.file_name().expect("input filename").to_os_string();
        name.push(if index == 0 {
            ".bak".into()
        } else {
            format!(".bak.{index}")
        });
        slots.push(TargetSnapshot::new(&path.with_file_name(name))?);
    }
    // Read and validate all slots before changing any. Existing older slots
    // outside this requested range remain available if the count is reduced.
    for index in (1..count).rev() {
        if let Some(previous) = &slots[index - 1].contents {
            write_target(
                &slots[index],
                previous,
                Some((path, original)),
                Some(permissions.clone()),
            )?;
        }
    }
    write_target(
        &slots[0],
        original,
        Some((path, original)),
        Some(permissions),
    )
}

fn atomic_write(path: &Path, original: &[u8], formatted: &[u8]) -> Result<(), String> {
    // tempfile passes these paths directly to MoveFileExW. Use the extended
    // absolute parent for both names, without following a final-component link.
    #[cfg(windows)]
    let absolute = {
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::canonicalize(parent)
            .map_err(|e| format!("{}: {e}", path.display()))?
            .join(
                path.file_name()
                    .ok_or_else(|| format!("{}: missing filename", path.display()))?,
            )
    };
    #[cfg(windows)]
    let path = absolute.as_path();
    let metadata = fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !metadata.is_file() {
        return Err(format!(
            "{}: input is no longer a regular file",
            path.display()
        ));
    }
    let current = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if current != original {
        return Err(format!(
            "{}: file changed during formatting; original content was preserved",
            path.display()
        ));
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .map_err(|e| format!("{}: cannot create temporary file: {e}", path.display()))?;
    temporary
        .write_all(formatted)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    temporary
        .flush()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    temporary
        .as_file()
        .set_permissions(metadata.permissions())
        .map_err(|e| format!("{}: {e}", path.display()))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    // Recheck immediately before the atomic rename, since formatting may have
    // run concurrently with an editor saving the source.
    if !fs::symlink_metadata(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .is_file()
        || fs::read(path).map_err(|e| format!("{}: {e}", path.display()))? != original
    {
        return Err(format!(
            "{}: file changed during formatting; original content was preserved",
            path.display()
        ));
    }
    temporary
        .persist(path)
        .map_err(|e| format!("{}: cannot replace file: {}", path.display(), e.error))?;
    Ok(())
}

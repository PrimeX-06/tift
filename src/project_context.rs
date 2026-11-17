use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use ignore::WalkBuilder;
use tift::config::{ProtectConfig, SyntaxConfig};
use tift::syntax::{self, TokenKind};
use tift::{CompiledConfig, Config, FileKind};

use super::config_io::absolute;

/// Immutable input snapshots and declaration contexts shared by one CLI run.
/// Explicit root comments supply context; unmarked files never search for parents.
#[derive(Default)]
pub struct ProjectContext {
    sources: HashMap<PathBuf, Arc<String>>,
    /// Canonical identities are stable for this invocation's source snapshot.
    /// Relative requests retain their working directory in the cache key.
    normalized_paths: HashMap<(PathBuf, PathBuf), PathBuf>,
    /// Dependency probes share this invocation's immutable source snapshot.
    /// Missing candidates are rechecked by the next ProjectContext.
    input_paths: HashMap<PathBuf, Option<PathBuf>>,
    siblings: HashMap<PathBuf, Vec<Arc<String>>>,
    sibling_configs: HashMap<(PathBuf, usize), Arc<CompiledConfig>>,
    graphs: HashMap<(PathBuf, usize), Arc<InputGraph>>,
    compiled: HashMap<(PathBuf, usize), Arc<CompiledConfig>>,
    compiled_states: HashMap<(usize, SyntaxConfig, ProtectConfig), Arc<CompiledConfig>>,
    bases: HashMap<usize, Arc<CompiledConfig>>,
}

#[derive(Default)]
struct InputGraph {
    source: String,
    declaration_index: syntax::DeclarationIndex,
    // Keep the full source for positions, restoration and error diagnostics.
    // Independently neutral spans need only one newline in later inference.
    inference_source: String,
    // Literal binding targets, including definitions with unknown signatures.
    // Their calls may invalidate later dynamic names and must stay in history.
    binding_targets: HashSet<String>,
    literal_inputs_index: Option<LiteralInputState>,

    file_ends: HashMap<PathBuf, usize>,
    file_inference_ends: HashMap<PathBuf, usize>,
    file_options: HashMap<PathBuf, Arc<Config>>,
    alias_events: HashMap<PathBuf, Vec<tift::config::EnvironmentAliasEvent>>,
    argument_events: HashMap<PathBuf, Vec<tift::config::EnvironmentArgumentEvent>>,
    package_events: HashMap<PathBuf, Vec<tift::config::PackageLoadEvent>>,
    preamble_end: Option<usize>,
    revision: u64,
    last_options: Option<DeclarationMemo>,
    // This graph belongs to one immutable explicit base configuration. A
    // historical policy refusal can retain the conservative false answer;
    // incomplete source must keep checking when later inputs complete it.
    boundary_policy_refused: bool,
}

struct DeclarationMemo {
    revision: u64,
    options: Arc<Config>,
    safe: bool,
    independent_append_safe: bool,
    reader_settled: bool,
    compiled: Option<Arc<syntax::CompiledSyntax>>,
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

type DeclarationSnapshot = (
    std::collections::BTreeMap<String, Option<tift::config::EnvironmentAlias>>,
    std::collections::BTreeMap<String, Option<tift::config::EnvironmentArguments>>,
    std::collections::BTreeMap<String, bool>,
);

fn declaration_snapshot(source: &str, config: &Config) -> DeclarationSnapshot {
    let model = tift::source_model::SourceModel::from_declaration_source(source, config);
    let aliases = model
        .exported_aliases()
        .iter()
        .map(|(name, alias)| (name.clone(), Some(alias.clone())))
        .chain(
            model
                .exported_unknown_aliases()
                .iter()
                .map(|name| (name.clone(), None)),
        )
        .collect();
    (
        aliases,
        model.exported_environment_arguments().clone(),
        model.exported_packages(),
    )
}

impl InputGraph {
    fn declaration_snapshot(&self, config: &Config) -> DeclarationSnapshot {
        declaration_snapshot(&self.source, config)
    }
    fn expand_input_name(
        &mut self,
        expression: &str,
        prefix: &str,
        base: &Config,
        options: &Arc<Config>,
    ) -> Option<String> {
        let original_length = self.source.len();
        self.source.push_str(prefix);
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
        self.source.truncate(original_length);
        result.filter(|name| !name.is_empty() && !name.contains(['\\', '{', '}', '%', '#']))
    }

    fn append(&mut self, source: &str) {
        self.record_binding_targets(source);
        self.source.push_str(source);
        self.inference_source.push_str(source);
        self.revision = self.revision.wrapping_add(1);
    }

    fn append_segment(&mut self, source: &str, neutral: bool) {
        if neutral && source.len() >= 256 && self.compaction_preserves_bindings(source) {
            self.source.push_str(source);
            self.inference_source.push('\n');
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
        let Some(compiled) = memo.compiled.as_ref() else {
            return false;
        };
        let Ok(protected) = syntax::protected_ranges_with_syntax(source, &memo.options, compiled)
        else {
            return false;
        };
        let mut protected_index = 0;
        for (start, _) in source.match_indices('\\') {
            while protected
                .get(protected_index)
                .is_some_and(|range| range.end <= start)
            {
                protected_index += 1;
            }
            if protected
                .get(protected_index)
                .is_some_and(|range| range.start < start)
            {
                continue;
            }
            let Some((name, end)) = syntax::command_at(source, start) else {
                return false;
            };
            let name = name.trim_end_matches('*');
            if self.binding_targets.contains(name)
                || memo.options.syntax.commands.contains_key(name)
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
        self.append_segment(source, neutral);
        self.append("\n");
        if reusable {
            self.last_options
                .as_mut()
                .expect("reusable options")
                .revision = self.revision;
        }
    }

    fn append_prepared_input(&mut self, prefix: &str, input: &str, reusable: bool) {
        self.append_segment(prefix, reusable);
        self.append(input);
        self.append("\n");
        if reusable {
            self.last_options
                .as_mut()
                .expect("reusable options")
                .revision = self.revision;
        }
    }

    fn infer(&mut self, path: &Path, config: &Config) -> Result<Arc<Config>, String> {
        if let Some(memo) = &self.last_options
            && memo.revision == self.revision
        {
            return Ok(Arc::clone(&memo.options));
        }

        let result = syntax::infer_declaration_config_indexed(
            &self.inference_source,
            config,
            &mut self.declaration_index,
        );
        // Re-run failures against the full source so error positions and the
        // established diagnostic remain exact after neutral text compaction.
        let (inferred, independent_append_safe, inferred_boundary) = result
            .or_else(|error| {
                if self.inference_source.len() != self.source.len() {
                    syntax::infer_declaration_config_with_reuse(&self.source, config)
                } else {
                    Err(error)
                }
            })
            .map_err(|error| format!("{}: project declarations: {error}", path.display()))?;

        let options = self
            .last_options
            .as_ref()
            .filter(|memo| {
                memo.options.syntax == inferred.syntax && memo.options.protect == inferred.protect
            })
            .map_or_else(|| Arc::new(inferred), |memo| Arc::clone(&memo.options));

        let boundary = if self.boundary_policy_refused {
            syntax::DeclarationReuseBoundary::PolicyRefused
        } else if self.inference_source.len() == self.source.len() {
            inferred_boundary
        } else {
            syntax::declaration_reuse_boundary_status(&self.source, config)
        };

        self.boundary_policy_refused |= boundary == syntax::DeclarationReuseBoundary::PolicyRefused;
        let safe = boundary == syntax::DeclarationReuseBoundary::Safe;
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
            reader_settled: self.declaration_index.reader_settled(),
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
        if let Some(memo) = &self.last_options
            && memo.revision == self.revision
        {
            let neutral = memo.neutral(suffix);
            if suffix.is_empty() || neutral || memo.whitespace(suffix, self.source.ends_with('\n'))
            {
                return Ok((Arc::clone(&memo.options), neutral));
            }
        }
        // Preview the next input's prefix without copying the cumulative graph.
        // The memo is untouched, and truncation restores its immutable revision.
        let length = self.inference_source.len();
        self.inference_source.push_str(suffix);

        let mut preview_index = self.declaration_index.clone();
        let inferred = syntax::infer_declaration_config_indexed(
            &self.inference_source,
            config,
            &mut preview_index,
        )
        .map(|(options, _, _)| options)
        .or_else(|error| {
            if length != self.source.len() {
                let full_length = self.source.len();
                self.source.push_str(suffix);
                let result = syntax::infer_declaration_config(&self.source, config);
                self.source.truncate(full_length);
                result
            } else {
                Err(error)
            }
        });

        self.inference_source.truncate(length);
        inferred
            .map(|options| (Arc::new(options), false))
            .map_err(|error| format!("{}: project declarations: {error}", path.display()))
    }

    fn literal_inputs(&self, source: &str) -> Result<(Vec<Input>, Option<usize>), String> {
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

    pub fn read_source(&mut self, path: &Path) -> Result<Arc<String>, String> {
        if let Some(source) = self.sources.get(path) {
            return Ok(Arc::clone(source));
        }
        let path = self.normalized_path(path)?;
        self.read_normalized_source(&path)
    }

    pub fn read_source_with_identity(
        &mut self,
        path: &Path,
        identity: Option<PathBuf>,
    ) -> Result<Arc<String>, String> {
        if let Some(source) = self.sources.get(path) {
            return Ok(Arc::clone(source));
        }
        let Some(identity) = identity else {
            return self.read_source(path);
        };
        let cwd = if path.is_absolute() {
            PathBuf::new()
        } else {
            std::env::current_dir().map_err(|error| error.to_string())?
        };
        let key = (path.to_path_buf(), cwd.clone());
        // A previous editor source or immutable alias snapshot takes priority
        // over a later filesystem identity supplied by the input collector.
        if let Some(source) = self
            .normalized_paths
            .get(&key)
            .and_then(|cached| self.sources.get(cached))
        {
            return Ok(Arc::clone(source));
        }
        let identity = absolute(&identity, &cwd);
        self.normalized_paths.insert(key, identity.clone());
        self.normalized_paths
            .entry((identity.clone(), PathBuf::new()))
            .or_insert_with(|| identity.clone());
        self.read_normalized_source(&identity)
    }

    fn read_normalized_source(&mut self, path: &Path) -> Result<Arc<String>, String> {
        if let Some(source) = self.sources.get(path) {
            return Ok(Arc::clone(source));
        }
        let source = Arc::new(
            fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?,
        );
        self.sources.insert(path.to_path_buf(), Arc::clone(&source));
        Ok(source)
    }

    pub fn for_source(
        &mut self,
        path: &Path,
        source: Arc<String>,
        config: Arc<CompiledConfig>,
    ) -> Result<Arc<CompiledConfig>, String> {
        if config.config.replacement_only
            || matches!(
                FileKind::from_path(path),
                Some(FileKind::Bibtex | FileKind::Dtx | FileKind::Ins)
            )
        {
            return Ok(config);
        }
        let path = if self.sources.contains_key(path) {
            path.to_path_buf()
        } else {
            self.normalized_path(path)?
        };
        let key = (path.clone(), Arc::as_ptr(&config) as usize);
        // Keep identities used in graph keys alive for this invocation.
        self.bases
            .entry(key.1)
            .or_insert_with(|| Arc::clone(&config));
        if let Some(cached) = self.compiled.get(&key) {
            return Ok(Arc::clone(cached));
        }
        self.sources.insert(path.clone(), Arc::clone(&source));
        let marker = root_marker(&source, &config.config)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        let root = self.resolve_root(&path, marker.as_deref(), &config.config)?;
        let has_inputs = marker.is_none() && {
            let candidates = input_candidates(&source);
            candidates.iter().any(|input| {
                let loader = syntax::command_at(&source, input.span.start)
                    .expect("input candidate")
                    .0;
                if config.config.syntax.commands.contains_key(loader)
                    || ignored_resource_input(input, loader, &config.config.syntax)
                {
                    return false;
                }
                if !matches!(loader, "input" | "include")
                    && !input
                        .name
                        .split(',')
                        .any(|name| self.input_path(name, &path, &root).ok().flatten().is_some())
                {
                    return false;
                }
                // CompiledConfig already validated this exact syntax. Reuse
                // it across files instead of rebuilding it for each candidate.
                literal_inputs_with_syntax(
                    &source[..input.span.end],
                    &config.config,
                    config.compiled_syntax(),
                )
                .is_ok_and(|(inputs, _)| {
                    inputs.iter().any(|candidate| candidate.span == input.span)
                })
            })
        };
        let result = if marker.is_some() || has_inputs {
            let graph_key = (root.clone(), key.1);
            let graph = if let Some(graph) = self.graphs.get(&graph_key) {
                Arc::clone(graph)
            } else {
                let mut graph = InputGraph::default();

                self.expand_inputs(
                    &root,
                    &root,
                    &config.config,
                    &mut HashSet::new(),
                    &mut graph,
                )?;
                let graph = Arc::new(graph);
                self.graphs.insert(graph_key, Arc::clone(&graph));
                graph
            };
            let options = if let Some(options) = graph.file_options.get(&path) {
                Arc::clone(options)
            } else if let Some(end) = graph.file_ends.get(&path) {
                let inference_end = graph.file_inference_ends[&path];

                Arc::new(
                    syntax::infer_declaration_config(
                        &graph.inference_source[..inference_end],
                        &config.config,
                    )
                    .or_else(|error| {
                        if inference_end != *end {
                            syntax::infer_declaration_config(&graph.source[..*end], &config.config)
                        } else {
                            Err(error)
                        }
                    })
                    .map_err(|error| {
                        format!("{}: project declarations: {error}", path.display())
                    })?,
                )
            } else {
                // A not-yet-included chapter can use the explicit root's
                // preamble without importing unrelated chapters.
                let end = graph.preamble_end.unwrap_or(graph.source.len());
                let mut declarations = graph.source[..end].to_owned();
                declarations.push('\n');
                declarations.push_str(&source);

                Arc::new(
                    syntax::infer_declaration_config(&declarations, &config.config).map_err(
                        |error| format!("{}: project declarations: {error}", path.display()),
                    )?,
                )
            };
            let mut options = options.as_ref().clone();
            if let Some(events) = graph.alias_events.get(&path) {
                options.syntax.inferred_environment_aliases.clear();
                options.syntax.environment_alias_events = events.clone();
            }
            if let Some(events) = graph.argument_events.get(&path) {
                options.syntax.environment_argument_events = events.clone();
            }
            if let Some(events) = graph.package_events.get(&path) {
                options.syntax.package_load_events = events.clone();
            }
            self.compile_options(&config, &options)?
        } else {
            // Sibling declarations only supply list environment names. With
            // the standard environment grammar, a file without an opener has
            // no environment whose item layout could use those names.
            if source.contains("\\begin")
                || source.contains("\\csname")
                || config.config.syntax.environment_pattern.is_some()
                || !config.config.syntax.special.is_empty()
            {
                self.sibling_lists(&path, config)?
            } else {
                config
            }
        };
        self.compiled.insert(key, Arc::clone(&result));
        Ok(result)
    }

    fn compile_options(
        &mut self,
        base: &Arc<CompiledConfig>,
        options: &Config,
    ) -> Result<Arc<CompiledConfig>, String> {
        if options.syntax == base.config.syntax && options.protect == base.config.protect {
            return Ok(Arc::clone(base));
        }
        let state = (
            Arc::as_ptr(base) as usize,
            options.syntax.clone(),
            options.protect.clone(),
        );
        if let Some(cached) = self.compiled_states.get(&state) {
            return Ok(Arc::clone(cached));
        }
        let result = Arc::new(
            base.with_project_syntax(options.syntax.clone(), options.protect.clone())
                .map_err(|error| error.to_string())?,
        );
        self.compiled_states.insert(state, Arc::clone(&result));
        Ok(result)
    }

    fn resolve_root(
        &mut self,
        path: &Path,
        marker: Option<&str>,
        config: &Config,
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
        path: &Path,
        root: &Path,
        config: &Config,
        stack: &mut HashSet<PathBuf>,
        graph: &mut InputGraph,
    ) -> Result<(), String> {
        if stack.len() >= 128 || !stack.insert(path.to_path_buf()) {
            return Ok(());
        }
        let source = self.read_normalized_source(path)?;
        let loader_occurrences = tift::source_model::SourceModel::loader_occurrences(&source);
        let (entry_aliases, entry_arguments, entry_packages) = graph.declaration_snapshot(config);
        let mut alias_events = vec![tift::config::EnvironmentAliasEvent {
            at: 0,
            loader: None,
            aliases: entry_aliases,
        }];
        let mut argument_events = vec![tift::config::EnvironmentArgumentEvent {
            at: 0,
            loader: None,
            arguments: entry_arguments,
        }];
        let mut package_events = vec![tift::config::PackageLoadEvent {
            at: 0,
            loader: None,
            packages: entry_packages,
        }];
        let mut cursor = 0;
        let candidates = input_candidates(&source);
        for input in candidates {
            graph.infer(path, config)?;
            let command = syntax::command_at(&source, input.span.start)
                .expect("input command")
                .0;
            if config.syntax.commands.contains_key(command)
                || ignored_resource_input(&input, command, &config.syntax)
            {
                continue;
            }
            // A literal loader without a local target contributes only its
            // original source text. Leave it in the pending suffix instead of
            // creating a declaration checkpoint and scanning the cumulative
            // graph twice. Dynamic filenames still need prefix inference.

            let prune_unresolved = true;

            if prune_unresolved && !input.name.contains('\\') {
                let mut local_target = false;
                for name in input.name.split(',').filter(|name| !name.is_empty()) {
                    if self.input_path(name, path, root)?.is_some() {
                        local_target = true;
                        break;
                    }
                }
                if !local_target {
                    continue;
                }
            }
            // Read only as far as this candidate. Later text can use raw
            // environments whose definitions have not been imported yet.
            let Ok((inputs, document_start)) = graph.literal_inputs(&source[..input.span.end])
            else {
                continue;
            };
            if !inputs.iter().any(|candidate| candidate.span == input.span) {
                continue;
            }
            let (inferred, reusable_prefix) =
                graph.prepare_prefix(&source[cursor..input.span.start], path, config)?;
            if inferred.syntax.commands.contains_key(command) {
                continue;
            }
            let resolved_name = if input.name.contains('\\') {
                graph.expand_input_name(
                    &input.name,
                    &source[cursor..input.span.start],
                    config,
                    &inferred,
                )
            } else {
                None
            };
            let names = if input.name.contains('\\') {
                resolved_name.as_deref().unwrap_or("")
            } else {
                &input.name
            };
            if path == root && graph.preamble_end.is_none() {
                graph.preamble_end =
                    document_start.map(|start| graph.source.len() + start.saturating_sub(cursor));
            }
            let mut alias_prefix = graph.source.clone();
            alias_prefix.push_str(&source[cursor..input.span.start]);
            let (before_aliases, before_arguments, before_packages) =
                declaration_snapshot(&alias_prefix, config);
            graph.append_prepared_input(
                &source[cursor..input.span.start],
                &source[input.span.clone()],
                reusable_prefix,
            );
            for name in names.split(',').filter(|name| !name.is_empty()) {
                if matches!(command, "input" | "include")
                    && ignored_input_extension(name, &config.syntax)
                {
                    continue;
                }
                if let Some(child) = self.input_path(name, path, root)? {
                    self.expand_inputs(&child, root, config, stack, graph)?;
                }
            }
            graph.append_line("");
            let (after_aliases, after_arguments, after_packages) =
                graph.declaration_snapshot(config);
            let changes = before_aliases
                .keys()
                .chain(after_aliases.keys())
                .filter(|name| before_aliases.get(*name) != after_aliases.get(*name))
                .map(|name| (name.clone(), after_aliases.get(name).cloned().flatten()))
                .collect();
            alias_events.push(tift::config::EnvironmentAliasEvent {
                at: input.span.end,
                loader: loader_occurrences.get(&input.span.start).cloned(),
                aliases: changes,
            });
            let changes = before_arguments
                .keys()
                .chain(after_arguments.keys())
                .filter(|name| before_arguments.get(*name) != after_arguments.get(*name))
                .map(|name| (name.clone(), after_arguments.get(name).cloned().flatten()))
                .collect();
            argument_events.push(tift::config::EnvironmentArgumentEvent {
                at: input.span.end,
                loader: loader_occurrences.get(&input.span.start).cloned(),
                arguments: changes,
            });
            let packages = after_packages
                .into_iter()
                .filter(|(name, state)| before_packages.get(name) != Some(state))
                .collect();
            package_events.push(tift::config::PackageLoadEvent {
                at: input.span.end,
                loader: loader_occurrences.get(&input.span.start).cloned(),
                packages,
            });
            cursor = input.span.end;
        }
        if path == root && graph.preamble_end.is_none() && source.contains("document") {
            graph.infer(path, config)?;
            graph.preamble_end = graph
                .literal_inputs(&source)
                .ok()
                .and_then(|(_, start)| start)
                .map(|start| graph.source.len() + start.saturating_sub(cursor));
        }
        graph.append_line(&source[cursor..]);
        if !graph.file_ends.contains_key(path) {
            graph.alias_events.insert(path.to_path_buf(), alias_events);
            graph
                .argument_events
                .insert(path.to_path_buf(), argument_events);
            graph
                .package_events
                .insert(path.to_path_buf(), package_events);
            // Retain the exact first-inclusion state. Incomplete file tails
            // keep lazy restoration; a later source may complete their reader.
            let _ = graph.infer(path, config);
            graph
                .file_ends
                .insert(path.to_path_buf(), graph.source.len());
            graph
                .file_inference_ends
                .insert(path.to_path_buf(), graph.inference_source.len());
            // Retain only a memo that already describes this exact first
            // inclusion. Invalidated suffixes keep lazy full inference, so a
            // chapter can still complete declarations from preceding inputs.
            if let Some(memo) = &graph.last_options
                && memo.revision == graph.revision
            {
                graph
                    .file_options
                    .insert(path.to_path_buf(), Arc::clone(&memo.options));
            }
        }
        stack.remove(path);
        Ok(())
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
                let result = if self.sources.contains_key(&candidate) {
                    Some(candidate)
                } else if let Some(cached) = self.input_paths.get(&candidate) {
                    cached.clone().or_else(|| {
                        let path = self
                            .normalized_paths
                            .get(&(candidate.clone(), PathBuf::new()))
                            .cloned()
                            .unwrap_or_else(|| absolute(&candidate, Path::new("")));
                        self.sources.contains_key(&path).then_some(path)
                    })
                } else {
                    let path = self.normalized_path(&candidate)?;
                    let result =
                        (self.sources.contains_key(&path) || path.is_file()).then_some(path);
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
        &mut self,
        path: &Path,
        config: Arc<CompiledConfig>,
    ) -> Result<Arc<CompiledConfig>, String> {
        if !config.config.syntax.infer_list_environments {
            return Ok(config);
        }
        let directory = path
            .parent()
            .expect("an absolute file has a parent")
            .to_path_buf();
        let key = (directory.clone(), Arc::as_ptr(&config) as usize);
        if let Some(cached) = self.sibling_configs.get(&key) {
            return Ok(Arc::clone(cached));
        }
        let declarations = if let Some(cached) = self.siblings.get(&directory) {
            cached.clone()
        } else {
            let mut sources = Vec::new();
            let mut walker = WalkBuilder::new(&directory);
            walker
                .hidden(false)
                .follow_links(false)
                .max_depth(Some(1))
                .sort_by_file_path(|a, b| a.cmp(b));
            for entry in walker.build().filter_map(Result::ok) {
                if !entry.file_type().is_some_and(|kind| kind.is_file())
                    || !matches!(
                        FileKind::from_path(entry.path()),
                        Some(FileKind::Tex | FileKind::Tikz | FileKind::Style | FileKind::Class)
                    )
                {
                    continue;
                }
                // The directory is canonical and this depth-one entry is a
                // regular file. Its direct child path already has canonical
                // parents, so another filesystem normalization is redundant.
                if let Ok(text) = self.read_normalized_source(entry.path()) {
                    // Most siblings contain ordinary document content. A
                    // conservative marker check avoids seven full-text searches.
                    if !text.contains("environment")
                        && !text.contains("DocumentEnvironment")
                        && !text.contains("\\newlist")
                    {
                        continue;
                    }
                    if [
                        "\\newenvironment",
                        "\\renewenvironment",
                        "\\NewDocumentEnvironment",
                        "\\RenewDocumentEnvironment",
                        "\\ProvideDocumentEnvironment",
                        "\\DeclareDocumentEnvironment",
                        "\\newlist",
                    ]
                    .iter()
                    .any(|command| text.contains(command))
                    {
                        sources.push(text);
                    }
                }
            }
            self.siblings.insert(directory.clone(), sources.clone());
            sources
        };
        let mut known = config.config.syntax.list_environments.clone();
        loop {
            let before = known.len();
            for source in declarations.iter() {
                known.extend(syntax::infer_list_environments_with_config(
                    source,
                    &known,
                    &config.config,
                ));
            }
            if known.len() == before {
                break;
            }
        }
        let result = if known.len() == config.config.syntax.list_environments.len() {
            config
        } else {
            let mut options = config.config.clone();
            options.syntax.list_environments = known;
            self.compile_options(&config, &options)?
        };
        self.sibling_configs.insert(key, Arc::clone(&result));
        Ok(result)
    }
}

fn root_marker(source: &str, config: &Config) -> Result<Option<String>, String> {
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
        let options = macro_protected_config(config);
        let Ok(ranges) = syntax::protected_ranges(prefix, &options) else {
            continue;
        };
        if syntax::is_protected(percent, &ranges) {
            continue;
        }
        let compiled = syntax::compile(&options.syntax).map_err(|error| error.to_string())?;
        let tokens = syntax::lex_with_ranges(prefix, &options, &compiled, &ranges)
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

fn literal_inputs(source: &str, config: &Config) -> Result<(Vec<Input>, Option<usize>), String> {
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
) -> Result<(Vec<Input>, Option<usize>), String> {
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
                if depth != 0
                    || conditions != 0
                    || environments != 0
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
    Ok((inputs, document_start))
}

fn input_candidates(source: &str) -> Vec<Input> {
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
        .collect()
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

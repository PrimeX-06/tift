use std::collections::HashMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use tift::config::merge_values;
use tift::{CompiledConfig, Config};

struct Document {
    value: toml::Value,
    raw_value: toml::Value,
    exclude_root: PathBuf,
    overrides_root: PathBuf,
    overrides: Vec<Override>,
    sources: Vec<PathBuf>,
}

struct Override {
    matcher: Gitignore,
    value: toml::Value,
}

struct CachedConfig {
    config: Arc<CompiledConfig>,
    excludes: Gitignore,
}

#[derive(Clone)]
pub struct EffectiveConfig {
    pub config: Arc<CompiledConfig>,
    pub excluded: bool,
    pub sources: Vec<PathBuf>,
    pub matched_overrides: usize,
}

/// Resolve configs before dispatching files to worker threads. The compiled
/// configurations are shared by all files with the same applicable overrides.
pub struct Loader {
    explicit: Option<PathBuf>,
    no_config: bool,
    cli: toml::Value,
    cwd: PathBuf,
    documents: HashMap<PathBuf, Arc<Document>>,
    discovered: HashMap<PathBuf, Option<PathBuf>>,
    compiled: HashMap<(Option<PathBuf>, Vec<usize>), CachedConfig>,
    matching_parents: HashMap<PathBuf, Option<PathBuf>>,
    global: Option<Option<PathBuf>>,
}

impl Loader {
    pub fn new(explicit: Option<&Path>, no_config: bool, cli: toml::Value) -> Result<Self, String> {
        let cwd =
            std::env::current_dir().map_err(|e| format!("cannot read current directory: {e}"))?;
        let explicit = explicit
            .map(|path| {
                fs::canonicalize(path)
                    .map(|path| absolute(&path, &cwd))
                    .map_err(|e| format!("{}: {e}", path.display()))
            })
            .transpose()?;
        let mut loader = Self {
            explicit,
            no_config,
            cli,
            cwd,
            documents: HashMap::new(),
            discovered: HashMap::new(),
            compiled: HashMap::new(),
            matching_parents: HashMap::new(),
            global: None,
        };
        if let Some(path) = loader.explicit.clone() {
            loader.load(&path, &mut Vec::new())?;
        }
        Ok(loader)
    }

    pub fn for_path(&mut self, path: &Path) -> Result<EffectiveConfig, String> {
        let absolute = absolute(path, &self.cwd);
        let source = if self.no_config {
            None
        } else if self.explicit.is_some() {
            self.explicit.clone()
        } else {
            let directory = absolute.parent().unwrap_or(&self.cwd).to_path_buf();
            match self.discover(&directory)? {
                Some(path) => Some(path),
                None => {
                    if self.global.is_none() {
                        self.global = Some(discover_global(&self.cwd)?);
                    }
                    self.global.clone().flatten()
                }
            }
        };
        let document = match &source {
            Some(path) => Some(self.load(path, &mut Vec::new())?),
            None => None,
        };
        let matched: Vec<usize> = document
            .as_ref()
            .map(|doc| {
                doc.overrides
                    .iter()
                    .enumerate()
                    .filter_map(|(index, rule)| {
                        matches_path(&rule.matcher, &absolute, &mut self.matching_parents)
                            .then_some(index)
                    })
                    .collect()
            })
            .unwrap_or_default();
        let key = (source.clone(), matched.clone());
        let matched_overrides = matched.len();
        if let Some(cached) = self.compiled.get(&key) {
            return Ok(EffectiveConfig {
                config: Arc::clone(&cached.config),
                excluded: matches_path(&cached.excludes, &absolute, &mut self.matching_parents),
                sources: document
                    .as_ref()
                    .map_or_else(Vec::new, |doc| doc.sources.clone()),
                matched_overrides,
            });
        }
        {
            let mut value = document
                .as_ref()
                .map(|doc| doc.value.clone())
                .unwrap_or_else(|| toml::Value::Table(toml::Table::new()));
            if let Some(doc) = &document {
                for index in matched {
                    merge_values(&mut value, doc.overrides[index].value.clone());
                }
            }
            merge_values(&mut value, self.cli.clone());
            normalize_cache_directory(
                value.as_table_mut().expect("configuration table"),
                &self.cwd,
                source.as_deref(),
            )?;
            let config =
                Config::from_value(value).map_err(|e| config_error(source.as_deref(), e))?;
            if config.files.threads == Some(0) {
                return Err(config_error(
                    source.as_deref(),
                    "files.threads must be positive",
                ));
            }
            let root = document
                .as_ref()
                .map_or(self.cwd.as_path(), |doc| doc.exclude_root.as_path());
            let excludes = matcher(root, &config.files.exclude)
                .map_err(|e| config_error(source.as_deref(), e))?;
            let config = Arc::new(
                CompiledConfig::new(config).map_err(|e| config_error(source.as_deref(), e))?,
            );
            let effective = EffectiveConfig {
                config: Arc::clone(&config),
                excluded: matches_path(&excludes, &absolute, &mut self.matching_parents),
                sources: document
                    .as_ref()
                    .map_or_else(Vec::new, |doc| doc.sources.clone()),
                matched_overrides,
            };
            self.compiled.insert(key, CachedConfig { config, excludes });
            Ok(effective)
        }
    }

    fn discover(&mut self, start: &Path) -> Result<Option<PathBuf>, String> {
        if let Some(found) = self.discovered.get(start) {
            return Ok(found.clone());
        }
        let mut directory = start.to_path_buf();
        let mut visited = Vec::new();
        let found = loop {
            if let Some(found) = self.discovered.get(&directory) {
                break found.clone();
            }
            visited.push(directory.clone());
            let candidate = directory.join("tift.toml");
            match fs::metadata(&candidate) {
                Ok(meta) if meta.is_file() => {
                    break Some(
                        fs::canonicalize(&candidate)
                            .map(|path| absolute(&path, &self.cwd))
                            .map_err(|e| format!("{}: {e}", candidate.display()))?,
                    );
                }
                Ok(_) => {
                    return Err(format!(
                        "{}: configuration must be a file",
                        candidate.display()
                    ));
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(format!("{}: {e}", candidate.display())),
            }
            if directory.join(".git").exists() || !directory.pop() {
                break None;
            }
        };
        for directory in visited {
            self.discovered.insert(directory, found.clone());
        }
        Ok(found)
    }

    fn load(&mut self, path: &Path, stack: &mut Vec<PathBuf>) -> Result<Arc<Document>, String> {
        if stack.iter().any(|entry| entry == path) {
            let mut chain = stack
                .iter()
                .map(|entry| entry.display().to_string())
                .collect::<Vec<_>>();
            chain.push(path.display().to_string());
            let chain = chain.join(" -> ");
            return Err(format!("configuration extend cycle: {chain}"));
        }
        if let Some(document) = self.documents.get(path) {
            return Ok(Arc::clone(document));
        }
        let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut local: toml::Value =
            toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        let table = local
            .as_table_mut()
            .ok_or_else(|| config_error(Some(path), "expected a TOML table"))?;
        let extend = table
            .remove("extend")
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| config_error(Some(path), "extend must be a path string"))
            })
            .transpose()?;
        let root = path.parent().unwrap_or(&self.cwd).to_path_buf();
        normalize_cache_directory(table, &root, Some(path))?;
        let local_excludes = table
            .get("files")
            .and_then(toml::Value::as_table)
            .is_some_and(|files| files.contains_key("exclude"));
        let local_overrides = table.contains_key("overrides");
        stack.push(path.to_path_buf());
        let inherited = if let Some(extend) = extend {
            let extended = fs::canonicalize(root.join(&extend))
                .map(|path| absolute(&path, &self.cwd))
                .map_err(|e| config_error(Some(path), format!("cannot extend {extend:?}: {e}")))?;
            Some(self.load(&extended, stack)?)
        } else {
            None
        };
        stack.pop();
        let exclude_root = if local_excludes {
            root.clone()
        } else {
            inherited
                .as_ref()
                .map_or_else(|| root.clone(), |doc| doc.exclude_root.clone())
        };
        let overrides_root = if local_overrides {
            root.clone()
        } else {
            inherited
                .as_ref()
                .map_or_else(|| root.clone(), |doc| doc.overrides_root.clone())
        };
        let mut value = inherited.as_ref().map_or_else(
            || toml::Value::Table(toml::Table::new()),
            |doc| doc.raw_value.clone(),
        );
        merge_values(&mut value, local);
        let raw_value = value.clone();
        let raw_overrides = value
            .as_table_mut()
            .expect("TOML table")
            .remove("overrides");
        let mut overrides = Vec::new();
        if let Some(raw) = raw_overrides {
            let array = raw
                .as_array()
                .ok_or_else(|| config_error(Some(path), "overrides must be an array of tables"))?;
            for (index, rule_value) in array.iter().enumerate() {
                let mut table = rule_value.as_table().cloned().ok_or_else(|| {
                    config_error(Some(path), format!("overrides[{index}] must be a table"))
                })?;
                let patterns = table
                    .remove("files")
                    .and_then(|value| value.as_array().cloned())
                    .ok_or_else(|| {
                        config_error(
                            Some(path),
                            format!("overrides[{index}].files must be an array of patterns"),
                        )
                    })?;
                if patterns.is_empty() {
                    return Err(config_error(
                        Some(path),
                        format!("overrides[{index}].files must not be empty"),
                    ));
                }
                let patterns = patterns
                    .iter()
                    .map(|value| {
                        value
                            .as_str()
                            .filter(|value| !value.is_empty())
                            .map(str::to_owned)
                            .ok_or_else(|| {
                                config_error(
                                    Some(path),
                                    format!(
                                        "overrides[{index}].files must contain nonempty strings"
                                    ),
                                )
                            })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                if table.contains_key("extend") || table.contains_key("overrides") {
                    return Err(config_error(
                        Some(path),
                        format!(
                            "overrides[{index}] cannot extend configs or define nested overrides"
                        ),
                    ));
                }
                normalize_cache_directory(&mut table, &overrides_root, Some(path))?;
                let overlay = toml::Value::Table(table);
                // Validate every override, including ones that do not match the
                // current file, with the inherited base as its starting point.
                let mut validation = value.clone();
                merge_values(&mut validation, overlay.clone());
                merge_values(&mut validation, self.cli.clone());
                let validation = Config::from_value(validation)
                    .map_err(|e| config_error(Some(path), format!("overrides[{index}]: {e}")))?;
                CompiledConfig::new(validation)
                    .map_err(|e| config_error(Some(path), format!("overrides[{index}]: {e}")))?;
                overrides.push(Override {
                    matcher: matcher(&overrides_root, &patterns)
                        .map_err(|e| config_error(Some(path), e))?,
                    value: overlay,
                });
            }
        }
        Config::from_value(value.clone()).map_err(|e| config_error(Some(path), e))?;
        let mut sources = inherited
            .as_ref()
            .map_or_else(Vec::new, |doc| doc.sources.clone());
        sources.push(path.to_path_buf());
        let document = Arc::new(Document {
            value,
            raw_value,
            exclude_root,
            overrides_root,
            overrides,
            sources,
        });
        self.documents
            .insert(path.to_path_buf(), Arc::clone(&document));
        Ok(document)
    }
}

fn normalize_cache_directory(
    table: &mut toml::Table,
    root: &Path,
    source: Option<&Path>,
) -> Result<(), String> {
    let Some(value) = table
        .get_mut("cache")
        .and_then(toml::Value::as_table_mut)
        .and_then(|cache| cache.get_mut("directory"))
    else {
        return Ok(());
    };
    let Some(directory) = value.as_str() else {
        return Err(config_error(
            source,
            "cache.directory must be a path string",
        ));
    };
    if directory.is_empty() {
        return Err(config_error(source, "cache.directory must not be empty"));
    }
    *value = toml::Value::String(
        absolute(Path::new(directory), root)
            .to_string_lossy()
            .into_owned(),
    );
    Ok(())
}

/// Personal defaults are a fallback when no project configuration is found.
/// An explicit directory also makes discovery reproducible in editors and CI.
fn discover_global(cwd: &Path) -> Result<Option<PathBuf>, String> {
    let directory = if let Some(directory) = std::env::var_os("TIFT_CONFIG_HOME") {
        if directory.is_empty() {
            return Ok(None);
        }
        Some(PathBuf::from(directory))
    } else {
        platform_config_directory()
    };
    let Some(directory) = directory else {
        return Ok(None);
    };
    let candidate = absolute(&directory.join("tift.toml"), cwd);
    match fs::metadata(&candidate) {
        Ok(meta) if meta.is_file() => fs::canonicalize(&candidate)
            .map(|path| Some(absolute(&path, cwd)))
            .map_err(|error| format!("{}: {error}", candidate.display())),
        Ok(_) => Err(format!(
            "{}: global configuration must be a file",
            candidate.display()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("{}: {error}", candidate.display())),
    }
}

fn platform_config_directory() -> Option<PathBuf> {
    let variable = |name| {
        std::env::var_os(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    #[cfg(windows)]
    {
        variable("APPDATA")
            .or_else(|| variable("USERPROFILE").map(|home| home.join("AppData/Roaming")))
            .map(|directory| directory.join("tift"))
    }
    #[cfg(not(windows))]
    {
        if let Some(directory) = variable("XDG_CONFIG_HOME") {
            return Some(directory.join("tift"));
        }
        #[cfg(target_os = "macos")]
        let suffix = "Library/Application Support/tift";
        #[cfg(not(target_os = "macos"))]
        let suffix = ".config/tift";
        variable("HOME").map(|home| home.join(suffix))
    }
}

fn config_error(path: Option<&Path>, error: impl std::fmt::Display) -> String {
    match path {
        Some(path) => format!("{}: {error}", path.display()),
        None => format!("configuration: {error}"),
    }
}

fn matcher(root: &Path, patterns: &[String]) -> Result<Gitignore, String> {
    let mut builder = GitignoreBuilder::new(root);
    for pattern in patterns {
        builder
            .add_line(None, pattern)
            .map_err(|e| format!("invalid file pattern {pattern:?}: {e}"))?;
    }
    builder.build().map_err(|e| e.to_string())
}

fn matches_path(
    matcher: &Gitignore,
    path: &Path,
    parents: &mut HashMap<PathBuf, Option<PathBuf>>,
) -> bool {
    // Explicit configurations may live outside the input directory. Patterns
    // only address descendants of their declaring configuration's directory.
    if let Ok(relative) = path.strip_prefix(matcher.path()) {
        return matcher
            .matched_path_or_any_parents(relative, false)
            .is_ignore();
    }
    let (Some(parent), Some(filename)) = (path.parent(), path.file_name()) else {
        return false;
    };
    // Directory aliases must share the configuration's canonical root, while
    // a symlinked file keeps its input name for relative pattern matching.
    let canonical = parents.entry(parent.to_path_buf()).or_insert_with(|| {
        fs::canonicalize(parent)
            .ok()
            .map(|canonical| absolute(&canonical, parent))
    });
    canonical.as_ref().is_some_and(|parent| {
        parent
            .join(filename)
            .strip_prefix(matcher.path())
            .ok()
            .is_some_and(|relative| {
                matcher
                    .matched_path_or_any_parents(relative, false)
                    .is_ignore()
            })
    })
}

pub fn absolute(path: &Path, cwd: &Path) -> PathBuf {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    let mut result = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            #[cfg(windows)]
            Component::Prefix(prefix) => match prefix.kind() {
                std::path::Prefix::VerbatimDisk(letter) => {
                    result.push(format!("{}:", char::from(letter)))
                }
                std::path::Prefix::VerbatimUNC(server, share) => {
                    let mut unc = std::ffi::OsString::from("\\\\");
                    unc.push(server);
                    unc.push("\\");
                    unc.push(share);
                    result.push(unc);
                }
                _ => result.push(component.as_os_str()),
            },
            _ => result.push(component.as_os_str()),
        }
    }
    result
}



//! Best-effort CLI result storage. Project configuration is prepared afresh
//! before lookup; in-process analysis reuse is independent of this storage.
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use tift::{CompiledConfig, Diagnostic, FileKind, FormatOutput, Severity};

const SCHEMA: u32 = 2;
const MAX_ENTRY_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Default)]
pub struct PersistentCache {
    configs: Mutex<HashMap<usize, Arc<Options>>>,
    groups: HashMap<(blake3::Hash, blake3::Hash), Arc<Group>>,
    default_directory: OnceLock<Option<PathBuf>>,
}

struct Options {
    _config: Arc<CompiledConfig>,
    fingerprint: OnceLock<Option<blake3::Hash>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Bypassed,
    Hit,
    Miss,
}

pub struct Request {
    slot: Slot,
    key: String,
}

pub struct Slot {
    group: Arc<Group>,
    name: String,
    kind: u8,
}

impl Slot {
    pub fn may_hit(&self) -> bool {
        self.group.entries().contains_key(&self.name)
    }
}

struct Group {
    path: PathBuf,
    entries: OnceLock<BTreeMap<String, Entry>>,
    pending: Mutex<BTreeMap<String, Entry>>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GroupFile {
    schema: u32,
    entries: BTreeMap<String, Entry>,
}

#[derive(Serialize)]
struct GroupWrite<'a> {
    schema: u32,
    entries: &'a BTreeMap<String, Entry>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    schema: u32,
    key: String,
    text: String,
    diagnostics: Vec<StoredDiagnostic>,
    checksum: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredDiagnostic {
    severity: u8,
    message: String,
    start: usize,
    end: usize,
}

impl PersistentCache {
    pub fn slot(&mut self, path: &Path, kind: FileKind, config: &CompiledConfig) -> Option<Slot> {
        if !config.config.cache.enabled {
            return None;
        }
        let directory = config.config.cache.directory.as_deref().or_else(|| {
            self.default_directory
                .get_or_init(default_directory)
                .as_deref()
        })?;
        let kind = match kind {
            FileKind::Tex => 0,
            FileKind::Style => 1,
            FileKind::Class => 2,
            FileKind::Bibtex => 3,
            FileKind::Dtx => 4,
            FileKind::Ins => 5,
            FileKind::Tikz => 6,
            FileKind::Bst => 7,
        };
        let mut slot = blake3::Hasher::new();
        field(&mut slot, path.as_os_str().as_encoded_bytes());
        slot.update(&[kind]);
        let parent = path.parent().unwrap_or(path);
        let group_name = blake3::hash(parent.as_os_str().as_encoded_bytes());
        let directory_id = blake3::hash(directory.as_os_str().as_encoded_bytes());
        let group = self
            .groups
            .entry((directory_id, group_name))
            .or_insert_with(|| {
                Arc::new(Group {
                    path: directory
                        .join("v2")
                        .join(format!("{}.json", group_name.to_hex())),
                    entries: OnceLock::new(),
                    pending: Mutex::default(),
                })
            });
        Some(Slot {
            group: Arc::clone(group),
            name: slot.finalize().to_hex().to_string(),
            kind,
        })
    }

    fn config_fingerprint(&self, config: &Arc<CompiledConfig>) -> Option<blake3::Hash> {
        let identity = Arc::as_ptr(config) as usize;
        let options = Arc::clone(
            self.configs
                .lock()
                .ok()?
                .entry(identity)
                .or_insert_with(|| {
                    Arc::new(Options {
                        _config: Arc::clone(config),
                        fingerprint: OnceLock::new(),
                    })
                }),
        );
        *options.fingerprint.get_or_init(|| {
            let mut options = config.config.clone();
            options.cache = Default::default();
            let mut fingerprint = blake3::Hasher::new();
            serde_json::to_writer(&mut fingerprint, &options).ok()?;
            // Imported bindings affect formatting while remaining absent from
            // serialized user configuration. Include them in result identity.
            fingerprint.update(b"project-environment-aliases");
            serde_json::to_writer(
                &mut fingerprint,
                &options.syntax.inferred_environment_aliases,
            )
            .ok()?;
            fingerprint.update(b"project-environment-alias-events");
            serde_json::to_writer(&mut fingerprint, &options.syntax.environment_alias_events)
                .ok()?;
            fingerprint.update(b"project-environment-arguments");
            serde_json::to_writer(
                &mut fingerprint,
                &options.syntax.inferred_environment_arguments,
            )
            .ok()?;
            fingerprint.update(b"project-environment-argument-events");
            serde_json::to_writer(
                &mut fingerprint,
                &options.syntax.environment_argument_events,
            )
            .ok()?;
            fingerprint.update(b"project-package-load-events");
            serde_json::to_writer(&mut fingerprint, &options.syntax.package_load_events).ok()?;
            fingerprint.update(b"project-command-import-events");
            serde_json::to_writer(&mut fingerprint, &options.syntax.command_import_events).ok()?;
            Some(fingerprint.finalize())
        })
    }

    pub fn request(
        &self,
        slot: Slot,
        source: &str,
        config: &Arc<CompiledConfig>,
        input_graph: Option<blake3::Hash>,
    ) -> Option<Request> {
        let engine = engine_hash()?;
        let mut hash = blake3::Hasher::new();
        hash.update(b"tift-result-cache-v3");
        hash.update(engine.as_bytes());
        hash.update(self.config_fingerprint(config)?.as_bytes());
        hash.update(&[slot.kind]);
        field(
            &mut hash,
            input_graph.as_ref().map_or(&[], |graph| graph.as_bytes()),
        );
        field(&mut hash, source.as_bytes());
        let key = hash.finalize().to_hex().to_string();
        Some(Request { slot, key })
    }

    pub fn flush(&self) {
        for group in self.groups.values() {
            group.flush();
        }
    }
}

impl Request {
    pub fn load(&self, source: &str) -> Option<FormatOutput> {
        let entry = self.slot.group.entries().get(&self.slot.name)?;
        if entry.schema != SCHEMA || entry.key != self.key || entry.checksum != entry.checksum() {
            return None;
        }
        let mut diagnostics = Vec::with_capacity(entry.diagnostics.len());
        for diagnostic in &entry.diagnostics {
            if diagnostic.severity != 0
                || diagnostic.start > diagnostic.end
                || diagnostic.end > source.len()
                || !source.is_char_boundary(diagnostic.start)
                || !source.is_char_boundary(diagnostic.end)
            {
                return None;
            }
            diagnostics.push(Diagnostic {
                severity: Severity::Warning,
                message: diagnostic.message.clone(),
                span: diagnostic.start..diagnostic.end,
            });
        }
        Some(FormatOutput {
            text: entry.text.clone(),
            diagnostics,
        })
    }

    pub fn store(&self, output: &FormatOutput) {
        if output.text.len() as u64 > MAX_ENTRY_BYTES
            || output
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.severity == Severity::Error)
        {
            return;
        }
        let mut entry = Entry {
            schema: SCHEMA,
            key: self.key.clone(),
            text: output.text.clone(),
            diagnostics: output
                .diagnostics
                .iter()
                .map(|diagnostic| StoredDiagnostic {
                    severity: 0,
                    message: diagnostic.message.clone(),
                    start: diagnostic.span.start,
                    end: diagnostic.span.end,
                })
                .collect(),
            checksum: String::new(),
        };
        entry.checksum = entry.checksum();
        if let Ok(mut pending) = self.slot.group.pending.lock() {
            pending.insert(self.slot.name.clone(), entry);
        }
    }
}

impl Group {
    fn entries(&self) -> &BTreeMap<String, Entry> {
        self.entries
            .get_or_init(|| read_group(&self.path).unwrap_or_default())
    }

    fn flush(&self) {
        let Ok(mut pending) = self.pending.lock() else {
            return;
        };
        if pending.is_empty() {
            return;
        }
        let Some(parent) = self.path.parent() else {
            return;
        };
        if fs::create_dir_all(parent).is_err()
            || fs::symlink_metadata(&self.path).is_ok_and(|metadata| !metadata.is_file())
        {
            return;
        }
        // An OS lock survives neither process exit nor a crash. Keeping its
        // inode in place ensures independent processes lock the same object.
        let lock_path = self.path.with_extension("lock");
        if fs::symlink_metadata(&lock_path).is_ok_and(|metadata| !metadata.is_file()) {
            return;
        }
        let Ok(lock) = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)
        else {
            return;
        };
        if fs2::FileExt::try_lock_exclusive(&lock).is_err() {
            return;
        }
        let mut merged = read_group(&self.path).unwrap_or_default();
        merged.append(&mut pending);
        for (slot, entry) in self.entries() {
            if !merged.contains_key(slot) {
                merged.insert(slot.clone(), entry.clone());
            }
        }
        let file = GroupWrite {
            schema: SCHEMA,
            entries: &merged,
        };
        let Ok(mut temporary) = tempfile::NamedTempFile::new_in(parent) else {
            return;
        };
        let written = {
            let mut writer = BufWriter::new(&mut temporary);
            serde_json::to_writer(&mut writer, &file).is_ok() && writer.flush().is_ok()
        };
        if !written
            || temporary
                .as_file()
                .metadata()
                .map_or(true, |metadata| metadata.len() > MAX_ENTRY_BYTES)
        {
            return;
        }
        let _ = temporary.persist(&self.path);
    }
}

fn read_group(path: &Path) -> Option<BTreeMap<String, Entry>> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_ENTRY_BYTES {
        return None;
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .ok()?
        .take(MAX_ENTRY_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_ENTRY_BYTES {
        return None;
    }
    let file: GroupFile = serde_json::from_slice(&bytes).ok()?;
    (file.schema == SCHEMA).then_some(file.entries)
}

impl Entry {
    fn checksum(&self) -> String {
        let mut hash = blake3::Hasher::new();
        field(&mut hash, self.text.as_bytes());
        for diagnostic in &self.diagnostics {
            hash.update(&[diagnostic.severity]);
            hash.update(&(diagnostic.start as u64).to_le_bytes());
            hash.update(&(diagnostic.end as u64).to_le_bytes());
            field(&mut hash, diagnostic.message.as_bytes());
        }
        hash.finalize().to_hex().to_string()
    }
}

fn field(hash: &mut blake3::Hasher, value: &[u8]) {
    hash.update(&(value.len() as u64).to_le_bytes());
    hash.update(value);
}

pub(super) fn engine_hash() -> Option<blake3::Hash> {
    static ENGINE: OnceLock<Option<blake3::Hash>> = OnceLock::new();
    *ENGINE.get_or_init(|| {
        let mut reader = fs::File::open(std::env::current_exe().ok()?).ok()?;
        let mut hash = blake3::Hasher::new();
        std::io::copy(&mut reader, &mut hash).ok()?;
        Some(hash.finalize())
    })
}

pub(super) fn default_directory() -> Option<PathBuf> {
    let variable = |name| {
        std::env::var_os(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    if let Some(directory) = std::env::var_os("TIFT_CACHE_HOME") {
        return (!directory.is_empty()).then(|| PathBuf::from(directory));
    }
    #[cfg(windows)]
    {
        variable("LOCALAPPDATA")
            .or_else(|| variable("USERPROFILE").map(|profile| profile.join("AppData/Local")))
            .map(|directory| directory.join("tift/cache"))
    }
    #[cfg(target_os = "macos")]
    {
        variable("HOME").map(|directory| directory.join("Library/Caches/tift"))
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        variable("XDG_CACHE_HOME")
            .or_else(|| variable("HOME").map(|directory| directory.join(".cache")))
            .map(|directory| directory.join("tift"))
    }
}

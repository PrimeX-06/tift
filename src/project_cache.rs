use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Serialize, de::DeserializeOwned};
use tift::Config;

const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_SLOTS: usize = 1024;

type Slots = BTreeMap<String, String>;

pub(super) struct ProjectCache {
    path: PathBuf,
    base: blake3::Hasher,
    slots: Slots,
    dirty: bool,
    size: usize,
}

impl ProjectCache {
    pub(super) fn open(root: &Path, config: &Config) -> Option<Self> {
        let directory = config
            .cache
            .directory
            .clone()
            .or_else(super::persistent_cache::default_directory)?;
        let mut base = blake3::Hasher::new();
        base.update(super::persistent_cache::engine_hash()?.as_bytes());
        serde_json::to_writer(&mut base, config).ok()?;
        let root = blake3::hash(root.as_os_str().as_encoded_bytes());
        let path = directory.join("project-v1").join(root.to_hex().as_str());
        let slots = Self::read(&path).unwrap_or_default();
        let size = slots.values().map(|value| 2 * value.len() + 72).sum();
        Some(Self {
            path,
            base,
            slots,
            dirty: false,
            size,
        })
    }

    fn read(path: &Path) -> Option<Slots> {
        let metadata = fs::symlink_metadata(path).ok()?;
        if !metadata.is_file() || metadata.len() > MAX_BYTES as u64 {
            return None;
        }
        let mut bytes = Vec::new();
        fs::File::open(path)
            .ok()?
            .take(MAX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        let (checksum, payload) = bytes.split_at_checked(32)?;
        (bytes.len() <= MAX_BYTES && blake3::hash(payload).as_bytes() == checksum).then_some(())?;
        serde_json::from_slice(payload).ok()
    }

    pub(super) fn key(&self, kind: u8, hashes: &[blake3::Hash]) -> String {
        let mut key = self.base.clone();
        key.update(&[kind]);
        for hash in hashes {
            key.update(hash.as_bytes());
        }
        key.finalize().to_hex().to_string()
    }

    pub(super) fn get<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        serde_json::from_str(self.slots.get(key)?).ok()
    }

    pub(super) fn put(&mut self, key: String, value: &impl Serialize) {
        let Ok(data) = serde_json::to_string(value) else {
            return;
        };
        let entry_bytes = 2 * data.len() + 72;
        if entry_bytes + 34 > MAX_BYTES {
            return;
        }
        if self.size + entry_bytes + 34 > MAX_BYTES || self.slots.len() >= MAX_SLOTS {
            self.slots.clear();
            self.size = 0;
        }
        self.slots.insert(key, data);
        self.size += entry_bytes;
        self.dirty = true;
    }

    pub(super) fn flush(&self) -> Option<()> {
        if !self.dirty {
            return Some(());
        }
        let parent = self.path.parent()?;
        let payload = serde_json::to_vec(&self.slots).ok()?;
        if payload.len() + 32 > MAX_BYTES {
            return None;
        }
        fs::create_dir_all(parent).ok()?;
        let mut temp = tempfile::NamedTempFile::new_in(parent).ok()?;
        temp.write_all(blake3::hash(&payload).as_bytes()).ok()?;
        temp.write_all(&payload).ok()?;
        temp.persist(&self.path).ok()?;
        Some(())
    }
}

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};

use serde::{Serialize, de::DeserializeOwned};
use tift::Config;

const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_SLOTS: usize = 1024;
const MAGIC: &[u8; 8] = b"TIFTPRJ2";
const HEADER_BYTES: usize = MAGIC.len() + blake3::OUT_LEN;
const MAX_PAYLOAD: usize = MAX_BYTES - HEADER_BYTES;
const SLOT_HEADER_BYTES: usize = 2 * size_of::<u32>();

enum Slot {
    OnDisk(Range<usize>),
    Pending(Vec<u8>),
}

impl Slot {
    fn bytes<'a>(&'a self, file: &'a [u8]) -> &'a [u8] {
        match self {
            Self::OnDisk(range) => &file[range.clone()],
            Self::Pending(value) => value,
        }
    }
}

#[derive(Default)]
struct Slots {
    file: Vec<u8>,
    entries: BTreeMap<String, Slot>,
    size: usize,
}

pub(super) struct ProjectCache {
    path: PathBuf,
    base: blake3::Hasher,
    slots: Slots,
    dirty: bool,
}

impl ProjectCache {
    pub(super) fn has_saved_entries(&self) -> bool {
        !self.slots.file.is_empty()
    }

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
        Some(Self {
            path,
            base,
            slots,
            dirty: false,
        })
    }

    fn read(path: &Path) -> Option<Slots> {
        let metadata = fs::symlink_metadata(path).ok()?;
        if !metadata.is_file() || metadata.len() > MAX_BYTES as u64 {
            return None;
        }
        let mut file = Vec::with_capacity(metadata.len() as usize);
        fs::File::open(path)
            .ok()?
            .take(MAX_BYTES as u64 + 1)
            .read_to_end(&mut file)
            .ok()?;
        if file.len() > MAX_BYTES {
            return None;
        }
        let (checksum, payload) = file
            .strip_prefix(MAGIC)?
            .split_first_chunk::<{ blake3::OUT_LEN }>()?;
        if blake3::hash(payload).as_bytes() != checksum {
            return None;
        }
        let size = payload.len();
        let mut rest = payload;
        let mut entries = BTreeMap::new();
        while !rest.is_empty() {
            let key_len = read_length(&mut rest)?;
            let value_len = read_length(&mut rest)?;
            if entries.len() == MAX_SLOTS || value_len == 0 {
                return None;
            }
            let key = std::str::from_utf8(rest.split_off(..key_len)?).ok()?;
            let start = file.len() - rest.len();
            rest.split_off(..value_len)?;
            let slot = Slot::OnDisk(start..start + value_len);
            if entries.insert(key.to_owned(), slot).is_some() {
                return None;
            }
        }
        Some(Slots {
            file,
            entries,
            size,
        })
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
        serde_json::from_slice(self.slots.entries.get(key)?.bytes(&self.slots.file)).ok()
    }

    pub(super) fn put(&mut self, key: String, value: &impl Serialize) {
        let Some(limit) = (MAX_PAYLOAD - SLOT_HEADER_BYTES).checked_sub(key.len()) else {
            return;
        };
        let mut writer = SlotWriter {
            data: Vec::new(),
            limit,
        };
        if serde_json::to_writer(&mut writer, value).is_err() {
            return;
        }
        let slots = &mut self.slots;
        let overhead = SLOT_HEADER_BYTES + key.len();
        let entry_size = overhead + writer.data.len();
        if let Some(previous) = slots.entries.get(&key) {
            slots.size -= overhead + previous.bytes(&slots.file).len();
        } else if slots.entries.len() == MAX_SLOTS {
            *slots = Slots::default();
        }
        if slots.size + entry_size > MAX_PAYLOAD {
            *slots = Slots::default();
        }
        slots.entries.insert(key, Slot::Pending(writer.data));
        slots.size += entry_size;
        self.dirty = true;
    }

    pub(super) fn flush(&self) -> Option<()> {
        self.write_snapshot().ok()
    }

    fn write_snapshot(&self) -> io::Result<()> {
        if !self.dirty {
            return Ok(());
        }
        let parent = self.path.parent().ok_or(io::ErrorKind::InvalidInput)?;
        let mut payload = Vec::with_capacity(self.slots.size);
        for (key, slot) in &self.slots.entries {
            let value = slot.bytes(&self.slots.file);
            payload.extend_from_slice(&(key.len() as u32).to_le_bytes());
            payload.extend_from_slice(&(value.len() as u32).to_le_bytes());
            payload.extend_from_slice(key.as_bytes());
            payload.extend_from_slice(value);
        }
        debug_assert_eq!(payload.len(), self.slots.size);
        fs::create_dir_all(parent)?;
        let mut temp = tempfile::NamedTempFile::new_in(parent)?;
        temp.write_all(MAGIC)?;
        temp.write_all(blake3::hash(&payload).as_bytes())?;
        temp.write_all(&payload)?;
        publish_snapshot(temp.into_temp_path(), &self.path)
    }
}

#[cfg(not(windows))]
fn publish_snapshot(temp: tempfile::TempPath, path: &Path) -> io::Result<()> {
    temp.persist(path).map_err(|error| error.error)
}

#[cfg(windows)]
fn publish_snapshot(mut temp: tempfile::TempPath, path: &Path) -> io::Result<()> {
    let mut delays = [1, 1, 2, 4, 8, 16].into_iter();
    loop {
        match temp.persist(path) {
            Ok(()) => return Ok(()),
            Err(failure) => {
                let Some(delay) = delays
                    .next()
                    .filter(|_| replace_contention(&failure.error, path))
                else {
                    return Err(failure.error);
                };
                temp = failure.path;
                std::thread::sleep(std::time::Duration::from_millis(delay));
            }
        }
    }
}

#[cfg(windows)]
fn replace_contention(error: &io::Error, path: &Path) -> bool {
    use std::os::windows::fs::OpenOptionsExt;

    const DELETE_ACCESS: u32 = 0x0001_0000;
    match error.raw_os_error() {
        Some(32 | 33) => true,
        Some(5)
            if fs::symlink_metadata(path)
                .is_ok_and(|meta| meta.is_file() && !meta.permissions().readonly()) =>
        {
            match fs::OpenOptions::new().access_mode(DELETE_ACCESS).open(path) {
                Ok(_) => true,
                Err(probe) => matches!(probe.raw_os_error(), Some(32 | 33)),
            }
        }
        _ => false,
    }
}

fn read_length(bytes: &mut &[u8]) -> Option<usize> {
    Some(u32::from_le_bytes(*bytes.split_off(..4)?.first_chunk()?) as usize)
}

struct SlotWriter {
    data: Vec<u8>,
    limit: usize,
}

impl Write for SlotWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit - self.data.len() {
            return Err(io::ErrorKind::FileTooLarge.into());
        }
        self.data.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

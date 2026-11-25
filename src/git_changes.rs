//! Select worktree lines against the requested Git tree and the exact formatting input.

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;

use super::{config_io, source_encoding::SourceFile};
use tift::LineRanges;

pub(super) struct Changes {
    root: PathBuf,
    // A new file has no base blob. Renamed files keep their original blob.
    files: HashMap<PathBuf, Option<String>>,
}

fn git(dir: &Path, args: &[impl AsRef<OsStr>]) -> Result<Vec<u8>, String> {
    let output = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .map_err(|e| format!("--changed-since: cannot run git in {}: {e}", dir.display()))?;
    if !output.status.success() {
        return Err(format!(
            "--changed-since: {}",
            String::from_utf8_lossy(&output.stderr).trim_end()
        ));
    }
    Ok(output.stdout)
}

#[cfg(unix)]
fn git_path(bytes: &[u8]) -> Result<&Path, String> {
    use std::os::unix::ffi::OsStrExt;
    Ok(Path::new(std::ffi::OsStr::from_bytes(bytes)))
}

#[cfg(not(unix))]
fn git_path(bytes: &[u8]) -> Result<&Path, String> {
    std::str::from_utf8(bytes)
        .map(Path::new)
        .map_err(|e| format!("invalid Git path: {e}"))
}

impl Changes {
    pub fn load(revision: &str, paths: &[PathBuf]) -> Result<Self, String> {
        let first = &paths[0];
        let directory = if first.is_dir() {
            first.as_path()
        } else {
            first.parent().unwrap_or(first)
        };
        let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
        let directory = config_io::absolute(directory, &cwd);
        let cdup = git(&directory, &["rev-parse", "--show-cdup"])?;
        let root = std::fs::canonicalize(
            directory.join(
                std::str::from_utf8(&cdup)
                    .map_err(|e| e.to_string())?
                    .trim_end(),
            ),
        )
        .map_err(|e| format!("cannot resolve Git worktree: {e}"))?;
        for path in paths {
            let absolute =
                std::fs::canonicalize(path).map_err(|e| format!("{}: {e}", path.display()))?;
            if !absolute.starts_with(&root) {
                return Err(format!(
                    "{} is outside Git worktree {}",
                    path.display(),
                    root.display()
                ));
            }
        }
        let tree = git(
            &root,
            &[
                "rev-parse",
                "--verify",
                "--end-of-options",
                &format!("{revision}^{{tree}}"),
            ],
        )?;
        let tree = std::str::from_utf8(&tree)
            .map_err(|e| e.to_string())?
            .trim_end();
        let raw = git(
            &root,
            &[
                "diff",
                "--raw",
                "-z",
                "--no-abbrev",
                "--no-ext-diff",
                "--no-textconv",
                "--find-renames",
                "--ignore-submodules=all",
                tree,
                "--",
            ],
        )?;
        let untracked = git(&root, &["ls-files", "--others", "--exclude-standard", "-z"])?;
        let mut files = HashMap::new();
        for path in untracked.split(|&b| b == 0).filter(|p| !p.is_empty()) {
            files.insert(git_path(path)?.to_owned(), None);
        }

        let mut records = raw.split(|&b| b == 0).filter(|field| !field.is_empty());
        while let Some(header) = records.next() {
            let malformed = || "invalid Git raw diff record".to_owned();
            let header = std::str::from_utf8(header).map_err(|_| malformed())?;
            let fields: Vec<_> = header.trim_start_matches(':').split(' ').collect();
            let [old_mode, _, old_blob, _, status] = fields.as_slice() else {
                return Err(malformed());
            };
            let mut path = git_path(records.next().ok_or_else(malformed)?)?;
            if status.starts_with(['R', 'C']) {
                path = git_path(records.next().ok_or_else(malformed)?)?;
            }
            if *status == "D" && !files.contains_key(path) {
                continue;
            }
            let base = old_mode.starts_with("100").then(|| (*old_blob).to_owned());
            files.insert(path.to_owned(), base);
        }
        Ok(Self { root, files })
    }

    pub fn contains(&self, canonical: &Path) -> bool {
        canonical
            .strip_prefix(&self.root)
            .is_ok_and(|relative| self.files.contains_key(relative))
    }

    pub fn ranges(&self, canonical: &Path, current: &str) -> Result<Option<LineRanges>, String> {
        match canonical
            .strip_prefix(&self.root)
            .ok()
            .and_then(|p| self.files.get_key_value(p))
        {
            Some((path, Some(blob))) => {
                // Compare checkout bytes on both sides, including working-tree-encoding.
                let mut path_arg = OsString::from("--path=");
                path_arg.push(path);
                let args: [&OsStr; 4] = [
                    "cat-file".as_ref(),
                    "--filters".as_ref(),
                    &path_arg,
                    blob.as_ref(),
                ];
                let base = SourceFile::decode(git(&self.root, &args)?)
                    .map_err(|error| format!("{}: Git base: {error}", canonical.display()))?;
                Ok(Some(LineRanges::changed_since(&base.text, current)))
            }
            Some((_, None)) => Ok(None),
            None => Ok(Some(LineRanges::default())),
        }
    }
}

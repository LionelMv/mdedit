//! File watching on top of `notify`.
//!
//! Files are watched through their parent directory (non-recursive) so atomic saves by other
//! programs (write temp + rename, which replaces the inode) keep being noticed. Directories
//! are watched recursively and report any changed path below them.

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher as _};

/// Watches files/folders; poll `changed()` once per frame. STABLE.
pub struct Watcher {
    inner: RecommendedWatcher,
    rx: Receiver<notify::Result<notify::Event>>,
    /// Normalized file path → path as given by the caller.
    files: HashMap<PathBuf, PathBuf>,
    /// Parent dirs watched for files → number of watched files in them.
    parents: HashMap<PathBuf, usize>,
    /// Normalized dir path → path as given.
    dirs: HashMap<PathBuf, PathBuf>,
}

impl std::fmt::Debug for Watcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Watcher").field("files", &self.files.values()).field("dirs", &self.dirs.values()).finish()
    }
}

/// Absolute, symlink-resolved form of `path` (resolving the parent if the file is missing).
fn normalize(path: &Path) -> PathBuf {
    if let Ok(p) = std::fs::canonicalize(path) {
        return p;
    }
    let abs = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    match (abs.parent(), abs.file_name()) {
        (Some(parent), Some(name)) => std::fs::canonicalize(parent).map(|p| p.join(name)).unwrap_or(abs),
        _ => abs,
    }
}

impl Watcher {
    pub fn new() -> io::Result<Self> {
        let (tx, rx) = channel();
        let inner = notify::recommended_watcher(tx).map_err(io::Error::other)?;
        Ok(Self { inner, rx, files: HashMap::new(), parents: HashMap::new(), dirs: HashMap::new() })
    }

    /// Start watching a file or directory. Errors are logged (the path may not exist).
    pub fn watch(&mut self, path: &Path) {
        let norm = normalize(path);
        if norm.is_dir() {
            if self.dirs.contains_key(&norm) {
                return;
            }
            match self.inner.watch(&norm, RecursiveMode::Recursive) {
                Ok(()) => {
                    self.dirs.insert(norm, path.to_path_buf());
                }
                Err(e) => log::warn!("cannot watch {}: {e}", path.display()),
            }
            return;
        }
        if self.files.contains_key(&norm) {
            return;
        }
        let Some(parent) = norm.parent().map(Path::to_path_buf) else { return };
        let count = self.parents.get(&parent).copied().unwrap_or(0);
        // A recursive dir watch on the same path already covers it; don't downgrade it.
        if count == 0
            && !self.dirs.contains_key(&parent)
            && let Err(e) = self.inner.watch(&parent, RecursiveMode::NonRecursive)
        {
            log::warn!("cannot watch {}: {e}", path.display());
            return;
        }
        self.parents.insert(parent, count + 1);
        self.files.insert(norm, path.to_path_buf());
    }

    pub fn unwatch(&mut self, path: &Path) {
        let norm = normalize(path);
        if self.dirs.remove(&norm).is_some() {
            let _ = self.inner.unwatch(&norm);
            if self.parents.contains_key(&norm) {
                // Files in this dir are still watched individually.
                let _ = self.inner.watch(&norm, RecursiveMode::NonRecursive);
            }
            return;
        }
        // The file may have been deleted since; fall back to matching the original spelling.
        let key = if self.files.contains_key(&norm) {
            Some(norm)
        } else {
            self.files.iter().find(|(_, orig)| orig.as_path() == path).map(|(k, _)| k.clone())
        };
        let Some(key) = key else { return };
        self.files.remove(&key);
        if let Some(parent) = key.parent()
            && let Some(c) = self.parents.get_mut(parent)
        {
            *c = c.saturating_sub(1);
            if *c == 0 {
                self.parents.remove(parent);
                if !self.dirs.contains_key(parent) {
                    let _ = self.inner.unwatch(parent);
                }
            }
        }
    }

    /// Paths that changed since the last call (deduplicated). Never blocks.
    ///
    /// For watched files, the path is returned as it was passed to [`Self::watch`]; for
    /// watched directories, the changed path below the directory is returned.
    pub fn changed(&mut self) -> Vec<PathBuf> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        while let Ok(res) = self.rx.try_recv() {
            let event = match res {
                Ok(ev) => ev,
                Err(e) => {
                    log::warn!("file watcher error: {e}");
                    continue;
                }
            };
            if matches!(event.kind, EventKind::Access(_)) {
                continue;
            }
            for p in event.paths {
                let hit = if let Some(orig) = self.files.get(&p) {
                    Some(orig.clone())
                } else if self.dirs.keys().any(|d| p.starts_with(d)) {
                    Some(p)
                } else {
                    None
                };
                if let Some(h) = hit
                    && seen.insert(h.clone())
                {
                    out.push(h);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn wait_for(w: &mut Watcher, want: &Path) -> Vec<PathBuf> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut all = Vec::new();
        while Instant::now() < deadline {
            all.extend(w.changed());
            if all.iter().any(|p| p == want) {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        all
    }

    #[test]
    fn file_changes_and_atomic_replace() {
        let t = tempfile::tempdir().unwrap();
        let f = t.path().join("a.md");
        let other = t.path().join("other.md");
        std::fs::write(&f, "1").unwrap();
        std::fs::write(&other, "1").unwrap();
        let mut w = Watcher::new().unwrap();
        w.watch(&f);
        w.watch(&f);
        assert!(w.changed().is_empty());
        std::fs::write(&f, "2").unwrap();
        std::fs::write(&other, "2").unwrap();
        let got = wait_for(&mut w, &f);
        assert_eq!(got.iter().filter(|p| *p == &f).count(), 1, "{got:?}");
        assert!(!got.contains(&other));
        // Atomic replace (rename over) is still seen.
        std::thread::sleep(Duration::from_millis(50));
        let _ = w.changed();
        mdedit_buffer::fsutil::atomic_write(&f, b"3", None).unwrap();
        assert!(wait_for(&mut w, &f).contains(&f));
        // After unwatch, nothing more.
        w.unwatch(&f);
        std::thread::sleep(Duration::from_millis(50));
        let _ = w.changed();
        std::fs::write(&f, "4").unwrap();
        std::thread::sleep(Duration::from_millis(300));
        assert!(!w.changed().contains(&f));
    }

    #[test]
    fn dir_watch_recursive() {
        let t = tempfile::tempdir().unwrap();
        std::fs::create_dir(t.path().join("sub")).unwrap();
        let mut w = Watcher::new().unwrap();
        w.watch(t.path());
        let f = std::fs::canonicalize(t.path()).unwrap().join("sub/new.md");
        std::fs::write(&f, "x").unwrap();
        assert!(wait_for(&mut w, &f).contains(&f));
        w.unwatch(t.path());
        w.watch(&t.path().join("missing-file.md")); // no panic on missing files
    }
}

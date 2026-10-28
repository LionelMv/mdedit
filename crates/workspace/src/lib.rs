//! App-level persistence and files: config, recent files, crash recovery, folder tree,
//! folder search, file watching. No egui.
//!
//! Paths follow XDG via `directories` (`~/.config/mdedit`, `~/.local/state/mdedit`).
//! Files this crate writes that may hold document text are created with mode 0600.
//!
//! # Base directories and `MDEDIT_HOME`
//!
//! | What | Default | With `MDEDIT_HOME=/x` |
//! |---|---|---|
//! | config (`config.toml`) | `~/.config/mdedit` | `/x/config` |
//! | state (`recent.toml`, `recovery/`) | `~/.local/state/mdedit` | `/x/state` |
//! | data | `~/.local/share/mdedit` | `/x/data` |
//!
//! If the environment variable `MDEDIT_HOME` is set to a non-empty value, all XDG dirs are
//! replaced by subdirectories of it (useful for tests and portable setups). Every function that
//! uses a default location also has a `*_in` / `*_from` / `*_to` variant taking explicit paths.
//!
//! OWNER: agent "buffer" (also owns the buffer crate). STABLE items are used by the app.

use std::io;
use std::path::{Path, PathBuf};

pub mod recovery;
mod tree;
mod watch;

pub use tree::{MAX_DEPTH, MAX_ENTRIES, MAX_HITS, MAX_SEARCH_FILE_SIZE, is_markdown_path, scan_tree, search_folder};
pub use watch::Watcher;

/// Environment variable that, if set, replaces all base directories (see crate docs).
pub const HOME_ENV: &str = "MDEDIT_HOME";

fn home_override() -> Option<PathBuf> {
    std::env::var_os(HOME_ENV).filter(|v| !v.is_empty()).map(PathBuf::from)
}

fn project_dirs() -> Option<directories::ProjectDirs> {
    directories::ProjectDirs::from("io.github", "mdedit", "mdedit")
}

/// Directory holding `config.toml`. `None` if no home directory can be determined.
pub fn config_dir() -> Option<PathBuf> {
    match home_override() {
        Some(h) => Some(h.join("config")),
        None => project_dirs().map(|d| d.config_dir().to_path_buf()),
    }
}

/// Directory for state (recent files, recovery snapshots).
pub fn state_dir() -> Option<PathBuf> {
    match home_override() {
        Some(h) => Some(h.join("state")),
        None => project_dirs().map(|d| d.state_dir().unwrap_or_else(|| d.data_local_dir()).to_path_buf()),
    }
}

/// Directory for data files.
pub fn data_dir() -> Option<PathBuf> {
    match home_override() {
        Some(h) => Some(h.join("data")),
        None => project_dirs().map(|d| d.data_dir().to_path_buf()),
    }
}

fn no_dir() -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, "no home directory for mdedit configuration")
}

/// Path of the config file.
pub fn config_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("config.toml"))
}

/// Path of the recent-files list.
pub fn recent_path() -> Option<PathBuf> {
    state_dir().map(|d| d.join("recent.toml"))
}

/// Create `dir` (and parents). On unix, a newly created leaf gets `mode`.
fn create_dir(dir: &Path, mode: u32) -> io::Result<()> {
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        b.mode(mode);
    }
    #[cfg(not(unix))]
    let _ = mode;
    b.create(dir)
}

fn write_atomic(path: &Path, data: &[u8], mode: Option<u32>) -> io::Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        create_dir(parent, 0o700)?;
    }
    mdedit_buffer::fsutil::atomic_write(path, data, mode)
}

/// User preferences, stored as TOML. STABLE fields (add more freely; use `#[serde(default)]`).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Config {
    /// "light", "dark" or "system".
    pub theme: String,
    pub body_size: f32,
    pub code_size: f32,
    pub line_height: f32,
    pub max_width: f32,
    /// Start in source mode instead of live preview.
    pub source_mode: bool,
    pub autosave_secs: u64,
    /// Folder (relative to the document) where pasted images go.
    pub image_folder: String,
    /// Load `http(s)` images in the editor (privacy: off by default).
    pub remote_images: bool,
    pub show_sidebar: bool,
    pub last_folder: Option<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: "system".into(),
            body_size: 16.0,
            code_size: 14.0,
            line_height: 1.5,
            max_width: 820.0,
            source_mode: false,
            autosave_secs: 30,
            image_folder: "assets".into(),
            remote_images: false,
            show_sidebar: true,
            last_folder: None,
        }
    }
}

impl Config {
    /// Load from the config dir; missing or invalid file → defaults (logged). STABLE.
    pub fn load() -> Self {
        match config_path() {
            Some(p) => Self::load_from(&p),
            None => {
                log::warn!("no config directory; using default settings");
                Self::default()
            }
        }
    }
    /// Save to the config dir (atomic write). STABLE.
    pub fn save(&self) -> io::Result<()> {
        self.save_to(&config_path().ok_or_else(no_dir)?)
    }
    /// Load from an explicit file; missing or invalid → defaults (logged). (Added API.)
    pub fn load_from(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(s) => toml::from_str(&s).unwrap_or_else(|e| {
                log::warn!("invalid config {}: {e}; using defaults", path.display());
                Self::default()
            }),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Self::default(),
            Err(e) => {
                log::warn!("cannot read config {}: {e}; using defaults", path.display());
                Self::default()
            }
        }
    }
    /// Save to an explicit file (atomic write, parent dirs created). (Added API.)
    pub fn save_to(&self, path: &Path) -> io::Result<()> {
        let s = toml::to_string_pretty(self).map_err(io::Error::other)?;
        write_atomic(path, s.as_bytes(), None)
    }
}

/// Most-recently-used files, max 20, persisted. STABLE.
#[derive(Clone, Debug, Default)]
pub struct RecentFiles {
    pub files: Vec<PathBuf>,
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct RecentStore {
    #[serde(default)]
    files: Vec<PathBuf>,
}

impl RecentFiles {
    /// Maximum number of entries.
    pub const MAX: usize = 20;

    /// Load from the state dir; entries whose files no longer exist are dropped.
    pub fn load() -> Self {
        recent_path().map(|p| Self::load_from(&p)).unwrap_or_default()
    }
    /// Move `path` to the front (canonicalized) and save.
    pub fn add(&mut self, path: &Path) {
        self.push(path);
        if let Err(e) = self.save() {
            log::warn!("cannot save recent files: {e}");
        }
    }
    /// Save to the state dir.
    pub fn save(&self) -> io::Result<()> {
        self.save_to(&recent_path().ok_or_else(no_dir)?)
    }
    /// Move `path` to the front (canonicalized, deduplicated, max 20) without saving.
    /// (Added API.)
    pub fn push(&mut self, path: &Path) {
        let p = canonical(path);
        self.files.retain(|f| f != &p);
        self.files.insert(0, p);
        self.files.truncate(Self::MAX);
    }
    /// Remove `path` (e.g. after it failed to open) without saving. (Added API.)
    pub fn remove(&mut self, path: &Path) {
        let p = canonical(path);
        self.files.retain(|f| f != &p && f != path);
    }
    /// Load from an explicit file. (Added API.)
    pub fn load_from(path: &Path) -> Self {
        let store: RecentStore = match std::fs::read_to_string(path) {
            Ok(s) => toml::from_str(&s).unwrap_or_else(|e| {
                log::warn!("invalid recent-files list {}: {e}", path.display());
                RecentStore::default()
            }),
            Err(_) => RecentStore::default(),
        };
        let mut files: Vec<PathBuf> = Vec::new();
        for f in store.files {
            if f.exists() && !files.contains(&f) {
                files.push(f);
            }
        }
        files.truncate(Self::MAX);
        Self { files }
    }
    /// Save to an explicit file (atomic, mode 0600). (Added API.)
    pub fn save_to(&self, path: &Path) -> io::Result<()> {
        // Non-UTF-8 paths can't be stored in TOML; skip them rather than failing.
        let files = self.files.iter().filter(|p| p.to_str().is_some()).cloned().collect();
        let s = toml::to_string_pretty(&RecentStore { files }).map_err(io::Error::other)?;
        write_atomic(path, s.as_bytes(), Some(0o600))
    }
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf()))
}

/// One entry of the folder sidebar. STABLE.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeEntry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub children: Vec<TreeEntry>,
}

/// Folder search hit. STABLE.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchHit {
    pub path: PathBuf,
    /// 0-based line number (same convention as `mdedit_buffer::Buffer`).
    pub line: usize,
    /// The line's text, trimmed and truncated to ~200 chars.
    pub text: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_round_trip_and_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("sub/config.toml");
        assert_eq!(Config::load_from(&p), Config::default());
        let c = Config { theme: "dark".into(), body_size: 18.5, last_folder: Some("/tmp/x".into()), ..Config::default() };
        c.save_to(&p).unwrap();
        assert_eq!(Config::load_from(&p), c);
        // Partial file: missing fields take defaults.
        std::fs::write(&p, "theme = \"light\"\nunknown_key = 3\n").unwrap();
        let l = Config::load_from(&p);
        assert_eq!(l.theme, "light");
        assert_eq!(l.autosave_secs, 30);
        // Invalid → defaults.
        std::fs::write(&p, "theme = [[[").unwrap();
        assert_eq!(Config::load_from(&p), Config::default());
    }

    #[test]
    fn recent_files_behaviour() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("state/recent.toml");
        let mut r = RecentFiles::default();
        let mut paths = Vec::new();
        for i in 0..25 {
            let p = dir.path().join(format!("f{i}.md"));
            std::fs::write(&p, "x").unwrap();
            paths.push(p);
        }
        for p in &paths {
            r.push(p);
        }
        assert_eq!(r.files.len(), RecentFiles::MAX);
        assert_eq!(r.files[0], std::fs::canonicalize(&paths[24]).unwrap());
        // Dedup through a non-canonical spelling.
        let alt = dir.path().join(".").join("f10.md");
        r.push(&alt);
        assert_eq!(r.files.len(), RecentFiles::MAX);
        assert_eq!(r.files[0], std::fs::canonicalize(&paths[10]).unwrap());
        assert_eq!(r.files.iter().filter(|f| f.ends_with("f10.md")).count(), 1);
        r.save_to(&store).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&store).unwrap().permissions().mode() & 0o777, 0o600);
        }
        std::fs::remove_file(&paths[10]).unwrap();
        let l = RecentFiles::load_from(&store);
        assert_eq!(l.files.len(), RecentFiles::MAX - 1);
        assert!(!l.files.iter().any(|f| f.ends_with("f10.md")));
        let mut l = l;
        l.remove(&paths[24]);
        assert_eq!(l.files.len(), RecentFiles::MAX - 2);
        assert!(RecentFiles::load_from(&dir.path().join("missing.toml")).files.is_empty());
    }
}

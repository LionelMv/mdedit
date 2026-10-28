//! Folder tree scan and folder search.

use std::path::{Path, PathBuf};

use crate::{SearchHit, TreeEntry};

/// Maximum directory depth below the root.
pub const MAX_DEPTH: usize = 8;
/// Maximum number of entries visited by a scan.
pub const MAX_ENTRIES: usize = 20_000;
/// Maximum folder-search hits.
pub const MAX_HITS: usize = 1_000;
/// Files larger than this are skipped by folder search.
pub const MAX_SEARCH_FILE_SIZE: u64 = 10 * 1024 * 1024;

const EXTENSIONS: [&str; 5] = ["md", "markdown", "mdown", "mkd", "txt"];
const SKIP_DIRS: [&str; 2] = ["target", "node_modules"];
const MAX_HIT_CHARS: usize = 200;

/// Whether `path` has a Markdown-ish extension (case-insensitive). (Added API.)
pub fn is_markdown_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

fn display_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string())
}

struct Walk {
    visited: usize,
}

impl Walk {
    /// Children of `dir` (filtered, sorted), recursing up to MAX_DEPTH.
    fn children(&mut self, dir: &Path, depth: usize) -> Vec<TreeEntry> {
        let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
        let mut out = Vec::new();
        for entry in rd.flatten() {
            if self.visited >= MAX_ENTRIES {
                break;
            }
            self.visited += 1;
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with('.') {
                continue;
            }
            let Ok(ft) = entry.file_type() else { continue };
            let path = entry.path();
            if ft.is_dir() {
                // Real directory (DirEntry::file_type doesn't follow symlinks).
                if SKIP_DIRS.contains(&name_str.as_ref()) || depth >= MAX_DEPTH {
                    continue;
                }
                let children = self.children(&path, depth + 1);
                if !children.is_empty() {
                    out.push(TreeEntry { path, name: name_str.into_owned(), is_dir: true, children });
                }
            } else if is_markdown_path(&path) {
                // Regular files and symlinks to files (symlinked dirs are not followed).
                let is_file = ft.is_file() || (ft.is_symlink() && std::fs::metadata(&path).is_ok_and(|m| m.is_file()));
                if is_file {
                    out.push(TreeEntry { path, name: name_str.into_owned(), is_dir: false, children: Vec::new() });
                }
            }
        }
        out.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                .then_with(|| a.name.cmp(&b.name))
        });
        out
    }
}

/// Scan `root` for Markdown files (`.md`, `.markdown`, `.mdown`, `.mkd`, `.txt`) and the
/// folders containing them; hidden entries and `target`/`node_modules` skipped; dirs first,
/// then names, case-insensitive; max depth 8, max 20 000 entries; symlinked dirs not followed.
/// STABLE.
///
/// The root itself is always returned (`is_dir: true`), even if empty or unreadable.
pub fn scan_tree(root: &Path) -> TreeEntry {
    let mut walk = Walk { visited: 0 };
    let children = walk.children(root, 0);
    TreeEntry { path: root.to_path_buf(), name: display_name(root), is_dir: true, children }
}

fn collect_files(entry: &TreeEntry, out: &mut Vec<PathBuf>) {
    for c in &entry.children {
        if c.is_dir {
            collect_files(c, out);
        } else {
            out.push(c.path.clone());
        }
    }
}

fn truncate_chars(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_owned(),
    }
}

/// Case-insensitive plain-text search over Markdown files under `root` (same filters as
/// [`scan_tree`]), max 1 000 hits, files > 10 MB skipped. STABLE.
///
/// Hits are ordered by file (tree order) then line; `line` is 0-based. Non-UTF-8 files are
/// skipped. An empty query returns nothing.
pub fn search_folder(root: &Path, query: &str) -> Vec<SearchHit> {
    if query.is_empty() {
        return Vec::new();
    }
    let Ok(re) = regex::RegexBuilder::new(&regex::escape(query)).case_insensitive(true).build() else {
        return Vec::new();
    };
    let mut files = Vec::new();
    collect_files(&scan_tree(root), &mut files);
    let mut hits = Vec::new();
    for path in files {
        if std::fs::metadata(&path).map_or(true, |m| m.len() > MAX_SEARCH_FILE_SIZE) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        for (i, line) in text.lines().enumerate() {
            let line = line.strip_suffix('\r').unwrap_or(line);
            if re.is_match(line) {
                hits.push(SearchHit { path: path.clone(), line: i, text: truncate_chars(line.trim(), MAX_HIT_CHARS) });
                if hits.len() >= MAX_HITS {
                    return hits;
                }
            }
        }
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn names(e: &TreeEntry) -> Vec<&str> {
        e.children.iter().map(|c| c.name.as_str()).collect()
    }

    fn setup() -> tempfile::TempDir {
        let t = tempfile::tempdir().unwrap();
        let r = t.path();
        fs::create_dir_all(r.join("docs/deep")).unwrap();
        fs::create_dir_all(r.join("empty")).unwrap();
        fs::create_dir_all(r.join("code")).unwrap();
        fs::create_dir_all(r.join(".git")).unwrap();
        fs::create_dir_all(r.join("node_modules/pkg")).unwrap();
        fs::create_dir_all(r.join("target")).unwrap();
        fs::write(r.join("b.md"), "Hello world\nsecond HELLO line\n").unwrap();
        fs::write(r.join("A.markdown"), "nothing\r\nhello again\r\n").unwrap();
        fs::write(r.join("notes.TXT"), "x").unwrap();
        fs::write(r.join("image.png"), "hello").unwrap();
        fs::write(r.join(".hidden.md"), "hello").unwrap();
        fs::write(r.join("docs/deep/z.mkd"), "deep hello").unwrap();
        fs::write(r.join("code/main.rs"), "hello").unwrap();
        fs::write(r.join(".git/x.md"), "hello").unwrap();
        fs::write(r.join("node_modules/pkg/readme.md"), "hello").unwrap();
        fs::write(r.join("target/t.md"), "hello").unwrap();
        fs::write(r.join("bad.md"), b"hello \xff").unwrap();
        t
    }

    #[test]
    fn tree_filters_and_sorts() {
        let t = setup();
        let tree = scan_tree(t.path());
        assert!(tree.is_dir);
        assert_eq!(names(&tree), vec!["docs", "A.markdown", "b.md", "bad.md", "notes.TXT"]);
        let docs = &tree.children[0];
        assert_eq!(names(docs), vec!["deep"]);
        assert_eq!(names(&docs.children[0]), vec!["z.mkd"]);
        assert_eq!(docs.children[0].children[0].path, t.path().join("docs/deep/z.mkd"));
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_dirs_not_followed() {
        let t = setup();
        std::os::unix::fs::symlink(t.path().join("docs"), t.path().join("docs-link")).unwrap();
        std::os::unix::fs::symlink(t.path().join("b.md"), t.path().join("c-link.md")).unwrap();
        // A loop must not hang.
        std::os::unix::fs::symlink(t.path(), t.path().join("docs/loop")).unwrap();
        let tree = scan_tree(t.path());
        assert_eq!(names(&tree), vec!["docs", "A.markdown", "b.md", "bad.md", "c-link.md", "notes.TXT"]);
    }

    #[test]
    fn depth_limit() {
        let t = tempfile::tempdir().unwrap();
        let mut p = t.path().to_path_buf();
        for i in 0..12 {
            p = p.join(format!("d{i}"));
        }
        fs::create_dir_all(&p).unwrap();
        fs::write(p.join("x.md"), "x").unwrap();
        fs::write(t.path().join("d0/d1/y.md"), "y").unwrap();
        let tree = scan_tree(t.path());
        let mut depth = 0;
        let mut e = &tree;
        while let Some(d) = e.children.iter().find(|c| c.is_dir) {
            e = d;
            depth += 1;
        }
        assert_eq!(depth, 2); // the 12-deep file is beyond the limit, so only d0/d1 remain
    }

    #[test]
    fn missing_root() {
        let tree = scan_tree(Path::new("/definitely/not/here"));
        assert!(tree.children.is_empty());
        assert!(search_folder(Path::new("/definitely/not/here"), "x").is_empty());
    }

    #[test]
    fn folder_search() {
        let t = setup();
        let hits = search_folder(t.path(), "hello");
        let summary: Vec<(String, usize, &str)> = hits
            .iter()
            .map(|h| (h.path.strip_prefix(t.path()).unwrap().display().to_string(), h.line, h.text.as_str()))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("docs/deep/z.mkd".to_string(), 0, "deep hello"),
                ("A.markdown".to_string(), 1, "hello again"),
                ("b.md".to_string(), 0, "Hello world"),
                ("b.md".to_string(), 1, "second HELLO line"),
            ]
        );
        assert!(search_folder(t.path(), "").is_empty());
        assert_eq!(search_folder(t.path(), "(x").len(), 0);
    }

    #[test]
    fn folder_search_caps_hits_and_truncates() {
        let t = tempfile::tempdir().unwrap();
        let long = format!("{}needle\n", "é".repeat(300));
        fs::write(t.path().join("a.md"), long.repeat(1500)).unwrap();
        let hits = search_folder(t.path(), "NEEDLE");
        assert_eq!(hits.len(), MAX_HITS);
        assert_eq!(hits[0].text.chars().count(), MAX_HIT_CHARS + 1);
    }
}

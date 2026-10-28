//! A buffer bound to a file: load/save with line-ending and BOM preservation, atomic writes.

use std::io::{self, Read};
use std::path::{Path, PathBuf};

use crate::Buffer;

/// Files larger than this are refused by [`Document::open`].
pub const MAX_FILE_SIZE: u64 = 200 * 1024 * 1024;

const BOM: char = '\u{FEFF}';

/// Strip a BOM, detect the dominant line ending and normalize CRLF / lone CR to LF.
/// Returns (normalized text, line ending, had BOM). (Added API.)
pub fn normalize_text(text: &str) -> (String, LineEnding, bool) {
    let (text, bom) = match text.strip_prefix(BOM) {
        Some(rest) => (rest, true),
        None => (text, false),
    };
    let bytes = text.as_bytes();
    let (mut crlf, mut lf, mut cr) = (0usize, 0usize, 0usize);
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\r' if bytes.get(i + 1) == Some(&b'\n') => {
                crlf += 1;
                i += 1;
            }
            b'\r' => cr += 1,
            b'\n' => lf += 1,
            _ => {}
        }
        i += 1;
    }
    let ending = if crlf > lf { LineEnding::CrLf } else { LineEnding::Lf };
    let normalized = if crlf == 0 && cr == 0 { text.to_owned() } else { text.replace("\r\n", "\n").replace('\r', "\n") };
    (normalized, ending, bom)
}

/// STABLE.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LineEnding {
    #[default]
    Lf,
    CrLf,
}

/// An open document. STABLE API.
pub struct Document {
    pub buffer: Buffer,
    /// `None` for an unsaved new document.
    pub path: Option<PathBuf>,
    /// Title for a document without a path (e.g. the built-in cheatsheet).
    pub name: Option<String>,
    pub line_ending: LineEnding,
    pub bom: bool,
    saved_version: u64,
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

impl Document {
    pub fn new() -> Self {
        Self { buffer: Buffer::default(), path: None, name: None, line_ending: LineEnding::Lf, bom: false, saved_version: 0 }
    }
    /// A document from text not tied to a file (normalizes line endings).
    ///
    /// A leading BOM is stripped and remembered; the dominant line ending is remembered.
    pub fn from_text(text: &str) -> Self {
        let (normalized, line_ending, bom) = normalize_text(text);
        let buffer = Buffer::new(&normalized);
        let saved_version = buffer.version();
        Self { buffer, path: None, name: None, line_ending, bom, saved_version }
    }
    /// Read a file. Rejects non-UTF-8 with `InvalidData`. Strips and remembers a BOM, detects
    /// the dominant line ending and normalizes to `\n`. Files over 200 MB are refused
    /// (`FileTooLarge`).
    pub fn open(path: &Path) -> io::Result<Self> {
        let file = std::fs::File::open(path)?;
        let meta = file.metadata()?;
        if meta.is_dir() {
            return Err(io::Error::new(io::ErrorKind::IsADirectory, format!("{} is a directory", path.display())));
        }
        let too_large =
            || io::Error::new(io::ErrorKind::FileTooLarge, format!("{} is larger than 200 MB", path.display()));
        if meta.len() > MAX_FILE_SIZE {
            return Err(too_large());
        }
        let mut bytes = Vec::with_capacity(meta.len() as usize);
        file.take(MAX_FILE_SIZE + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_FILE_SIZE {
            return Err(too_large());
        }
        let text = String::from_utf8(bytes).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{} is not valid UTF-8 (at byte {})", path.display(), e.utf8_error().valid_up_to()),
            )
        })?;
        let mut doc = Self::from_text(&text);
        doc.path = Some(path.to_path_buf());
        Ok(doc)
    }
    /// Save to `self.path` atomically (temp file in the same dir + fsync + rename), keeping
    /// the original file's permissions and writing through symlinks to their target.
    ///
    /// Errors with `InvalidInput` if the document has no path. On success the document is
    /// marked saved.
    pub fn save(&mut self) -> io::Result<()> {
        let path = self
            .path
            .clone()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "document has no file path"))?;
        crate::fsutil::atomic_write(&path, self.disk_text().as_bytes(), None)?;
        self.mark_saved();
        Ok(())
    }
    pub fn save_as(&mut self, path: &Path) -> io::Result<()> {
        self.path = Some(path.to_path_buf());
        self.save()
    }
    /// Text as it would be written to disk (line endings + BOM restored).
    pub fn disk_text(&self) -> String {
        let text = self.buffer.text();
        let mut out = String::with_capacity(text.len() + 3);
        if self.bom {
            out.push(BOM);
        }
        match self.line_ending {
            LineEnding::Lf => out.push_str(&text),
            LineEnding::CrLf => out.push_str(&text.replace('\n', "\r\n")),
        }
        out
    }
    pub fn is_dirty(&self) -> bool {
        self.buffer.version() != self.saved_version
    }
    /// File name, else `name`, else "Untitled".
    pub fn title(&self) -> String {
        match self.path.as_ref().and_then(|p| p.file_name()) {
            Some(n) => n.to_string_lossy().into_owned(),
            None => self.name.clone().unwrap_or_else(|| "Untitled".into()),
        }
    }
    /// Mark the current version as saved (used after reload).
    pub fn mark_saved(&mut self) {
        self.saved_version = self.buffer.version();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Edit, EditKind, Selection};

    #[test]
    fn normalize_detects_endings() {
        assert_eq!(normalize_text("a\r\nb\r\n"), ("a\nb\n".into(), LineEnding::CrLf, false));
        assert_eq!(normalize_text("a\nb\r\nc\n"), ("a\nb\nc\n".into(), LineEnding::Lf, false));
        assert_eq!(normalize_text("a\rb\r"), ("a\nb\n".into(), LineEnding::Lf, false));
        assert_eq!(normalize_text("\u{FEFF}x"), ("x".into(), LineEnding::Lf, true));
        assert_eq!(normalize_text(""), ("".into(), LineEnding::Lf, false));
    }

    #[test]
    fn from_text_is_clean() {
        let d = Document::from_text("a\r\nb");
        assert_eq!(d.buffer.text(), "a\nb");
        assert!(!d.is_dirty());
        assert_eq!(d.disk_text(), "a\r\nb");
        assert_eq!(d.title(), "Untitled");
    }

    #[test]
    fn open_save_round_trip_crlf_bom() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.md");
        let original = "\u{FEFF}# T\r\n\r\nbody \u{e9}\r\n";
        std::fs::write(&p, original).unwrap();
        let mut d = Document::open(&p).unwrap();
        assert!(d.bom);
        assert_eq!(d.line_ending, LineEnding::CrLf);
        assert_eq!(d.buffer.text(), "# T\n\nbody \u{e9}\n");
        assert!(!d.is_dirty());
        d.save().unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), original.as_bytes());
        d.buffer.apply(vec![Edit::insert(0, "x")], Selection::caret(1), EditKind::Other);
        assert!(d.is_dirty());
        d.save().unwrap();
        assert!(!d.is_dirty());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "\u{FEFF}x# T\r\n\r\nbody \u{e9}\r\n");
        // No temp files left behind.
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn open_rejects_non_utf8() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("bad.md");
        std::fs::write(&p, b"ok \xff\xfe nope").unwrap();
        let err = Document::open(&p).err().unwrap();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn open_missing_and_dir_errors() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Document::open(&dir.path().join("nope")).err().unwrap().kind(), io::ErrorKind::NotFound);
        assert!(Document::open(dir.path()).is_err());
    }

    #[test]
    fn save_without_path_errors() {
        let mut d = Document::new();
        assert_eq!(d.save().err().unwrap().kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn save_as_new_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("new.md");
        let mut d = Document::from_text("hello\n");
        d.buffer.insert_at_selection("x", EditKind::Typing);
        d.save_as(&p).unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "xhello\n");
        assert!(!d.is_dirty());
        assert_eq!(d.title(), "new.md");
    }

    #[cfg(unix)]
    #[test]
    fn save_preserves_mode_and_symlink() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.md");
        std::fs::write(&real, "a\n").unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o600)).unwrap();
        let link = dir.path().join("link.md");
        std::os::unix::fs::symlink("real.md", &link).unwrap();
        let mut d = Document::open(&link).unwrap();
        d.buffer.insert_at_selection("b", EditKind::Other);
        d.save().unwrap();
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "ba\n");
        assert_eq!(std::fs::metadata(&real).unwrap().permissions().mode() & 0o777, 0o600);
    }

    proptest::proptest! {
        #[test]
        fn disk_round_trip_byte_exact(
            lines in proptest::collection::vec("[a-z \u{e9}\u{1F600}]{0,8}", 0..8),
            crlf in proptest::bool::ANY,
            bom in proptest::bool::ANY,
            trailing in proptest::bool::ANY,
        ) {
            let nl = if crlf { "\r\n" } else { "\n" };
            let mut s = String::new();
            if bom { s.push(BOM); }
            s.push_str(&lines.join(nl));
            if trailing && !lines.is_empty() { s.push_str(nl); }
            let d = Document::from_text(&s);
            // A file without any newline can't reveal CRLF; it round-trips either way.
            proptest::prop_assert_eq!(d.disk_text(), s);
        }
    }
}

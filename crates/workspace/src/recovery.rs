//! Crash recovery snapshots of unsaved documents. STABLE.
//!
//! Snapshots live in `<state dir>/recovery/<id>.snapshot` (dir mode 0700, files 0600).
//! File format (version 1):
//!
//! ```text
//! mdedit-recovery 1\n
//! <byte length of the original path, or -1 for none>\n
//! <raw path bytes>\n
//! <document text, to end of file>
//! ```

use std::io;
use std::path::{Path, PathBuf};

use super::{create_dir, state_dir};

const MAGIC: &str = "mdedit-recovery 1\n";
const EXT: &str = "snapshot";

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub id: String,
    /// Original file path, if the document had one.
    pub path: Option<PathBuf>,
    pub text: String,
    pub modified: std::time::SystemTime,
}

/// `[A-Za-z0-9_-]{1,64}`. (Added API.)
pub fn is_valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn check_id(id: &str) -> io::Result<()> {
    if is_valid_id(id) {
        Ok(())
    } else {
        Err(io::Error::new(io::ErrorKind::InvalidInput, format!("invalid recovery id {id:?}")))
    }
}

/// The default recovery directory, `<state dir>/recovery`. (Added API.)
pub fn dir() -> Option<PathBuf> {
    state_dir().map(|d| d.join("recovery"))
}

fn default_dir() -> io::Result<PathBuf> {
    dir().ok_or_else(super::no_dir)
}

/// Write/overwrite snapshot `id` (atomic, mode 0600). `id` must be `[A-Za-z0-9_-]+`.
pub fn write(id: &str, path: Option<&Path>, text: &str) -> io::Result<()> {
    write_in(&default_dir()?, id, path, text)
}
pub fn remove(id: &str) -> io::Result<()> {
    remove_in(&default_dir()?, id)
}
/// All readable snapshots, newest first.
pub fn list() -> Vec<Snapshot> {
    dir().map(|d| list_in(&d)).unwrap_or_default()
}

fn path_bytes(p: &Path) -> Vec<u8> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        p.as_os_str().as_bytes().to_vec()
    }
    #[cfg(not(unix))]
    {
        p.to_string_lossy().into_owned().into_bytes()
    }
}

fn path_from_bytes(b: &[u8]) -> Option<PathBuf> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        Some(PathBuf::from(std::ffi::OsStr::from_bytes(b)))
    }
    #[cfg(not(unix))]
    {
        std::str::from_utf8(b).ok().map(PathBuf::from)
    }
}

fn encode(path: Option<&Path>, text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() + 64);
    out.extend_from_slice(MAGIC.as_bytes());
    match path {
        Some(p) => {
            let b = path_bytes(p);
            out.extend_from_slice(format!("{}\n", b.len()).as_bytes());
            out.extend_from_slice(&b);
        }
        None => out.extend_from_slice(b"-1\n"),
    }
    out.push(b'\n');
    out.extend_from_slice(text.as_bytes());
    out
}

fn decode(data: &[u8]) -> Option<(Option<PathBuf>, String)> {
    let rest = data.strip_prefix(MAGIC.as_bytes())?;
    let nl = rest.iter().position(|&b| b == b'\n')?;
    let len_str = std::str::from_utf8(&rest[..nl]).ok()?;
    let rest = &rest[nl + 1..];
    let (path, rest) = if len_str == "-1" {
        (None, rest)
    } else {
        let len: usize = len_str.parse().ok()?;
        let p = rest.get(..len)?;
        (Some(path_from_bytes(p)?), rest.get(len..)?)
    };
    let rest = rest.strip_prefix(b"\n")?;
    let text = String::from_utf8(rest.to_vec()).ok()?;
    Some((path, text))
}

/// [`write`] into an explicit recovery directory. (Added API.)
pub fn write_in(dir: &Path, id: &str, path: Option<&Path>, text: &str) -> io::Result<()> {
    check_id(id)?;
    create_dir(dir, 0o700)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    mdedit_buffer::fsutil::atomic_write(&dir.join(format!("{id}.{EXT}")), &encode(path, text), Some(0o600))
}

/// [`remove`] from an explicit recovery directory. A missing snapshot is not an error.
/// (Added API.)
pub fn remove_in(dir: &Path, id: &str) -> io::Result<()> {
    check_id(id)?;
    match std::fs::remove_file(dir.join(format!("{id}.{EXT}"))) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        r => r,
    }
}

/// [`list`] from an explicit recovery directory: valid snapshots, newest first. (Added API.)
pub fn list_in(dir: &Path) -> Vec<Snapshot> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out = Vec::new();
    for entry in rd.flatten() {
        let p = entry.path();
        if p.extension().and_then(|e| e.to_str()) != Some(EXT) {
            continue;
        }
        let Some(id) = p.file_stem().and_then(|s| s.to_str()).filter(|s| is_valid_id(s)) else { continue };
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let Ok(data) = std::fs::read(&p) else { continue };
        let Some((path, text)) = decode(&data) else {
            log::warn!("ignoring unreadable recovery snapshot {}", p.display());
            continue;
        };
        let modified = meta.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        out.push(Snapshot { id: id.to_owned(), path, text, modified });
    }
    out.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.id.cmp(&b.id)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids() {
        assert!(is_valid_id("tab-1_A"));
        assert!(!is_valid_id(""));
        assert!(!is_valid_id("../x"));
        assert!(!is_valid_id("a b"));
        assert!(!is_valid_id(&"a".repeat(65)));
        assert!(is_valid_id(&"a".repeat(64)));
    }

    #[test]
    fn write_list_remove() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path().join("recovery");
        write_in(&d, "one", Some(Path::new("/home/u/notes\nweird.md")), "hello\n\nworld").unwrap();
        write_in(&d, "two", None, "").unwrap();
        write_in(&d, "two", None, "second\n").unwrap();
        assert_eq!(write_in(&d, "../evil", None, "x").err().unwrap().kind(), io::ErrorKind::InvalidInput);
        std::fs::write(d.join("junk.snapshot"), "garbage").unwrap();
        std::fs::write(d.join("other.txt"), "x").unwrap();
        let mut l = list_in(&d);
        l.sort_by(|a, b| a.id.cmp(&b.id));
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].id, "one");
        assert_eq!(l[0].path.as_deref(), Some(Path::new("/home/u/notes\nweird.md")));
        assert_eq!(l[0].text, "hello\n\nworld");
        assert_eq!(l[1].path, None);
        assert_eq!(l[1].text, "second\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(d.join("one.snapshot")).unwrap().permissions().mode() & 0o777, 0o600);
            assert_eq!(std::fs::metadata(&d).unwrap().permissions().mode() & 0o777, 0o700);
        }
        remove_in(&d, "one").unwrap();
        remove_in(&d, "one").unwrap();
        assert!(remove_in(&d, "bad/id").is_err());
        assert_eq!(list_in(&d).len(), 1);
        assert!(list_in(&tmp.path().join("nope")).is_empty());
    }

    proptest::proptest! {
        #[test]
        fn encode_decode_round_trip(path in proptest::option::of("[^\u{0}]{0,30}"), text in "\\PC{0,60}") {
            let p = path.as_ref().map(PathBuf::from);
            let enc = encode(p.as_deref(), &text);
            proptest::prop_assert_eq!(decode(&enc), Some((p, text)));
        }

        #[test]
        fn decode_never_panics(data in proptest::collection::vec(proptest::num::u8::ANY, 0..80)) {
            let mut d = MAGIC.as_bytes().to_vec();
            d.extend_from_slice(&data);
            let _ = decode(&d);
            let _ = decode(&data);
        }
    }
}

//! Filesystem helpers: atomic writes that keep permissions and write through symlinks.
//! (Added API; also used by `mdedit-workspace`.)

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Follow symlinks at `path` (up to 40 hops) and return the final target path, which may not
/// exist yet. Relative link targets resolve against the link's directory.
pub fn resolve_symlinks(path: &Path) -> io::Result<PathBuf> {
    let mut p = path.to_path_buf();
    for _ in 0..40 {
        match fs::symlink_metadata(&p) {
            Ok(m) if m.file_type().is_symlink() => {
                let target = fs::read_link(&p)?;
                p = if target.is_absolute() {
                    target
                } else {
                    p.parent().map(|d| d.join(&target)).unwrap_or(target)
                };
            }
            _ => return Ok(p),
        }
    }
    Err(io::Error::other(format!("too many levels of symbolic links: {}", path.display())))
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Atomically replace the file at `path` with `data`: write a temp file in the same directory,
/// fsync it, rename it over the target, fsync the directory.
///
/// Symlinks are resolved so the link's target is replaced, not the link. If `force_mode` is
/// `Some`, the file gets exactly that mode (unix); otherwise an existing file keeps its mode
/// and a new file is created 0644 (minus umask).
pub fn atomic_write(path: &Path, data: &[u8], force_mode: Option<u32>) -> io::Result<()> {
    let target = resolve_symlinks(path)?;
    let file_name = target
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, format!("not a file path: {}", path.display())))?
        .to_os_string();
    let dir = match target.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let existing = fs::metadata(&target).ok();
    if existing.as_ref().is_some_and(|m| m.is_dir()) {
        return Err(io::Error::new(io::ErrorKind::IsADirectory, format!("is a directory: {}", target.display())));
    }

    let mut tmp_name = std::ffi::OsString::from(".");
    tmp_name.push(&file_name);
    tmp_name.push(format!(".mdedit-tmp-{}-{}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed)));
    let tmp = dir.join(tmp_name);

    let mut opts = OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        use std::os::unix::fs::PermissionsExt;
        // Create with the final mode so the new content is never more readable than the
        // original file, even briefly (0o777: setuid/setgid bits are applied afterwards).
        let initial = force_mode.or(existing.as_ref().map(|m| m.permissions().mode())).unwrap_or(0o644);
        opts.mode(initial & 0o777);
    }
    let result = (|| -> io::Result<()> {
        let mut f = opts.open(&tmp)?;
        f.write_all(data)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = match (force_mode, &existing) {
                (Some(m), _) => Some(m),
                (None, Some(meta)) => Some(meta.permissions().mode()),
                (None, None) => None,
            };
            if let Some(m) = mode {
                f.set_permissions(fs::Permissions::from_mode(m & 0o7777))?;
            }
        }
        #[cfg(not(unix))]
        if let Some(meta) = &existing {
            f.set_permissions(meta.permissions())?;
        }
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp, &target)?;
        Ok(())
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    // Make the rename durable; failure here is not fatal (some filesystems refuse).
    if let Ok(d) = File::open(&dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

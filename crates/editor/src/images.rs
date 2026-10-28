//! Image URL resolution for `Decoration::Image`, plus safety checks for local files
//! (no devices/FIFOs, size and pixel-dimension limits) before egui's loaders touch them.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Largest local image file we hand to the loaders.
pub(crate) const MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;
/// Largest raster image (either side, in pixels) we decode.
pub(crate) const MAX_IMAGE_SIDE: u32 = 8192;

/// Check a local image file before loading: must be a regular file (not a device, FIFO,
/// directory), at most [`MAX_IMAGE_BYTES`], and — for raster formats — at most
/// [`MAX_IMAGE_SIDE`]² pixels according to its header. SVGs are allowed (the editor asks the
/// loader to rasterize them at display size). `Err` holds a short reason for the placeholder.
pub(crate) fn check_local(path: &Path) -> Result<(), String> {
    let meta = std::fs::metadata(path).map_err(|_| "not found".to_owned())?;
    if !meta.is_file() {
        return Err("not a file".into());
    }
    if meta.len() > MAX_IMAGE_BYTES {
        return Err("too large".into());
    }
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if ext == "svg" || ext == "svgz" {
        return Ok(());
    }
    let reader = image::ImageReader::open(path).map_err(|_| "unreadable".to_owned())?;
    let reader = reader.with_guessed_format().map_err(|_| "unreadable".to_owned())?;
    if reader.format().is_none() {
        // Not a raster format we can inspect; allow only if it looks like SVG text.
        let head = std::fs::read(path).ok().map(|b| b.into_iter().take(512).collect::<Vec<u8>>()).unwrap_or_default();
        let head = String::from_utf8_lossy(&head).to_ascii_lowercase();
        return if head.contains("<svg") { Ok(()) } else { Err("unsupported format".into()) };
    }
    let (w, h) = reader.into_dimensions().map_err(|_| "unsupported or corrupt image".to_owned())?;
    if w > MAX_IMAGE_SIDE || h > MAX_IMAGE_SIDE {
        return Err(format!("too large: {w}×{h} px"));
    }
    Ok(())
}

/// Caches [`check_local`] results per path, invalidated when size or mtime change.
type CheckEntry = (Option<SystemTime>, u64, Result<(), String>);

#[derive(Default)]
pub(crate) struct ImageGuard {
    cache: HashMap<PathBuf, CheckEntry>,
}

impl ImageGuard {
    /// Check a `file://` URI (other URIs pass). Costs one `stat` per call on a cache hit.
    pub fn check_uri(&mut self, uri: &str) -> Result<(), String> {
        let Some(p) = uri.strip_prefix("file://") else { return Ok(()) };
        self.check(Path::new(p))
    }

    pub fn check(&mut self, path: &Path) -> Result<(), String> {
        let meta = std::fs::metadata(path).ok();
        let key = (meta.as_ref().and_then(|m| m.modified().ok()), meta.as_ref().map_or(0, |m| m.len()));
        if let Some((mt, len, r)) = self.cache.get(path)
            && (*mt, *len) == key
        {
            return r.clone();
        }
        let r = check_local(path);
        if self.cache.len() > 1000 {
            self.cache.clear();
        }
        self.cache.insert(path.to_path_buf(), (key.0, key.1, r.clone()));
        r
    }
}

/// How an image URL from the document should be loaded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ImageSource {
    /// A URI for egui's loaders (`file://...` or `http(s)://...`).
    Uri(String),
    /// Remote image but remote loading is disabled.
    RemoteBlocked,
    /// Can't be resolved (relative path without a base dir, unsupported scheme).
    Unresolvable,
}

/// Decode `%XX` escapes (invalid escapes are kept as written).
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = |c: u8| (c as char).to_digit(16);
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_owned())
}

/// Resolve `url` (as written in the Markdown) against `base_dir`.
pub(crate) fn resolve(url: &str, base_dir: Option<&Path>, remote: bool) -> ImageSource {
    let url = url.trim();
    let url = url.strip_prefix('<').and_then(|u| u.strip_suffix('>')).unwrap_or(url);
    if url.is_empty() {
        return ImageSource::Unresolvable;
    }
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return if remote { ImageSource::Uri(url.to_owned()) } else { ImageSource::RemoteBlocked };
    }
    if let Some(rest) = lower.strip_prefix("file://") {
        let _ = rest;
        return ImageSource::Uri(format!("file://{}", percent_decode(&url[7..])));
    }
    // Any other scheme (data:, javascript:, ftp:, ...) is not loaded.
    if let Some(colon) = url.find(':') {
        let scheme = &url[..colon];
        if scheme.len() > 1 && scheme.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c)) {
            return ImageSource::Unresolvable;
        }
    }
    // Strip a query/fragment.
    let path_part = url.split(['?', '#']).next().unwrap_or(url);
    let decoded = percent_decode(path_part);
    let p = Path::new(&decoded);
    let full = if p.is_absolute() {
        p.to_path_buf()
    } else if let Some(stripped) = decoded.strip_prefix("~/") {
        match std::env::var_os("HOME") {
            Some(h) => Path::new(&h).join(stripped),
            None => return ImageSource::Unresolvable,
        }
    } else if let Some(base) = base_dir {
        base.join(p)
    } else {
        return ImageSource::Unresolvable;
    };
    ImageSource::Uri(format!("file://{}", full.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal PNG (signature + IHDR only) claiming the given dimensions.
    fn png_header(w: u32, h: u32) -> Vec<u8> {
        let mut v = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        let mut chunk = b"IHDR".to_vec();
        chunk.extend_from_slice(&w.to_be_bytes());
        chunk.extend_from_slice(&h.to_be_bytes());
        chunk.extend_from_slice(&[8, 6, 0, 0, 0]);
        v.extend_from_slice(&13u32.to_be_bytes());
        v.extend_from_slice(&chunk);
        v.extend_from_slice(&crc32(&chunk).to_be_bytes());
        // Empty IDAT + IEND so the decoder's header parsing completes.
        for name in [b"IDAT", b"IEND"] {
            v.extend_from_slice(&0u32.to_be_bytes());
            v.extend_from_slice(name);
            v.extend_from_slice(&crc32(name).to_be_bytes());
        }
        v
    }

    fn crc32(data: &[u8]) -> u32 {
        let mut c = 0xffff_ffffu32;
        for &b in data {
            c ^= u32::from(b);
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    }

    #[test]
    fn rejects_devices_fifos_and_bombs() {
        assert_eq!(check_local(Path::new("/dev/zero")), Err("not a file".into()));
        let dir = tempfile::tempdir().expect("tempdir");
        let d = dir.path();
        assert_eq!(check_local(d), Err("not a file".into()));
        assert!(check_local(&d.join("missing.png")).is_err());

        let fifo = d.join("fifo.png");
        let made = std::process::Command::new("mkfifo").arg(&fifo).status().is_ok_and(|s| s.success());
        if made {
            assert_eq!(check_local(&fifo), Err("not a file".into()), "must not block on a FIFO");
        }

        let big = d.join("big.png");
        let f = std::fs::File::create(&big).expect("create");
        f.set_len(MAX_IMAGE_BYTES + 1).expect("set_len");
        assert_eq!(check_local(&big), Err("too large".into()));

        let huge = d.join("huge.png");
        std::fs::write(&huge, png_header(60_000, 60_000)).expect("write");
        assert!(check_local(&huge).is_err_and(|e| e.contains("60000")), "{:?}", check_local(&huge));

        let ok = d.join("ok.png");
        std::fs::write(&ok, png_header(100, 50)).expect("write");
        assert_eq!(check_local(&ok), Ok(()));

        let svg = d.join("bomb.svg");
        std::fs::write(&svg, r#"<svg width="60000" height="60000"></svg>"#).expect("write");
        assert_eq!(check_local(&svg), Ok(()), "SVG allowed; rasterized at display size");
        let svg2 = d.join("noext");
        std::fs::write(&svg2, r#"<svg xmlns="http://www.w3.org/2000/svg"></svg>"#).expect("write");
        assert_eq!(check_local(&svg2), Ok(()));
        let junk = d.join("junk.png");
        std::fs::write(&junk, b"not an image").expect("write");
        assert!(check_local(&junk).is_err());

        // Guard caches and re-checks when the file changes.
        let mut g = ImageGuard::default();
        assert_eq!(g.check_uri(&format!("file://{}", ok.display())), Ok(()));
        std::fs::write(&ok, png_header(9000, 10)).expect("write");
        assert!(g.check_uri(&format!("file://{}", ok.display())).is_err());
        assert_eq!(g.check_uri("https://e.com/x.png"), Ok(()));
        assert_eq!(g.check_uri("file:///dev/zero"), Err("not a file".into()));
    }

    #[test]
    fn resolves() {
        let base = Path::new("/home/u/docs");
        assert_eq!(resolve("img/a.png", Some(base), false), ImageSource::Uri("file:///home/u/docs/img/a.png".into()));
        assert_eq!(resolve("my%20pic.png", Some(base), false), ImageSource::Uri("file:///home/u/docs/my pic.png".into()));
        assert_eq!(resolve("/abs/x.png", None, false), ImageSource::Uri("file:///abs/x.png".into()));
        assert_eq!(resolve("x.png", None, false), ImageSource::Unresolvable);
        assert_eq!(resolve("https://e.com/x.png", Some(base), false), ImageSource::RemoteBlocked);
        assert_eq!(resolve("https://e.com/x.png", Some(base), true), ImageSource::Uri("https://e.com/x.png".into()));
        assert_eq!(resolve("data:image/png;base64,xx", Some(base), true), ImageSource::Unresolvable);
        assert_eq!(resolve("file:///t/a.png", None, false), ImageSource::Uri("file:///t/a.png".into()));
        assert_eq!(resolve("<a b.png>", Some(base), false), ImageSource::Uri("file:///home/u/docs/a b.png".into()));
        assert_eq!(resolve("", Some(base), false), ImageSource::Unresolvable);
        assert_eq!(percent_decode("%"), "%");
        assert_eq!(percent_decode("a%2"), "a%2");
        assert_eq!(percent_decode("%ff"), "%ff");
    }
}

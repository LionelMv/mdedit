//! Image URL resolution for `Decoration::Image`, plus safety checks for local files
//! (no devices/FIFOs, size and pixel-dimension limits) before egui's loaders touch them.

use std::collections::{HashMap, HashSet};
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Largest local image file we hand to the loaders.
pub(crate) const MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;
/// Largest raster image (either side, in pixels) we decode, however large the GPU's limit.
pub(crate) const MAX_IMAGE_SIDE: u32 = 8192;

/// Largest image side usable right now: the painter panics on textures above the GPU's limit.
pub(crate) fn max_side(ctx: &egui::Context) -> u32 {
    ctx.input(|i| i.max_texture_side).clamp(16, MAX_IMAGE_SIDE as usize) as u32
}

/// Check a local image file before loading: must be a regular file (not a device, FIFO,
/// directory), at most [`MAX_IMAGE_BYTES`], and — for raster formats — at most `max_side`²
/// pixels according to its header. SVGs are allowed (the editor asks the loader to rasterize
/// them at display size). `Err` holds a short reason for the placeholder.
pub(crate) fn check_local(path: &Path, max_side: u32) -> Result<(), String> {
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
        let mut head = Vec::with_capacity(512);
        if let Ok(f) = std::fs::File::open(path) {
            let _ = f.take(512).read_to_end(&mut head);
        }
        let head = String::from_utf8_lossy(&head).to_ascii_lowercase();
        return if head.contains("<svg") { Ok(()) } else { Err("unsupported format".into()) };
    }
    let (w, h) = reader.into_dimensions().map_err(|_| "unsupported or corrupt image".to_owned())?;
    if w > max_side || h > max_side {
        return Err(format!("too large: {w}×{h} px"));
    }
    Ok(())
}

/// Check a decoded image, for what [`check_local`] can't see: remote images, or a file replaced
/// after its check. Decoded images are cached by the loaders, so this costs a lookup.
pub(crate) fn check_decoded(ctx: &egui::Context, uri: &str, hint: egui::SizeHint, max_side: u32) -> Result<(), String> {
    match ctx.try_load_image(uri, hint) {
        Ok(egui::load::ImagePoll::Ready { image }) if image.width().max(image.height()) > max_side as usize => {
            Err(format!("too large: {}×{} px", image.width(), image.height()))
        }
        _ => Ok(()),
    }
}

/// Caches [`check_local`] results per path, invalidated when size, mtime or the side limit change.
type CheckEntry = (Option<SystemTime>, u64, u32, Result<(), String>);

#[derive(Default)]
pub(crate) struct ImageGuard {
    cache: HashMap<PathBuf, CheckEntry>,
    /// Every URI this editor has asked for, so they can be released when it closes.
    seen: HashSet<String>,
}

impl ImageGuard {
    /// Check a `file://` URI (other URIs pass), dropping what the loaders hold of it if the file
    /// changed. Costs one `stat` per call on a cache hit.
    pub fn check_uri(&mut self, ctx: &egui::Context, uri: &str, max_side: u32) -> Result<(), String> {
        if !self.seen.contains(uri) {
            self.seen.insert(uri.to_owned());
        }
        let Some(p) = uri.strip_prefix("file://") else { return Ok(()) };
        let (r, changed) = self.check(Path::new(p), max_side);
        if changed {
            ctx.forget_image(uri);
        }
        r
    }

    /// The check result, and whether the file differs from the last time it was checked.
    fn check(&mut self, path: &Path, max_side: u32) -> (Result<(), String>, bool) {
        let meta = std::fs::metadata(path).ok();
        let key = (meta.as_ref().and_then(|m| m.modified().ok()), meta.as_ref().map_or(0, |m| m.len()));
        let mut changed = false;
        if let Some((mt, len, side, r)) = self.cache.get(path) {
            changed = (*mt, *len) != key;
            if !changed && *side == max_side {
                return (r.clone(), false);
            }
        }
        let r = check_local(path, max_side);
        if self.cache.len() > 1000 {
            self.cache.clear();
        }
        self.cache.insert(path.to_path_buf(), (key.0, key.1, max_side, r.clone()));
        (r, changed)
    }

    pub fn seen(&self) -> impl Iterator<Item = &str> {
        self.seen.iter().map(String::as_str)
    }

    pub fn has_seen(&self, uri: &str) -> bool {
        self.seen.contains(uri)
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

    const SIDE: u32 = MAX_IMAGE_SIDE;

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
        assert_eq!(check_local(Path::new("/dev/zero"), SIDE), Err("not a file".into()));
        let dir = tempfile::tempdir().expect("tempdir");
        let d = dir.path();
        assert_eq!(check_local(d, SIDE), Err("not a file".into()));
        assert!(check_local(&d.join("missing.png"), SIDE).is_err());

        let fifo = d.join("fifo.png");
        let made = std::process::Command::new("mkfifo").arg(&fifo).status().is_ok_and(|s| s.success());
        if made {
            assert_eq!(check_local(&fifo, SIDE), Err("not a file".into()), "must not block on a FIFO");
        }

        let big = d.join("big.png");
        let f = std::fs::File::create(&big).expect("create");
        f.set_len(MAX_IMAGE_BYTES + 1).expect("set_len");
        assert_eq!(check_local(&big, SIDE), Err("too large".into()));

        let huge = d.join("huge.png");
        std::fs::write(&huge, png_header(60_000, 60_000)).expect("write");
        assert!(check_local(&huge, SIDE).is_err_and(|e| e.contains("60000")), "{:?}", check_local(&huge, SIDE));

        let ok = d.join("ok.png");
        std::fs::write(&ok, png_header(100, 50)).expect("write");
        assert_eq!(check_local(&ok, SIDE), Ok(()));

        let svg = d.join("bomb.svg");
        std::fs::write(&svg, r#"<svg width="60000" height="60000"></svg>"#).expect("write");
        assert_eq!(check_local(&svg, SIDE), Ok(()), "SVG allowed; rasterized at display size");
        let svg2 = d.join("noext");
        std::fs::write(&svg2, r#"<svg xmlns="http://www.w3.org/2000/svg"></svg>"#).expect("write");
        assert_eq!(check_local(&svg2, SIDE), Ok(()));
        let junk = d.join("junk.png");
        std::fs::write(&junk, b"not an image").expect("write");
        assert!(check_local(&junk, SIDE).is_err());

        // Guard caches and re-checks when the file changes.
        let ctx = egui::Context::default();
        let mut g = ImageGuard::default();
        assert_eq!(g.check_uri(&ctx, &format!("file://{}", ok.display()), SIDE), Ok(()));
        std::fs::write(&ok, png_header(9000, 10)).expect("write");
        assert!(g.check_uri(&ctx, &format!("file://{}", ok.display()), SIDE).is_err());
        assert_eq!(g.check_uri(&ctx, "https://e.com/x.png", SIDE), Ok(()));
        assert_eq!(g.check_uri(&ctx, "file:///dev/zero", SIDE), Err("not a file".into()));
    }

    #[test]
    fn side_limit_follows_the_gpu() {
        let dir = tempfile::tempdir().expect("tempdir");
        let png = dir.path().join("a.png");
        std::fs::write(&png, png_header(3000, 10)).expect("write");
        assert_eq!(check_local(&png, 4096), Ok(()));
        assert!(check_local(&png, 2048).is_err_and(|e| e.contains("too large")));

        // The cached verdict is only good for the limit it was checked against.
        let mut g = ImageGuard::default();
        assert_eq!(g.check(&png, 4096), (Ok(()), false));
        let (r, changed) = g.check(&png, 2048);
        assert!(r.is_err_and(|e| e.contains("too large")));
        assert!(!changed, "a new limit is not a changed file");
        assert_eq!(g.check(&png, 4096), (Ok(()), false));

        let ctx = egui::Context::default();
        assert_eq!(max_side(&ctx), 2048, "egui's default until the backend reports");
        ctx.input_mut(|i| i.max_texture_side = 1 << 20);
        assert_eq!(max_side(&ctx), MAX_IMAGE_SIDE);
    }

    #[test]
    fn svg_sniff_reads_only_the_head() {
        let dir = tempfile::tempdir().expect("tempdir");
        let late = dir.path().join("late");
        std::fs::write(&late, format!("{}<svg></svg>", " ".repeat(600))).expect("write");
        assert_eq!(check_local(&late, SIDE), Err("unsupported format".into()));
        let early = dir.path().join("early");
        std::fs::write(&early, format!("<svg>{}</svg>", " ".repeat(600))).expect("write");
        assert_eq!(check_local(&early, SIDE), Ok(()));
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

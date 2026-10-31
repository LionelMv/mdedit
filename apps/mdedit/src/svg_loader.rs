//! SVG loader that never follows an `<image href>` out of the document. usvg's default resolver
//! reads any local path, past our size limit and FIFO checks; `egui_extras`' SVG loader can't be
//! configured, so this one takes its place (same URIs, same caching) and only embeds data URIs.
//! It also renders off the UI thread, since a hostile file can take any time to parse, and limits
//! what can grow in memory: the SVG's size, gzipped SVGs and images embedded in it.

use std::collections::HashMap;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::task::Poll;

use egui::ColorImage;
use egui::load::{BytesPoll, ImageLoadResult, ImageLoader, ImagePoll, LoadError, SizeHint};
use egui::mutex::Mutex;
use resvg::usvg::{ImageHrefResolver, Options};

/// Largest SVG we parse, the same as for a local image file.
const MAX_SVG_BYTES: usize = 20 * 1024 * 1024;
/// Largest image embedded in an SVG through a data URI.
const MAX_EMBEDDED_BYTES: usize = 8 * 1024 * 1024;
/// Largest side of an embedded raster image: the renderer decodes it in full.
const MAX_EMBEDDED_SIDE: u32 = 4096;
/// SVGs rendered at once. A render can't be stopped, so hostile files can only take these slots.
const MAX_RENDERS: usize = 4;
/// Deeply nested SVGs recurse far in the parser, and an overflow would abort the whole app.
const RENDER_STACK_BYTES: usize = 64 * 1024 * 1024;

struct Entry {
    last_used: u64,
    /// Tells this request's render from one for an entry since forgotten.
    job: u64,
    result: Poll<Result<Arc<ColorImage>, String>>,
}

#[derive(Default)]
struct State {
    pass_index: u64,
    /// One entry per rasterized size of a URI.
    cache: HashMap<String, HashMap<SizeHint, Entry>>,
    next_job: u64,
    renders: usize,
}

struct SvgLoader {
    state: Arc<Mutex<State>>,
    options: Arc<Options<'static>>,
}

fn options() -> Options<'static> {
    let resolve_data = ImageHrefResolver::default_data_resolver();
    let image_href_resolver = ImageHrefResolver {
        resolve_string: Box::new(|_, _| None),
        resolve_data: Box::new(move |mime, data, options| if embedded_allowed(&data) { resolve_data(mime, data, options) } else { None }),
    };
    Options { image_href_resolver, ..Options::default() }
}

fn is_gzip(data: &[u8]) -> bool {
    data.starts_with(&[0x1f, 0x8b])
}

/// Size limits for an embedded image. usvg inflates gzip without any limit.
fn embedded_allowed(data: &[u8]) -> bool {
    if data.len() > MAX_EMBEDDED_BYTES || is_gzip(data) {
        return false;
    }
    let Ok(reader) = image::ImageReader::new(std::io::Cursor::new(data)).with_guessed_format() else { return false };
    if reader.format().is_none() {
        return true; // not raster: an SVG, which can't embed anything further
    }
    reader.into_dimensions().is_ok_and(|(w, h)| w.max(h) <= MAX_EMBEDDED_SIDE)
}

fn render(bytes: &[u8], hint: SizeHint, options: &Options<'_>) -> Result<Arc<ColorImage>, String> {
    if bytes.len() > MAX_SVG_BYTES {
        return Err("too large".into());
    }
    if is_gzip(bytes) {
        return Err("compressed SVG not supported".into());
    }
    std::panic::catch_unwind(AssertUnwindSafe(|| egui_extras::image::load_svg_bytes_with_size(bytes, hint, options)))
        .unwrap_or_else(|_| Err("can't be rendered".into()))
        .map(Arc::new)
}

/// Install on top of the `egui_extras` loaders: the newest loader is tried first, and this one
/// accepts every URI theirs would, so theirs is never reached.
pub fn install(ctx: &egui::Context) {
    ctx.add_image_loader(Arc::new(SvgLoader::default()));
}

impl Default for SvgLoader {
    fn default() -> Self {
        Self { state: Arc::default(), options: Arc::new(options()) }
    }
}

impl SvgLoader {
    /// Render on a thread of its own. The caller holds the state lock, and has checked for a free slot.
    fn start(&self, state: &mut State, ctx: &egui::Context, uri: &str, hint: SizeHint, bytes: Arc<[u8]>) -> Result<u64, String> {
        let job = state.next_job;
        let (shared, options, ctx, key) = (Arc::clone(&self.state), Arc::clone(&self.options), ctx.clone(), uri.to_owned());
        std::thread::Builder::new()
            .name("svg".into())
            .stack_size(RENDER_STACK_BYTES)
            .spawn(move || {
                let result = render(&bytes, hint, &options);
                let mut state = shared.lock();
                state.renders -= 1;
                let entry = state.cache.get_mut(&key).and_then(|bucket| bucket.get_mut(&hint));
                if let Some(entry) = entry.filter(|e| e.job == job) {
                    entry.result = Poll::Ready(result);
                }
                drop(state);
                ctx.request_repaint();
            })
            .map_err(|e| e.to_string())?;
        state.next_job += 1;
        state.renders += 1;
        Ok(job)
    }
}

impl ImageLoader for SvgLoader {
    fn id(&self) -> &'static str {
        egui::generate_loader_id!(SvgLoader)
    }

    fn load(&self, ctx: &egui::Context, uri: &str, size_hint: SizeHint) -> ImageLoadResult {
        if !egui::load::has_extension(uri, "svg") {
            return Err(LoadError::NotSupported);
        }
        let mut state = self.state.lock();
        let pass_index = state.pass_index;
        if let Some(entry) = state.cache.get_mut(uri).and_then(|bucket| bucket.get_mut(&size_hint)) {
            entry.last_used = pass_index;
            return match &entry.result {
                Poll::Pending => Ok(ImagePoll::Pending { size: None }),
                Poll::Ready(Ok(image)) => Ok(ImagePoll::Ready { image: Arc::clone(image) }),
                Poll::Ready(Err(err)) => Err(LoadError::Loading(err.clone())),
            };
        }
        let bytes = match ctx.try_load_bytes(uri)? {
            BytesPoll::Pending { size } => return Ok(ImagePoll::Pending { size }),
            BytesPoll::Ready { bytes, .. } => bytes,
        };
        // All slots taken: the next render to end repaints, and this asks again.
        if state.renders >= MAX_RENDERS {
            return Ok(ImagePoll::Pending { size: None });
        }
        let job = self.start(&mut state, ctx, uri, size_hint, bytes.as_ref().into()).map_err(LoadError::Loading)?;
        state.cache.entry(uri.to_owned()).or_default().insert(size_hint, Entry { last_used: pass_index, job, result: Poll::Pending });
        Ok(ImagePoll::Pending { size: None })
    }

    fn forget(&self, uri: &str) {
        self.state.lock().cache.remove(uri);
    }

    fn forget_all(&self) {
        self.state.lock().cache.clear();
    }

    fn byte_size(&self) -> usize {
        let state = self.state.lock();
        let entries = state.cache.values().flat_map(HashMap::values);
        entries
            .map(|e| match &e.result {
                Poll::Pending => 0,
                Poll::Ready(Ok(image)) => image.pixels.len() * size_of::<egui::Color32>(),
                Poll::Ready(Err(err)) => err.len(),
            })
            .sum()
    }

    /// Drop sizes no longer shown, e.g. while the window is being resized.
    fn end_pass(&self, pass_index: u64) {
        let mut state = self.state.lock();
        state.pass_index = pass_index;
        state.cache.retain(|_, bucket| {
            if bucket.len() > 1 {
                bucket.retain(|_, e| pass_index <= e.last_used + 1);
            }
            !bucket.is_empty()
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HINT: SizeHint = SizeHint::Size { width: 8, height: 8, maintain_aspect_ratio: true };

    /// A red 8x8 SVG, to be embedded through `href`.
    const RED: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="red"/></svg>"#;

    fn draw(svg: &str, options: &Options<'_>) -> ColorImage {
        egui_extras::image::load_svg_bytes_with_size(svg.as_bytes(), HINT, options).expect("render")
    }

    fn painted(image: &ColorImage) -> bool {
        image.pixels.iter().any(|p| p.a() != 0)
    }

    fn with_image(attr: &str) -> String {
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="8" height="8"><image width="8" height="8" {attr}/></svg>"#
        )
    }

    #[test]
    fn external_hrefs_are_not_read() {
        let dir = tempfile::tempdir().expect("tempdir");
        let red = dir.path().join("red.svg");
        std::fs::write(&red, RED).expect("write");
        let path = red.display().to_string();
        let encoded: String = path.chars().map(|c| format!("&#{};", c as u32)).collect();
        for attr in [format!(r#"href="{path}""#), format!(r#"xlink:href="{path}""#), format!(r#"href="{encoded}""#)] {
            let svg = with_image(&attr);
            assert!(painted(&draw(&svg, &Options::default())), "the default resolver reads {attr}");
            assert!(!painted(&draw(&svg, &options())), "{attr}");
        }
    }

    #[test]
    fn fifo_href_does_not_block() {
        let dir = tempfile::tempdir().expect("tempdir");
        let fifo = dir.path().join("fifo.svg");
        if !std::process::Command::new("mkfifo").arg(&fifo).status().is_ok_and(|s| s.success()) {
            return;
        }
        let svg = with_image(&format!(r#"href="{}""#, fifo.display()));
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || tx.send(painted(&draw(&svg, &options()))));
        let done = rx.recv_timeout(std::time::Duration::from_secs(10));
        assert_eq!(done, Ok(false));
    }

    #[test]
    fn data_uris_and_fragments_still_render() {
        let data = format!("data:image/svg+xml;utf8,{}", RED.replace('"', "%22").replace('<', "%3C"));
        assert!(painted(&draw(&with_image(&format!(r#"href="{data}""#)), &options())), "data URI");
        let fragment = r##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><defs><rect id="r" width="8" height="8" fill="red"/></defs><use href="#r"/></svg>"##;
        assert!(painted(&draw(fragment, &options())), "#fragment");
    }

    /// Poll until the render ends. The first answer must be Pending: the UI thread doesn't wait.
    fn load_slowly(loader: &SvgLoader, ctx: &egui::Context, uri: &str) -> Result<Arc<ColorImage>, String> {
        assert!(matches!(loader.load(ctx, uri, HINT), Ok(ImagePoll::Pending { .. })), "rendered on the caller's thread");
        for _ in 0..3000 {
            match loader.load(ctx, uri, HINT) {
                Ok(ImagePoll::Ready { image }) => return Ok(image),
                Err(LoadError::Loading(e)) => return Err(e),
                Err(_) => panic!("not supported"),
                Ok(ImagePoll::Pending { .. }) => std::thread::sleep(std::time::Duration::from_millis(10)),
            }
        }
        panic!("render never ended");
    }

    fn load_bytes(bytes: Vec<u8>) -> Result<Arc<ColorImage>, String> {
        let ctx = egui::Context::default();
        egui_extras::install_image_loaders(&ctx);
        ctx.include_bytes("bytes://a.svg", bytes);
        load_slowly(&SvgLoader::default(), &ctx, "bytes://a.svg")
    }

    #[test]
    fn renders_off_the_calling_thread() {
        assert!(load_bytes(RED.into()).is_ok_and(|image| painted(&image)));
    }

    #[test]
    fn oversized_and_gzipped_svgs_are_refused() {
        let mut big = RED.to_owned();
        big.push_str(&" ".repeat(MAX_SVG_BYTES));
        assert_eq!(load_bytes(big.into()).err().as_deref(), Some("too large"));
        assert_eq!(load_bytes(vec![0x1f, 0x8b, 8, 0]).err().as_deref(), Some("compressed SVG not supported"));
    }

    #[test]
    fn deep_nesting_and_bad_input_do_not_crash() {
        let deep = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8">{}{}</svg>"#, "<g>".repeat(1000), "</g>".repeat(1000));
        assert!(load_bytes(deep.into()).is_ok());
        assert!(load_bytes(b"<svg".to_vec()).is_err());
    }

    /// The renders in flight don't outlive their slots, and a forgotten one leaves nothing behind.
    #[test]
    fn forgotten_render_is_dropped() {
        let ctx = egui::Context::default();
        egui_extras::install_image_loaders(&ctx);
        ctx.include_bytes("bytes://a.svg", RED.as_bytes());
        let loader = SvgLoader::default();
        assert!(matches!(loader.load(&ctx, "bytes://a.svg", HINT), Ok(ImagePoll::Pending { .. })));
        loader.forget("bytes://a.svg");
        for _ in 0..1000 {
            if loader.state.lock().renders == 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(loader.state.lock().renders, 0);
        assert_eq!(loader.byte_size(), 0);
    }

    #[test]
    fn busy_slots_wait_their_turn() {
        let ctx = egui::Context::default();
        egui_extras::install_image_loaders(&ctx);
        ctx.include_bytes("bytes://a.svg", RED.as_bytes());
        let loader = SvgLoader::default();
        loader.state.lock().renders = MAX_RENDERS;
        assert!(matches!(loader.load(&ctx, "bytes://a.svg", HINT), Ok(ImagePoll::Pending { .. })));
        assert!(loader.state.lock().cache.is_empty(), "no render started");
        loader.state.lock().renders = 0;
        assert!(load_slowly(&loader, &ctx, "bytes://a.svg").is_ok_and(|image| painted(&image)));
    }

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut png = std::io::Cursor::new(Vec::new());
        image::RgbaImage::new(w, h).write_to(&mut png, image::ImageFormat::Png).expect("png");
        png.into_inner()
    }

    #[test]
    fn embedded_images_are_limited() {
        assert!(embedded_allowed(RED.as_bytes()));
        assert!(embedded_allowed(&png(MAX_EMBEDDED_SIDE, 1)));
        assert!(!embedded_allowed(&png(MAX_EMBEDDED_SIDE + 1, 1)));
        assert!(!embedded_allowed(&vec![b' '; MAX_EMBEDDED_BYTES + 1]));
        assert!(!embedded_allowed(&[0x1f, 0x8b, 8, 0]));

        // The limits are wired in: an embedded SVG over the size limit isn't drawn.
        let big = RED.replace("<rect", &format!("{}<rect", " ".repeat(MAX_EMBEDDED_BYTES)));
        let uri = format!("data:image/svg+xml;utf8,{}", big.replace('"', "%22").replace('<', "%3C"));
        assert!(!painted(&draw(&with_image(&format!(r#"href="{uri}""#)), &options())));
        assert!(painted(&draw(&with_image(&format!(r#"href="{uri}""#)), &Options::default())), "the default resolver draws it");
    }

    #[test]
    fn takes_over_from_the_stock_loader() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("a.svg");
        std::fs::write(&path, RED).expect("write");
        let ctx = egui::Context::default();
        egui_extras::install_image_loaders(&ctx);
        install(&ctx);
        assert!(ctx.loaders().image.lock().last().is_some_and(|l| l.id().contains("mdedit")));

        // Rendering through the context, then forgetting, leaves nothing behind.
        let uri = format!("file://{}", path.display());
        let mut ready = false;
        for _ in 0..200 {
            ready = matches!(ctx.try_load_image(&uri, HINT), Ok(ImagePoll::Ready { .. }));
            if ready {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(ready);
        let held = |ctx: &egui::Context| ctx.loaders().image.lock().iter().map(|l| l.byte_size()).sum::<usize>();
        assert!(held(&ctx) > 0);
        ctx.forget_image(&uri);
        assert_eq!(held(&ctx), 0);
    }
}

//! SVG loader that never follows an `<image href>` out of the document. usvg's default resolver
//! reads any local path, past our size limit and FIFO checks; `egui_extras`' SVG loader can't be
//! configured, so this one takes its place (same URIs, same caching) and only embeds data URIs.

use std::collections::HashMap;
use std::sync::Arc;

use egui::ColorImage;
use egui::load::{BytesPoll, ImageLoadResult, ImageLoader, ImagePoll, LoadError, SizeHint};
use egui::mutex::Mutex;
use resvg::usvg::{ImageHrefResolver, Options};

struct Entry {
    last_used: u64,
    result: Result<Arc<ColorImage>, String>,
}

#[derive(Default)]
struct State {
    pass_index: u64,
    /// One entry per rasterized size of a URI.
    cache: HashMap<String, HashMap<SizeHint, Entry>>,
}

struct SvgLoader {
    state: Mutex<State>,
    options: Options<'static>,
}

fn options() -> Options<'static> {
    let image_href_resolver = ImageHrefResolver { resolve_string: Box::new(|_, _| None), ..ImageHrefResolver::default() };
    Options { image_href_resolver, ..Options::default() }
}

/// Install on top of the `egui_extras` loaders: the newest loader is tried first, and this one
/// accepts every URI theirs would, so theirs is never reached.
pub fn install(ctx: &egui::Context) {
    ctx.add_image_loader(Arc::new(SvgLoader { state: Mutex::default(), options: options() }));
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
        let bucket = state.cache.entry(uri.to_owned()).or_default();
        let result = if let Some(entry) = bucket.get_mut(&size_hint) {
            entry.last_used = pass_index;
            entry.result.clone()
        } else {
            match ctx.try_load_bytes(uri)? {
                BytesPoll::Pending { size } => return Ok(ImagePoll::Pending { size }),
                BytesPoll::Ready { bytes, .. } => {
                    let result = egui_extras::image::load_svg_bytes_with_size(&bytes, size_hint, &self.options).map(Arc::new);
                    bucket.insert(size_hint, Entry { last_used: pass_index, result: result.clone() });
                    result
                }
            }
        };
        match result {
            Ok(image) => Ok(ImagePoll::Ready { image }),
            Err(err) => Err(LoadError::Loading(err)),
        }
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
        entries.map(|e| e.result.as_ref().map_or_else(String::len, |i| i.pixels.len() * size_of::<egui::Color32>())).sum()
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

    fn render(svg: &str, options: &Options<'_>) -> ColorImage {
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
            assert!(painted(&render(&svg, &Options::default())), "the default resolver reads {attr}");
            assert!(!painted(&render(&svg, &options())), "{attr}");
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
        std::thread::spawn(move || tx.send(painted(&render(&svg, &options()))));
        let done = rx.recv_timeout(std::time::Duration::from_secs(10));
        assert_eq!(done, Ok(false));
    }

    #[test]
    fn data_uris_and_fragments_still_render() {
        let data = format!("data:image/svg+xml;utf8,{}", RED.replace('"', "%22").replace('<', "%3C"));
        assert!(painted(&render(&with_image(&format!(r#"href="{data}""#)), &options())), "data URI");
        let fragment = r##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><defs><rect id="r" width="8" height="8" fill="red"/></defs><use href="#r"/></svg>"##;
        assert!(painted(&render(fragment, &options())), "#fragment");
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

//! HTTP loader that stops at [`MAX_REMOTE_BYTES`]. `egui_extras`' loader holds a whole response in
//! memory before anyone can look at its size, so a hostile server could fill the memory; this one
//! takes its place (same URIs, same caching).

use std::collections::HashMap;
use std::ops::ControlFlow;
use std::sync::Arc;
use std::task::Poll;

use egui::load::{Bytes, BytesLoadResult, BytesLoader, BytesPoll, LoadError};
use egui::mutex::Mutex;
use ehttp::streaming::Part;

/// Largest remote image we download, the same as for a local file.
const MAX_REMOTE_BYTES: usize = 20 * 1024 * 1024;

#[derive(Clone)]
struct File {
    bytes: Arc<[u8]>,
    mime: Option<String>,
}

type Cache = Arc<Mutex<HashMap<String, Poll<Result<File, String>>>>>;

#[derive(Default)]
struct HttpLoader {
    cache: Cache,
}

/// Install on top of the `egui_extras` loaders: the newest loader is tried first, and this one
/// accepts every URI theirs would, so theirs is never reached.
pub fn install(ctx: &egui::Context) {
    ctx.add_bytes_loader(Arc::new(HttpLoader::default()));
}

/// One download. `None` means go on, `Some` is the outcome.
#[derive(Default)]
struct Download {
    bytes: Vec<u8>,
    mime: Option<String>,
}

impl Download {
    fn on_part(&mut self, part: ehttp::Result<Part>) -> Option<Result<File, String>> {
        match part {
            Err(_) => Some(Err("download failed".into())),
            Ok(Part::Response(r)) if !r.ok => Some(Err(format!("{} {}", r.status, r.status_text))),
            Ok(Part::Response(r)) => {
                let length = r.headers.get("content-length").and_then(|l| l.parse::<usize>().ok());
                self.mime = r.headers.get("content-type").map(str::to_owned);
                length.is_some_and(|l| l > MAX_REMOTE_BYTES).then(|| Err("too large".into()))
            }
            Ok(Part::Chunk(chunk)) if chunk.is_empty() => {
                Some(Ok(File { bytes: std::mem::take(&mut self.bytes).into(), mime: self.mime.take() }))
            }
            Ok(Part::Chunk(chunk)) => {
                if self.bytes.len() + chunk.len() > MAX_REMOTE_BYTES {
                    return Some(Err("too large".into()));
                }
                self.bytes.extend_from_slice(&chunk);
                None
            }
        }
    }
}

impl BytesLoader for HttpLoader {
    fn id(&self) -> &'static str {
        egui::generate_loader_id!(HttpLoader)
    }

    fn load(&self, ctx: &egui::Context, uri: &str) -> BytesLoadResult {
        if !uri.starts_with("http://") && !uri.starts_with("https://") {
            return Err(LoadError::NotSupported);
        }
        let mut cache = self.cache.lock();
        match cache.get(uri) {
            Some(Poll::Ready(Ok(file))) => Ok(BytesPoll::Ready { size: None, bytes: Bytes::Shared(file.bytes.clone()), mime: file.mime.clone() }),
            Some(Poll::Ready(Err(err))) => Err(LoadError::Loading(err.clone())),
            Some(Poll::Pending) => Ok(BytesPoll::Pending { size: None }),
            None => {
                cache.insert(uri.to_owned(), Poll::Pending);
                drop(cache);
                let (cache, ctx, key) = (Arc::clone(&self.cache), ctx.clone(), uri.to_owned());
                let download = Mutex::new(Download::default());
                ehttp::streaming::fetch(ehttp::Request::get(uri), move |part| {
                    let Some(result) = download.lock().on_part(part) else { return ControlFlow::Continue(()) };
                    // A forgotten URI stays forgotten.
                    if let Some(entry) = cache.lock().get_mut(&key) {
                        *entry = Poll::Ready(result);
                    }
                    ctx.request_repaint();
                    ControlFlow::Break(())
                });
                Ok(BytesPoll::Pending { size: None })
            }
        }
    }

    fn forget(&self, uri: &str) {
        self.cache.lock().remove(uri);
    }

    fn forget_all(&self) {
        self.cache.lock().clear();
    }

    fn byte_size(&self) -> usize {
        let cache = self.cache.lock();
        cache.values().map(|e| if let Poll::Ready(Ok(f)) = e { f.bytes.len() } else { 0 }).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read as _, Write as _};

    /// Answer one request with `head` and then `body` until the client hangs up.
    fn serve(head: &'static str, body: Vec<u8>, repeat: bool) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        std::thread::spawn(move || {
            let (mut s, _) = listener.accept().expect("accept");
            let _ = s.read(&mut [0; 4096]);
            let _ = s.write_all(head.as_bytes());
            while s.write_all(&body).is_ok() && repeat {}
        });
        format!("http://{addr}/a.png")
    }

    fn fetch(uri: &str) -> Result<Arc<[u8]>, String> {
        let (loader, ctx) = (HttpLoader::default(), egui::Context::default());
        for _ in 0..1500 {
            match loader.load(&ctx, uri) {
                Ok(BytesPoll::Ready { bytes, .. }) => return Ok(bytes.as_ref().into()),
                Err(LoadError::Loading(e)) => return Err(e),
                Err(_) => return Err("not supported".into()),
                Ok(BytesPoll::Pending { .. }) => std::thread::sleep(std::time::Duration::from_millis(10)),
            }
        }
        panic!("no result");
    }

    #[test]
    fn downloads_a_normal_image() {
        let uri = serve("HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: 3\r\nConnection: close\r\n\r\n", b"abc".to_vec(), false);
        assert_eq!(fetch(&uri).as_deref(), Ok(&b"abc"[..]));
    }

    #[test]
    fn endless_response_stops_at_the_limit() {
        let uri = serve("HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n", vec![0; 64 * 1024], true);
        assert_eq!(fetch(&uri).err().as_deref(), Some("too large"));
    }

    #[test]
    fn announced_size_over_the_limit_is_not_downloaded() {
        let uri = serve("HTTP/1.1 200 OK\r\nContent-Length: 999999999\r\n\r\n", vec![], false);
        assert_eq!(fetch(&uri).err().as_deref(), Some("too large"));
    }

    #[test]
    fn http_error_is_an_error() {
        let uri = serve("HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", vec![], false);
        assert!(fetch(&uri).is_err_and(|e| e.contains("404")));
    }
}

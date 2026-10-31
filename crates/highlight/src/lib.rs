//! Syntax highlighting of code blocks with syntect (+ two-face grammars).
//! docs/code-blocks.md CB-24. No egui dependency: colours are plain RGB.
//!
//! Syntaxes (two-face's extended set, which includes TypeScript, TOML, Dockerfile, ...) and
//! the two themes (GitHub for light, OneHalfDark for dark) are loaded lazily on first use and
//! shared process-wide. Call [`Highlighter::preload`] from a background thread at startup to
//! avoid the ~100 ms first-use hitch on the UI thread.
//!
//! Grammars can take seconds on hostile or big input, so the UI path uses
//! [`Highlighter::highlight_nonblocking`]: it never runs syntect on the calling thread. A worker
//! does, and [`Highlighter::generation`] advances (and the waker runs) when a result is ready.
//! [`Highlighter::highlight`] and [`Highlighter::highlight_shared`] stay synchronous, for callers
//! on their own thread such as HTML export.
//!
//! OWNER: agent "highlight". STABLE items are used by other crates.

use std::collections::{HashMap, VecDeque};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use syntect::easy::HighlightLines;
use syntect::highlighting::{FontStyle, Theme};
use syntect::parsing::{SyntaxReference, SyntaxSet};
use syntect::util::LinesWithEndings;

/// STABLE.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rgb(pub u8, pub u8, pub u8);

/// A highlighted piece of one line; `range` is relative to the start of the code text.
/// STABLE.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HlSpan {
    pub range: Range<usize>,
    pub fg: Rgb,
    pub bold: bool,
    pub italic: bool,
}

/// STABLE.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HlTheme {
    Light,
    Dark,
}

/// Largest code text that is highlighted (CB-24 guard).
pub const MAX_HIGHLIGHT_BYTES: usize = 2 * 1024 * 1024;
/// Number of cached highlight results per [`Highlighter`].
pub const CACHE_CAPACITY: usize = 256;
/// Approximate bytes of cached spans per [`Highlighter`]; the newest entry is always kept.
pub const CACHE_MAX_BYTES: usize = 32 * 1024 * 1024;
/// Background jobs leave longer lines unhighlighted: a grammar can be quadratic in the line
/// length, and a job can only be dropped between lines.
pub const MAX_BACKGROUND_LINE: usize = 8 * 1024;
/// Worker threads, so one slow block doesn't hold up the others.
const WORKERS: usize = 2;
/// Queued jobs kept (the oldest is dropped; its chunk asks again on the next result).
const MAX_PENDING: usize = 32;
/// Latest results kept with their code, to answer for an edited version of that code.
const RECENT: usize = 4;

struct Assets {
    syntaxes: SyntaxSet,
    light: Theme,
    dark: Theme,
}

fn assets() -> &'static Assets {
    static ASSETS: OnceLock<Assets> = OnceLock::new();
    ASSETS.get_or_init(|| {
        let themes = two_face::theme::extra();
        Assets {
            syntaxes: two_face::syntax::extra_newlines(),
            light: themes.get(two_face::theme::EmbeddedThemeName::Github).clone(),
            dark: themes.get(two_face::theme::EmbeddedThemeName::OneHalfDark).clone(),
        }
    })
}

impl Assets {
    fn theme(&self, t: HlTheme) -> &Theme {
        match t {
            HlTheme::Light => &self.light,
            HlTheme::Dark => &self.dark,
        }
    }
}

/// Extra aliases on top of syntect's extension / name lookup. Keys are lowercase.
const ALIASES: &[(&str, &str)] = &[
    ("rust", "rs"),
    ("python", "py"),
    ("python3", "py"),
    ("py3", "py"),
    ("bash", "bash"),
    ("shell", "bash"),
    ("sh", "bash"),
    ("zsh", "bash"),
    ("console", "bash"),
    ("shellsession", "bash"),
    ("javascript", "js"),
    ("node", "js"),
    ("typescript", "ts"),
    ("c++", "cpp"),
    ("cxx", "cpp"),
    ("cplusplus", "cpp"),
    ("c#", "cs"),
    ("csharp", "cs"),
    ("golang", "go"),
    ("yml", "yaml"),
    ("docker", "Dockerfile"),
    ("dockerfile", "Dockerfile"),
    ("make", "Makefile"),
    ("makefile", "Makefile"),
    ("md", "md"),
    ("markdown", "md"),
    ("patch", "diff"),
    ("udiff", "diff"),
    ("html", "html"),
    ("xhtml", "html"),
    ("xml", "xml"),
    ("svg", "xml"),
    ("objective-c", "m"),
    ("objc", "m"),
    ("kotlin", "kt"),
    ("ruby", "rb"),
    ("perl", "pl"),
    ("haskell", "hs"),
    ("elixir", "ex"),
    ("erlang", "erl"),
    ("viml", "vim"),
    ("vimscript", "vim"),
    ("tex", "tex"),
    ("latex", "tex"),
    ("protobuf", "proto"),
    ("terraform", "tf"),
    ("hcl", "tf"),
    ("jsonc", "json"),
    ("json5", "json"),
    ("ini", "ini"),
    ("conf", "ini"),
    ("nix", "nix"),
    ("zig", "zig"),
];

/// Languages listed first by [`Highlighter::languages`].
const COMMON: &[&str] = &[
    "rust",
    "python",
    "bash",
    "javascript",
    "typescript",
    "c",
    "cpp",
    "go",
    "java",
    "json",
    "yaml",
    "toml",
    "html",
    "css",
    "sql",
    "markdown",
    "dockerfile",
    "makefile",
    "diff",
];

fn find_syntax<'a>(ss: &'a SyntaxSet, lang: &str) -> Option<&'a SyntaxReference> {
    let lang = lang.trim();
    if lang.is_empty() {
        return None;
    }
    let lower = lang.to_ascii_lowercase();
    let found = ALIASES
        .iter()
        .find(|(k, _)| *k == lower)
        .and_then(|(_, tok)| ss.find_syntax_by_token(tok))
        .or_else(|| ss.find_syntax_by_token(&lower))
        .or_else(|| ss.find_syntax_by_token(lang))
        .or_else(|| ss.syntaxes().iter().find(|s| s.name.eq_ignore_ascii_case(lang)));
    // Plain text is "no highlighting": the caller falls back to its own code colour.
    found.filter(|s| s.name != "Plain Text")
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    lang: String,
    hash: u64,
    len: usize,
    theme: HlTheme,
    /// Long lines left unhighlighted (see [`MAX_BACKGROUND_LINE`]): not the synchronous result.
    capped: bool,
}

type Shared = Option<Arc<Vec<HlSpan>>>;

#[derive(Default)]
struct Cache {
    /// Value + last-use tick.
    map: HashMap<Key, (Shared, u64)>,
    tick: u64,
    /// Estimated size of the values in `map`.
    bytes: usize,
}

fn entry_bytes(val: &Shared) -> usize {
    val.as_ref().map_or(0, |v| v.len() * size_of::<HlSpan>()) + size_of::<Key>() + 64
}

impl Cache {
    fn get(&mut self, key: &Key) -> Option<Shared> {
        self.tick += 1;
        let tick = self.tick;
        self.map.get_mut(key).map(|e| {
            e.1 = tick;
            e.0.clone()
        })
    }
    fn put(&mut self, key: Key, val: Shared) {
        if self.map.len() >= CACHE_CAPACITY && !self.map.contains_key(&key) {
            // Evict the least recently used quarter (amortised O(1) per insert).
            let mut ticks: Vec<u64> = self.map.values().map(|e| e.1).collect();
            ticks.sort_unstable();
            let cut = ticks[CACHE_CAPACITY / 4];
            self.map.retain(|_, e| e.1 >= cut);
            self.bytes = self.map.values().map(|e| entry_bytes(&e.0)).sum();
        }
        self.tick += 1;
        self.bytes += entry_bytes(&val);
        if let Some(old) = self.map.insert(key, (val, self.tick)) {
            self.bytes -= entry_bytes(&old.0);
        }
        if self.bytes > CACHE_MAX_BYTES {
            self.evict_to_budget();
        }
    }
    /// Drop least recently used entries down to 3/4 of the budget (amortised, like the entry
    /// cap), never the newest: a result must survive until its chunk has fetched it.
    fn evict_to_budget(&mut self) {
        let mut ticks: Vec<(u64, usize)> = self.map.values().map(|e| (e.1, entry_bytes(&e.0))).collect();
        ticks.sort_unstable();
        let mut cut = 0;
        for &(tick, size) in &ticks[..ticks.len() - 1] {
            if self.bytes <= CACHE_MAX_BYTES / 4 * 3 {
                break;
            }
            self.bytes -= size;
            cut = tick + 1;
        }
        self.map.retain(|_, e| e.1 >= cut);
    }
}

/// Answer of [`Highlighter::highlight_nonblocking`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Highlight {
    /// The exact result (`None`: nothing to highlight, e.g. an unknown language).
    Ready(Option<Arc<Vec<HlSpan>>>),
    /// A worker is on it. Carries the spans of a similar earlier version of the code, adjusted
    /// to this text, if there is one; the caller shows those or plain text meanwhile.
    Pending(Option<Arc<Vec<HlSpan>>>),
}

impl Highlight {
    pub fn spans(&self) -> Option<&Arc<Vec<HlSpan>>> {
        match self {
            Self::Ready(s) | Self::Pending(s) => s.as_ref(),
        }
    }
    pub fn is_pending(&self) -> bool {
        matches!(self, Self::Pending(_))
    }
}

struct Job {
    key: Key,
    lang: String,
    code: Arc<str>,
    cancel: Arc<AtomicBool>,
}

struct Recent {
    key: Key,
    code: Arc<str>,
    spans: Arc<Vec<HlSpan>>,
}

#[derive(Default)]
struct Queue {
    pending: Vec<Job>,
    running: Vec<Job>,
    workers: usize,
    shutdown: bool,
    recent: VecDeque<Recent>,
    /// Last adjusted result: every chunk of a big block asks for the same one.
    adjusted: Option<(Key, Shared)>,
}

type Waker = Arc<dyn Fn() + Send + Sync>;

#[derive(Default)]
struct Inner {
    cache: Mutex<Cache>,
    queue: Mutex<Queue>,
    work: Condvar,
    generation: AtomicU64,
    waker: Mutex<Option<Waker>>,
}

fn relock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // A poisoned lock still guards a structurally valid value; keep using it.
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Length of the common prefix of `a` and `b`, comparing blocks first (memcmp is much faster
/// than a byte loop, which matters at megabytes per keystroke).
fn prefix_len(a: &[u8], b: &[u8]) -> usize {
    const BLOCK: usize = 4096;
    let max = a.len().min(b.len());
    let mut p = 0;
    while p + BLOCK <= max && a[p..p + BLOCK] == b[p..p + BLOCK] {
        p += BLOCK;
    }
    p + a[p..max].iter().zip(&b[p..max]).take_while(|(x, y)| x == y).count()
}

fn suffix_len(a: &[u8], b: &[u8]) -> usize {
    const BLOCK: usize = 4096;
    let max = a.len().min(b.len());
    let (a, b) = (&a[a.len() - max..], &b[b.len() - max..]);
    let mut s = 0;
    while s + BLOCK <= max && a[max - s - BLOCK..max - s] == b[max - s - BLOCK..max - s] {
        s += BLOCK;
    }
    s + a[..max - s].iter().rev().zip(b[..max - s].iter().rev()).take_while(|(x, y)| x == y).count()
}

/// Common prefix and suffix (not overlapping, on char boundaries) of two strings.
fn affixes(a: &str, b: &str) -> (usize, usize) {
    let mut p = prefix_len(a.as_bytes(), b.as_bytes());
    while !(a.is_char_boundary(p) && b.is_char_boundary(p)) {
        p -= 1;
    }
    let mut s = suffix_len(&a.as_bytes()[p..], &b.as_bytes()[p..]);
    while !(a.is_char_boundary(a.len() - s) && b.is_char_boundary(b.len() - s)) {
        s -= 1;
    }
    (p, s)
}

/// Whether `b` looks like an edit of `a`: most of it is unchanged.
fn is_edit_of(a: &str, b: &str) -> bool {
    let (p, s) = affixes(a, b);
    (p + s) * 2 >= a.len().max(b.len())
}

/// `spans` of `old`, adjusted to `new`: spans in the common prefix stay, those in the common
/// suffix move, and the changed text is left without a span.
fn adjust_spans(spans: &[HlSpan], old: &str, new: &str) -> Vec<HlSpan> {
    let (p, s) = affixes(old, new);
    let old_end = old.len() - s;
    let new_end = new.len() - s;
    let mut out = Vec::with_capacity(spans.len());
    for sp in spans {
        if sp.range.start < p {
            out.push(HlSpan { range: sp.range.start..sp.range.end.min(p), ..sp.clone() });
        }
        if sp.range.end > old_end {
            let start = sp.range.start.max(old_end) - old_end + new_end;
            out.push(HlSpan { range: start..sp.range.end - old_end + new_end, ..sp.clone() });
        }
    }
    out
}

fn worker(inner: &Inner) {
    loop {
        let job = {
            let mut q = relock(&inner.queue);
            while q.pending.is_empty() && !q.shutdown {
                q = inner.work.wait(q).unwrap_or_else(|e| e.into_inner());
            }
            if q.shutdown {
                return;
            }
            // Smallest first: short blocks shouldn't wait behind a huge one.
            let i = (0..q.pending.len()).min_by_key(|&i| q.pending[i].code.len()).unwrap_or(0);
            let job = q.pending.swap_remove(i);
            let (code, cancel, key, lang) = (job.code.clone(), job.cancel.clone(), job.key.clone(), job.lang.clone());
            q.running.push(job);
            (code, cancel, key, lang)
        };
        let (code, cancel, key, lang) = job;
        // A grammar bug must not take the worker down; that block just stays plain.
        let run = std::panic::AssertUnwindSafe(|| compute_with(&lang, &code, key.theme, Some(&cancel)));
        let result = std::panic::catch_unwind(run).unwrap_or(Some(None));
        {
            let mut q = relock(&inner.queue);
            q.running.retain(|j| !Arc::ptr_eq(&j.cancel, &cancel));
            let Some(result) = result else { continue };
            let result = result.map(Arc::new);
            if let Some(spans) = &result {
                if q.recent.len() >= RECENT {
                    q.recent.pop_front();
                }
                q.recent.push_back(Recent { key: key.clone(), code: code.clone(), spans: spans.clone() });
            }
            relock(&inner.cache).put(key, result);
        }
        inner.generation.fetch_add(1, Ordering::Release);
        let waker = relock(&inner.waker).clone();
        if let Some(w) = waker {
            w();
        }
    }
}

/// Loads syntaxes/themes once (lazily, they're big) and caches results. Cheap to share by
/// reference; `Send + Sync`. STABLE API.
pub struct Highlighter {
    inner: Arc<Inner>,
    /// [`Self::highlight_nonblocking`] computes on the calling thread (tests, tools).
    inline: bool,
}

impl Drop for Highlighter {
    fn drop(&mut self) {
        let mut q = relock(&self.inner.queue);
        q.shutdown = true;
        q.pending.clear();
        for j in &q.running {
            j.cancel.store(true, Ordering::Relaxed);
        }
        drop(q);
        self.inner.work.notify_all();
    }
}

impl Default for Highlighter {
    fn default() -> Self {
        Self::new()
    }
}

impl Highlighter {
    /// Cheap: grammars load on first use (or [`Self::preload`]).
    pub fn new() -> Self {
        Self { inner: Arc::default(), inline: false }
    }

    /// A highlighter whose [`Self::highlight_nonblocking`] answers on the calling thread, always
    /// [`Highlight::Ready`]: for tests and tools that want the final result at once.
    pub fn synchronous() -> Self {
        Self { inner: Arc::default(), inline: true }
    }

    /// Load grammars and themes now (call from a background thread at startup).
    pub fn preload() {
        let _ = assets();
    }

    /// Highlight `code` written in `lang` (already normalized: "rust", "python", ...).
    /// Returns spans for the whole text with ranges relative to `code` (covering every
    /// non-newline byte), or `None` if the language is unknown. Results are cached by
    /// (lang, hash of code, theme); a cache hit must be O(1)-ish. Code longer than 2 MB returns
    /// `None` (CB-24 large-block guard). Runs syntect on this thread on a cache miss, which can
    /// take seconds: the UI uses [`Self::highlight_nonblocking`].
    pub fn highlight(&self, lang: &str, code: &str, theme: HlTheme) -> Option<Vec<HlSpan>> {
        self.highlight_shared(lang, code, theme).map(|v| (*v).clone())
    }

    /// Like [`Self::highlight`] but returns the cached result without copying it.
    pub fn highlight_shared(&self, lang: &str, code: &str, theme: HlTheme) -> Option<Arc<Vec<HlSpan>>> {
        if code.len() > MAX_HIGHLIGHT_BYTES || lang.trim().is_empty() {
            return None;
        }
        let key = make_key(lang, code, theme, false);
        if let Some(hit) = self.lock().get(&key) {
            return hit;
        }
        let result = compute(lang, code, theme).map(Arc::new);
        self.lock().put(key, result.clone());
        result
    }

    /// [`Self::highlight_shared`] that never runs syntect on this thread: a cache miss queues a
    /// background job and answers [`Highlight::Pending`]. Once [`Self::generation`] has moved on,
    /// ask again. A new job replaces queued and running jobs for earlier versions of the code.
    pub fn highlight_nonblocking(&self, lang: &str, code: &str, theme: HlTheme) -> Highlight {
        if self.inline {
            return Highlight::Ready(self.highlight_shared(lang, code, theme));
        }
        if code.len() > MAX_HIGHLIGHT_BYTES || lang.trim().is_empty() {
            return Highlight::Ready(None);
        }
        let capped = code.split('\n').any(|l| l.len() > MAX_BACKGROUND_LINE);
        let key = make_key(lang, code, theme, capped);
        if let Some(hit) = self.lock().get(&key) {
            return Highlight::Ready(hit);
        }
        let mut q = relock(&self.inner.queue);
        // The newest results survive cache eviction, so visible blocks that together exceed
        // the budget can't keep evicting and recomputing each other.
        if let Some(r) = q.recent.iter().find(|r| r.key == key) {
            return Highlight::Ready(Some(r.spans.clone()));
        }
        let in_flight = q.pending.iter().chain(&q.running).any(|j| j.key == key && !j.cancel.load(Ordering::Relaxed));
        if !in_flight {
            self.enqueue(&mut q, key.clone(), lang, code);
        }
        Highlight::Pending(adjusted_from_recent(&mut q, &key, code))
    }

    fn enqueue(&self, q: &mut Queue, key: Key, lang: &str, code: &str) {
        let same_kind = |k: &Key| k.lang == key.lang && k.theme == key.theme;
        q.pending.retain(|j| !(same_kind(&j.key) && is_edit_of(&j.code, code)));
        for j in q.running.iter().filter(|j| same_kind(&j.key)) {
            if is_edit_of(&j.code, code) {
                j.cancel.store(true, Ordering::Relaxed);
            }
        }
        if q.pending.len() >= MAX_PENDING {
            q.pending.remove(0);
        }
        q.pending.push(Job { key, lang: lang.to_string(), code: code.into(), cancel: Arc::default() });
        if q.workers < WORKERS && q.workers <= q.running.len() {
            let inner = self.inner.clone();
            let spawned = std::thread::Builder::new().name("mdedit-highlight".into()).spawn(move || worker(&inner));
            // Without a thread the text simply stays plain.
            if spawned.is_ok() {
                q.workers += 1;
            }
        }
        self.inner.work.notify_one();
    }

    /// Advances each time a background result is ready. A caller that got
    /// [`Highlight::Pending`] should ask again when this differs from before its request.
    pub fn generation(&self) -> u64 {
        self.inner.generation.load(Ordering::Acquire)
    }

    /// Run `f` when a background result is ready (from a worker thread). No egui in this crate:
    /// the editor passes a repaint request.
    pub fn set_waker(&self, f: impl Fn() + Send + Sync + 'static) {
        *relock(&self.inner.waker) = Some(Arc::new(f));
    }

    pub fn has_waker(&self) -> bool {
        relock(&self.inner.waker).is_some()
    }

    /// Wait for the background jobs to finish, up to `timeout`; returns whether they did.
    pub fn wait_idle(&self, timeout: Duration) -> bool {
        let start = Instant::now();
        loop {
            {
                let q = relock(&self.inner.queue);
                if q.pending.is_empty() && q.running.is_empty() {
                    return true;
                }
            }
            if start.elapsed() >= timeout {
                return false;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn lock(&self) -> MutexGuard<'_, Cache> {
        relock(&self.inner.cache)
    }

    /// Whether `lang` has a grammar.
    pub fn supports(&self, lang: &str) -> bool {
        find_syntax(&assets().syntaxes, lang).is_some()
    }

    /// Language names for the language selector (lowercase, common ones first, then the rest
    /// sorted). Every name is accepted by [`Self::highlight`] and contains no whitespace.
    pub fn languages(&self) -> Vec<String> {
        let ss = &assets().syntaxes;
        let mut out: Vec<String> = COMMON
            .iter()
            .filter(|l| find_syntax(ss, l).is_some())
            .map(|l| (*l).to_string())
            .collect();
        let mut seen: std::collections::HashSet<&str> = COMMON
            .iter()
            .filter_map(|l| find_syntax(ss, l).map(|s| s.name.as_str()))
            .collect();
        let mut rest = Vec::new();
        for s in ss.syntaxes() {
            if s.name == "Plain Text" || s.hidden || !seen.insert(s.name.as_str()) {
                continue;
            }
            let lower = s.name.to_ascii_lowercase();
            let token = if !lower.contains(char::is_whitespace)
                && find_syntax(ss, &lower).is_some_and(|f| f.name == s.name)
            {
                Some(lower)
            } else {
                s.file_extensions
                    .iter()
                    .map(|e| e.to_ascii_lowercase())
                    .find(|e| {
                        !e.contains(char::is_whitespace)
                            && find_syntax(ss, e).is_some_and(|f| f.name == s.name)
                    })
            };
            if let Some(t) = token {
                rest.push(t);
            }
        }
        rest.sort();
        rest.dedup();
        rest.retain(|r| !out.contains(r));
        out.extend(rest);
        out
    }

    /// HTML `<span style="color:#rrggbb">` markup for `code` (escaped), for HTML export.
    /// Falls back to escaped plain text for unknown languages.
    pub fn highlight_html(&self, lang: &str, code: &str, theme: HlTheme) -> String {
        let Some(spans) = self.highlight_shared(lang, code, theme) else {
            return html_escape(code);
        };
        let mut out = String::with_capacity(code.len() * 2);
        let mut pos = 0;
        for s in spans.iter() {
            if s.range.start > pos {
                out.push_str(&html_escape(&code[pos..s.range.start]));
            }
            let Rgb(r, g, b) = s.fg;
            out.push_str(&format!("<span style=\"color:#{r:02x}{g:02x}{b:02x}"));
            if s.bold {
                out.push_str(";font-weight:bold");
            }
            if s.italic {
                out.push_str(";font-style:italic");
            }
            out.push_str("\">");
            out.push_str(&html_escape(&code[s.range.clone()]));
            out.push_str("</span>");
            pos = s.range.end;
        }
        if pos < code.len() {
            out.push_str(&html_escape(&code[pos..]));
        }
        out
    }
}

fn make_key(lang: &str, code: &str, theme: HlTheme, capped: bool) -> Key {
    let mut h = DefaultHasher::new();
    code.hash(&mut h);
    Key { lang: lang.to_ascii_lowercase(), hash: h.finish(), len: code.len(), theme, capped }
}

/// The spans of the most similar recent result, adjusted to `code`.
fn adjusted_from_recent(q: &mut Queue, key: &Key, code: &str) -> Shared {
    if let Some((k, v)) = &q.adjusted
        && k == key
    {
        return v.clone();
    }
    let best = q.recent.iter().rev().find(|r| r.key.lang == key.lang && r.key.theme == key.theme && is_edit_of(&r.code, code));
    let out = best.map(|r| Arc::new(adjust_spans(&r.spans, &r.code, code)));
    q.adjusted = Some((key.clone(), out.clone()));
    out
}

fn compute(lang: &str, code: &str, theme: HlTheme) -> Option<Vec<HlSpan>> {
    compute_with(lang, code, theme, None).flatten()
}

/// Highlight `code`. With `cancel` (a background job): returns `None` once it is set, checked
/// between lines, and leaves lines over [`MAX_BACKGROUND_LINE`] in the theme's plain colour.
fn compute_with(lang: &str, code: &str, theme: HlTheme, cancel: Option<&AtomicBool>) -> Option<Option<Vec<HlSpan>>> {
    let a = assets();
    let Some(syntax) = find_syntax(&a.syntaxes, lang) else { return Some(None) };
    let mut hl = HighlightLines::new(syntax, a.theme(theme));
    let mut spans: Vec<HlSpan> = Vec::new();
    let mut offset = 0usize;
    for line in LinesWithEndings::from(code) {
        let line_start = offset;
        offset += line.len();
        if let Some(c) = cancel {
            if c.load(Ordering::Relaxed) {
                return None;
            }
            if line.len() > MAX_BACKGROUND_LINE {
                let plain = a.theme(theme).settings.foreground.unwrap_or(syntect::highlighting::Color::BLACK);
                let end = line_start + line.trim_end_matches(['\n', '\r']).len();
                spans.push(HlSpan { range: line_start..end, fg: Rgb(plain.r, plain.g, plain.b), bold: false, italic: false });
                continue;
            }
        }
        let pieces = match hl.highlight_line(line, &a.syntaxes) {
            Ok(p) => p,
            Err(_) => {
                // Grammar failure: leave the rest unhighlighted rather than fail.
                break;
            }
        };
        let mut p_off = line_start;
        for (style, piece) in pieces {
            let start = p_off;
            p_off += piece.len();
            // Exclude the line terminator from spans.
            let trimmed = piece.trim_end_matches(['\n', '\r']);
            let end = start + trimmed.len();
            if end <= start {
                continue;
            }
            let fg = Rgb(style.foreground.r, style.foreground.g, style.foreground.b);
            let bold = style.font_style.contains(FontStyle::BOLD);
            let italic = style.font_style.contains(FontStyle::ITALIC);
            if let Some(last) = spans.last_mut()
                && last.range.end == start
                && last.fg == fg
                && last.bold == bold
                && last.italic == italic
            {
                last.range.end = end;
                continue;
            }
            spans.push(HlSpan { range: start..end, fg, bold, italic });
        }
    }
    spans.shrink_to_fit();
    Some(Some(spans))
}

/// Escape `&`, `<`, `>`, `"`, `'`. STABLE.
pub fn html_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&#39;"),
            c => o.push(c),
        }
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn is_send_sync() {
        assert_send_sync::<Highlighter>();
    }

    fn check_spans(code: &str, spans: &[HlSpan]) {
        let mut prev = 0;
        for s in spans {
            assert!(s.range.start >= prev, "spans overlap or unsorted");
            assert!(s.range.start < s.range.end);
            assert!(s.range.end <= code.len());
            assert!(code.is_char_boundary(s.range.start) && code.is_char_boundary(s.range.end));
            prev = s.range.end;
        }
        // Every non-newline byte is covered.
        let mut covered = vec![false; code.len()];
        for s in spans {
            for c in &mut covered[s.range.clone()] {
                *c = true;
            }
        }
        for (i, b) in code.bytes().enumerate() {
            if b != b'\n' && b != b'\r' {
                assert!(covered[i], "byte {i} ({:?}) not covered", b as char);
            }
        }
    }

    #[test]
    fn rust_has_colours() {
        let h = Highlighter::new();
        let code = "fn main() {\n    let x: u32 = 42; // answer\n    println!(\"hi {x}\");\n}\n";
        for theme in [HlTheme::Light, HlTheme::Dark] {
            let spans = h.highlight("rust", code, theme).expect("rust supported");
            check_spans(code, &spans);
            let colours: std::collections::HashSet<_> = spans.iter().map(|s| s.fg).collect();
            assert!(colours.len() >= 2, "{colours:?}");
        }
    }

    #[test]
    fn common_languages_supported() {
        let h = Highlighter::new();
        for l in [
            "rust", "python", "bash", "javascript", "typescript", "cpp", "go", "json", "yaml",
            "toml", "html", "css", "sql", "markdown", "dockerfile", "makefile", "diff", "Rust",
            "PYTHON", "rs", "py", "sh", "js", "ts", "yml", "c++", "c", "java",
        ] {
            assert!(h.supports(l), "{l} should be supported");
            assert!(h.highlight(l, "x = 1\n", HlTheme::Light).is_some(), "{l}");
        }
    }

    #[test]
    fn unknown_lang_none() {
        let h = Highlighter::new();
        assert!(h.highlight("nosuchlang-xyz", "abc", HlTheme::Light).is_none());
        assert!(h.highlight("", "abc", HlTheme::Light).is_none());
        assert!(!h.supports("nosuchlang-xyz"));
        assert_eq!(h.highlight_html("nosuchlang", "<a>&", HlTheme::Dark), "&lt;a&gt;&amp;");
    }

    #[test]
    fn too_large_none() {
        let h = Highlighter::new();
        let big = "a".repeat(MAX_HIGHLIGHT_BYTES + 1);
        assert!(h.highlight("rust", &big, HlTheme::Light).is_none());
    }

    #[test]
    fn tabs_unicode_preserved() {
        let h = Highlighter::new();
        let code = "fn é() {\n\tlet s = \"日本語 🎉\";\t// ünïcödé\r\n}";
        let spans = h.highlight("rust", code, HlTheme::Dark).expect("rust");
        check_spans(code, &spans);
        let rebuilt: String = {
            let mut s = String::new();
            let mut pos = 0;
            for sp in &spans {
                s.push_str(&code[pos..sp.range.start]);
                s.push_str(&code[sp.range.clone()]);
                pos = sp.range.end;
            }
            s.push_str(&code[pos..]);
            s
        };
        assert_eq!(rebuilt, code);
    }

    #[test]
    fn cache_hit_fast() {
        let h = Highlighter::new();
        let code: String = (0..2000).map(|i| format!("fn f{i}() -> u32 {{ {i} }}\n")).collect();
        let t0 = Instant::now();
        let a = h.highlight_shared("rust", &code, HlTheme::Light).expect("rust");
        let first = t0.elapsed();
        let t1 = Instant::now();
        let b = h.highlight_shared("rust", &code, HlTheme::Light).expect("rust");
        let second = t1.elapsed();
        assert!(Arc::ptr_eq(&a, &b));
        assert!(second * 10 < first.max(std::time::Duration::from_micros(100)), "{first:?} vs {second:?}");
    }

    #[test]
    fn cache_bounded() {
        let h = Highlighter::new();
        for i in 0..(CACHE_CAPACITY * 2) {
            let _ = h.highlight("json", &format!("{{\"a\": {i}}}"), HlTheme::Light);
        }
        assert!(h.lock().map.len() <= CACHE_CAPACITY);
    }

    #[test]
    fn cache_bounded_by_bytes() {
        let mut c = Cache::default();
        let span = HlSpan { range: 0..1, fg: Rgb(0, 0, 0), bold: false, italic: false };
        let key = |hash| Key { lang: "rust".into(), hash, len: 0, theme: HlTheme::Light, capped: false };
        let entry = |bytes: usize| Some(Arc::new(vec![span.clone(); bytes / size_of::<HlSpan>()]));
        for i in 0..64 {
            c.put(key(i), entry(CACHE_MAX_BYTES / 8));
            assert!(c.bytes <= CACHE_MAX_BYTES, "{} bytes", c.bytes);
            assert_eq!(c.bytes, c.map.values().map(|e| entry_bytes(&e.0)).sum::<usize>());
            assert!(c.map.contains_key(&key(i)), "newest entry kept");
        }
        assert!(!c.map.contains_key(&key(0)), "oldest evicted");
        // Too big for the budget on its own: kept anyway, or its block would recompute forever.
        c.put(key(100), entry(CACHE_MAX_BYTES * 2));
        assert_eq!(c.map.len(), 1);
        assert!(c.map.contains_key(&key(100)));
        c.put(key(100), entry(16));
        assert_eq!(c.bytes, entry_bytes(&c.map[&key(100)].0), "replacing an entry keeps the count right");
    }

    const WAIT: Duration = Duration::from_secs(60);

    #[test]
    fn nonblocking_delivers_in_the_background() {
        let h = Highlighter::new();
        let woken = Arc::new(AtomicU64::new(0));
        let w = woken.clone();
        h.set_waker(move || {
            w.fetch_add(1, Ordering::SeqCst);
        });
        let code = "fn main() { let x = 1; }\n";
        let before = h.generation();
        let first = h.highlight_nonblocking("rust", code, HlTheme::Dark);
        assert!(h.wait_idle(WAIT));
        assert!(h.generation() > before);
        assert!(woken.load(Ordering::SeqCst) >= 1);
        let Highlight::Ready(Some(spans)) = h.highlight_nonblocking("rust", code, HlTheme::Dark) else { panic!("not ready") };
        check_spans(code, &spans);
        assert_eq!(*spans, h.highlight("rust", code, HlTheme::Dark).unwrap(), "same as the synchronous result");
        assert!(first.is_pending() || first.spans().is_some());
    }

    #[test]
    fn nonblocking_does_not_block_on_hostile_input() {
        let h = Highlighter::new();
        Highlighter::preload();
        // Takes seconds synchronously (quadratic in the line); capped in the background.
        let code = "`${".repeat(32 * 1024 / 3);
        let t = Instant::now();
        let r = h.highlight_nonblocking("javascript", &code, HlTheme::Light);
        assert!(t.elapsed() < Duration::from_millis(100), "{:?}", t.elapsed());
        assert!(r.is_pending());
        assert!(h.wait_idle(Duration::from_secs(5)), "the long line is skipped");
        let Highlight::Ready(Some(spans)) = h.highlight_nonblocking("javascript", &code, HlTheme::Light) else { panic!("not ready") };
        check_spans(&code, &spans);
    }

    #[test]
    fn long_lines_do_not_change_synchronous_results() {
        let h = Highlighter::new();
        let code = format!("let s = \"{}\";\n", "x".repeat(MAX_BACKGROUND_LINE));
        let sync = h.highlight("rust", &code, HlTheme::Light).unwrap();
        let _ = h.highlight_nonblocking("rust", &code, HlTheme::Light);
        assert!(h.wait_idle(WAIT));
        let Highlight::Ready(Some(bg)) = h.highlight_nonblocking("rust", &code, HlTheme::Light) else { panic!("not ready") };
        assert_eq!(bg.len(), 1, "capped line is one plain span");
        check_spans(&code, &bg);
        assert_eq!(h.highlight("rust", &code, HlTheme::Light).unwrap(), sync);
    }

    #[test]
    fn superseded_jobs_are_dropped() {
        let h = Highlighter::new();
        Highlighter::preload();
        let base: String = (0..20_000).map(|i| format!("fn f{i}() -> u32 {{ {i} }}\n")).collect();
        let edit = |i: usize| format!("{base}// {i}\n");
        let t = Instant::now();
        for i in 0..50 {
            let _ = h.highlight_nonblocking("rust", &edit(i), HlTheme::Dark);
        }
        let last = edit(49);
        assert!(h.wait_idle(WAIT));
        assert!(t.elapsed() < WAIT);
        assert!(matches!(h.highlight_nonblocking("rust", &last, HlTheme::Dark), Highlight::Ready(Some(_))), "newest finished");
        let cached = h.lock().map.len();
        assert!(cached < 10, "superseded versions weren't all computed: {cached}");
    }

    #[test]
    fn pending_answers_with_adjusted_previous_spans() {
        let h = Highlighter::new();
        let old = "fn a() {}\nfn b() {}\n";
        let _ = h.highlight_nonblocking("rust", old, HlTheme::Dark);
        assert!(h.wait_idle(WAIT));
        let new = "fn a() {}\n// new line\nfn b() {}\n";
        let r = h.highlight_nonblocking("rust", new, HlTheme::Dark);
        if let Highlight::Pending(Some(sp)) = &r {
            let from = new.find("fn b").unwrap();
            assert!(sp.iter().any(|s| s.range.start >= from), "tail spans moved: {sp:?}");
            assert!(sp.iter().all(|s| s.range.end <= new.len()));
            assert!(sp.windows(2).all(|w| w[0].range.end <= w[1].range.start));
        } else {
            assert!(!r.is_pending(), "a similar result exists: {r:?}");
        }
    }

    #[test]
    fn synchronous_mode_is_always_ready() {
        let h = Highlighter::synchronous();
        let r = h.highlight_nonblocking("rust", "fn main() {}\n", HlTheme::Light);
        assert!(matches!(r, Highlight::Ready(Some(_))));
        assert_eq!(h.generation(), 0);
    }

    #[test]
    fn html_output() {
        let h = Highlighter::new();
        let html = h.highlight_html("rust", "let a = \"<b>\";", HlTheme::Light);
        assert!(html.contains("<span style=\"color:#"));
        assert!(html.contains("&lt;b&gt;"));
        assert!(!html.contains("<b>"));
    }

    #[test]
    fn languages_list() {
        let h = Highlighter::new();
        let langs = h.languages();
        assert_eq!(langs.first().map(String::as_str), Some("rust"));
        assert!(langs.len() > 50, "{}", langs.len());
        for l in &langs {
            assert!(!l.contains(char::is_whitespace));
            assert_eq!(l, &l.to_ascii_lowercase());
            assert!(h.supports(l), "{l}");
        }
        let mut d = langs.clone();
        d.sort();
        d.dedup();
        assert_eq!(d.len(), langs.len());
    }

    proptest::proptest! {
        #[test]
        fn never_panics(code in "[a-z{}()\"'\\t\\n é日/*#<>=;]{0,200}", lang in proptest::sample::select(vec!["rust", "python", "html", "bash", "json", "markdown"])) {
            let h = Highlighter::new();
            if let Some(spans) = h.highlight(lang, &code, HlTheme::Light) {
                check_spans(&code, &spans);
            }
        }

        #[test]
        fn adjusted_spans_stay_sorted_and_in_range(
            old in "[a-zé \\n]{0,80}", at in 0usize..80, del in 0usize..20, ins in "[a-zé\\n]{0,8}"
        ) {
            let h = Highlighter::new();
            let spans = h.highlight("rust", &old, HlTheme::Light).unwrap_or_default();
            let mut a = at.min(old.len());
            while !old.is_char_boundary(a) { a -= 1; }
            let mut b = (a + del).min(old.len());
            while !old.is_char_boundary(b) { b -= 1; }
            let mut new = old.clone();
            new.replace_range(a..b, &ins);
            let out = adjust_spans(&spans, &old, &new);
            let mut prev = 0;
            for s in &out {
                proptest::prop_assert!(s.range.start >= prev && s.range.start < s.range.end && s.range.end <= new.len());
                prev = s.range.end;
            }
        }
    }
}

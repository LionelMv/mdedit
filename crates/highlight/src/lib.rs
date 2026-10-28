//! Syntax highlighting of code blocks with syntect (+ two-face grammars).
//! docs/code-blocks.md CB-24. No egui dependency: colours are plain RGB.
//!
//! Syntaxes (two-face's extended set, which includes TypeScript, TOML, Dockerfile, ...) and
//! the two themes (GitHub for light, OneHalfDark for dark) are loaded lazily on first use and
//! shared process-wide. Call [`Highlighter::preload`] from a background thread at startup to
//! avoid the ~100 ms first-use hitch on the UI thread.
//!
//! OWNER: agent "highlight". STABLE items are used by other crates.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::{Arc, Mutex, OnceLock};

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
    ("powershell", "ps1"),
    ("ps", "ps1"),
    ("pwsh", "ps1"),
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
}

type Shared = Option<Arc<Vec<HlSpan>>>;

#[derive(Default)]
struct Cache {
    /// Value + last-use tick.
    map: HashMap<Key, (Shared, u64)>,
    tick: u64,
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
        }
        self.tick += 1;
        self.map.insert(key, (val, self.tick));
    }
}

/// Loads syntaxes/themes once (lazily, they're big) and caches results. Cheap to share by
/// reference; `Send + Sync`. STABLE API.
pub struct Highlighter {
    cache: Mutex<Cache>,
}

impl Default for Highlighter {
    fn default() -> Self {
        Self::new()
    }
}

impl Highlighter {
    /// Cheap: grammars load on first use (or [`Self::preload`]).
    pub fn new() -> Self {
        Self { cache: Mutex::new(Cache::default()) }
    }

    /// Load grammars and themes now (call from a background thread at startup).
    pub fn preload() {
        let _ = assets();
    }

    /// Highlight `code` written in `lang` (already normalized: "rust", "python", ...).
    /// Returns spans for the whole text with ranges relative to `code` (covering every
    /// non-newline byte), or `None` if the language is unknown. Results are cached by
    /// (lang, hash of code, theme); a cache hit must be O(1)-ish. Code longer than 2 MB returns
    /// `None` (CB-24 large-block guard).
    pub fn highlight(&self, lang: &str, code: &str, theme: HlTheme) -> Option<Vec<HlSpan>> {
        self.highlight_shared(lang, code, theme).map(|v| (*v).clone())
    }

    /// Like [`Self::highlight`] but returns the cached result without copying it.
    pub fn highlight_shared(&self, lang: &str, code: &str, theme: HlTheme) -> Option<Arc<Vec<HlSpan>>> {
        if code.len() > MAX_HIGHLIGHT_BYTES || lang.trim().is_empty() {
            return None;
        }
        let mut h = DefaultHasher::new();
        code.hash(&mut h);
        let key = Key { lang: lang.to_ascii_lowercase(), hash: h.finish(), len: code.len(), theme };
        if let Some(hit) = self.lock().get(&key) {
            return hit;
        }
        let result = compute(lang, code, theme).map(Arc::new);
        self.lock().put(key, result.clone());
        result
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Cache> {
        // A poisoned cache is still structurally valid; keep using it.
        self.cache.lock().unwrap_or_else(|e| e.into_inner())
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

fn compute(lang: &str, code: &str, theme: HlTheme) -> Option<Vec<HlSpan>> {
    let a = assets();
    let syntax = find_syntax(&a.syntaxes, lang)?;
    let mut hl = HighlightLines::new(syntax, a.theme(theme));
    let mut spans: Vec<HlSpan> = Vec::new();
    let mut offset = 0usize;
    for line in LinesWithEndings::from(code) {
        let line_start = offset;
        offset += line.len();
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
    Some(spans)
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
    }
}

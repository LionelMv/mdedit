//! HTML export. OWNER: agent "syntax". STABLE API.
//!
//! Output is a standalone HTML file: embedded CSS, highlighted code (docs/code-blocks.md
//! CB-62: `<pre><code class="language-x">`), task list checkboxes, tables, footnotes.
//! Raw HTML from the Markdown is **sanitized with ammonia** (no scripts, no event handlers,
//! no `javascript:` URLs) unless `allow_raw_html` is set.
//!
//! The renderer walks pulldown-cmark events itself (same options as `mdedit_syntax::parse`)
//! so that code blocks, math, task items and URLs are under our control:
//!
//! * code blocks → `<pre><code class="language-{lang}">` + `Highlighter::highlight_html`;
//! * math → `<span class="math math-inline|math-display">` with the TeX source as text;
//! * task items → `<input type="checkbox" disabled>`;
//! * link/image URLs with `javascript:`, `vbscript:`, `data:` (except `data:image/…` in
//!   images) or other unknown schemes are dropped — always, even with `allow_raw_html`;
//! * YAML front matter at the start of the document is not rendered.

use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::Range;

use mdedit_highlight::{HlTheme, Highlighter, html_escape};
use pulldown_cmark::{
    Alignment, BlockQuoteKind, CodeBlockKind, Event, HeadingLevel, LinkType, Options, Parser, Tag, TagEnd,
};

#[derive(Clone, Debug)]
pub struct HtmlOptions {
    pub title: String,
    pub theme: HlTheme,
    pub allow_raw_html: bool,
}

/// Convert Markdown to a full HTML document.
pub fn to_html(markdown: &str, opts: &HtmlOptions, highlighter: &Highlighter) -> String {
    let (body, first_heading) = render(markdown, opts.theme, highlighter);
    let body = if opts.allow_raw_html { body } else { sanitize_html(&body) };
    let title = if opts.title.trim().is_empty() {
        first_heading.unwrap_or_else(|| "Untitled".to_string())
    } else {
        opts.title.trim().to_string()
    };
    let mut out = String::with_capacity(body.len() + 4096);
    out.push_str("<!DOCTYPE html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n");
    out.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    out.push_str("<meta name=\"generator\" content=\"mdedit\">\n");
    out.push_str(&format!("<title>{}</title>\n<style>\n", html_escape(&title)));
    out.push_str(css(opts.theme));
    out.push_str("</style>\n</head>\n<body>\n<main class=\"markdown-body\">\n");
    out.push_str(&body);
    out.push_str("</main>\n</body>\n</html>\n");
    out
}

/// Convert Markdown to an HTML fragment (body only), for "copy as HTML".
/// Always sanitized; code is highlighted with the light theme.
pub fn to_html_fragment(markdown: &str, highlighter: &Highlighter) -> String {
    let (body, _) = render(markdown, HlTheme::Light, highlighter);
    sanitize_html(&body)
}

/// The embedded stylesheet for `theme` (exposed so the app can reuse it, e.g. for print).
pub fn css(theme: HlTheme) -> &'static str {
    match theme {
        HlTheme::Light => CSS_LIGHT,
        HlTheme::Dark => CSS_DARK,
    }
}

macro_rules! css_common {
    () => {
        r#"
*{box-sizing:border-box}
body{margin:0;background:var(--bg);color:var(--fg);font:16px/1.6 system-ui,-apple-system,"Segoe UI","Noto Sans",Cantarell,Ubuntu,sans-serif}
.markdown-body{max-width:50rem;margin:0 auto;padding:2rem 1.25rem 4rem;overflow-wrap:break-word}
h1,h2,h3,h4,h5,h6{line-height:1.25;margin:1.6em 0 .6em;font-weight:600}
h1{font-size:2em;border-bottom:1px solid var(--border);padding-bottom:.3em}
h2{font-size:1.5em;border-bottom:1px solid var(--border);padding-bottom:.3em}
h3{font-size:1.25em}h4{font-size:1em}h5{font-size:.875em}h6{font-size:.85em;color:var(--muted)}
p,ul,ol,blockquote,pre,table,dl{margin:0 0 1em}
a{color:var(--link);text-decoration:none}a:hover{text-decoration:underline}
img{max-width:100%}
hr{border:0;border-top:2px solid var(--border);margin:2em 0}
blockquote{margin-left:0;padding:0 1em;color:var(--muted);border-left:.25em solid var(--border)}
ul,ol{padding-left:2em}li+li{margin-top:.25em}
li>input[type=checkbox]{margin:0 .4em 0 -1.3em;vertical-align:middle}
li:has(>input[type=checkbox]){list-style:none}
code,pre{font-family:ui-monospace,"DejaVu Sans Mono","Liberation Mono",monospace;font-size:.9em}
:not(pre)>code{background:var(--code-bg);padding:.15em .35em;border-radius:4px}
pre{background:var(--code-bg);border:1px solid var(--border);border-radius:6px;padding:8px 12px;overflow:auto;line-height:1.45;tab-size:4}
pre code{background:none;padding:0;white-space:pre}
table{border-collapse:collapse;display:block;overflow:auto;max-width:100%}
th,td{border:1px solid var(--border);padding:.4em .8em}
th{background:var(--code-bg);font-weight:600}
.math{font-family:"Latin Modern Math","STIX Two Math",serif}
.math-display{display:block;text-align:center;margin:1em 0;overflow:auto}
.footnote-definition{font-size:.9em;color:var(--muted);margin-top:1em}
.footnote-definition>p{display:inline}
.footnote-definition-label{margin-right:.4em}
.markdown-alert{border-left-color:var(--link);color:var(--fg)}
.markdown-alert-title{font-weight:600;margin-bottom:.3em}
.markdown-alert-tip{border-left-color:#1a7f37}.markdown-alert-important{border-left-color:#8250df}
.markdown-alert-warning{border-left-color:#9a6700}.markdown-alert-caution{border-left-color:#cf222e}
@media print{body{background:#fff;color:#000}.markdown-body{max-width:none;padding:0}pre{white-space:pre-wrap;break-inside:avoid}pre code{white-space:pre-wrap}a{color:inherit;text-decoration:underline}}
"#
    };
}

const CSS_LIGHT: &str = concat!(
    ":root{color-scheme:light;--bg:#ffffff;--fg:#1f2328;--muted:#59636e;--border:#d1d9e0;--link:#0969da;--code-bg:#f6f8fa}\n",
    css_common!()
);

const CSS_DARK: &str = concat!(
    ":root{color-scheme:dark;--bg:#0d1117;--fg:#e6edf3;--muted:#9198a1;--border:#3d444d;--link:#4493f8;--code-bg:#151b23}\n",
    css_common!()
);

// ---------------------------------------------------------------------------------------
// URL safety
// ---------------------------------------------------------------------------------------

/// Return the URL if it is safe to put in `href`/`src`, else `None`.
///
/// Allowed: relative URLs, fragments, `http`, `https`, `mailto`, `tel`, `ftp`; for images
/// also `data:image/…`. Control characters and whitespace are ignored when detecting the
/// scheme (so `java\tscript:` is caught).
pub fn safe_url(url: &str, image: bool) -> Option<String> {
    let compact: String = url
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && !c.is_control() && *c != '\u{0}')
        .collect::<String>()
        .to_ascii_lowercase();
    let scheme_end = compact.find(':');
    let delim = compact.find(['/', '?', '#']);
    let scheme = match (scheme_end, delim) {
        (Some(c), Some(d)) if d < c => None,
        (Some(c), _) => Some(&compact[..c]),
        _ => None,
    };
    match scheme {
        None => Some(url.to_string()),
        Some("http" | "https" | "mailto" | "tel" | "ftp") => Some(url.to_string()),
        Some("data") if image && compact.starts_with("data:image/") => Some(url.to_string()),
        _ => None,
    }
}

/// Keep only harmless declarations in a `style` attribute (highlight colours, table
/// alignment).
fn filter_style(value: &str) -> Option<String> {
    let mut out = Vec::new();
    for decl in value.split(';') {
        let Some((k, v)) = decl.split_once(':') else { continue };
        let k = k.trim().to_ascii_lowercase();
        let v = v.trim().to_ascii_lowercase();
        let ok = match k.as_str() {
            "color" | "background-color" => {
                v.len() <= 9 && v.starts_with('#') && v.len() >= 4 && v[1..].bytes().all(|b| b.is_ascii_hexdigit())
            }
            "font-weight" => matches!(v.as_str(), "bold" | "normal" | "600" | "700"),
            "font-style" => matches!(v.as_str(), "italic" | "normal"),
            "text-decoration" => matches!(v.as_str(), "underline" | "none" | "line-through"),
            "text-align" => matches!(v.as_str(), "left" | "center" | "right"),
            _ => false,
        };
        if ok {
            out.push(format!("{k}:{v}"));
        }
    }
    if out.is_empty() { None } else { Some(out.join(";")) }
}

/// Sanitize an HTML fragment with ammonia using the export's allow-list: the usual
/// formatting tags, tables, images (http/https/relative/`data:image`), `class` everywhere,
/// `id` (footnotes, headings), `style` on `span`/`th`/`td` restricted to colours / alignment,
/// and `input type=checkbox disabled checked`.
pub fn sanitize_html(html: &str) -> String {
    let mut b = ammonia::Builder::default();
    b.add_tags(["input"])
        .add_generic_attributes(["class", "id"])
        .add_tag_attributes("span", ["style"])
        .add_tag_attributes("th", ["style"])
        .add_tag_attributes("td", ["style"])
        .add_tag_attributes("input", ["type", "checked", "disabled"])
        .add_tag_attributes("ol", ["start"])
        .add_tag_attributes("img", ["title"])
        .add_tag_attributes("a", ["title"])
        .set_tag_attribute_value("input", "disabled", "")
        .url_schemes(["http", "https", "mailto", "tel", "ftp", "data"].into_iter().collect())
        .link_rel(Some("noopener noreferrer"))
        .attribute_filter(|el, attr, value| match (el, attr) {
            (_, "style") => filter_style(value).map(Cow::Owned),
            ("input", "type") => Some(Cow::Borrowed("checkbox")),
            ("img", "src") => safe_url(value, true).map(Cow::Owned),
            (_, "href" | "src" | "cite" | "action" | "formaction" | "poster" | "background" | "longdesc") => {
                safe_url(value, false).map(Cow::Owned)
            }
            _ => Some(Cow::Borrowed(value)),
        });
    b.clean(&limit_html_depth(html)).to_string()
}

/// Deepest element nesting passed to the sanitizer; see [`limit_html_depth`]. Well above
/// what the renderer emits for `mdedit_syntax::MAX_NEST` containers and for real raw HTML.
const MAX_HTML_DEPTH: usize = 256;

/// Delete the tags (never the text) nested deeper than [`MAX_HTML_DEPTH`], and their end tags.
///
/// html5ever walks the stack of open elements for nearly every tag, so sanitizing takes time
/// quadratic in the nesting depth: 1 MB of `<div>` takes minutes. Bounding the stack makes it
/// linear. Input that stays under the limit is returned untouched.
///
/// This isn't a security boundary, since ammonia still parses whatever comes out. It only has
/// to bound the depth html5ever will reach, so whenever the HTML parsing rules are subtle the
/// model errs towards counting too deep: an end tag only pops what it surely pops in the real
/// parser (a misnested `</span>` closes nothing), and text in attributes and comments may
/// open elements but not close ones opened outside it, because a raw-text element can make
/// html5ever read it as markup. The cost of erring is flattening a document with hundreds of
/// unclosed elements. Every scan is bounded by the limit or moves forward, so this is linear.
fn limit_html_depth(html: &str) -> Cow<'_, str> {
    let b = html.as_bytes();
    let mut stack: Vec<String> = Vec::new();
    let mut dropped: HashMap<String, usize> = HashMap::new();
    let (mut out, mut copied) = (String::new(), 0);
    // inside an attribute section or comment: no popping below `floor` until `region_end`
    let (mut floor, mut region_end) = (0, 0);
    let mut i = 0;
    while let Some(off) = html[i..].find('<') {
        let lt = i + off;
        let in_region = lt < region_end;
        if !in_region {
            floor = 0;
        }
        let close = b.get(lt + 1) == Some(&b'/');
        let name_start = lt + 1 + usize::from(close);
        if !b.get(name_start).is_some_and(u8::is_ascii_alphabetic) {
            if !in_region && (close || matches!(b.get(lt + 1), Some(b'!' | b'?'))) {
                floor = stack.len();
                region_end = markup_comment_end(html, lt);
            }
            i = lt + 1;
            continue;
        }
        let name_len = b[name_start..].iter().position(|c| matches!(c, b' ' | b'\t' | b'\r' | b'\n' | 0x0c | b'/' | b'>'));
        let name_end = name_len.map_or(b.len(), |n| name_start + n);
        let name = html[name_start..name_end].to_ascii_lowercase();
        // not in a region: its bytes are scanned once as a tag, once as region text
        let tag_end = if in_region { 0 } else { tag_end(b, name_end) };
        let keep = if close {
            match dropped.get_mut(&name) {
                Some(n) if *n > 0 => {
                    *n -= 1;
                    false
                }
                _ => {
                    pop_closed(&mut stack, floor, &name);
                    true
                }
            }
        } else {
            pop_implied(&mut stack, floor, &name);
            // void elements only nest in foreign content, where they aren't void
            if is_void(&name) && !stack.iter().any(|s| s == "svg" || s == "math") {
                true
            } else if stack.len() < MAX_HTML_DEPTH {
                stack.push(name);
                true
            } else {
                *dropped.entry(name).or_insert(0) += 1;
                false
            }
        };
        if keep {
            if !in_region {
                floor = stack.len();
                region_end = tag_end;
            }
            i = name_end;
        } else {
            // in a region the tag may not be a real one: stop at the first `>` regardless
            let end = if in_region { html[lt..].find('>').map_or(html.len(), |n| lt + n + 1) } else { tag_end };
            out.push_str(&html[copied..lt]);
            copied = end;
            i = end;
        }
    }
    if copied == 0 {
        return Cow::Borrowed(html);
    }
    out.push_str(&html[copied..]);
    Cow::Owned(out)
}

/// End of the comment or bogus comment (`<!…>`, `<?…>`, `</ …>`) at `lt`.
fn markup_comment_end(html: &str, lt: usize) -> usize {
    let rest = &html[lt..];
    let Some(c) = rest.strip_prefix("<!--") else {
        return rest.find('>').map_or(html.len(), |n| lt + n + 1);
    };
    if c.starts_with('>') {
        return lt + 5;
    }
    if c.starts_with("->") {
        return lt + 6;
    }
    let mut from = 0;
    while let Some(n) = c[from..].find("--") {
        let p = from + n + 2;
        if c[p..].starts_with('>') {
            return lt + 4 + p + 1;
        }
        if c[p..].starts_with("!>") {
            return lt + 4 + p + 2;
        }
        from = p - 1;
    }
    html.len()
}

/// End (past the `>`) of the tag whose name ends at `i`, following the HTML tokenizer's rules
/// for attributes; `b.len()` if it never ends.
fn tag_end(b: &[u8], mut i: usize) -> usize {
    let is_ws = |c: u8| matches!(c, b' ' | b'\t' | b'\r' | b'\n' | 0x0c);
    let mut in_name = false; // in an attribute name, where `=` starts the value
    while let Some(&c) = b.get(i) {
        match c {
            b'>' => return i + 1,
            b'/' => in_name = false,
            b'=' if in_name => {
                i += 1;
                while b.get(i).copied().is_some_and(is_ws) {
                    i += 1;
                }
                match b.get(i) {
                    Some(&q @ (b'"' | b'\'')) => match b[i + 1..].iter().position(|&x| x == q) {
                        Some(n) => i += n + 2,
                        None => return b.len(),
                    },
                    Some(b'>') | None => continue,
                    Some(_) => {
                        while b.get(i).is_some_and(|&x| !is_ws(x) && x != b'>') {
                            i += 1;
                        }
                    }
                }
                in_name = false;
                continue;
            }
            c if is_ws(c) => {}
            _ => in_name = true,
        }
        i += 1;
    }
    b.len()
}

fn is_void(name: &str) -> bool {
    matches!(
        name,
        "area" | "base" | "basefont" | "bgsound" | "br" | "col" | "embed" | "frame" | "hr" | "img" | "input"
            | "keygen" | "link" | "meta" | "param" | "source" | "track" | "wbr"
    )
}

/// Elements whose end tag closes everything open inside them, and whose start tag closes a `p`.
fn is_block(name: &str) -> bool {
    matches!(
        name,
        "address" | "article" | "aside" | "blockquote" | "center" | "details" | "dialog" | "dir" | "div" | "dl"
            | "dd" | "dt" | "fieldset" | "figcaption" | "figure" | "footer" | "h1" | "h2" | "h3" | "h4" | "h5"
            | "h6" | "header" | "hgroup" | "li" | "listing" | "main" | "menu" | "nav" | "ol" | "p" | "pre"
            | "search" | "section" | "summary" | "ul"
    )
}

/// Elements a block's end tag can't close past: scope boundaries (their contents don't see
/// outer end tags), and formatting elements (the parser reopens them after the block).
fn stops_block_end(name: &str) -> bool {
    matches!(
        name,
        "applet" | "button" | "caption" | "marquee" | "object" | "select" | "table" | "td" | "template" | "th"
            | "annotation-xml" | "desc" | "foreignobject" | "mi" | "mn" | "mo" | "ms" | "mtext" | "title"
            | "a" | "b" | "big" | "code" | "em" | "font" | "i" | "nobr" | "s" | "small" | "strike" | "strong"
            | "tt" | "u"
    )
}

/// Apply the end tag `name` to the open elements above `floor`.
fn pop_closed(stack: &mut Vec<String>, floor: usize, name: &str) {
    let table_part = matches!(name, "table" | "tbody" | "thead" | "tfoot" | "tr" | "td" | "th" | "caption");
    let mut j = stack.len();
    while j > floor {
        j -= 1;
        let s = stack[j].as_str();
        if s == name {
            stack.truncate(j);
            return;
        }
        let passes = if table_part {
            s != "table" && s != "template"
        } else {
            is_block(name) && !stops_block_end(s) && !(name == "li" && matches!(s, "ul" | "ol"))
        };
        if !passes {
            return;
        }
    }
}

/// Close what the start tag `name` implicitly ends, when it is directly on top of the stack.
fn pop_implied(stack: &mut Vec<String>, floor: usize, name: &str) {
    let table_rank = |n: &str| match n {
        "td" | "th" => 1,
        "tr" => 2,
        "tbody" | "thead" | "tfoot" | "caption" | "colgroup" => 3,
        _ => 0,
    };
    let rank = table_rank(name);
    while stack.len() > floor {
        let top = stack[stack.len() - 1].as_str();
        let ends = match name {
            _ if top == "p" => is_block(name) || name == "table",
            "li" => top == "li",
            "dd" | "dt" => matches!(top, "dd" | "dt"),
            "option" => top == "option",
            "optgroup" => matches!(top, "option" | "optgroup"),
            "rp" | "rt" => matches!(top, "rp" | "rt"),
            _ => (1..=rank).contains(&table_rank(top)),
        };
        if !ends {
            return;
        }
        stack.pop();
    }
}

// ---------------------------------------------------------------------------------------
// renderer
// ---------------------------------------------------------------------------------------

struct Renderer<'h> {
    hl: &'h Highlighter,
    theme: HlTheme,
    /// Output buffers; headings and images render into a nested buffer.
    out: Vec<String>,
    // table state
    aligns: Vec<Alignment>,
    in_head: bool,
    cell: usize,
    body_open: bool,
    // code block
    code: Option<(Option<String>, String)>,
    // image alt text collection
    image: Vec<(String, String, String)>,
    // heading: (level, explicit id, classes, plain text)
    heading: Option<(u8, Option<String>, Vec<String>, String)>,
    slugs: HashMap<String, usize>,
    footnotes: HashMap<String, usize>,
    first_heading: Option<String>,
    skip_meta: bool,
}

fn level_num(l: HeadingLevel) -> u8 {
    match l {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn slugify(s: &str) -> String {
    let mut out = String::new();
    for c in s.trim().chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() || c == '_' || c == '-' {
            out.push(c);
        } else if c.is_whitespace() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

fn class_safe(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => c,
            '+' => 'p',
            '#' => 's',
            _ => '-',
        })
        .collect()
}

impl<'h> Renderer<'h> {
    fn w(&mut self) -> &mut String {
        if self.out.is_empty() {
            self.out.push(String::new());
        }
        let n = self.out.len();
        &mut self.out[n - 1]
    }

    fn push(&mut self, s: &str) {
        if !self.image.is_empty() {
            return; // inside an image only alt text is collected
        }
        self.w().push_str(s);
    }

    fn text(&mut self, s: &str) {
        if let Some(img) = self.image.last_mut() {
            img.2.push_str(s);
            return;
        }
        if let Some(h) = self.heading.as_mut() {
            h.3.push_str(s);
        }
        let e = html_escape(s);
        self.w().push_str(&e);
    }

    fn footnote_num(&mut self, label: &str) -> usize {
        let n = self.footnotes.len() + 1;
        *self.footnotes.entry(label.to_string()).or_insert(n)
    }

    fn unique_slug(&mut self, base: String) -> String {
        let base = if base.is_empty() { "section".to_string() } else { base };
        let n = self.slugs.entry(base.clone()).or_insert(0);
        *n += 1;
        if *n == 1 { base } else { format!("{base}-{}", *n - 1) }
    }

    fn event(&mut self, ev: Event<'_>) {
        if self.skip_meta {
            if matches!(ev, Event::End(TagEnd::MetadataBlock(_))) {
                self.skip_meta = false;
            }
            return;
        }
        if let Some((_, code)) = self.code.as_mut() {
            match ev {
                Event::Text(t) => code.push_str(&t),
                Event::End(TagEnd::CodeBlock) => {
                    let (lang, code) = self.code.take().unwrap_or_default();
                    let html = match &lang {
                        Some(l) => format!(
                            "<pre><code class=\"language-{}\">{}</code></pre>\n",
                            class_safe(l),
                            self.hl.highlight_html(l, &code, self.theme)
                        ),
                        None => format!("<pre><code>{}</code></pre>\n", html_escape(&code)),
                    };
                    self.push(&html);
                }
                _ => {}
            }
            return;
        }
        match ev {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(t) => self.text(&t),
            Event::Code(t) => {
                if let Some(img) = self.image.last_mut() {
                    img.2.push_str(&t);
                    return;
                }
                if let Some(h) = self.heading.as_mut() {
                    h.3.push_str(&t);
                }
                let s = format!("<code>{}</code>", html_escape(&t));
                self.push(&s);
            }
            Event::InlineMath(t) => {
                let s = format!("<span class=\"math math-inline\">{}</span>", html_escape(&t));
                self.push(&s);
            }
            Event::DisplayMath(t) => {
                let s = format!("<span class=\"math math-display\">{}</span>", html_escape(&t));
                self.push(&s);
            }
            Event::Html(t) | Event::InlineHtml(t) => self.push(&t),
            Event::FootnoteReference(label) => {
                let n = self.footnote_num(&label);
                let id = slugify(&label);
                let s = format!(
                    "<sup class=\"footnote-reference\"><a href=\"#fn-{id}\" id=\"fnref-{id}\">{n}</a></sup>",
                    id = html_escape(&id)
                );
                self.push(&s);
            }
            Event::SoftBreak => {
                if let Some(img) = self.image.last_mut() {
                    img.2.push(' ');
                    return;
                }
                if let Some(h) = self.heading.as_mut() {
                    h.3.push(' ');
                }
                self.push("\n");
            }
            Event::HardBreak => self.push("<br />\n"),
            Event::Rule => self.push("<hr />\n"),
            Event::TaskListMarker(checked) => {
                self.push(if checked {
                    "<input type=\"checkbox\" disabled=\"\" checked=\"\" /> "
                } else {
                    "<input type=\"checkbox\" disabled=\"\" /> "
                });
            }
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => self.push("<p>"),
            Tag::Heading { level, id, classes, .. } => {
                self.heading = Some((
                    level_num(level),
                    id.map(|s| s.to_string()),
                    classes.iter().map(|c| c.to_string()).collect(),
                    String::new(),
                ));
                self.out.push(String::new());
            }
            Tag::BlockQuote(kind) => {
                let (class, title) = match kind {
                    None => ("", ""),
                    Some(BlockQuoteKind::Note) => ("note", "Note"),
                    Some(BlockQuoteKind::Tip) => ("tip", "Tip"),
                    Some(BlockQuoteKind::Important) => ("important", "Important"),
                    Some(BlockQuoteKind::Warning) => ("warning", "Warning"),
                    Some(BlockQuoteKind::Caution) => ("caution", "Caution"),
                };
                if class.is_empty() {
                    self.push("<blockquote>\n");
                } else {
                    let s = format!(
                        "<blockquote class=\"markdown-alert markdown-alert-{class}\">\n<p class=\"markdown-alert-title\">{title}</p>\n"
                    );
                    self.push(&s);
                }
            }
            Tag::CodeBlock(kind) => {
                let lang = match kind {
                    CodeBlockKind::Fenced(info) => {
                        let w = info.split_whitespace().next().unwrap_or("");
                        let w = w.trim_start_matches('{').trim_end_matches('}').trim_start_matches('.');
                        let w = w.strip_prefix("language-").unwrap_or(w);
                        let w = w.split([',', '{']).next().unwrap_or(w);
                        if w.is_empty() { None } else { Some(mdedit_syntax::normalize_lang(w)) }
                    }
                    CodeBlockKind::Indented => None,
                };
                self.code = Some((lang, String::new()));
            }
            Tag::HtmlBlock => {}
            Tag::List(Some(1)) => self.push("<ol>\n"),
            Tag::List(Some(n)) => {
                let s = format!("<ol start=\"{n}\">\n");
                self.push(&s);
            }
            Tag::List(None) => self.push("<ul>\n"),
            Tag::Item => self.push("<li>"),
            Tag::FootnoteDefinition(label) => {
                let n = self.footnote_num(&label);
                let id = slugify(&label);
                let s = format!(
                    "<div class=\"footnote-definition\" id=\"fn-{}\"><sup class=\"footnote-definition-label\"><a href=\"#fnref-{}\">{n}</a></sup>\n",
                    html_escape(&id),
                    html_escape(&id)
                );
                self.push(&s);
            }
            Tag::DefinitionList => self.push("<dl>\n"),
            Tag::DefinitionListTitle => self.push("<dt>"),
            Tag::DefinitionListDefinition => self.push("<dd>"),
            Tag::Table(aligns) => {
                self.aligns = aligns;
                self.body_open = false;
                self.push("<table>\n");
            }
            Tag::TableHead => {
                self.in_head = true;
                self.cell = 0;
                self.push("<thead>\n<tr>");
            }
            Tag::TableRow => {
                self.cell = 0;
                if !self.body_open {
                    self.body_open = true;
                    self.push("<tbody>\n");
                }
                self.push("<tr>");
            }
            Tag::TableCell => {
                let t = if self.in_head { "th" } else { "td" };
                let style = match self.aligns.get(self.cell) {
                    Some(Alignment::Left) => " style=\"text-align: left\"",
                    Some(Alignment::Center) => " style=\"text-align: center\"",
                    Some(Alignment::Right) => " style=\"text-align: right\"",
                    _ => "",
                };
                let s = format!("<{t}{style}>");
                self.push(&s);
            }
            Tag::Emphasis => self.push("<em>"),
            Tag::Strong => self.push("<strong>"),
            Tag::Strikethrough => self.push("<del>"),
            Tag::Superscript => self.push("<sup>"),
            Tag::Subscript => self.push("<sub>"),
            Tag::Link { link_type, dest_url, title, .. } => {
                let url = if link_type == LinkType::Email && !dest_url.starts_with("mailto:") {
                    format!("mailto:{dest_url}")
                } else {
                    dest_url.to_string()
                };
                let mut s = String::from("<a");
                if let Some(u) = safe_url(&url, false) {
                    s.push_str(&format!(" href=\"{}\"", html_escape(&u)));
                }
                if !title.is_empty() {
                    s.push_str(&format!(" title=\"{}\"", html_escape(&title)));
                }
                s.push('>');
                self.push(&s);
            }
            Tag::Image { dest_url, title, .. } => {
                // push before switching to alt collection
                self.image.push((dest_url.to_string(), title.to_string(), String::new()));
            }
            Tag::MetadataBlock(_) => self.skip_meta = true,
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => self.push("</p>\n"),
            TagEnd::Heading(_) => {
                let inner = if self.out.len() > 1 { self.out.pop().unwrap_or_default() } else { String::new() };
                if let Some((level, id, classes, plain)) = self.heading.take() {
                    let plain = plain.trim().to_string();
                    if self.first_heading.is_none() && !plain.is_empty() {
                        self.first_heading = Some(plain.clone());
                    }
                    let id = match id {
                        Some(id) => {
                            self.slugs.entry(id.clone()).or_insert(1);
                            id
                        }
                        None => self.unique_slug(slugify(&plain)),
                    };
                    let mut s = format!("<h{level} id=\"{}\"", html_escape(&id));
                    if !classes.is_empty() {
                        s.push_str(&format!(" class=\"{}\"", html_escape(&classes.join(" "))));
                    }
                    s.push('>');
                    s.push_str(&inner);
                    s.push_str(&format!("</h{level}>\n"));
                    self.push(&s);
                } else {
                    self.push(&inner);
                }
            }
            TagEnd::BlockQuote(_) => self.push("</blockquote>\n"),
            TagEnd::CodeBlock | TagEnd::HtmlBlock => {}
            TagEnd::List(true) => self.push("</ol>\n"),
            TagEnd::List(false) => self.push("</ul>\n"),
            TagEnd::Item => self.push("</li>\n"),
            TagEnd::FootnoteDefinition => self.push("</div>\n"),
            TagEnd::DefinitionList => self.push("</dl>\n"),
            TagEnd::DefinitionListTitle => self.push("</dt>\n"),
            TagEnd::DefinitionListDefinition => self.push("</dd>\n"),
            TagEnd::Table => {
                if self.body_open {
                    self.push("</tbody>\n");
                }
                self.push("</table>\n");
            }
            TagEnd::TableHead => {
                self.in_head = false;
                self.push("</tr>\n</thead>\n");
            }
            TagEnd::TableRow => self.push("</tr>\n"),
            TagEnd::TableCell => {
                let t = if self.in_head { "</th>" } else { "</td>" };
                self.push(t);
                self.cell += 1;
            }
            TagEnd::Emphasis => self.push("</em>"),
            TagEnd::Strong => self.push("</strong>"),
            TagEnd::Strikethrough => self.push("</del>"),
            TagEnd::Superscript => self.push("</sup>"),
            TagEnd::Subscript => self.push("</sub>"),
            TagEnd::Link => self.push("</a>"),
            TagEnd::Image => {
                if let Some((url, title, alt)) = self.image.pop() {
                    if let Some(outer) = self.image.last_mut() {
                        // nested image inside image alt: keep its alt text only
                        outer.2.push_str(&alt);
                        return;
                    }
                    if let Some(h) = self.heading.as_mut() {
                        h.3.push_str(&alt);
                    }
                    let mut s = String::from("<img");
                    if let Some(u) = safe_url(&url, true) {
                        s.push_str(&format!(" src=\"{}\"", html_escape(&u)));
                    }
                    s.push_str(&format!(" alt=\"{}\"", html_escape(&alt)));
                    if !title.is_empty() {
                        s.push_str(&format!(" title=\"{}\"", html_escape(&title)));
                    }
                    s.push_str(" />");
                    self.push(&s);
                }
            }
            TagEnd::MetadataBlock(_) => self.skip_meta = false,
        }
    }
}

/// Render Markdown to (unsanitized) body HTML; also returns the first heading's text.
fn render(markdown: &str, theme: HlTheme, hl: &Highlighter) -> (String, Option<String>) {
    let mut r = Renderer {
        hl,
        theme,
        out: vec![String::with_capacity(markdown.len() * 3 / 2)],
        aligns: Vec::new(),
        in_head: false,
        cell: 0,
        body_open: false,
        code: None,
        image: Vec::new(),
        heading: None,
        slugs: HashMap::new(),
        footnotes: HashMap::new(),
        first_heading: None,
        skip_meta: false,
    };
    let opts = mdedit_syntax::parser_options();
    let mut iter = Parser::new_ext(markdown, opts).into_offset_iter();
    while let Some((ev, range)) = iter.next() {
        if let Event::Start(Tag::MetadataBlock(_)) = &ev
            && !mdedit_syntax::at_doc_start(markdown, range.start) {
                // only front matter at the very start is metadata; render the rest as Markdown
                for (e2, _) in iter.by_ref() {
                    if matches!(e2, Event::End(TagEnd::MetadataBlock(_))) {
                        break;
                    }
                }
                let slice: Range<usize> = range;
                if let Some(src) = markdown.get(slice) {
                    let o2 = opts - Options::ENABLE_YAML_STYLE_METADATA_BLOCKS;
                    for e2 in Parser::new_ext(src, o2) {
                        r.event(e2);
                    }
                }
                continue;
            }
        r.event(ev);
    }
    // unbalanced buffers can't happen with pulldown, but be safe
    while r.out.len() > 1 {
        let s = r.out.pop().unwrap_or_default();
        r.w().push_str(&s);
    }
    let first = r.first_heading.take();
    (r.out.pop().unwrap_or_default(), first)
}

#[cfg(test)]
mod tests;

//! Markdown structure for mdedit: a block index with byte ranges into the source, inline
//! spans, an outline, and pure text-editing commands (bold toggle, Enter in lists, ...).
//!
//! The source text is the truth; nothing here rewrites text the user didn't touch.
//! Code blocks follow docs/code-blocks.md (CB-1..CB-11): info string kept verbatim, content
//! literal (tabs kept), one block node per code block.
//!
//! OWNER: agent "syntax". STABLE items are used by other crates.
//!
//! # Range conventions
//!
//! * All ranges are byte ranges into the parsed text and lie on char boundaries.
//! * `Block::range` never includes the trailing line ending, nor trailing blank lines.
//! * Top-level blocks are in source order and never overlap; children lie inside parents.
//! * Tight list items get a synthetic `Paragraph` child that holds their inline text, so a
//!   `ListItem`'s own `inlines` are always empty.
//! * Table cell inlines are stored on the `Table` block (`Block::inlines`), sorted by start.
//! * `inlines` are a flat list in pre-order: sorted by start, an outer span before the
//!   spans nested inside it.
//! * Fenced code: `open_fence` / `close_fence` start at the first fence character (container
//!   prefixes like `> ` and indentation are *not* included) and end at the end of the line
//!   (no `\r`). `content` runs from the start of the line after the opening fence to the end
//!   of the last content line (no final newline). Content lines still contain container
//!   prefixes (`> `, list indentation) and up to `fence.indent` spaces; use
//!   [`code_lines`] for the exact per-line code text.

use std::ops::Range;

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, LinkType, Options, Parser, Tag, TagEnd};

pub mod commands;
mod incremental;
pub use incremental::{reparse, reparse_in_place, shift};

/// STABLE.
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub kind: BlockKind,
    /// Byte range in the source, including markers (`#`, fences, `> `). Does not include the
    /// trailing newline.
    pub range: Range<usize>,
    /// The "content" part: code text without fences, heading text without `#`, etc.
    /// Equals `range` when there is no distinction.
    pub content: Range<usize>,
    /// Nested blocks (list items, quote contents). Top-level list: `List` → `ListItem` → blocks.
    pub children: Vec<Block>,
    /// Inline spans of this block's own text (paragraphs, headings, table cells). Empty for code.
    pub inlines: Vec<Span>,
}

/// STABLE.
#[derive(Clone, Debug, PartialEq)]
pub enum BlockKind {
    Paragraph,
    Heading { level: u8 },
    BlockQuote,
    ThematicBreak,
    /// `start` is the first number for ordered lists, 0 for bullet lists.
    List { ordered: bool, start: u64 },
    /// `task`: `Some(checked)` for `- [ ]` / `- [x]`; `checkbox`: byte range of `[ ]`.
    /// The item's `content` starts after the marker (and checkbox).
    ListItem { task: Option<bool>, checkbox: Option<Range<usize>> },
    CodeBlock(CodeBlockInfo),
    /// GFM table; `aligns` per column. Rows/cells in `table_rows`.
    Table { aligns: Vec<Align>, rows: Vec<TableRow> },
    Html,
    FrontMatter,
    MathBlock,
    FootnoteDefinition { label: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    None,
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TableRow {
    pub header: bool,
    pub range: Range<usize>,
    /// Byte range of each cell's content (trimmed, without pipes). An empty cell gets an
    /// empty range inside the cell (after its first space if any).
    pub cells: Vec<Range<usize>>,
}

/// STABLE.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeBlockInfo {
    /// `None` for indented code.
    pub fence: Option<Fence>,
    /// Full info string verbatim (`rust title=main.rs`); empty if none.
    pub info: String,
    /// Normalized language (first word of info through [`normalize_lang`]).
    pub lang: Option<String>,
    /// `false` if the fence runs to the end of its container.
    pub closed: bool,
    /// Byte range of the opening fence line and of the closing fence line (if any).
    pub open_fence: Option<Range<usize>>,
    pub close_fence: Option<Range<usize>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fence {
    pub ch: char,
    pub len: usize,
    /// Indentation of the opening fence relative to its container's content column (0..=3).
    pub indent: usize,
}

/// An inline span. STABLE.
#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    pub kind: SpanKind,
    /// Whole span including markers, e.g. `**bold**`.
    pub range: Range<usize>,
    /// Inner content, e.g. `bold`.
    pub content: Range<usize>,
}

/// STABLE.
#[derive(Clone, Debug, PartialEq)]
pub enum SpanKind {
    Strong,
    Emphasis,
    Strikethrough,
    Code,
    /// `content` is the link text; `url` the destination.
    Link { url: String },
    Image { url: String, alt: String },
    Autolink { url: String },
    InlineMath,
    DisplayMath,
    FootnoteReference { label: String },
    HardBreak,
    Html,
}

/// The pulldown-cmark options used by [`parse`] (and by the HTML export, so both agree).
pub fn parser_options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
        | Options::ENABLE_MATH
        | Options::ENABLE_GFM
        | Options::ENABLE_HEADING_ATTRIBUTES
}

// ---------------------------------------------------------------------------------------
// small text helpers (crate-internal, also used by commands.rs)
// ---------------------------------------------------------------------------------------

/// Largest char boundary `<= i` (and `<= text.len()`).
pub(crate) fn floor_boundary(text: &str, i: usize) -> usize {
    let mut i = i.min(text.len());
    while i > 0 && !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Start of the line containing byte `i`.
pub(crate) fn line_start(text: &str, i: usize) -> usize {
    let i = floor_boundary(text, i);
    text.as_bytes()[..i].iter().rposition(|&b| b == b'\n').map_or(0, |p| p + 1)
}

/// Position of the `\n` ending the line containing `i` (or `text.len()`).
pub(crate) fn line_end(text: &str, i: usize) -> usize {
    let i = floor_boundary(text, i);
    text.as_bytes()[i..].iter().position(|&b| b == b'\n').map_or(text.len(), |p| i + p)
}

/// Like [`line_end`] but before a `\r` that precedes the `\n`.
pub(crate) fn line_end_no_cr(text: &str, i: usize) -> usize {
    let e = line_end(text, i);
    let s = line_start(text, i);
    let i = floor_boundary(text, i);
    if e > s && e > i && text.as_bytes()[e - 1] == b'\r' { e - 1 } else { e }
}

fn is_blank(s: &str) -> bool {
    s.bytes().all(|b| b == b' ' || b == b'\t' || b == b'\r' || b == b'\n')
}

/// Remove trailing line endings and trailing blank lines from `r`.
fn trim_block_end(text: &str, r: Range<usize>) -> Range<usize> {
    let b = text.as_bytes();
    let mut end = r.end.min(text.len());
    loop {
        let mut e = end;
        while e > r.start && (b[e - 1] == b'\n' || b[e - 1] == b'\r') {
            e -= 1;
        }
        // drop a whitespace-only last line (only if there is a line before it)
        let ls = b[r.start..e].iter().rposition(|&c| c == b'\n').map(|p| r.start + p + 1);
        match ls {
            Some(ls) if is_blank(&text[ls..e]) => {
                end = ls;
            }
            _ => {
                end = e;
                break;
            }
        }
    }
    r.start..end.max(r.start)
}

/// Remove exactly one trailing line ending from `r`.
fn trim_one_newline(text: &str, r: Range<usize>) -> Range<usize> {
    let b = text.as_bytes();
    let mut end = r.end.min(text.len());
    if end > r.start && b[end - 1] == b'\n' {
        end -= 1;
        if end > r.start && b[end - 1] == b'\r' {
            end -= 1;
        }
    }
    r.start..end
}

fn trim_spaces(text: &str, r: Range<usize>) -> Range<usize> {
    let b = text.as_bytes();
    let (mut s, mut e) = (r.start, r.end.min(text.len()));
    while s < e && matches!(b[s], b' ' | b'\t' | b'\r' | b'\n') {
        s += 1;
    }
    while e > s && matches!(b[e - 1], b' ' | b'\t' | b'\r' | b'\n') {
        e -= 1;
    }
    s..e
}

/// Parsed list marker at the start of `s` (which must start at the marker, not indentation).
/// Returns (marker length, length incl. following spaces).
/// Byte offset of a list item's marker (pulldown's item range may start at indentation).
pub(crate) fn item_marker_pos(text: &str, item: &Block) -> usize {
    let s = item.range.start.min(text.len());
    s + text[s..item.range.end.min(text.len()).max(s)].bytes().take_while(|&b| b == b' ' || b == b'\t').count()
}

pub(crate) fn list_marker_len(s: &str) -> Option<(usize, usize)> {
    let b = s.as_bytes();
    let mlen = match b.first()? {
        b'-' | b'*' | b'+' => 1,
        c if c.is_ascii_digit() => {
            let d = b.iter().take_while(|c| c.is_ascii_digit()).count();
            if d > 9 {
                return None;
            }
            match b.get(d) {
                Some(b'.') | Some(b')') => d + 1,
                _ => return None,
            }
        }
        _ => return None,
    };
    let sp = b[mlen..].iter().take_while(|&&c| c == b' ' || c == b'\t').count();
    match b.get(mlen) {
        None | Some(b'\n') | Some(b'\r') | Some(b' ') | Some(b'\t') => {}
        _ => return None,
    }
    // ≥ 5 spaces: indented code inside the item, only one space belongs to the marker.
    let sp = if sp > 4 { 1 } else { sp };
    Some((mlen, mlen + sp))
}

// ---------------------------------------------------------------------------------------
// parser
// ---------------------------------------------------------------------------------------

struct OpenSpan {
    /// Index into the host's `inlines`, `None` for tags we don't report (sup/sub).
    idx: Option<usize>,
    first: Option<usize>,
    last: usize,
    alt: String,
    is_image: bool,
}

struct Open {
    block: Block,
    synthetic: bool,
    is_host: bool,
    spans: Vec<OpenSpan>,
    inl: Option<Range<usize>>,
    /// Content column for containers (list item, quote), byte-based.
    content_col: usize,
    /// Code / front matter: end of the last text event; decoded fenced info.
    text_range: Option<Range<usize>>,
    code_info: Option<String>,
    cur_row: Option<TableRow>,
    cur_cell: Option<usize>,
}

impl Open {
    fn new(kind: BlockKind, range: Range<usize>) -> Self {
        let is_host = matches!(
            kind,
            BlockKind::Paragraph | BlockKind::Heading { .. } | BlockKind::Table { .. }
        );
        Open {
            block: Block { kind, content: range.clone(), range, children: Vec::new(), inlines: Vec::new() },
            synthetic: false,
            is_host,
            spans: Vec::new(),
            inl: None,
            content_col: 0,
            text_range: None,
            code_info: None,
            cur_row: None,
            cur_cell: None,
        }
    }
}

/// Maximum container nesting (quotes, lists, items, footnotes) kept in the block tree.
/// Deeper containers are flattened into the deepest kept one, so every recursive consumer
/// (Clone/Drop/PartialEq, outline, style) is bounded. The tree depth is at most
/// `MAX_NEST + 2` (containers + a leaf + a synthetic paragraph).
pub const MAX_NEST: usize = 32;

struct Builder<'t> {
    /// Number of currently open containers that were not opened because of [`MAX_NEST`].
    suppressed: usize,
    text: &'t str,
    /// Number of open image spans (alt text is only collected inside images).
    images: usize,
    stack: Vec<Open>,
    top: Vec<Block>,
}

fn col_of(text: &str, i: usize) -> usize {
    i - line_start(text, i)
}

fn union(a: Option<Range<usize>>, r: &Range<usize>) -> Option<Range<usize>> {
    Some(match a {
        None => r.clone(),
        Some(a) => a.start.min(r.start)..a.end.max(r.end),
    })
}

impl<'t> Builder<'t> {
    fn attach(&mut self, b: Block) {
        match self.stack.last_mut() {
            Some(p) => p.block.children.push(b),
            None => self.top.push(b),
        }
    }

    fn close_synthetic(&mut self) {
        if self.stack.last().is_some_and(|o| o.synthetic)
            && let Some(o) = self.stack.pop() {
                let b = self.finish(o);
                self.attach(b);
            }
    }

    fn push(&mut self, mut o: Open) {
        // container content column
        let t = self.text;
        match o.block.kind {
            BlockKind::ListItem { .. } => {
                let s = o.block.range.start;
                let s = s + t[s..].bytes().take_while(|&b| b == b' ' || b == b'\t').count();
                let w = list_marker_len(&t[s..]).map_or(2, |m| m.1);
                o.content_col = col_of(t, s) + w;
            }
            BlockKind::BlockQuote => {
                let s = o.block.range.start;
                let b = t.as_bytes();
                let mut w = 1;
                if b.get(s + 1) == Some(&b' ') {
                    w = 2;
                }
                o.content_col = col_of(t, s) + w;
            }
            _ => {
                o.content_col = self.stack.last().map_or(0, |p| p.content_col);
            }
        }
        self.stack.push(o);
    }

    /// Make sure the top of the stack can hold inline content and return it.
    fn host(&mut self, r: &Range<usize>) -> Option<&mut Open> {
        let need_synth = match self.stack.last() {
            Some(o) => !o.is_host && !o.synthetic,
            None => true,
        };
        if need_synth {
            let mut o = Open::new(BlockKind::Paragraph, r.clone());
            o.synthetic = true;
            o.is_host = true;
            self.push(o);
        }
        let o = self.stack.last_mut()?;
        o.inl = union(o.inl.take(), r);
        if let Some(sp) = o.spans.last_mut() {
            if sp.first.is_none() {
                sp.first = Some(r.start);
            }
            sp.last = sp.last.max(r.end);
        }
        Some(o)
    }

    fn inline_leaf(&mut self, kind: Option<SpanKind>, r: Range<usize>) {
        let t = self.text;
        let images = self.images;
        let Some(o) = self.host(&r) else { return };
        let Some(kind) = kind else { return };
        let content = match &kind {
            SpanKind::Code => {
                let s = &t[r.clone()];
                let n = s.bytes().take_while(|&c| c == b'`').count();
                // The closing run is counted from the end: pulldown's range can end in
                // something other than backticks (e.g. a `\r`), and `r.end - n` could then
                // land inside a multi-byte char.
                let m = s.bytes().rev().take_while(|&c| c == b'`').count().min(s.len() - n);
                let mut c = clamp_range(t, (r.start + n)..(r.end - m), &r);
                let inner = &t[c.clone()];
                if inner.len() >= 2
                    && inner.starts_with([' ', '\n'])
                    && inner.ends_with([' ', '\n'])
                    && !is_blank(inner)
                {
                    c = (c.start + 1)..(c.end - 1);
                }
                c
            }
            SpanKind::InlineMath => shrink(&r, 1),
            SpanKind::DisplayMath => shrink(&r, 2),
            SpanKind::FootnoteReference { .. } => {
                let s = (r.start + 2).min(r.end);
                s..r.end.saturating_sub(1).max(s)
            }
            _ => r.clone(),
        };
        let content = clamp_range(t, content, &r);
        if let (Some(sp), true) = (o.spans.last_mut(), images > 0)
            && matches!(kind, SpanKind::Code) {
                sp.alt.push_str(&t[content.clone()]);
            }
        o.block.inlines.push(Span { kind, range: r, content });
    }

    fn start_span(&mut self, kind: Option<SpanKind>, r: Range<usize>) {
        let is_image = matches!(kind, Some(SpanKind::Image { .. }));
        if is_image {
            self.images += 1;
        }
        let Some(o) = self.host(&r) else { return };
        let idx = kind.map(|kind| {
            o.block.inlines.push(Span { kind, content: r.clone(), range: r.clone() });
            o.block.inlines.len() - 1
        });
        o.spans.push(OpenSpan { idx, first: None, last: r.start, alt: String::new(), is_image });
    }

    fn end_span(&mut self) {
        let t = self.text;
        let Some(o) = self.stack.last_mut() else { return };
        let Some(sp) = o.spans.pop() else { return };
        if sp.is_image {
            self.images = self.images.saturating_sub(1);
        }
        // propagate alt text of nested spans inside an image
        if let Some(parent) = o.spans.last_mut() {
            parent.alt.push_str(&sp.alt);
        }
        let Some(idx) = sp.idx else { return };
        let Some(span) = o.block.inlines.get_mut(idx) else { return };
        let r = span.range.clone();
        let content = match &mut span.kind {
            SpanKind::Strong => shrink(&r, 2),
            SpanKind::Emphasis => shrink(&r, 1),
            SpanKind::Strikethrough => {
                let s = &t[r.clone()];
                if s.len() >= 4 && s.starts_with("~~") && s.ends_with("~~") { shrink(&r, 2) } else { shrink(&r, 1) }
            }
            SpanKind::Autolink { .. } => shrink(&r, 1),
            SpanKind::Image { alt, .. } => {
                *alt = sp.alt;
                match sp.first {
                    Some(f) => f..sp.last,
                    None => (r.start + 2).min(r.end)..(r.start + 2).min(r.end),
                }
            }
            _ => match sp.first {
                Some(f) => f..sp.last,
                None => (r.start + 1).min(r.end)..(r.start + 1).min(r.end),
            },
        };
        span.content = clamp_range(t, content, &r);
    }

    fn finish(&self, o: Open) -> Block {
        let t = self.text;
        let mut b = o.block;
        match &mut b.kind {
            BlockKind::CodeBlock(info) => {
                let r = trim_one_newline(t, b.range.clone());
                b.range = r.clone();
                if info.fence.is_some() {
                    finish_fenced(t, &mut b.content, info, &r, o.text_range, o.content_col);
                } else {
                    b.content = r;
                }
                if let Some(decoded) = o.code_info {
                    info.lang = first_word_lang(&decoded);
                }
            }
            BlockKind::FrontMatter => {
                let r = trim_block_end(t, b.range.clone());
                b.range = r.clone();
                b.content = match o.text_range {
                    Some(tr) => clamp_range(t, trim_one_newline(t, tr), &r),
                    None => {
                        let s = (line_end(t, r.start) + 1).min(r.end);
                        s..s
                    }
                };
            }
            BlockKind::Heading { .. } => {
                let r = trim_block_end(t, b.range.clone());
                b.range = r.clone();
                b.content = match o.inl {
                    Some(c) => clamp_range(t, c, &r),
                    None => {
                        // empty heading: after the `#`s and spaces
                        let s = &t[r.clone()];
                        let n = s.bytes().take_while(|&c| c == b'#').count();
                        let sp = s[n..].bytes().take_while(|&c| c == b' ' || c == b'\t').count();
                        let p = floor_boundary(t, r.start + n + sp).min(r.end);
                        p..p
                    }
                };
            }
            BlockKind::ListItem { checkbox, .. } => {
                let r = trim_block_end(t, b.range.clone());
                b.range = r.clone();
                let ms = r.start + t[r.clone()].bytes().take_while(|&b| b == b' ' || b == b'\t').count();
                let mut cs = list_marker_len(&t[ms..r.end]).map_or(r.start, |m| ms + m.1);
                if let Some(cb) = checkbox {
                    cs = cs.max(cb.end);
                    if t.as_bytes().get(cs) == Some(&b' ') {
                        cs += 1;
                    }
                }
                let cs = floor_boundary(t, cs.min(r.end));
                b.content = cs..r.end;
            }
            BlockKind::Table { rows, .. } => {
                if let Some(row) = o.cur_row {
                    rows.push(row);
                }
                let r = trim_block_end(t, b.range.clone());
                b.range = r.clone();
                b.content = r;
            }
            _ => {
                let r = if o.synthetic {
                    o.inl.clone().unwrap_or(b.range.clone())
                } else {
                    b.range.clone()
                };
                let r = trim_block_end(t, r);
                b.range = r.clone();
                b.content = r;
            }
        }
        if matches!(b.kind, BlockKind::Paragraph)
            && let [only] = b.inlines.as_slice()
                && only.kind == SpanKind::DisplayMath && trim_spaces(t, b.range.clone()) == only.range {
                    let mut c = only.content.clone();
                    // drop the newline right after `$$` and right before the closing `$$`
                    let bt = t.as_bytes();
                    if bt.get(c.start) == Some(&b'\r') && bt.get(c.start + 1) == Some(&b'\n') {
                        c.start += 2;
                    } else if bt.get(c.start) == Some(&b'\n') {
                        c.start += 1;
                    }
                    c.start = c.start.min(c.end);
                    c = trim_one_newline(t, c);
                    b.kind = BlockKind::MathBlock;
                    b.content = c;
                    b.inlines.clear();
                }
        // a container never ends before its last child (blank-line trimming vs. code content)
        if let Some(last) = b.children.last()
            && last.range.end > b.range.end {
                if b.content.end == b.range.end {
                    b.content.end = last.range.end;
                }
                b.range.end = last.range.end;
            }
        b.content = clamp_range(t, b.content.clone(), &b.range);
        b
    }

    fn event(&mut self, ev: Event<'_>, r: Range<usize>) {
        let t = self.text;
        let r = floor_boundary(t, r.start)..floor_boundary(t, r.end);
        match ev {
            Event::Start(tag) => self.start(tag, r),
            Event::End(end) => self.end(end, r),
            Event::Text(s) => {
                if let Some(o) = self.stack.last_mut() {
                    match o.block.kind {
                        BlockKind::CodeBlock(_) | BlockKind::FrontMatter | BlockKind::Html => {
                            o.text_range = union(o.text_range.take(), &r);
                            return;
                        }
                        _ => {}
                    }
                }
                let images = self.images;
                if let Some(o) = self.host(&r)
                    && let (Some(sp), true) = (o.spans.last_mut(), images > 0) {
                        sp.alt.push_str(&s);
                    }
            }
            Event::Code(_) => self.inline_leaf(Some(SpanKind::Code), r),
            Event::InlineMath(_) => self.inline_leaf(Some(SpanKind::InlineMath), r),
            Event::DisplayMath(_) => self.inline_leaf(Some(SpanKind::DisplayMath), r),
            Event::InlineHtml(_) => self.inline_leaf(Some(SpanKind::Html), r),
            Event::Html(_) => {
                if let Some(o) = self.stack.last_mut()
                    && matches!(o.block.kind, BlockKind::Html) {
                        o.text_range = union(o.text_range.take(), &r);
                        return;
                    }
                self.inline_leaf(Some(SpanKind::Html), r);
            }
            Event::FootnoteReference(label) => {
                self.inline_leaf(Some(SpanKind::FootnoteReference { label: label.to_string() }), r)
            }
            Event::HardBreak => self.inline_leaf(Some(SpanKind::HardBreak), r),
            Event::SoftBreak => self.inline_leaf(None, r),
            Event::Rule => {
                self.close_synthetic();
                let r = trim_block_end(t, r);
                self.attach(Block {
                    kind: BlockKind::ThematicBreak,
                    content: r.clone(),
                    range: r,
                    children: Vec::new(),
                    inlines: Vec::new(),
                });
            }
            Event::TaskListMarker(_) if self.suppressed > 0 => {}
            Event::TaskListMarker(checked) => {
                if let Some(o) = self
                    .stack
                    .iter_mut()
                    .rev()
                    .find(|o| matches!(o.block.kind, BlockKind::ListItem { .. }))
                {
                    o.block.kind = BlockKind::ListItem { task: Some(checked), checkbox: Some(r) };
                }
            }
        }
    }

    fn start(&mut self, tag: Tag<'_>, r: Range<usize>) {
        let t = self.text;
        match tag {
            Tag::Emphasis => return self.start_span(Some(SpanKind::Emphasis), r),
            Tag::Strong => return self.start_span(Some(SpanKind::Strong), r),
            Tag::Strikethrough => return self.start_span(Some(SpanKind::Strikethrough), r),
            Tag::Superscript | Tag::Subscript => return self.start_span(None, r),
            Tag::Link { link_type, dest_url, .. } => {
                let url = dest_url.to_string();
                let k = match link_type {
                    LinkType::Autolink | LinkType::Email => SpanKind::Autolink { url },
                    _ => SpanKind::Link { url },
                };
                return self.start_span(Some(k), r);
            }
            Tag::Image { dest_url, .. } => {
                return self.start_span(Some(SpanKind::Image { url: dest_url.to_string(), alt: String::new() }), r);
            }
            Tag::TableHead | Tag::TableRow => {
                if let Some(o) = self.stack.last_mut() {
                    if let (Some(row), BlockKind::Table { rows, .. }) = (o.cur_row.take(), &mut o.block.kind) {
                        rows.push(row);
                    }
                    o.cur_row = Some(TableRow {
                        header: matches!(tag, Tag::TableHead),
                        range: trim_block_end(t, r),
                        cells: Vec::new(),
                    });
                }
                return;
            }
            Tag::TableCell => {
                if let Some(o) = self.stack.last_mut() {
                    o.cur_cell = Some(r.start);
                }
                return;
            }
            Tag::BlockQuote(_) | Tag::List(_) | Tag::Item | Tag::FootnoteDefinition(_)
                if self.suppressed > 0 || self.stack.len() >= MAX_NEST =>
            {
                self.suppressed += 1;
                return;
            }
            _ => {}
        }
        self.close_synthetic();
        let kind = match tag {
            Tag::Paragraph => BlockKind::Paragraph,
            Tag::Heading { level, .. } => BlockKind::Heading { level: heading_level(level) },
            Tag::BlockQuote(_) => BlockKind::BlockQuote,
            Tag::CodeBlock(kind) => {
                let mut o = match kind {
                    CodeBlockKind::Indented => Open::new(
                        BlockKind::CodeBlock(CodeBlockInfo {
                            fence: None,
                            info: String::new(),
                            lang: None,
                            closed: true,
                            open_fence: None,
                            close_fence: None,
                        }),
                        r,
                    ),
                    CodeBlockKind::Fenced(info) => {
                        let mut o = Open::new(
                            BlockKind::CodeBlock(CodeBlockInfo {
                                fence: Some(Fence { ch: '`', len: 3, indent: 0 }),
                                info: String::new(),
                                lang: None,
                                closed: false,
                                open_fence: None,
                                close_fence: None,
                            }),
                            r,
                        );
                        o.code_info = Some(info.to_string());
                        o
                    }
                };
                o.is_host = false;
                self.push(o);
                return;
            }
            Tag::HtmlBlock => BlockKind::Html,
            Tag::List(start) => BlockKind::List { ordered: start.is_some(), start: start.unwrap_or(0) },
            Tag::Item => BlockKind::ListItem { task: None, checkbox: None },
            Tag::FootnoteDefinition(label) => BlockKind::FootnoteDefinition { label: label.to_string() },
            Tag::Table(aligns) => BlockKind::Table {
                aligns: aligns
                    .iter()
                    .map(|a| match a {
                        pulldown_cmark::Alignment::None => Align::None,
                        pulldown_cmark::Alignment::Left => Align::Left,
                        pulldown_cmark::Alignment::Center => Align::Center,
                        pulldown_cmark::Alignment::Right => Align::Right,
                    })
                    .collect(),
                rows: Vec::new(),
            },
            Tag::MetadataBlock(_) => BlockKind::FrontMatter,
            // Definition lists are not enabled; treat any other container as a quote-like
            // container so the tree stays balanced.
            _ => BlockKind::BlockQuote,
        };
        self.push(Open::new(kind, r));
    }

    fn end(&mut self, end: TagEnd, r: Range<usize>) {
        let t = self.text;
        match end {
            TagEnd::Emphasis
            | TagEnd::Strong
            | TagEnd::Strikethrough
            | TagEnd::Superscript
            | TagEnd::Subscript
            | TagEnd::Link
            | TagEnd::Image => return self.end_span(),
            TagEnd::TableCell => {
                if let Some(o) = self.stack.last_mut() {
                    let start = o.cur_cell.take().unwrap_or(r.start);
                    let cell = trim_spaces(t, start..r.end.max(start));
                    let cell = if cell.is_empty() {
                        // put the empty range inside the cell
                        let p = if r.end > start { (start + 1).min(r.end) } else { start };
                        p..p
                    } else {
                        cell
                    };
                    if let Some(row) = o.cur_row.as_mut() {
                        row.cells.push(cell);
                    }
                }
                return;
            }
            TagEnd::TableHead | TagEnd::TableRow => {
                if let Some(o) = self.stack.last_mut()
                    && let (Some(row), BlockKind::Table { rows, .. }) = (o.cur_row.take(), &mut o.block.kind) {
                        rows.push(row);
                    }
                return;
            }
            TagEnd::BlockQuote(_) | TagEnd::List(_) | TagEnd::Item | TagEnd::FootnoteDefinition
                if self.suppressed > 0 =>
            {
                self.suppressed -= 1;
                return;
            }
            _ => {}
        }
        self.close_synthetic();
        if let Some(o) = self.stack.pop() {
            let b = self.finish(o);
            self.attach(b);
        }
    }
}

fn shrink(r: &Range<usize>, n: usize) -> Range<usize> {
    if r.end >= r.start + 2 * n { (r.start + n)..(r.end - n) } else { r.start..r.start }
}

fn clamp_range(t: &str, c: Range<usize>, outer: &Range<usize>) -> Range<usize> {
    let s = floor_boundary(t, c.start.clamp(outer.start, outer.end));
    let e = floor_boundary(t, c.end.clamp(s, outer.end)).max(s);
    s..e
}

fn heading_level(l: HeadingLevel) -> u8 {
    match l {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn first_word_lang(info: &str) -> Option<String> {
    let w = info.split_whitespace().next()?;
    let w = w.trim_start_matches('{').trim_end_matches('}').trim_start_matches('.');
    let w = w.strip_prefix("language-").unwrap_or(w);
    // pandoc-style `{.rust}` or `rust,ignore`
    let w = w.split([',', '{']).next().unwrap_or(w);
    if w.is_empty() { None } else { Some(normalize_lang(w)) }
}

fn finish_fenced(
    t: &str,
    content: &mut Range<usize>,
    info: &mut CodeBlockInfo,
    r: &Range<usize>,
    text_range: Option<Range<usize>>,
    container_col: usize,
) {
    let b = t.as_bytes();
    let s = r.start;
    let ch = b.get(s).copied().unwrap_or(b'`');
    let ch = if ch == b'~' { b'~' } else { b'`' };
    let len = b[s..r.end].iter().take_while(|&&c| c == ch).count().max(1);
    let col = col_of(t, s);
    let indent = col.saturating_sub(container_col).min(3);
    let open_end = line_end_no_cr(t, s).min(r.end).max(s);
    info.fence = Some(Fence { ch: ch as char, len, indent });
    info.open_fence = Some(s..open_end);
    info.info = t[floor_boundary(t, (s + len).min(open_end))..open_end].trim().to_string();
    let cstart = (line_end(t, s) + 1).min(r.end).max(open_end);
    let cstart = floor_boundary(t, cstart);

    // Closed? The part of the block after the last content text holds a closing fence run.
    let covered = text_range.map_or(cstart, |tr| tr.end.max(cstart)).min(r.end);
    let tail = &t[covered..r.end];
    let run = tail.bytes().filter(|&c| c == ch).count();
    let has_close = covered < r.end && run >= len && r.end > open_end;
    if has_close {
        let cls = line_start(t, r.end);
        let cls = cls.max(covered);
        // first fence char of the closing line
        let fs = t.as_bytes()[cls..r.end].iter().position(|&c| c == ch).map_or(cls, |p| cls + p);
        info.closed = true;
        info.close_fence = Some(fs..r.end);
        // content ends before the newline preceding the closing line
        let ce = trim_one_newline(t, cstart..line_start(t, fs).max(cstart)).end;
        *content = cstart..ce.max(cstart);
    } else {
        info.closed = false;
        info.close_fence = None;
        *content = if cstart >= r.end && r.end <= open_end { r.end..r.end } else { cstart..r.end.max(cstart) };
    }
}

/// Parse the whole document. Must be fast: target ≥ 50 MB/s on a release build.
/// STABLE.
pub fn parse(text: &str) -> Vec<Block> {
    let mut bld = Builder { text, suppressed: 0, images: 0, stack: Vec::new(), top: Vec::new() };
    let mut iter = Parser::new_ext(text, parser_options()).into_offset_iter();
    while let Some((ev, r)) = iter.next() {
        if let Event::Start(Tag::MetadataBlock(_)) = &ev
            && !at_doc_start(text, r.start) {
                // Front matter is only recognised at the very start of the document:
                // re-parse this region as ordinary Markdown.
                for (ev2, _) in iter.by_ref() {
                    if matches!(ev2, Event::End(TagEnd::MetadataBlock(_))) {
                        break;
                    }
                }
                let r = floor_boundary(text, r.start)..floor_boundary(text, r.end);
                let opts = parser_options() - Options::ENABLE_YAML_STYLE_METADATA_BLOCKS;
                for (ev2, r2) in Parser::new_ext(&text[r.clone()], opts).into_offset_iter() {
                    bld.event(ev2, (r2.start + r.start)..(r2.end + r.start));
                }
                continue;
            }
        bld.event(ev, r);
    }
    while let Some(o) = bld.stack.pop() {
        let b = bld.finish(o);
        bld.attach(b);
    }
    bld.top
}

/// Whether byte `i` is the start of the document (ignoring a UTF-8 BOM).
pub fn at_doc_start(text: &str, i: usize) -> bool {
    i == 0 || (i == 3 && text.starts_with('\u{feff}'))
}

/// Per-line code text of a code block: each range is one content line without its line
/// ending, without container prefixes (`> `, list indentation) and without the up-to-
/// `fence.indent` spaces CommonMark strips (CB-6). Tabs are kept. Empty for non-code blocks.
pub fn code_lines(text: &str, block: &Block) -> Vec<Range<usize>> {
    let BlockKind::CodeBlock(info) = &block.kind else { return Vec::new() };
    let c = block.content.clone();
    let r = block.range.clone();
    let ok = |i: usize| i <= text.len() && text.is_char_boundary(i);
    if c.start > c.end || r.start > r.end || ![c.start, c.end, r.start, r.end].into_iter().all(ok) {
        return Vec::new();
    }
    // prefix of the first line before the block (container + fence indentation)
    let pstart = line_start(text, block.range.start);
    let prefix = &text[pstart..block.range.start];
    let fence_indent = info.fence.map_or(0, |f| f.indent);
    let container_prefix = if info.fence.is_some() {
        &prefix[..floor_boundary(prefix, prefix.len().saturating_sub(fence_indent))]
    } else {
        // indented code: drop the code's own 4 columns of indentation from the prefix
        let mut cut = prefix.len();
        let mut cols = 0;
        while cut > 0 && cols < 4 {
            match prefix.as_bytes()[cut - 1] {
                b' ' => cols += 1,
                b'\t' => cols = 4,
                _ => break,
            }
            cut -= 1;
        }
        &prefix[..cut]
    };
    if c.is_empty() {
        let no_lines = match (&info.open_fence, &info.close_fence) {
            (_, Some(cf)) => line_start(text, cf.start) <= c.start,
            (Some(of), None) => c.start <= of.end,
            _ => false,
        };
        if no_lines {
            return Vec::new();
        }
    }
    let mut out = Vec::new();
    let mut ls = c.start;
    let first_is_block_start = info.fence.is_none();
    loop {
        let le = line_end_no_cr(text, ls).min(c.end.max(ls));
        let mut s = ls;
        if !(first_is_block_start && ls == c.start) {
            let line = &text[ls..le];
            // strip the container prefix if present, then up to `fence_indent` spaces
            if !container_prefix.is_empty() {
                if line.starts_with(container_prefix) {
                    s += container_prefix.len();
                } else {
                    // lazy / shorter prefix: strip matching leading spaces and `>` markers
                    let n = line
                        .bytes()
                        .zip(container_prefix.bytes())
                        .take_while(|(a, b)| a == b || (*a == b' ' && *b == b' '))
                        .count();
                    s += n;
                }
            }
            if info.fence.is_some() {
                let n = text.as_bytes()[s..le].iter().take(fence_indent).take_while(|&&b| b == b' ').count();
                s += n;
            } else {
                // indented code: strip 4 columns
                let mut col = 0;
                while s < le && col < 4 {
                    match text.as_bytes()[s] {
                        b' ' => col += 1,
                        b'\t' => col = 4,
                        _ => break,
                    }
                    s += 1;
                }
            }
        }
        out.push(s.min(le)..le);
        let next = line_end(text, ls) + 1;
        if next > c.end || next > text.len() || le >= c.end {
            break;
        }
        ls = next;
    }
    out
}

/// Heading for the outline sidebar. STABLE.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Heading {
    pub level: u8,
    /// Plain text without markers.
    pub text: String,
    /// Byte offset of the heading block start.
    pub at: usize,
}

/// Headings from parsed blocks (only top-level and inside quotes/lists). STABLE.
pub fn outline(text: &str, blocks: &[Block]) -> Vec<Heading> {
    fn walk(text: &str, blocks: &[Block], out: &mut Vec<Heading>) {
        for b in blocks {
            match &b.kind {
                BlockKind::Heading { level } => out.push(Heading { level: *level, text: plain_text(text, b), at: b.range.start }),
                BlockKind::BlockQuote | BlockKind::List { .. } | BlockKind::ListItem { .. } => walk(text, &b.children, out),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(text, blocks, &mut out);
    out
}

/// Plain text of a block's content: inline markers, URLs, HTML tags and backslash escapes
/// removed, line breaks and whitespace runs collapsed to single spaces. Inline code keeps its
/// text. For code blocks returns the content verbatim.
pub fn plain_text(text: &str, block: &Block) -> String {
    let c = block.content.clone();
    if c.end > text.len() || c.start > c.end || !text.is_char_boundary(c.start) || !text.is_char_boundary(c.end) {
        return String::new();
    }
    if matches!(block.kind, BlockKind::CodeBlock(_)) {
        return text[c].to_string();
    }
    let mut skip: Vec<Range<usize>> = Vec::new();
    for sp in &block.inlines {
        match sp.kind {
            SpanKind::Html | SpanKind::FootnoteReference { .. } => skip.push(sp.range.clone()),
            SpanKind::HardBreak => skip.push(sp.range.clone()),
            _ => {
                skip.push(sp.range.start..sp.content.start);
                skip.push(sp.content.end..sp.range.end);
            }
        }
    }
    let code: Vec<Range<usize>> = block
        .inlines
        .iter()
        .filter(|s| matches!(s.kind, SpanKind::Code | SpanKind::InlineMath | SpanKind::DisplayMath))
        .map(|s| s.content.clone())
        .collect();
    let mut out = String::with_capacity(c.len());
    let mut i = c.start;
    let b = text.as_bytes();
    let mut last_space = true;
    while i < c.end {
        if let Some(r) = skip.iter().find(|r| r.start <= i && i < r.end) {
            i = r.end.max(i + 1);
            if !last_space && block.inlines.iter().any(|s| s.kind == SpanKind::HardBreak && s.range.start <= r.start && r.end <= s.range.end) {
                out.push(' ');
                last_space = true;
            }
            continue;
        }
        let in_code = code.iter().any(|r| r.start <= i && i < r.end);
        let Some(ch) = text[i..].chars().next() else { break };
        if !in_code && ch == '\\' && i + 1 < c.end && b[i + 1].is_ascii_punctuation() {
            i += 1;
            continue;
        }
        if ch.is_whitespace() {
            if !last_space {
                out.push(' ');
                last_space = true;
            }
        } else {
            out.push(ch);
            last_space = false;
        }
        i += ch.len_utf8();
    }
    while out.ends_with(' ') {
        out.pop();
    }
    out
}

/// `rs`→`rust`, `py`→`python`, `sh`/`shell`/`zsh`→`bash`, `yml`→`yaml`, `js`→`javascript`,
/// `ts`→`typescript`, `c++`/`cxx`→`cpp`, ... lowercase; unknown names pass through. STABLE.
pub fn normalize_lang(name: &str) -> String {
    let l = name.trim().to_ascii_lowercase();
    let mapped = match l.as_str() {
        "rs" => "rust",
        "py" | "py3" | "python3" | "python2" | "gyp" => "python",
        "sh" | "shell" | "zsh" | "console" | "shellsession" | "shell-session" | "bash" | "ksh" | "ash" => "bash",
        "yml" => "yaml",
        "js" | "jsx" | "mjs" | "cjs" | "node" | "ecmascript" => "javascript",
        "ts" | "tsx" | "mts" | "cts" => "typescript",
        "c++" | "cxx" | "hpp" | "cc" | "hh" | "hxx" | "h++" | "cplusplus" => "cpp",
        "h" => "c",
        "cs" | "c#" => "csharp",
        "golang" => "go",
        "md" | "mkd" | "mkdn" | "mdown" | "mdwn" => "markdown",
        "kt" | "kts" => "kotlin",
        "rb" | "gemfile" | "rake" => "ruby",
        "ps1" | "psm1" | "pwsh" | "posh" | "ps" => "powershell",
        "dockerfile" | "docker" | "containerfile" => "dockerfile",
        "make" | "makefile" | "mk" | "gnumake" | "mf" => "makefile",
        "jsonc" | "json5" | "geojson" => "json",
        "htm" | "xhtml" => "html",
        "pl" | "pm" => "perl",
        "hs" => "haskell",
        "ex" | "exs" => "elixir",
        "erl" | "hrl" => "erlang",
        "tex" => "latex",
        "viml" | "vimscript" => "vim",
        "elisp" | "emacs-lisp" => "lisp",
        "objc" | "obj-c" | "objectivec" | "objective-c" => "objective-c",
        "jl" => "julia",
        "patch" => "diff",
        "txt" | "plain" | "plaintext" | "plain-text" => "text",
        "cfg" | "conf-ini" | "dosini" => "ini",
        "proto" | "proto3" => "protobuf",
        "tf" | "hcl" => "terraform",
        "ml" => "ocaml",
        "fs" | "f#" | "fsx" => "fsharp",
        "clj" | "cljs" | "cljc" | "edn" => "clojure",
        "gradle" => "groovy",
        "bat" | "cmd" => "batch",
        "nasm" | "asm" | "s" => "asm",
        "gql" => "graphql",
        "scm" => "scheme",
        "rkt" => "racket",
        "sc" => "scala",
        "kts.gradle" => "kotlin",
        "zig" => "zig",
        "psql" | "postgres" | "postgresql" | "mysql" | "sqlite" | "plsql" => "sql",
        "xml" | "xsl" | "xslt" | "svg" | "plist" => "xml",
        "vue" => "vue",
        "nimrod" => "nim",
        "cmakelists" | "cmakelists.txt" => "cmake",
        "rst" | "restructuredtext" => "rst",
        "styl" => "stylus",
        other => other,
    };
    mapped.to_string()
}

/// Word and character counts of the prose (code blocks, URLs and markers excluded). STABLE.
///
/// * Words: whitespace-separated tokens of the prose text that contain at least one
///   alphanumeric character (so `—` or `-` alone don't count).
/// * Chars: the number of non-whitespace characters of the prose text (Markdown markers,
///   code, math, URLs, HTML and front matter excluded; link text and image alt included).
///
/// `blocks` is not needed (the count walks pulldown-cmark events) and may be empty.
pub fn word_count(text: &str, blocks: &[Block]) -> (usize, usize) {
    let _ = blocks;
    let mut words = 0usize;
    let mut chars = 0usize;
    let mut in_word = false;
    let mut word_has_alnum = false;
    let mut skip_depth = 0usize; // code block, metadata, html block
    let mut autolink_depth = 0usize;
    fn flush(in_word: &mut bool, has: &mut bool, words: &mut usize) {
        if *in_word && *has {
            *words += 1;
        }
        *in_word = false;
        *has = false;
    }
    let mut meta_skip = false;
    for (ev, r) in Parser::new_ext(text, parser_options()).into_offset_iter() {
        match ev {
            // front matter only counts as metadata at the document start
            Event::Start(Tag::MetadataBlock(_)) if !at_doc_start(text, r.start) => {}
            Event::End(TagEnd::MetadataBlock(_)) if !meta_skip => {}
            Event::Start(Tag::MetadataBlock(_)) => {
                meta_skip = true;
                skip_depth += 1;
            }
            Event::End(TagEnd::MetadataBlock(_)) => {
                meta_skip = false;
                skip_depth = skip_depth.saturating_sub(1);
            }
            Event::Start(Tag::CodeBlock(_)) | Event::Start(Tag::HtmlBlock) => {
                skip_depth += 1;
                flush(&mut in_word, &mut word_has_alnum, &mut words);
            }
            Event::End(TagEnd::CodeBlock) | Event::End(TagEnd::HtmlBlock) => {
                skip_depth = skip_depth.saturating_sub(1);
            }
            Event::Start(Tag::Link { link_type: LinkType::Autolink | LinkType::Email, .. }) => autolink_depth += 1,
            Event::End(TagEnd::Link) if autolink_depth > 0 => autolink_depth -= 1,
            Event::Text(s) if skip_depth == 0 && autolink_depth == 0 => {
                for c in s.chars() {
                    if c.is_whitespace() {
                        flush(&mut in_word, &mut word_has_alnum, &mut words);
                    } else {
                        chars += 1;
                        in_word = true;
                        word_has_alnum |= c.is_alphanumeric();
                    }
                }
            }
            Event::Text(_) => {}
            Event::Code(_) | Event::InlineMath(_) | Event::DisplayMath(_) | Event::SoftBreak | Event::HardBreak | Event::Rule => {
                flush(&mut in_word, &mut word_has_alnum, &mut words)
            }
            Event::End(TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::TableCell | TagEnd::Item) => {
                flush(&mut in_word, &mut word_has_alnum, &mut words)
            }
            _ => {}
        }
    }
    flush(&mut in_word, &mut word_has_alnum, &mut words);
    (words, chars)
}

#[cfg(test)]
mod tests;

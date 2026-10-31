//! Turns Markdown source into styled egui layout jobs for the editor ("live preview").
//!
//! ## The chunk contract (STABLE — the editor relies on it)
//!
//! The document is split into **chunks** that cover the text contiguously with no gaps:
//! each top-level block plus the blank lines after it, ending just after a `\n`
//! (the last chunk may not end with `\n`). Every byte belongs to exactly one chunk.
//!
//! A chunk is drawn as one egui `Galley` laid out from [`StyledChunk::job`]. **The job's text
//! is exactly the chunk's source text without its final `\n`** (if it has one), so galley char
//! index *i* = the *i*-th char of the chunk. Nothing is ever removed or inserted: Markdown
//! markers that live preview hides are kept but formatted invisible (tiny font size +
//! transparent colour, see [`HIDDEN_SIZE`]). This keeps caret/hit-testing trivial.
//! The implicit final `\n` is the boundary to the next chunk.
//!
//! Extra drawing that isn't text (code card backgrounds, quote bars, checkboxes, images,
//! rules) is returned as [`Decoration`]s with chunk-relative byte ranges; the editor turns
//! ranges into rects with the galley and paints them.
//!
//! ## Notes for the editor / app
//!
//! - Two kinds of hiding are used in live preview. Markers that should take no room (`#`,
//!   `**`, backticks, link syntax, code fences) use [`HIDDEN_SIZE`] + transparent colour.
//!   Markers that a decoration is drawn *over* (list bullets `-`, task boxes `[ ]`, quote `>`,
//!   thematic-break `---`) keep their normal size but are transparent, so the decoration
//!   has room and nested content keeps its indentation.
//! - When `focused`, markers are shown in [`Theme::marker`] colour and `Bullet`, `Checkbox`
//!   and the thematic-break `Rule` are not emitted (the text itself is visible).
//! - Bold/italic fonts: egui's default fonts have no bold or italic face. If the app
//!   registers the font families [`BOLD_FAMILY`], [`ITALIC_FAMILY`] and [`BOLD_ITALIC_FAMILY`]
//!   (`FontFamily::Name(..)`) it must set [`Theme::font_families`] = `true` (and give the theme
//!   a distinct `name`, since the editor's cache keys on it). Otherwise (the default) bold is
//!   faked with a stronger colour and italics use egui's synthetic slant.
//! - [`Decoration::Bullet`] and [`Decoration::QuoteBar`] depths are 1-based (1 = top level).
//! - [`Decoration::CodeHeader::range`] is the opening fence line (or the first content line of
//!   indented code).
//!
//! OWNER: agent "style" (also owns the highlight crate).

use std::ops::Range;

use egui::text::LayoutJob;
use egui::{Color32, FontFamily, FontId, Stroke, TextFormat};
use mdedit_highlight::{HlTheme, Highlight, Highlighter};
use mdedit_syntax::{Block, BlockKind, Span, SpanKind};

/// Font size used for hidden markers.
pub const HIDDEN_SIZE: f32 = 0.1;

/// Font family name the app may register for a bold proportional face
/// (`egui::FontFamily::Name(BOLD_FAMILY.into())`). Used only when [`Theme::font_families`].
pub const BOLD_FAMILY: &str = "mdedit-bold";
/// Font family name for an italic proportional face. See [`BOLD_FAMILY`].
pub const ITALIC_FAMILY: &str = "mdedit-italic";
/// Font family name for a bold italic proportional face. See [`BOLD_FAMILY`].
pub const BOLD_ITALIC_FAMILY: &str = "mdedit-bold-italic";

/// Heading size relative to body text for levels 1..=6.
pub fn heading_scale(level: u8) -> f32 {
    match level {
        0 | 1 => 2.0,
        2 => 1.6,
        3 => 1.3,
        4 => 1.15,
        5 => 1.0,
        _ => 0.9,
    }
}

/// STABLE.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Mode {
    /// Formatted; markers shown only in the chunk holding the caret/selection.
    #[default]
    LivePreview,
    /// All text shown, monospace, with Markdown syntax colouring.
    Source,
}

/// Colours and sizes. STABLE fields (add new ones freely).
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub name: String,
    pub dark: bool,
    pub bg: Color32,
    pub text: Color32,
    pub muted: Color32,
    /// Markdown markers when shown (`**`, `#`, fences).
    pub marker: Color32,
    pub heading: Color32,
    pub link: Color32,
    pub code_bg: Color32,
    pub code_text: Color32,
    pub code_border: Color32,
    pub inline_code_bg: Color32,
    pub quote_bar: Color32,
    pub selection: Color32,
    pub caret: Color32,
    pub search_match: Color32,
    pub search_current: Color32,
    pub rule: Color32,
    pub table_border: Color32,
    pub body_size: f32,
    pub code_size: f32,
    pub line_height: f32,
    /// Max text column width in points (editor centres it); 0 = full width.
    pub max_width: f32,
    /// True when the app registered [`BOLD_FAMILY`], [`ITALIC_FAMILY`] and
    /// [`BOLD_ITALIC_FAMILY`]; then strong/emphasis text uses them. Default `false`.
    pub font_families: bool,
}

impl Theme {
    pub fn light() -> Self {
        Self {
            name: "Light".into(),
            dark: false,
            bg: Color32::from_rgb(0xff, 0xff, 0xff),
            text: Color32::from_rgb(0x24, 0x29, 0x2f),
            muted: Color32::from_rgb(0x65, 0x6d, 0x76),
            marker: Color32::from_rgb(0xa0, 0xa8, 0xb0),
            heading: Color32::from_rgb(0x1f, 0x23, 0x28),
            link: Color32::from_rgb(0x09, 0x69, 0xda),
            code_bg: Color32::from_rgb(0xf6, 0xf8, 0xfa),
            code_text: Color32::from_rgb(0x24, 0x29, 0x2f),
            code_border: Color32::from_rgb(0xd0, 0xd7, 0xde),
            inline_code_bg: Color32::from_rgb(0xef, 0xf1, 0xf3),
            quote_bar: Color32::from_rgb(0xd0, 0xd7, 0xde),
            selection: Color32::from_rgba_unmultiplied(0x54, 0xae, 0xff, 0x66),
            caret: Color32::from_rgb(0x24, 0x29, 0x2f),
            search_match: Color32::from_rgba_unmultiplied(0xff, 0xd3, 0x3d, 0x80),
            search_current: Color32::from_rgba_unmultiplied(0xff, 0x8c, 0x00, 0xb0),
            rule: Color32::from_rgb(0xd0, 0xd7, 0xde),
            table_border: Color32::from_rgb(0xd0, 0xd7, 0xde),
            body_size: 16.0,
            code_size: 14.0,
            line_height: 1.5,
            max_width: 820.0,
            font_families: false,
        }
    }
    pub fn dark() -> Self {
        Self {
            name: "Dark".into(),
            dark: true,
            bg: Color32::from_rgb(0x0d, 0x11, 0x17),
            text: Color32::from_rgb(0xe6, 0xed, 0xf3),
            muted: Color32::from_rgb(0x8d, 0x96, 0xa0),
            marker: Color32::from_rgb(0x6e, 0x76, 0x81),
            heading: Color32::from_rgb(0xf0, 0xf6, 0xfc),
            link: Color32::from_rgb(0x4c, 0x9b, 0xff),
            code_bg: Color32::from_rgb(0x16, 0x1b, 0x22),
            code_text: Color32::from_rgb(0xe6, 0xed, 0xf3),
            code_border: Color32::from_rgb(0x30, 0x36, 0x3d),
            inline_code_bg: Color32::from_rgb(0x26, 0x2c, 0x36),
            quote_bar: Color32::from_rgb(0x30, 0x36, 0x3d),
            selection: Color32::from_rgba_unmultiplied(0x38, 0x8b, 0xfd, 0x66),
            caret: Color32::from_rgb(0xe6, 0xed, 0xf3),
            search_match: Color32::from_rgba_unmultiplied(0xbb, 0x80, 0x09, 0x80),
            search_current: Color32::from_rgba_unmultiplied(0xff, 0x8c, 0x00, 0xb0),
            rule: Color32::from_rgb(0x30, 0x36, 0x3d),
            table_border: Color32::from_rgb(0x30, 0x36, 0x3d),
            body_size: 16.0,
            code_size: 14.0,
            line_height: 1.5,
            max_width: 820.0,
            font_families: false,
        }
    }
}

/// STABLE.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chunk {
    /// Bytes of the source; ranges of consecutive chunks touch.
    pub range: Range<usize>,
    /// Index into the top-level `blocks` slice, or `None` for leading blank lines.
    pub block: Option<usize>,
}

fn floor_boundary(s: &str, mut i: usize) -> usize {
    if i >= s.len() {
        return s.len();
    }
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn ceil_boundary(s: &str, mut i: usize) -> usize {
    if i >= s.len() {
        return s.len();
    }
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

/// Start of the line containing byte `pos` (clamped to the text).
fn line_start(text: &str, pos: usize) -> usize {
    let pos = floor_boundary(text, pos);
    text.as_bytes()[..pos].iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1)
}

/// Split the document into chunks per the module contract. STABLE.
///
/// Blocks are expected sorted by start (as `parse` returns them); a block starting on the same
/// line as the previous one joins the previous chunk. With no blocks at all (e.g. the parser
/// hasn't run yet) the text is split at blank lines into `block: None` chunks so a big file
/// never becomes one giant galley.
pub fn chunks(text: &str, blocks: &[Block]) -> Vec<Chunk> {
    if text.is_empty() {
        return vec![Chunk { range: 0..0, block: None }];
    }
    let mut starts = block_starts(text, blocks, 0..blocks.len());
    if starts.is_empty() {
        starts = blank_line_split(text);
    }
    if starts.first().is_none_or(|&(p, _)| p > 0) {
        starts.insert(0, (0, None));
    }
    let mut out = Vec::with_capacity(starts.len());
    for (k, &(s, block)) in starts.iter().enumerate() {
        let e = starts.get(k + 1).map_or(text.len(), |n| n.0);
        out.push(Chunk { range: s..e, block });
    }
    out
}

/// Chunk starts of `blocks[which]`; `which.start` must not join the chunk of the block before it.
fn block_starts(text: &str, blocks: &[Block], which: Range<usize>) -> Vec<(usize, Option<usize>)> {
    let mut starts: Vec<(usize, Option<usize>)> = Vec::with_capacity(which.len() + 1);
    for i in which {
        let s = line_start(text, blocks[i].range.start);
        if starts.last().is_none_or(|&(p, _)| s > p) {
            starts.push((s, Some(i)));
        }
    }
    starts
}

/// The part of [`chunks`] for `blocks[which]`, the last chunk ending at `end`: what an edit has
/// to recompute. Same result as [`chunks`] if `which` starts and `end` lies on a chunk boundary.
/// Does not add the leading `block: None` chunk.
pub fn chunks_of_blocks(text: &str, blocks: &[Block], which: Range<usize>, end: usize) -> Vec<Chunk> {
    let starts = block_starts(text, blocks, which);
    let mut out = Vec::with_capacity(starts.len());
    for (k, &(s, block)) in starts.iter().enumerate() {
        let e = starts.get(k + 1).map_or(end, |n| n.0);
        out.push(Chunk { range: s..e, block });
    }
    out
}

/// Fallback chunking without a block index: a new chunk starts at each non-blank line that
/// follows a blank line.
fn blank_line_split(text: &str) -> Vec<(usize, Option<usize>)> {
    let mut out = vec![(0, None)];
    let mut pos = 0;
    let mut prev_blank = false;
    for line in text.split_inclusive('\n') {
        let blank = line.trim().is_empty();
        if !blank && prev_blank && pos > 0 {
            out.push((pos, None));
        }
        prev_blank = blank;
        pos += line.len();
    }
    out
}

/// Non-text drawing for a chunk. Ranges are **chunk-relative bytes**. STABLE.
#[derive(Clone, Debug, PartialEq)]
pub enum Decoration {
    /// A card behind the rows covering `range` (code blocks, front matter, math).
    /// Full text-column width, rounded, with border.
    Card { range: Range<usize>, fill: Color32, stroke: Color32 },
    /// Header line above a code card's first row: language label + copy button. `code` is
    /// the chunk-relative range of the code content (what Copy copies).
    CodeHeader { range: Range<usize>, lang: String, code: Range<usize> },
    /// Rounded background behind exactly the glyphs of `range` (inline code).
    InlineBg { range: Range<usize>, fill: Color32 },
    /// Vertical bar left of the rows covering `range` (block quotes). `depth` ≥ 1 for nesting.
    QuoteBar { range: Range<usize>, color: Color32, depth: u8 },
    /// Horizontal rule across the row of `range` (thematic break, under h1/h2).
    Rule { range: Range<usize>, color: Color32 },
    /// Clickable checkbox drawn over the (hidden) `[ ]` at `range`.
    Checkbox { range: Range<usize>, checked: bool },
    /// Bullet glyph drawn left of the hidden list marker at `range` (in the list indent).
    Bullet { range: Range<usize>, depth: u8 },
    /// Number of an ordered list item (`1.`), drawn left of the hidden marker at `range`.
    ListNumber { range: Range<usize>, text: String },
    /// Image shown below the row containing `range`; `url` as written in the source.
    Image { range: Range<usize>, url: String, alt: String },
    /// Table: cell content ranges per row, and each column's alignment. The editor lines the
    /// cells up in columns and draws the borders.
    TableGrid { rows: Vec<Vec<Range<usize>>>, aligns: Vec<mdedit_syntax::Align>, color: Color32 },
    /// Clickable link (Ctrl+click opens).
    Link { range: Range<usize>, url: String },
}

/// STABLE.
pub struct StyledChunk {
    pub job: LayoutJob,
    pub decorations: Vec<Decoration>,
    /// Extra vertical space above / below the galley in points (paragraph spacing).
    pub space_before: f32,
    pub space_after: f32,
    /// Left indent in points applied to the whole galley (code cards use padding inside).
    pub indent: f32,
    /// The result can change without the chunk's text changing, once
    /// [`Highlighter::generation`] moves on: highlighting was still pending, or the chunk is a
    /// piece of a code block (whose colours depend on text in other pieces).
    pub hl_volatile: bool,
}

/// Inputs for styling one chunk.
pub struct StyleInput<'a> {
    pub text: &'a str,
    pub blocks: &'a [Block],
    pub chunk: &'a Chunk,
    pub mode: Mode,
    /// True when the caret or selection touches this chunk: show markers (live preview).
    pub focused: bool,
    pub theme: &'a Theme,
    pub highlighter: &'a Highlighter,
    /// Width available for wrapping, in points.
    pub wrap_width: f32,
}

// ---------------------------------------------------------------------------------------------
// Styling internals

/// How a byte is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Vis {
    Normal,
    /// Hidden (tiny + transparent) unless focused / source mode, then marker colour.
    Marker,
    /// Transparent at normal size (a decoration covers it) unless focused / source mode.
    Ghost,
    /// Always shown in marker colour.
    Dim,
    /// Always hidden (e.g. `\r` of CRLF).
    Hidden,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct St {
    size: f32,
    mono: bool,
    color: Color32,
    bold: bool,
    italic: bool,
    strike: bool,
    underline: bool,
    top: bool,
    /// Line height factor relative to size.
    lh: f32,
    vis: Vis,
}

/// Per-byte style ids into an interned style table.
struct Paint {
    ids: Vec<u16>,
    table: Vec<St>,
}

impl Paint {
    fn new(len: usize, base: St) -> Self {
        Self { ids: vec![0; len], table: vec![base] }
    }

    fn intern(&mut self, st: St) -> u16 {
        if let Some(i) = self.table.iter().position(|t| *t == st) {
            return i as u16;
        }
        if self.table.len() >= u16::MAX as usize {
            return 0;
        }
        self.table.push(st);
        (self.table.len() - 1) as u16
    }

    /// Apply `f` to the style of every byte in `r` (already clamped, chunk-relative).
    fn apply(&mut self, r: Range<usize>, f: impl Fn(&mut St)) {
        let end = r.end.min(self.ids.len());
        if r.start >= end {
            return;
        }
        let mut memo: Vec<(u16, u16)> = Vec::new();
        for i in r.start..end {
            let old = self.ids[i];
            let new = match memo.iter().find(|m| m.0 == old) {
                Some(m) => m.1,
                None => {
                    let mut st = self.table[old as usize];
                    f(&mut st);
                    let n = self.intern(st);
                    memo.push((old, n));
                    n
                }
            };
            self.ids[i] = new;
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Depth {
    quote: u8,
    list: u8,
    /// Block nesting level (any kind), bounded by [`MAX_BLOCK_NEST`].
    nest: u16,
}

/// Blocks nested deeper than this are not styled (they render as plain text of their
/// parent). Defence in depth against stack overflow on e.g. `>>>>…`.
pub const MAX_BLOCK_NEST: u16 = 64;
/// Inline spans nested deeper than this are not styled (shown as plain text with their
/// markers). Keeps styling O(n · MAX_INLINE_NEST) on pathological input like `***…x***…`.
pub const MAX_INLINE_NEST: usize = 32;

/// Indices of `spans` whose nesting depth (number of spans strictly containing them) is
/// below [`MAX_INLINE_NEST`], in their original order. O(n log n).
fn shallow_spans(spans: &[&Span]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..spans.len()).collect();
    order.sort_by(|&a, &b| {
        let (ra, rb) = (&spans[a].range, &spans[b].range);
        ra.start.cmp(&rb.start).then(rb.end.cmp(&ra.end))
    });
    let mut keep = vec![false; spans.len()];
    let mut stack: Vec<usize> = Vec::new(); // ends of open containing spans
    for i in order {
        let r = &spans[i].range;
        while stack.last().is_some_and(|&e| e <= r.start) {
            stack.pop();
        }
        keep[i] = stack.len() < MAX_INLINE_NEST;
        stack.push(r.end);
    }
    (0..spans.len()).filter(|&i| keep[i]).collect()
}

struct Ctx<'a> {
    text: &'a str,
    /// Chunk text without its final `\n` (= job text).
    body: &'a str,
    /// Absolute offset of the chunk start.
    base: usize,
    theme: &'a Theme,
    hl: &'a Highlighter,
    live: bool,
    /// Markers visible (focused or source mode).
    show: bool,
    paint: Paint,
    decos: Vec<Decoration>,
    hl_volatile: bool,
}

impl Ctx<'_> {
    /// Absolute range → chunk-relative, clamped to the body and to char boundaries.
    fn rel(&self, r: &Range<usize>) -> Range<usize> {
        let s = r.start.saturating_sub(self.base).min(self.body.len());
        let e = r.end.saturating_sub(self.base).min(self.body.len()).max(s);
        floor_boundary(self.body, s)..ceil_boundary(self.body, e)
    }

    fn apply(&mut self, abs: &Range<usize>, f: impl Fn(&mut St)) {
        let r = self.rel(abs);
        self.paint.apply(r, f);
    }

    fn vis(&mut self, abs: &Range<usize>, v: Vis) {
        self.apply(abs, |s| s.vis = v);
    }

    fn deco(&mut self, d: Decoration) {
        if self.live {
            self.decos.push(d);
        }
    }

    /// Start of the container prefix before `at` on its line: indentation and `>` quote
    /// markers (`at` itself if there is other text before it).
    fn indent_start(&self, at: usize) -> usize {
        let ls = line_start(self.text, at);
        match self.text.get(ls..at) {
            Some(p) if p.chars().all(|c| matches!(c, ' ' | '\t' | '>')) => ls,
            _ => at,
        }
    }

    /// Whether `abs` overlaps this chunk. A chunk can be part of a block (the editor gives
    /// code blocks nested in lists a chunk of their own); parts outside it must not produce
    /// decorations, which would otherwise be clamped to an empty range at the chunk edge.
    fn in_chunk(&self, abs: &Range<usize>) -> bool {
        abs.start < self.base + self.body.len() && abs.end > self.base
    }

    /// The blocks of `blocks` (sorted, as parsed) that overlap this chunk, by binary search:
    /// styling a chunk must not cost the size of the list or quote around it.
    fn in_chunk_blocks<'b>(&self, blocks: &'b [Block]) -> &'b [Block] {
        let hi = self.base + self.body.len();
        let first = blocks.partition_point(|c| c.range.end <= self.base);
        let n = blocks[first..].partition_point(|c| c.range.start < hi);
        &blocks[first..first + n]
    }

    fn byte(&self, abs: usize) -> Option<u8> {
        self.text.as_bytes().get(abs).copied()
    }

    /// Extend `r` to include a following `\n` (or `\r\n`).
    fn with_newline(&self, r: &Range<usize>) -> Range<usize> {
        let mut e = r.end;
        if self.byte(e) == Some(b'\r') {
            e += 1;
        }
        if self.byte(e) == Some(b'\n') {
            e += 1;
        }
        r.start..e
    }

    /// Mark `range` minus `content` as markers (only when content lies inside range).
    fn outer_markers(&mut self, range: &Range<usize>, content: &Range<usize>, v: Vis) {
        if content.start < range.start || content.end > range.end || content.start > content.end {
            return;
        }
        self.vis(&(range.start..content.start), v);
        self.vis(&(content.end..range.end), v);
    }

    fn style_block(&mut self, b: &Block, d: Depth) {
        let d = Depth { nest: d.nest.saturating_add(1), ..d };
        if d.nest > MAX_BLOCK_NEST || !self.in_chunk(&b.range) {
            return;
        }
        let t = self.theme;
        match &b.kind {
            BlockKind::Paragraph => {}
            BlockKind::Heading { level } => {
                let size = t.body_size * heading_scale(*level);
                let col = t.heading;
                self.apply(&b.range, |s| {
                    s.size = size;
                    s.color = col;
                    s.bold = true;
                    s.lh = 1.25;
                });
                self.outer_markers(&b.range, &b.content, Vis::Marker);
                // `# ` / closing `#`s / a setext underline (incl. the newline before it, so
                // that row collapses) are everything outside `content`.
                if *level <= 2 {
                    let r = self.rel(&b.content);
                    self.deco(Decoration::Rule { range: r, color: t.rule });
                }
            }
            BlockKind::BlockQuote => {
                let depth = d.quote.saturating_add(1);
                let muted = t.muted;
                self.apply(&b.range, |s| s.color = muted);
                self.quote_markers(&b.range, depth);
                let r = self.rel(&b.range);
                self.deco(Decoration::QuoteBar { range: r, color: t.quote_bar, depth });
            }
            BlockKind::ThematicBreak => {
                self.vis(&b.range, Vis::Ghost);
                if !self.show {
                    let r = self.rel(&b.range);
                    self.deco(Decoration::Rule { range: r, color: t.rule });
                }
            }
            BlockKind::List { ordered, .. } => {
                // Source indentation is hidden in live preview: the editor indents each item by
                // its depth (so wrapped lines line up with the item's text).
                let mut ls = line_start(self.text, b.range.start.max(self.base));
                let stop = b.range.end.min(self.base + self.body.len());
                while ls < stop {
                    let le = self.text[ls..].find('\n').map_or(self.text.len(), |p| ls + p);
                    let ws = self.text[ls..le].len() - self.text[ls..le].trim_start_matches([' ', '\t', '>']).len();
                    if ws > 0 && ws < le - ls {
                        self.vis(&(ls..ls + ws), Vis::Marker);
                    }
                    ls = le + 1;
                }
                let nd = Depth { list: d.list.saturating_add(1), ..d };
                for c in self.in_chunk_blocks(&b.children) {
                    if let BlockKind::ListItem { .. } = c.kind {
                        self.list_item(c, *ordered, nd);
                    } else {
                        self.style_block(c, nd);
                    }
                }
                self.inlines(&b.inlines);
                return;
            }
            BlockKind::ListItem { .. } => {
                let nd = Depth { list: d.list.max(1), ..d };
                self.list_item(b, false, nd);
                return;
            }
            BlockKind::CodeBlock(info) => {
                self.code_block(b, info);
                return;
            }
            BlockKind::Table { aligns, rows } => {
                self.table(b, aligns, rows);
            }
            BlockKind::Html => {
                let (cs, muted) = (t.code_size, t.muted);
                self.apply(&b.range, |s| {
                    s.mono = true;
                    s.size = cs;
                    s.color = muted;
                    s.lh = 1.35;
                });
            }
            BlockKind::FrontMatter | BlockKind::MathBlock => {
                let front = matches!(b.kind, BlockKind::FrontMatter);
                let (cs, col) = (t.code_size, if front { t.muted } else { t.code_text });
                self.apply(&b.range, |s| {
                    s.mono = true;
                    s.size = cs;
                    s.color = col;
                    s.lh = 1.35;
                    s.italic = false;
                });
                if b.content != b.range {
                    let open = b.range.start..b.content.start;
                    let close = b.content.end..b.range.end;
                    self.vis(&open, Vis::Marker);
                    self.vis(&close, Vis::Marker);
                } else {
                    self.delimiter_lines(&b.range, front);
                }
                let r = self.rel(&b.range);
                self.deco(Decoration::Card { range: r, fill: t.code_bg, stroke: t.code_border });
            }
            BlockKind::FootnoteDefinition { .. } => {
                let end = b
                    .children
                    .first()
                    .map(|c| c.range.start)
                    .unwrap_or(b.content.start)
                    .clamp(b.range.start, b.range.end.max(b.range.start));
                let small = t.body_size * 0.85;
                self.apply(&(b.range.start..end), |s| {
                    s.vis = Vis::Dim;
                    s.size = small;
                });
            }
        }
        for c in self.in_chunk_blocks(&b.children) {
            let nd = if matches!(b.kind, BlockKind::BlockQuote) {
                Depth { quote: d.quote.saturating_add(1), ..d }
            } else {
                d
            };
            self.style_block(c, nd);
        }
        self.inlines(&b.inlines);
    }

    /// Mark the `depth`-th `>` (plus one following space) on each line of `range` as Ghost.
    fn quote_markers(&mut self, range: &Range<usize>, depth: u8) {
        let bytes = self.text.as_bytes();
        let end = range.end.min(bytes.len());
        let mut line = line_start(self.text, range.start).max(self.base);
        let stop = end.min(self.base + self.body.len());
        while line < stop {
            let mut i = line;
            let mut n = 0u8;
            loop {
                let mut sp = 0;
                while i < end && sp < 4 && (bytes[i] == b' ' || bytes[i] == b'\t') {
                    i += 1;
                    sp += 1;
                }
                if i < end && bytes[i] == b'>' {
                    n += 1;
                    let mut e = i + 1;
                    if e < end && bytes[e] == b' ' {
                        e += 1;
                    }
                    if n == depth {
                        // The editor indents the quote, so the marker takes no room; on an
                        // otherwise empty line it keeps its size so the blank row stays.
                        let le = bytes[e..end].iter().position(|&b| b == b'\n').map_or(end, |p| e + p);
                        let blank = bytes[e..le].iter().all(|b| matches!(b, b' ' | b'\t' | b'>' | b'\r'));
                        self.vis(&(i..e), if blank { Vis::Ghost } else { Vis::Marker });
                        break;
                    }
                    i = e;
                } else {
                    break;
                }
            }
            match bytes[line..end].iter().position(|&b| b == b'\n') {
                Some(p) => line += p + 1,
                None => break,
            }
        }
    }

    fn list_item(&mut self, item: &Block, ordered: bool, d: Depth) {
        let BlockKind::ListItem { task, checkbox } = &item.kind else {
            return;
        };
        if !self.in_chunk(&item.range) {
            return;
        }
        let bytes = self.text.as_bytes();
        let end = item.range.end.min(bytes.len());
        let mut i = item.range.start.min(end);
        while i < end && (bytes[i] == b' ' || bytes[i] == b'\t') {
            i += 1;
        }
        // The marker and the spaces after it are hidden in live preview; the bullet or number
        // is drawn as a decoration in the list indent. Shown as text while editing.
        let after_spaces = |mut e: usize| {
            while e < end && (bytes[e] == b' ' || bytes[e] == b'\t') {
                e += 1;
            }
            e
        };
        let is_bullet = i < end && matches!(bytes[i], b'-' | b'*' | b'+');
        if is_bullet && !ordered {
            let m = i..i + 1;
            self.vis(&(item.range.start..after_spaces(i + 1)), Vis::Marker);
            if !self.show && self.in_chunk(&m) {
                let r = self.rel(&m);
                self.deco(Decoration::Bullet { range: r, depth: d.list.max(1) });
            }
        } else if is_bullet || i < end && bytes[i].is_ascii_digit() {
            let mut e = i;
            while e < end && bytes[e].is_ascii_digit() {
                e += 1;
            }
            if e < end && matches!(bytes[e], b'.' | b')' | b'-' | b'*' | b'+') {
                e += 1;
            }
            let muted = self.theme.muted;
            self.apply(&(i..e), |s| s.color = muted);
            self.vis(&(item.range.start..after_spaces(e)), Vis::Marker);
            if !self.show && self.in_chunk(&(i..e)) {
                let r = self.rel(&(i..e));
                let text = self.text.get(i..e).unwrap_or_default().to_string();
                self.deco(Decoration::ListNumber { range: r, text });
            }
        }
        if let (Some(checked), Some(cb)) = (task, checkbox) {
            self.vis(cb, Vis::Ghost);
            if *checked {
                let muted = self.theme.muted;
                let ie = item.range.end.min(bytes.len());
                let mut ts = cb.end.min(ie);
                while ts < ie && (bytes[ts] == b' ' || bytes[ts] == b'\t') {
                    ts += 1;
                }
                let mut te = bytes[ts..ie].iter().position(|&b| b == b'\n').map_or(ie, |p| ts + p);
                if te > ts && bytes[te - 1] == b'\r' {
                    te -= 1;
                }
                self.apply(&(ts..te), |s| {
                    s.color = muted;
                    s.strike = true;
                });
            }
            if !self.show && self.in_chunk(cb) {
                let r = self.rel(cb);
                self.deco(Decoration::Checkbox { range: r, checked: *checked });
            }
        }
        for c in self.in_chunk_blocks(&item.children) {
            self.style_block(c, d);
        }
        self.inlines(&item.inlines);
    }

    fn code_block(&mut self, b: &Block, info: &mdedit_syntax::CodeBlockInfo) {
        let t = self.theme;
        let (cs, ct) = (t.code_size, t.code_text);
        self.apply(&b.range, |s| {
            s.mono = true;
            s.size = cs;
            s.color = ct;
            s.lh = 1.35;
            s.bold = false;
            s.italic = false;
            s.strike = false;
            s.underline = false;
        });
        let content = b.content.start.max(b.range.start)..b.content.end.min(b.range.end).max(b.content.start.max(b.range.start));
        // Fences.
        let fenced = info.fence.is_some() || info.open_fence.is_some();
        let open = info.open_fence.clone().or_else(|| fenced.then_some(b.range.start..content.start));
        let close = info.close_fence.clone().or_else(|| {
            (fenced && info.closed && content.end < b.range.end).then_some(content.end..b.range.end)
        });
        let marker_col = t.marker;
        for f in open.iter().chain(close.iter()) {
            // Include the container prefix (list indent, `> `) so the hidden fence line of a
            // nested block takes no room.
            let ls = line_start(self.text, f.start);
            let prefix = self.text.get(ls..f.start.max(ls)).unwrap_or("x");
            let ls = if prefix.chars().all(|c| matches!(c, ' ' | '\t' | '>')) { ls } else { f.start };
            let r = self.with_newline(&(ls..f.end));
            self.vis(&r, Vis::Marker);
            if self.show {
                self.apply(f, |s| s.color = marker_col);
            }
        }
        // Highlighting.
        let lang = info
            .lang
            .clone()
            .filter(|l| !l.is_empty())
            .or_else(|| info.info.split_whitespace().next().map(str::to_string))
            .unwrap_or_default();
        // Exact code text: container prefixes (`> `, list indent) are not part of it. They
        // stay in the source and are drawn muted.
        let lines: Vec<Range<usize>> = code_lines_checked(self.text, b)
            .into_iter()
            .filter(|l| {
                l.start <= l.end
                    && l.end <= self.text.len()
                    && self.text.is_char_boundary(l.start)
                    && self.text.is_char_boundary(l.end)
            })
            .collect();
        let muted = t.muted;
        // Only the lines around this chunk: it can be one piece of a big block.
        let (lo, hi) = (self.base, self.base + self.body.len());
        let first_line = lines.partition_point(|l| l.end < lo);
        let mut prev_end = if first_line > 0 { lines[first_line - 1].end.max(content.start) } else { content.start };
        for l in &lines[first_line..] {
            if l.start > prev_end {
                // Between the previous line's end and this line's start: newline + prefix.
                self.apply(&(prev_end..l.start), |s| s.color = muted);
            }
            // Indentation before the code (list indent, the 4 spaces of indented code) is
            // syntax: hidden in live preview so the code lines up with the card padding and
            // with its own wrapped lines.
            let ind = self.indent_start(l.start);
            if ind < l.start {
                self.vis(&(ind..l.start), Vis::Marker);
            }
            // The code's own indentation stays, even though a list hides its lines' indentation.
            self.vis(l, Vis::Normal);
            prev_end = prev_end.max(l.end);
            if l.start > hi {
                break;
            }
        }
        if !lang.is_empty() && !lines.is_empty() {
            let mut code = String::new();
            // (offset in `code`, absolute start, len) per line.
            let mut map: Vec<(usize, usize, usize)> = Vec::with_capacity(lines.len());
            for (i, l) in lines.iter().enumerate() {
                if i > 0 {
                    code.push('\n');
                }
                map.push((code.len(), l.start, l.len()));
                code.push_str(&self.text[l.clone()]);
            }
            let theme = if t.dark { HlTheme::Dark } else { HlTheme::Light };
            let answer = self.hl.highlight_nonblocking(&lang, &code, theme);
            let piece = content.start < lo || content.end > hi;
            self.hl_volatile |= piece || matches!(answer, Highlight::Pending(_));
            if let Some(spans) = answer.spans() {
                // Spans are sorted: skip to the ones on this chunk's lines.
                let m0 = map.partition_point(|m| m.1 + m.2 <= lo);
                let m1 = map.partition_point(|m| m.1 < hi);
                let code_lo = map.get(m0).map_or(code.len(), |m| m.0);
                let code_hi = if m1 > 0 { map[m1 - 1].0 + map[m1 - 1].2 } else { 0 };
                let from = spans.partition_point(|s| s.range.end <= code_lo);
                for sp in spans[from..].iter().take_while(|s| s.range.start < code_hi) {
                    // Lines containing the span (spans never cross a newline, but be safe).
                    let first = map.partition_point(|m| m.0 + m.2 <= sp.range.start);
                    for &(off, abs, len) in &map[first.min(map.len())..] {
                        if off >= sp.range.end {
                            break;
                        }
                        let s0 = sp.range.start.max(off) - off;
                        let s1 = sp.range.end.min(off + len).saturating_sub(off);
                        if s1 <= s0 {
                            continue;
                        }
                        let a = abs + s0..abs + s1;
                        if a.end <= lo || a.start >= hi {
                            continue;
                        }
                        let col = Color32::from_rgb(sp.fg.0, sp.fg.1, sp.fg.2);
                        let (bold, italic) = (sp.bold, sp.italic);
                        self.apply(&a, |s| {
                            s.color = col;
                            s.bold = bold;
                            s.italic = italic;
                        });
                    }
                }
            }
        }
        let range = self.rel(&b.range);
        self.deco(Decoration::Card { range: range.clone(), fill: t.code_bg, stroke: t.code_border });
        let header = match &open {
            Some(f) => self.rel(f),
            None => {
                let ls = content.start;
                let le = self.text[ls.min(self.text.len())..]
                    .find('\n')
                    .map_or(content.end, |p| (ls + p).min(content.end));
                self.rel(&(ls..le))
            }
        };
        let code = self.rel(&content);
        self.deco(Decoration::CodeHeader { range: header, lang, code });
    }

    fn table(&mut self, b: &Block, aligns: &[mdedit_syntax::Align], rows: &[mdedit_syntax::TableRow]) {
        let t = self.theme;
        // Proportional text: the editor aligns the columns, so monospace isn't needed (and
        // has no bold face for the header).
        let size = t.body_size * 0.95;
        self.apply(&b.range, |s| {
            s.size = size;
            s.lh = 1.6;
        });
        // Pipes: transparent at full width unless focused (keeps cells aligned with the
        // grid). Delimiter row: hidden (row collapses) unless focused.
        let r = self.rel(&b.range);
        let mut dims: Vec<(Range<usize>, Vis)> = Vec::new();
        let body = self.body;
        let mut pos = r.start;
        for line in body[r.clone()].split_inclusive('\n') {
            let l_end = pos + line.len();
            if line.trim().chars().all(|c| matches!(c, '|' | ':' | '-' | ' ' | '\t'))
                && line.contains('-')
            {
                dims.push((pos..l_end, Vis::Marker));
            } else {
                for (i, c) in line.char_indices() {
                    if c == '|' {
                        dims.push((pos + i..pos + i + 1, Vis::Ghost));
                    }
                }
            }
            pos = l_end;
        }
        for (d, v) in dims {
            self.paint.apply(d, |s| s.vis = v);
        }
        let head = t.heading;
        let mut grid = Vec::with_capacity(rows.len());
        for row in rows {
            if row.header {
                self.apply(&row.range, |s| {
                    s.bold = true;
                    s.color = head;
                });
            }
            grid.push(row.cells.iter().map(|c| self.rel(c)).collect::<Vec<_>>());
        }
        self.deco(Decoration::TableGrid { rows: grid, aligns: aligns.to_vec(), color: t.table_border });
    }

    /// For front matter / math blocks whose content range isn't split out: the first and the
    /// last line are delimiters if they look like one.
    fn delimiter_lines(&mut self, range: &Range<usize>, front: bool) {
        let end = range.end.min(self.text.len());
        let start = range.start.min(end);
        if !self.text.is_char_boundary(start) || !self.text.is_char_boundary(end) {
            return;
        }
        let src = &self.text[start..end];
        let is_delim = |l: &str| {
            let l = l.trim();
            if front { matches!(l, "---" | "+++" | "...") } else { l == "$$" }
        };
        let first_end = src.find('\n').unwrap_or(src.len());
        if is_delim(&src[..first_end]) {
            let r = self.with_newline(&(start..start + first_end));
            self.vis(&r, Vis::Marker);
        }
        if let Some(p) = src.trim_end_matches(['\n', '\r']).rfind('\n')
            && p + 1 > first_end
        {
            let last = &src[p + 1..];
            if is_delim(last) {
                self.vis(&(start + p + 1..end), Vis::Marker);
            }
        }
    }

    fn inlines(&mut self, spans: &[Span]) {
        let t = self.theme;
        // A span containing one in this chunk overlaps the chunk too, so nesting depths are the
        // same among just these. Spans elsewhere in the block would paint nothing here.
        let hi = self.base + self.body.len();
        let spans: Vec<&Span> = spans.iter().filter(|s| s.range.start < hi && s.range.end > self.base).collect();
        let keep = shallow_spans(&spans);
        for sp in keep.iter().map(|&i| spans[i]) {
            match &sp.kind {
                SpanKind::Strong => {
                    self.apply(&sp.content, |s| s.bold = true);
                    self.outer_markers(&sp.range, &sp.content, Vis::Marker);
                }
                SpanKind::Emphasis => {
                    self.apply(&sp.content, |s| s.italic = true);
                    self.outer_markers(&sp.range, &sp.content, Vis::Marker);
                }
                SpanKind::Strikethrough => {
                    self.apply(&sp.content, |s| s.strike = true);
                    self.outer_markers(&sp.range, &sp.content, Vis::Marker);
                }
                SpanKind::Code => {
                    let cs = t.code_size;
                    self.apply(&sp.range, |s| {
                        s.mono = true;
                        s.size = s.size * cs / t.body_size.max(1.0);
                    });
                    self.outer_markers(&sp.range, &sp.content, Vis::Marker);
                    let c = if sp.content.start < sp.content.end { &sp.content } else { &sp.range };
                    let r = self.rel(c);
                    self.deco(Decoration::InlineBg { range: r, fill: t.inline_code_bg });
                }
                SpanKind::Link { url } => {
                    let col = t.link;
                    self.apply(&sp.range, |s| {
                        s.color = col;
                        s.underline = true;
                    });
                    self.outer_markers(&sp.range, &sp.content, Vis::Marker);
                    let c = if sp.content.start < sp.content.end { &sp.content } else { &sp.range };
                    let r = self.rel(c);
                    self.deco(Decoration::Link { range: r, url: url.clone() });
                }
                SpanKind::Autolink { url } => {
                    let col = t.link;
                    self.apply(&sp.range, |s| {
                        s.color = col;
                        s.underline = true;
                    });
                    self.outer_markers(&sp.range, &sp.content, Vis::Marker);
                    let c = if sp.content.start < sp.content.end { &sp.content } else { &sp.range };
                    let r = self.rel(c);
                    self.deco(Decoration::Link { range: r, url: url.clone() });
                }
                SpanKind::Image { url, alt } => {
                    let muted = t.muted;
                    self.apply(&sp.range, |s| {
                        s.color = muted;
                        s.italic = true;
                    });
                    self.outer_markers(&sp.range, &sp.content, Vis::Marker);
                    let r = self.rel(&sp.range);
                    self.deco(Decoration::Image { range: r, url: url.clone(), alt: alt.clone() });
                }
                SpanKind::InlineMath | SpanKind::DisplayMath => {
                    self.apply(&sp.range, |s| {
                        s.mono = true;
                        s.italic = true;
                    });
                    self.outer_markers(&sp.range, &sp.content, Vis::Marker);
                }
                SpanKind::FootnoteReference { .. } => {
                    let col = t.link;
                    self.apply(&sp.range, |s| {
                        s.color = col;
                        s.size *= 0.75;
                        s.top = true;
                    });
                    if sp.content.start < sp.content.end {
                        self.outer_markers(&sp.range, &sp.content, Vis::Marker);
                    }
                }
                SpanKind::HardBreak => {
                    if self.byte(sp.range.start) == Some(b'\\') {
                        self.vis(&(sp.range.start..sp.range.start + 1), Vis::Marker);
                    }
                }
                SpanKind::Html => {
                    let muted = t.muted;
                    self.apply(&sp.range, |s| s.color = muted);
                }
            }
        }
    }

    fn format(&self, st: &St) -> TextFormat {
        let t = self.theme;
        let source = !self.live;
        let hidden = match st.vis {
            Vis::Hidden => true,
            Vis::Marker => !self.show,
            _ => false,
        };
        if hidden {
            return TextFormat {
                font_id: FontId::proportional(HIDDEN_SIZE),
                color: Color32::TRANSPARENT,
                ..Default::default()
            };
        }
        let size = if source { t.code_size } else { st.size.max(1.0) };
        let mono = source || st.mono;
        let fam_ok = t.font_families && !mono;
        let family = if mono {
            FontFamily::Monospace
        } else if fam_ok && st.bold && st.italic {
            FontFamily::Name(BOLD_ITALIC_FAMILY.into())
        } else if fam_ok && st.bold {
            FontFamily::Name(BOLD_FAMILY.into())
        } else if fam_ok && st.italic {
            FontFamily::Name(ITALIC_FAMILY.into())
        } else {
            FontFamily::Proportional
        };
        let mut color = st.color;
        if st.bold && !fam_ok && color == t.text {
            color = t.heading;
        }
        match st.vis {
            Vis::Ghost if !self.show => color = Color32::TRANSPARENT,
            Vis::Marker | Vis::Ghost | Vis::Dim => color = t.marker,
            _ => {}
        }
        let visible = color != Color32::TRANSPARENT;
        let line = |on: bool| if on && visible { Stroke::new(1.0, color) } else { Stroke::NONE };
        let underline = line(st.underline && !source && st.vis == Vis::Normal);
        // egui draws the underline at the bottom of the glyph's *line* box, so an inflated
        // line height would put it under the next line. Underlined sections therefore use the
        // natural font height, centred in the row — the same baseline as the centred
        // explicit-line-height sections around them.
        let (line_height, valign) = if underline != Stroke::NONE {
            (None, egui::Align::Center)
        } else if st.top {
            (Some(size * st.lh), egui::Align::TOP)
        } else {
            (Some(size * if source { 1.35 } else { st.lh }), egui::Align::BOTTOM)
        };
        TextFormat {
            font_id: FontId::new(size, family),
            line_height,
            color,
            italics: st.italic && !(fam_ok && st.italic),
            underline,
            strikethrough: line(st.strike),
            valign,
            ..Default::default()
        }
    }
}

/// `mdedit_syntax::code_lines`, called only on a block whose ranges are sane (it slices
/// the text directly and would panic on stale or out-of-range blocks).
fn code_lines_checked(text: &str, b: &Block) -> Vec<Range<usize>> {
    let ok = |r: &Range<usize>| {
        r.start <= r.end && r.end <= text.len() && text.is_char_boundary(r.start) && text.is_char_boundary(r.end)
    };
    if !ok(&b.range) || !ok(&b.content) || b.content.start < b.range.start || b.content.end > b.range.end {
        return Vec::new();
    }
    mdedit_syntax::code_lines(text, b)
}

fn space_for(kind: Option<&BlockKind>, body: f32) -> (f32, f32) {
    match kind {
        Some(BlockKind::Heading { level }) if *level <= 2 => (body * 0.6, body * 0.3),
        Some(BlockKind::Heading { .. }) => (body * 0.4, body * 0.2),
        Some(BlockKind::CodeBlock(_) | BlockKind::FrontMatter | BlockKind::MathBlock) => {
            (body * 0.25, body * 0.25)
        }
        _ => (0.0, 0.0),
    }
}

/// Build the layout job + decorations for one chunk. Must be pure (the editor caches the
/// result keyed by chunk text hash, focused, mode, theme name, wrap width). STABLE.
///
/// Cost is O(chunk length + blocks overlapping the chunk); blocks are located by binary search.
pub fn style_chunk(input: &StyleInput<'_>) -> StyledChunk {
    let text = input.text;
    let theme = input.theme;
    let start = floor_boundary(text, input.chunk.range.start);
    let end = ceil_boundary(text, input.chunk.range.end).max(start);
    let src = &text[start..end];
    let body = src.strip_suffix('\n').unwrap_or(src);
    let live = input.mode == Mode::LivePreview;

    let base_st = St {
        size: theme.body_size,
        mono: false,
        color: theme.text,
        bold: false,
        italic: false,
        strike: false,
        underline: false,
        top: false,
        lh: theme.line_height,
        vis: Vis::Normal,
    };
    let mut ctx = Ctx {
        text,
        body,
        base: start,
        theme,
        hl: input.highlighter,
        live,
        show: input.focused || !live,
        paint: Paint::new(body.len(), base_st),
        decos: Vec::new(),
        hl_volatile: false,
    };

    // Top-level blocks overlapping the chunk (blocks are sorted and disjoint).
    let blocks = input.blocks;
    let first = blocks.partition_point(|b| b.range.end <= start && b.range.start < start);
    let mut primary: Option<&BlockKind> = input.chunk.block.and_then(|i| blocks.get(i)).map(|b| &b.kind);
    for b in blocks[first.min(blocks.len())..].iter() {
        if b.range.start >= end && !(b.range.start == start && end == start) {
            break;
        }
        if b.range.end <= start && b.range.start < start {
            continue;
        }
        if primary.is_none() {
            primary = Some(&b.kind);
        }
        ctx.style_block(b, Depth::default());
    }

    // `\r` of CRLF line endings is never drawn.
    for (i, _) in body.match_indices('\r') {
        if matches!(body.as_bytes().get(i + 1), Some(b'\n') | None) {
            ctx.paint.apply(i..i + 1, |s| s.vis = Vis::Hidden);
        }
    }

    let mut job = LayoutJob::default();
    job.wrap.max_width = input.wrap_width;
    let ids = &ctx.paint.ids;
    let mut i = 0;
    if body.is_empty() {
        job.append("", 0.0, ctx.format(&base_st));
    }
    while i < body.len() {
        let id = ids[i];
        let mut j = i + 1;
        while j < body.len() && (ids[j] == id || !body.is_char_boundary(j)) {
            j += 1;
        }
        let fmt = ctx.format(&ctx.paint.table[id as usize]);
        job.append(&body[i..j], 0.0, fmt);
        i = j;
    }
    let (space_before, space_after) =
        if live { space_for(primary, theme.body_size) } else { (0.0, 0.0) };
    StyledChunk { job, decorations: ctx.decos, space_before, space_after, indent: 0.0, hl_volatile: ctx.hl_volatile }
}

#[cfg(test)]
mod tests;

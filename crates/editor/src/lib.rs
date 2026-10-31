//! The editor widget: draws a [`Document`] with live preview or source mode, handles
//! keyboard, mouse, IME, clipboard, scrolling, and runs editing commands.
//!
//! Built on the chunk contract in `mdedit_style` (one galley per chunk, galley chars map
//! 1:1 to source chars). Paints only visible chunks; caches styled/laid-out chunks.
//!
//! ## How it works
//!
//! - **Model** (rebuilt only when `buffer.version()` changes): text, `mdedit_syntax::parse`,
//!   `mdedit_style::chunks` → display chunks (oversized chunks split; a virtual empty chunk
//!   after a final `\n`), word count.
//! - **Heights**: a Fenwick tree of per-chunk heights. Unknown chunks use an estimate (line
//!   count × row height); heights are refined when a chunk is laid out. Heights of chunks
//!   untouched by an edit are kept.
//! - **Scrolling** is anchored to a chunk (`Anchor { byte, dy }`), so refining estimated
//!   heights above the viewport never makes the view jump. The egui `ScrollArea` offset is
//!   derived from the anchor each frame; wheel/scrollbar changes re-derive the anchor.
//! - **Layout cache**: key = (chunk text hash, focused, mode, theme, wrap width, ppp) →
//!   `Arc<Galley>` + decorations. Only chunks in the viewport (+ overscan) are laid out.
//!
//! OWNER: agent "editor". STABLE items are used by the app.

use std::collections::HashMap;
use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

use egui::text::LayoutJob;
use egui::{Color32, Event, Galley, Id, ImeEvent, Key, Modifiers, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2, pos2, vec2};
use mdedit_buffer::{Document, Edit, EditKind, Selection};
use mdedit_highlight::Highlighter;
use mdedit_style::{Decoration, Mode, Theme};
use mdedit_syntax::commands::{self, CommandResult, Sel};
pub use mdedit_syntax::commands::Command;

mod geom;
mod images;
mod model;

use model::{Heights, Model, byte_in_chunk, char_in_chunk, chunk_at};

/// Space above the first chunk.
const TOP_PAD: f32 = 24.0;
/// Minimum horizontal margin around the text column.
const SIDE_PAD: f32 = 24.0;
/// Extra layout above/below the viewport.
const OVERSCAN: f32 = 300.0;
/// Vertical padding of code cards around their rows.
const CARD_PAD_V: f32 = 10.0;
/// Horizontal padding of code cards (CB-20).
const CARD_PAD_H: f32 = 16.0;
static NEXT_SCROLL_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Corner radius of code cards.
const CARD_RADIUS: f32 = 12.0;
/// Max height of an inline image preview.
const IMAGE_MAX_H: f32 = 400.0;
/// Placeholder height for images that are loading or failed.
const IMAGE_PLACEHOLDER_H: f32 = 24.0;
/// Documents above this size recount words only after typing pauses.
const BIG_DOC: usize = 512 * 1024;

/// Per-document view state. STABLE API.
pub struct EditorState {
    pub mode: Mode,
    /// Search matches to paint (set by the app's find bar), and the current one.
    pub search_matches: Vec<Range<usize>>,
    pub search_current: Option<usize>,
    /// Typewriter mode: keep the caret line vertically centred.
    pub typewriter: bool,
    /// Focus mode: dim all chunks except the caret's.
    pub focus_mode: bool,
    /// Request focus on next frame.
    pub request_focus: bool,
    /// Load `http(s)://` images (off by default: no remote fetches unless enabled). Added API.
    pub remote_images: bool,

    model: Model,
    heights: Heights,
    cache: HashMap<LayoutKey, CacheEntry>,
    frame: u64,
    theme_gen: u64,
    last_theme: Option<Theme>,
    anchor: Anchor,
    view_h: f32,
    col_left: f32,
    col_w: f32,
    est_row_h: f32,
    est_cpr: f32,
    desired_x: Option<f32>,
    scroll_req: Option<ScrollReq>,
    preedit: Option<String>,
    last_activity: f64,
    drag: Option<Drag>,
    last_click: Option<(f64, Pos2, u8)>,
    copied: Option<(usize, f64)>,
    counts_dirty: bool,
    image_guard: std::cell::RefCell<images::ImageGuard>,
    stats: FrameStats,
    /// Identifies this editor's scroll area: egui keeps a scroll area's state (including the
    /// content size it clamps to) by id, so each tab needs its own.
    scroll_id: u64,
    /// Font atlas fill ratio and text options after the last `show`: a lower ratio or other
    /// options mean egui rebuilt the atlas.
    atlas_fill: f32,
    text_options: Option<egui::epaint::text::TextOptions>,
}

/// Debug counters for the last frame (tests, profiling). Added API.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameStats {
    /// Galleys laid out (cache misses) during the last `show`.
    pub layouts: usize,
    /// Chunks painted during the last `show`.
    pub painted_chunks: usize,
    /// Total display chunks in the document.
    pub chunks: usize,
}

impl Default for EditorState {
    fn default() -> Self {
        Self::new()
    }
}

/// What happened this frame. STABLE.
#[derive(Clone, Debug, Default)]
pub struct EditorOutput {
    /// The text changed this frame.
    pub changed: bool,
    /// 1-based line and column of the caret.
    pub line: usize,
    pub column: usize,
    /// (words, chars) of the document, recomputed when text changes.
    pub words: usize,
    pub chars: usize,
    /// A link the user Ctrl+clicked (raw URL as written); the app decides how to open it.
    pub open_link: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct LayoutKey {
    hash: u64,
    len: usize,
    block: bool,
    focused: bool,
    mode: Mode,
    theme: u64,
    width: u32,
    ppp: u32,
}

struct CacheEntry {
    laid: Arc<Laid>,
    used: u64,
}

/// A styled + laid-out chunk.
struct Laid {
    galley: Arc<Galley>,
    decorations: Vec<Decoration>,
    space_before: f32,
    space_after: f32,
    indent: f32,
    /// Height of the code card's header row (language + copy), reserved above the code.
    header_h: f32,
    /// Lay out again once the highlighter's generation differs from `hl_gen`: the key only
    /// covers the chunk's text, and these colours can still change without it.
    hl_volatile: bool,
    hl_gen: u64,
    /// Left offset of a code card nested in a list item or quote (from the text column).
    card_x: f32,
    /// Left offset of the outermost quote bar (a quote nested in a list item).
    quote_x: f32,
}

/// Scroll position relative to a chunk: the viewport top is `dy` below the top of the chunk
/// containing byte `byte`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Anchor {
    byte: usize,
    dy: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScrollReq {
    /// Keep the caret visible (or centred in typewriter mode).
    Caret,
    /// Put byte near the top.
    Top(usize),
    /// Make byte visible, centring it if it isn't.
    Reveal(usize),
}

#[derive(Clone, Debug)]
struct Drag {
    /// 1 = chars, 2 = words, 3 = lines.
    unit: u8,
    /// The originally clicked unit (word / line range).
    origin: Range<usize>,
}

/// Image reserved below a galley row.
#[derive(Clone, Debug)]
struct ImageSlot {
    row: usize,
    extra: f32,
    size: Vec2,
    texture: Option<egui::load::SizedTexture>,
    label: String,
}

/// Per-frame geometry of a chunk.
#[derive(Clone)]
struct ChunkGeom {
    laid: Arc<Laid>,
    images: Vec<ImageSlot>,
    height: f32,
}

impl ChunkGeom {
    /// Vertical shift of galley `row` caused by images reserved above it.
    fn shift(&self, row: usize) -> f32 {
        self.images.iter().filter(|s| s.row < row).map(|s| s.extra).sum()
    }
    fn extra_after(&self, row: usize) -> f32 {
        self.images.iter().filter(|s| s.row == row).map(|s| s.extra).sum()
    }
}

/// A chunk laid out this frame, at content y `y`.
struct Vis {
    i: usize,
    y: f32,
    g: ChunkGeom,
}

struct Env<'a> {
    ctx: egui::Context,
    theme: &'a Theme,
    hl: &'a Highlighter,
    base_dir: Option<&'a Path>,
}

/// Decoration resolved into content coordinates.
enum DecoGeom {
    Card { rect: Rect, fill: Color32, stroke: Color32 },
    Header { card: Rect, lang: String, code: Range<usize>, button: Rect },
    InlineBg { rects: Vec<Rect>, fill: Color32 },
    Bar { rect: Rect, color: Color32 },
    Rule { y: f32, x0: f32, x1: f32, color: Color32 },
    Checkbox { rect: Rect, checked: bool, at: usize },
    Bullet { center: Pos2, r: f32, depth: u8 },
    /// Ordered-list number, right-aligned at `pos`.
    Number { pos: Pos2, text: String },
    Image { rect: Rect, slot: ImageSlot },
    Lines { lines: Vec<[Pos2; 2]>, color: Color32 },
    Link { rects: Vec<Rect>, url: String },
}

fn to_sel(s: Selection) -> Sel {
    Sel { anchor: s.anchor, head: s.head }
}

impl EditorState {
    pub fn new() -> Self {
        Self {
            mode: Mode::LivePreview,
            search_matches: Vec::new(),
            search_current: None,
            typewriter: false,
            focus_mode: false,
            request_focus: true,
            remote_images: false,
            model: Model::default(),
            heights: Heights::default(),
            cache: HashMap::new(),
            frame: 0,
            theme_gen: 0,
            last_theme: None,
            anchor: Anchor { byte: 0, dy: -TOP_PAD },
            view_h: 600.0,
            col_left: SIDE_PAD,
            col_w: 700.0,
            est_row_h: 24.0,
            est_cpr: 80.0,
            desired_x: None,
            scroll_req: None,
            preedit: None,
            last_activity: 0.0,
            drag: None,
            last_click: None,
            copied: None,
            counts_dirty: false,
            image_guard: Default::default(),
            stats: FrameStats::default(),
            scroll_id: NEXT_SCROLL_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            atlas_fill: 0.0,
            text_options: None,
        }
    }

    /// Counters from the last `show` call. Added API (tests/profiling).
    pub fn frame_stats(&self) -> FrameStats {
        self.stats
    }

    /// Draw and interact. Fills the available rect of `ui` (it creates its own scroll area).
    /// `base_dir` resolves relative image paths. STABLE.
    pub fn show(&mut self, ui: &mut egui::Ui, doc: &mut Document, theme: &Theme, highlighter: &Highlighter, base_dir: Option<&std::path::Path>) -> EditorOutput {
        self.frame += 1;
        self.stats = FrameStats::default();
        let start_version = doc.buffer.version();
        let mut out = EditorOutput::default();

        // egui rebuilds the font atlas when it gets full, and when its text options change
        // (dark and light mode use different ones). Cached galleys then point at the wrong
        // glyphs and draw as gibberish or nothing, so drop them.
        let (fill, opts) = ui.ctx().fonts(|f| (f.font_atlas_fill_ratio(), *f.options()));
        if fill < self.atlas_fill || self.text_options.is_some_and(|o| o != opts) {
            self.cache.clear();
        }
        if self.last_theme.as_ref() != Some(theme) {
            self.theme_gen += 1;
            self.last_theme = Some(theme.clone());
        }
        let avail = ui.available_rect_before_wrap();
        let max_w = if theme.max_width > 0.0 { theme.max_width } else { f32::INFINITY };
        self.col_w = (avail.width() - 2.0 * SIDE_PAD).min(max_w).max(40.0);
        self.col_left = ((avail.width() - self.col_w) / 2.0).max(4.0);
        self.est_row_h = (theme.body_size * theme.line_height.max(1.0)).max(4.0);
        self.est_cpr = self.col_w / (theme.body_size * 0.5).max(1.0);
        if avail.height() > 0.0 && self.frame == 1 {
            self.view_h = avail.height();
        }

        self.sync(doc);
        doc.buffer.selection = doc.buffer.clamp_selection(doc.buffer.selection);
        if !highlighter.has_waker() {
            let ctx = ui.ctx().clone();
            highlighter.set_waker(move || ctx.request_repaint());
        }

        let env = Env { ctx: ui.ctx().clone(), theme, hl: highlighter, base_dir };
        let id = ui.scope_id().with("mdedit_editor");
        if self.request_focus {
            ui.memory_mut(|m| m.request_focus(id));
            self.request_focus = false;
        }
        let has_focus = ui.memory(|m| m.has_focus(id));
        if has_focus {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(id, egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: false })
            });
            self.handle_events(ui, &env, doc, &mut out);
            self.sync(doc);
        } else {
            self.preedit = None;
        }

        self.resolve_scroll(&env, doc);
        let offset = self.anchor_offset();
        let scroll = egui::ScrollArea::vertical()
            .id_salt(id.with("scroll").with(self.scroll_id))
            .auto_shrink([false, false])
            .scroll_source(egui::containers::scroll_area::ScrollSource {
                drag: egui::containers::scroll_area::DragScroll::Never,
                ..Default::default()
            })
            .vertical_scroll_offset(offset)
            .show_viewport(ui, |ui, viewport| self.viewport_ui(ui, viewport, &env, doc, id, has_focus, &mut out));
        let new_off = scroll.state.clamped_offset().y;
        self.view_h = scroll.inner_rect.height().max(1.0);
        // Any change came from the wheel or scrollbar. Touchpads send sub-pixel deltas every
        // frame, so even tiny moves must re-anchor or they are lost.
        if new_off != offset {
            self.anchor = self.anchor_from_offset(new_off);
        }

        // Evict layouts not used recently.
        if self.cache.len() > 300 {
            let frame = self.frame;
            self.cache.retain(|_, e| frame.saturating_sub(e.used) <= 60);
        }

        self.sync(doc);
        if self.counts_dirty {
            let idle = ui.input(|i| i.time) - self.last_activity;
            if idle >= 0.7 {
                self.model.all_blocks();
                self.model.counts = mdedit_syntax::word_count(&self.model.text, &self.model.blocks);
                self.counts_dirty = false;
            } else {
                ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64((0.75 - idle).max(0.05)));
            }
        }
        let head = doc.buffer.selection.head;
        let (line, col) = doc.buffer.line_col(head);
        out.line = line + 1;
        out.column = col + 1;
        out.words = self.model.counts.0;
        out.chars = self.model.counts.1;
        out.changed = out.changed || doc.buffer.version() != start_version;
        self.stats.chunks = self.model.chunks.len();
        (self.atlas_fill, self.text_options) = ui.ctx().fonts(|f| (f.font_atlas_fill_ratio(), Some(*f.options())));
        out
    }

    /// Run a formatting command at the current selection (menus, toolbar). STABLE.
    pub fn run_command(&mut self, doc: &mut Document, cmd: &Command) -> bool {
        self.sync(doc);
        let sel = doc.buffer.clamp_selection(doc.buffer.selection);
        self.model.all_blocks();
        match commands::run(&self.model.text, &self.model.blocks, to_sel(sel), cmd) {
            Some(r) => {
                self.apply_result(doc, r, EditKind::Other);
                true
            }
            None => false,
        }
    }

    /// Scroll so byte `at` is visible near the top and put the caret there. STABLE.
    pub fn scroll_to(&mut self, doc: &mut Document, at: usize) {
        let at = doc.buffer.clamp_offset(at);
        doc.buffer.selection = Selection::caret(at);
        doc.buffer.break_undo_group();
        self.desired_x = None;
        self.scroll_req = Some(ScrollReq::Top(at));
    }

    /// Select `range` and scroll to it (find next). STABLE.
    pub fn select(&mut self, doc: &mut Document, range: Range<usize>) {
        let s = doc.buffer.clamp_offset(range.start);
        let e = doc.buffer.clamp_offset(range.end.max(range.start));
        doc.buffer.selection = Selection { anchor: s, head: e };
        doc.buffer.break_undo_group();
        self.desired_x = None;
        self.scroll_req = Some(ScrollReq::Reveal(s));
    }

    /// Drop all caches (theme or font change). STABLE.
    pub fn invalidate(&mut self) {
        self.cache.clear();
        self.theme_gen += 1;
    }

    // ------------------------------------------------------------------ model

    /// Rebuild text/blocks/chunks/heights if the buffer changed.
    fn sync(&mut self, doc: &Document) {
        let key = (doc.buffer.version(), doc.buffer.len_bytes());
        if self.model.version == Some(key) {
            return;
        }
        let t0 = std::time::Instant::now();
        let had = self.model.version.is_some();
        let rope = doc.buffer.rope();
        let new_len = rope.len_bytes();
        let old_len = self.model.text.len();
        // The changed byte range. The text is then patched in place instead of copied.
        let (p, s) = if had {
            let chunks_from = |at: usize| {
                let (chunks, start, _, _) = rope.chunks_at_byte(at);
                (chunks, start)
            };
            model::chunk_affixes(&self.model.text, new_len, chunks_from, |i| rope.byte(i))
        } else {
            (0, 0)
        };
        let (old_end, new_end) = (old_len - s, new_len - s);
        let mut outcome = mdedit_syntax::Reparsed::Full;
        if had {
            self.model.prepare_edit(old_end);
            let removed_definition = mdedit_syntax::touches_definition(&self.model.text, p, old_end);
            let inserted = rope.byte_slice(p..new_end).to_string();
            self.model.text.replace_range(p..old_end, &inserted);
            // A link elsewhere can read this definition, and its chunk's text didn't change:
            // drop the layouts, which are keyed by text.
            if removed_definition || mdedit_syntax::touches_definition(&self.model.text, p, new_end) {
                self.theme_gen += 1;
            }
            let change = mdedit_syntax::TextChange {
                start: p,
                old_end,
                new_end,
                old_len,
                removed_definition,
                has_definitions: self.model.has_definitions,
                valid_blocks: self.model.valid_blocks,
            };
            outcome = mdedit_syntax::reparse_change(&mut self.model.blocks, &self.model.text, &change);
        } else {
            self.model.text = doc.buffer.text();
            self.model.blocks = mdedit_syntax::parse(&self.model.text);
        }
        match &outcome {
            mdedit_syntax::Reparsed::Unchanged => {}
            mdedit_syntax::Reparsed::Full => self.model.blocks_parsed(),
            mdedit_syntax::Reparsed::Spliced { old, new_len: n } => {
                self.model.valid_blocks = self.model.valid_blocks + n - old.len();
                self.model.owed_shift += new_len as isize - old_len as isize;
                if self.model.valid_blocks == self.model.blocks.len() {
                    self.model.owed_shift = 0;
                }
            }
        }
        let t1 = t0.elapsed();
        let delta = new_len as isize - old_len as isize;
        let splice = match &outcome {
            mdedit_syntax::Reparsed::Spliced { old, new_len: n } if self.model.chunks.len() == self.heights.len() => {
                model::ChunkSplice::plan(&self.model.text, &self.model.blocks, self.model.valid_blocks, &self.model.chunks, old, *n, p, new_end, delta)
            }
            _ => None,
        };
        let new_chunks = if splice.is_none() && outcome != mdedit_syntax::Reparsed::Unchanged {
            self.model.all_blocks();
            Some(model::build_chunks(&self.model.text, &self.model.blocks))
        } else {
            None
        };
        let t2 = t0.elapsed();
        // Word count is deferred for big documents (see `update_counts`).
        if new_len <= BIG_DOC {
            self.model.all_blocks();
            self.model.counts = mdedit_syntax::word_count(&self.model.text, &self.model.blocks);
        }
        self.counts_dirty = new_len > BIG_DOC;

        // Keep the heights of chunks the edit didn't touch.
        let known = had && self.model.chunks.len() == self.heights.len();
        let (text, old_chunks, heights) = (&self.model.text, &self.model.chunks, &self.heights);
        let (row_h, cpr) = (self.est_row_h, self.est_cpr);
        let height = |c: &model::DChunk| {
            let mut h = None;
            if known {
                let old_range = if c.range.end <= p {
                    Some(c.range.clone())
                } else if c.range.start >= new_len - s.min(new_len) && c.range.start + old_len >= new_len {
                    let st = c.range.start + old_len - new_len;
                    Some(st..st + c.range.len())
                } else {
                    None
                };
                if let Some(r) = old_range {
                    let k = chunk_at(old_chunks, r.start);
                    if old_chunks.get(k).is_some_and(|oc| oc.range == r) {
                        h = Some(heights.get(k));
                    }
                }
            }
            h.unwrap_or_else(|| f64::from(model::estimate_height(&text[c.range.clone()], row_h, cpr) + row_h * 0.4))
        };
        if let Some(sp) = splice {
            let vals: Vec<f64> = sp.new.iter().map(height).collect();
            self.heights.splice(sp.old.clone(), vals);
            sp.apply(&mut self.model.chunks, &mut self.model.hashes);
        } else if let Some(chunks) = new_chunks {
            let vals: Vec<f64> = chunks.iter().map(height).collect();
            self.heights = Heights::new(vals);
            self.model.hashes = vec![None; chunks.len()];
            self.model.chunks = chunks;
        }
        // Keep the scroll anchor on the same text when bytes before it changed.
        if had && self.anchor.byte >= old_len.saturating_sub(s) && self.anchor.byte >= p {
            self.anchor.byte = (self.anchor.byte + new_len).saturating_sub(old_len);
        }
        self.anchor.byte = model::floor_boundary(&self.model.text, self.anchor.byte);
        self.model.version = Some(key);
        log::debug!("model updated in {:?} [text+parse {t1:?}, chunks {t2:?}] ({} bytes, {} chunks)", t0.elapsed(), self.model.text.len(), self.model.chunks.len());
    }

    fn chunk_y(&self, i: usize) -> f32 {
        TOP_PAD + self.heights.prefix(i) as f32
    }

    fn content_height(&self) -> f32 {
        let bottom = if self.typewriter { self.view_h * 0.5 } else { (self.view_h * 0.3).max(40.0) };
        TOP_PAD + self.heights.total() as f32 + bottom
    }

    fn anchor_offset(&self) -> f32 {
        let i = chunk_at(&self.model.chunks, self.anchor.byte);
        let off = self.chunk_y(i) + self.anchor.dy;
        let max = (self.content_height() - self.view_h).max(0.0);
        off.clamp(0.0, max)
    }

    fn anchor_from_offset(&self, off: f32) -> Anchor {
        let i = self.heights.find(f64::from(off - TOP_PAD));
        let byte = self.model.chunks.get(i).map_or(0, |c| c.range.start);
        Anchor { byte, dy: off - self.chunk_y(i) }
    }

    /// Chunk index range touched by the selection (markers shown there in live preview).
    fn focus_range(&self, sel: Selection) -> (usize, usize) {
        let r = sel.range();
        let a = chunk_at(&self.model.chunks, r.start);
        let b = if r.is_empty() { a } else { chunk_at(&self.model.chunks, r.end - 1).max(a) };
        (a, b)
    }

    fn layout_key(&mut self, env: &Env<'_>, i: usize, focused: bool) -> LayoutKey {
        let hash = self.model.chunk_hash(i);
        let c = &self.model.chunks[i];
        LayoutKey {
            hash,
            len: c.range.len(),
            block: c.block.is_some(),
            focused,
            mode: self.mode,
            theme: self.theme_gen,
            width: self.col_w.to_bits(),
            ppp: env.ctx.pixels_per_point().to_bits(),
        }
    }

    /// Styled and laid-out chunk `i` (cached).
    fn laid(&mut self, env: &Env<'_>, i: usize, focused: bool) -> Arc<Laid> {
        let key = self.layout_key(env, i, focused);
        // Read before styling: a result arriving meanwhile then still triggers a new layout.
        let hl_gen = env.hl.generation();
        if let Some(e) = self.cache.get_mut(&key)
            && !(e.laid.hl_volatile && e.laid.hl_gen != hl_gen)
        {
            e.used = self.frame;
            return e.laid.clone();
        }
        let c = self.model.chunks[i].clone();
        self.model.blocks_through(c.range.end);
        let text = &self.model.text;
        let expected = &text[c.range.start..c.text_end(text)];
        let style_chunk = mdedit_style::Chunk { range: c.range.clone(), block: c.block };
        // Code cards (CB-20): the style draws text at x = 0 and wraps at the given width, so
        // the editor adds the card's horizontal padding and extra vertical room.
        let card = c.block.and_then(|b| self.model.blocks.get(b)).is_some_and(|b| {
            matches!(b.kind, mdedit_syntax::BlockKind::CodeBlock(_) | mdedit_syntax::BlockKind::FrontMatter | mdedit_syntax::BlockKind::MathBlock)
        });
        let nested_code = model::is_nested_code(text, &self.model.blocks, &c);
        let card = card || nested_code;
        let (pad_h, pad_v) = if card { (CARD_PAD_H, CARD_PAD_V) } else { (0.0, 0.0) };
        // Lists (live preview): each item is indented by its depth, the bullet / number sits in
        // the indent. A nested code card lines up with its item's text, or inside the quote.
        let (lists, quotes, outer_lists) = match c.block.and_then(|b| self.model.blocks.get(b)) {
            Some(b) if self.mode == Mode::LivePreview && matches!(b.kind, mdedit_syntax::BlockKind::List { .. } | mdedit_syntax::BlockKind::BlockQuote) => {
                model::container_depth(text, b, c.range.start)
            }
            _ => (0, 0, 0),
        };
        let body = env.theme.body_size;
        let list_x = |n: u8| if n > 0 { body * (LIST_INDENT + LIST_STEP * f32::from(n - 1)) } else { 0.0 };
        let extra = list_x(lists) + f32::from(quotes) * QUOTE_STEP;
        let card_x = if nested_code { extra } else { 0.0 };
        let styled = mdedit_style::style_chunk(&mdedit_style::StyleInput {
            text,
            blocks: self.model.valid(),
            chunk: &style_chunk,
            mode: self.mode,
            focused,
            theme: env.theme,
            highlighter: env.hl,
            wrap_width: (self.col_w - 2.0 * pad_h).max(10.0),
        });
        let indent = if styled.indent.is_finite() { styled.indent.clamp(0.0, self.col_w * 0.8) } else { 0.0 } + pad_h + extra.min(self.col_w * 0.6);
        let mut job = styled.job;
        let mut decorations = styled.decorations;
        if job.text != expected {
            log::warn!("style_chunk broke the chunk contract (text mismatch); using plain text");
            job = plain_job(expected, env.theme, self.mode, self.col_w);
            decorations.clear();
        }
        let max_wrap = (self.col_w - indent - pad_h).max(10.0);
        if job.wrap.max_width.is_finite() && job.wrap.max_width > max_wrap {
            job.wrap.max_width = max_wrap;
        }
        let table = decorations.iter().find_map(|d| match d {
            Decoration::TableGrid { rows, aligns, .. } => Some((rows.clone(), aligns.clone())),
            _ => None,
        });
        if table.is_some() {
            // A wrapped row would break out of the grid.
            job.wrap.max_width = f32::INFINITY;
        }
        let mut galley = env.ctx.fonts_mut(|f| f.layout_job(job.clone()));
        self.stats.layouts += 1;
        let is_table = table.is_some();
        if let Some((rows, aligns)) = table
            && let Some(aligned) = align_table(&galley, &job, &rows, &aligns)
        {
            galley = env.ctx.fonts_mut(|f| f.layout_job(aligned));
        }
        if focused && extra > 0.0 && !nested_code && !is_table && let Some(hung) = hang_prefixes(&galley, &job) {
            galley = env.ctx.fonts_mut(|f| f.layout_job(hung));
        }
        let fin = |v: f32| if v.is_finite() { v.clamp(0.0, 1000.0) } else { 0.0 };
        let has_header = decorations.iter().any(|d| matches!(d, Decoration::CodeHeader { .. }));
        let header_h = if has_header { (env.theme.code_size * 2.4).max(28.0) } else { 0.0 };
        let laid = Arc::new(Laid {
            galley,
            decorations,
            space_before: fin(styled.space_before) + pad_v + header_h,
            // Code cards with a header get a margin below, like the space above them.
            space_after: fin(styled.space_after) + pad_v + if has_header { 8.0 } else { 0.0 },
            indent,
            header_h,
            hl_volatile: styled.hl_volatile,
            hl_gen,
            card_x: card_x.min(self.col_w * 0.6),
            quote_x: list_x(outer_lists),
        });
        self.cache.insert(key, CacheEntry { laid: laid.clone(), used: self.frame });
        laid
    }

    /// Geometry of chunk `i` given selection `sel`; refines its stored height.
    fn geom(&mut self, env: &Env<'_>, sel: Selection, i: usize) -> ChunkGeom {
        let (a, b) = self.focus_range(sel);
        let focused = i >= a && i <= b;
        let laid = self.laid(env, i, focused);
        let images = self.image_slots(env, i, &laid);
        let extra: f32 = images.iter().map(|s| s.extra).sum();
        let height = laid.space_before + laid.galley.rect.height().max(1.0) + extra + laid.space_after;
        self.heights.set(i, f64::from(height));
        ChunkGeom { laid, images, height }
    }

    /// Free the images this editor loaded that none of `others` uses. For when it closes;
    /// textures painted this frame stay valid, as egui frees them after painting. Added API.
    pub fn release_images<'a>(&self, ctx: &egui::Context, others: impl Iterator<Item = &'a EditorState> + Clone) {
        for uri in self.image_guard.borrow().seen() {
            if !others.clone().any(|o| o.image_guard.borrow().has_seen(uri)) {
                ctx.forget_image(uri);
            }
        }
    }

    fn image_slots(&self, env: &Env<'_>, i: usize, laid: &Laid) -> Vec<ImageSlot> {
        let mut out = Vec::new();
        let c = &self.model.chunks[i];
        for d in &laid.decorations {
            let Decoration::Image { range, url, alt } = d else { continue };
            let ci = char_in_chunk(&self.model.text, c, c.range.start + range.start);
            let row = geom::row_of_char(&laid.galley, ci);
            let max_w = (self.col_w - laid.indent).max(16.0);
            let label = if alt.is_empty() { url.clone() } else { alt.clone() };
            let (size, texture, label) = match images::resolve(url, env.base_dir, self.remote_images) {
                images::ImageSource::Uri(uri) => {
                    // Rasterize vector images at display size (SVG bombs) and cap height.
                    let side = images::max_side(&env.ctx);
                    let hint = egui::SizeHint::Size {
                        width: ((max_w * env.ctx.pixels_per_point()) as u32).clamp(16, side),
                        height: side.min(4096),
                        maintain_aspect_ratio: true,
                    };
                    let checked = self.image_guard.borrow_mut().check_uri(&env.ctx, &uri, side);
                    let checked = checked.and_then(|()| images::check_decoded(&env.ctx, &uri, hint, side));
                    match checked.map(|()| env.ctx.try_load_texture(&uri, egui::TextureOptions::LINEAR, hint)) {
                        Err(why) => (vec2(max_w, IMAGE_PLACEHOLDER_H), None, format!("{label} ({why})")),
                        Ok(r) => match r {
                        Ok(egui::load::TexturePoll::Ready { texture }) => (fit(texture.size, max_w), Some(texture), label),
                        Ok(egui::load::TexturePoll::Pending { size }) => {
                            (size.map_or(vec2(max_w, IMAGE_PLACEHOLDER_H), |s| fit(s, max_w)), None, format!("Loading {label}…"))
                        }
                        Err(_) => (vec2(max_w, IMAGE_PLACEHOLDER_H), None, format!("Image not found: {label}")),
                        },
                    }
                }
                images::ImageSource::RemoteBlocked => (vec2(max_w, IMAGE_PLACEHOLDER_H), None, format!("Remote image blocked: {label}")),
                images::ImageSource::Unresolvable => (vec2(max_w, IMAGE_PLACEHOLDER_H), None, format!("Image: {label}")),
            };
            out.push(ImageSlot { row, extra: size.y + 8.0, size, texture, label });
        }
        out
    }

    /// Content-space origin of chunk `i`'s galley.
    fn galley_origin(&self, i: usize, g: &ChunkGeom) -> Pos2 {
        pos2(self.col_left + g.laid.indent, self.chunk_y(i) + g.laid.space_before)
    }

    /// Caret rect in content coordinates for byte `at`, plus chunk index.
    fn caret_content_rect(&mut self, env: &Env<'_>, sel: Selection, at: usize) -> (Rect, usize) {
        let i = chunk_at(&self.model.chunks, at);
        let g = self.geom(env, sel, i);
        let ci = char_in_chunk(&self.model.text, &self.model.chunks[i], at);
        let (r, row) = geom::caret_rect(&g.laid.galley, ci);
        let o = self.galley_origin(i, &g);
        (r.translate(o.to_vec2() + vec2(0.0, g.shift(row))), i)
    }

    /// Byte offset under content position `pos`.
    fn hit_test(&mut self, env: &Env<'_>, sel: Selection, pos: Pos2) -> usize {
        if self.model.chunks.is_empty() {
            return 0;
        }
        let i = self.heights.find(f64::from(pos.y - TOP_PAD));
        let g = self.geom(env, sel, i);
        let o = self.galley_origin(i, &g);
        let local = pos - o;
        let galley = &g.laid.galley;
        let mut gy = local.y;
        let n = galley.rows.len();
        for (r, row) in galley.rows.iter().enumerate() {
            let sh = g.shift(r);
            let bottom = row.max_y() + sh + g.extra_after(r);
            if local.y < bottom || r + 1 == n {
                gy = (local.y - sh).clamp(row.min_y(), (row.max_y() - 0.01).max(row.min_y()));
                break;
            }
        }
        let cc = galley.cursor_from_pos(vec2(local.x, gy));
        byte_in_chunk(&self.model.text, &self.model.chunks[i], cc.index.0)
    }

    // ------------------------------------------------------------------ scrolling

    fn resolve_scroll(&mut self, env: &Env<'_>, doc: &Document) {
        let Some(req) = self.scroll_req.take() else { return };
        let sel = doc.buffer.selection;
        let margin = (self.est_row_h * 1.5).min(self.view_h * 0.25);
        let at = match req {
            ScrollReq::Caret => sel.head,
            ScrollReq::Top(a) | ScrollReq::Reveal(a) => a,
        };
        let (rect, i) = self.caret_content_rect(env, sel, at);
        let base = self.chunk_y(i);
        let byte = self.model.chunks[i].range.start;
        let top = self.anchor_offset();
        let h = self.view_h;
        let new = match req {
            ScrollReq::Top(_) => Some(rect.min.y - margin),
            ScrollReq::Caret if self.typewriter => Some(rect.center().y - h * 0.5),
            ScrollReq::Caret => {
                if rect.min.y < top + margin.min(rect.height()) {
                    Some(rect.min.y - margin)
                } else if rect.max.y > top + h - margin {
                    Some(rect.max.y - h + margin)
                } else {
                    None
                }
            }
            ScrollReq::Reveal(_) => {
                if rect.min.y < top || rect.max.y > top + h {
                    Some(rect.center().y - h * 0.4)
                } else {
                    None
                }
            }
        };
        if let Some(y) = new {
            self.anchor = Anchor { byte, dy: y.max(0.0) - base };
        }
    }

    // ------------------------------------------------------------------ frame

    #[allow(clippy::too_many_arguments)]
    fn viewport_ui(&mut self, ui: &mut egui::Ui, viewport: Rect, env: &Env<'_>, doc: &mut Document, id: Id, has_focus: bool, out: &mut EditorOutput) {
        let origin = ui.max_rect().min;
        let screen_view = viewport.translate(origin.to_vec2());
        let mut vis = self.layout_visible(env, doc.buffer.selection, viewport);
        let mut decos = self.resolve_decos(env, &vis);

        let resp = ui.interact(screen_view, id, Sense::click_and_drag());
        let before = (doc.buffer.version(), doc.buffer.selection);
        self.handle_mouse(ui, env, doc, &resp, &decos, origin, screen_view, out);
        if (doc.buffer.version(), doc.buffer.selection) != before {
            self.sync(doc);
            vis = self.layout_visible(env, doc.buffer.selection, viewport);
            decos = self.resolve_decos(env, &vis);
        }

        ui.set_min_size(vec2(viewport.width().max(1.0), self.content_height()));
        let has_window_focus = ui.input(|i| i.focused);
        let caret = self.paint(ui, env, doc, &vis, &decos, origin, has_focus, has_window_focus);
        self.stats.painted_chunks = vis.len();

        // Re-anchor on the chunk at the top of the viewport.
        if let Some(v) = vis.iter().find(|v| v.y + v.g.height > viewport.min.y) {
            let byte = self.model.chunks[v.i].range.start;
            self.anchor = Anchor { byte, dy: viewport.min.y - v.y };
        }

        if has_focus {
            let to_global = ui.ctx().layer_transform_to_global(ui.layer_id()).unwrap_or_default();
            let cursor = caret.map_or(Rect::from_min_size(screen_view.min, vec2(1.0, 16.0)), |c| c.translate(origin.to_vec2()));
            ui.output_mut(|o| {
                o.ime = Some(egui::output::IMEOutput {
                    purpose: egui::IMEPurpose::Normal,
                    rect: to_global * screen_view,
                    cursor_rect: to_global * cursor,
                    should_interrupt_composition: false,
                });
            });
            if has_window_focus {
                let now = ui.input(|i| i.time);
                let elapsed = (now - self.last_activity).max(0.0);
                let to_next = 0.5 - (elapsed % 0.5);
                ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(to_next.max(0.01)));
            }
        }
    }

    fn layout_visible(&mut self, env: &Env<'_>, sel: Selection, viewport: Rect) -> Vec<Vis> {
        let mut vis = Vec::new();
        let n = self.model.chunks.len();
        if n == 0 {
            return vis;
        }
        let top = viewport.min.y - OVERSCAN;
        let mut i = self.heights.find(f64::from(top - TOP_PAD));
        let mut y = self.chunk_y(i);
        while i < n {
            let g = self.geom(env, sel, i);
            let h = g.height;
            vis.push(Vis { i, y, g });
            y += h;
            if y > viewport.max.y + OVERSCAN {
                break;
            }
            i += 1;
        }
        vis
    }

    /// Resolve decorations of visible chunks into content-space shapes.
    fn resolve_decos(&self, env: &Env<'_>, vis: &[Vis]) -> Vec<Vec<DecoGeom>> {
        let theme = env.theme;
        let col_l = self.col_left;
        let col_r = self.col_left + self.col_w;
        let text = &self.model.text;
        let mut all = Vec::with_capacity(vis.len());
        for v in vis {
            let c = &self.model.chunks[v.i];
            let galley = &v.g.laid.galley;
            let o = pos2(col_l + v.g.laid.indent, v.y + v.g.laid.space_before);
            let chars = |r: &Range<usize>| -> (usize, usize) {
                let s = char_in_chunk(text, c, c.range.start + r.start);
                let mut e = char_in_chunk(text, c, c.range.start + r.end.max(r.start));
                if c.range.start + r.end > c.text_end(text) {
                    e += 1;
                }
                (s, e)
            };
            let to_c = |row: usize, r: Rect| r.translate(o.to_vec2() + vec2(0.0, v.g.shift(row)));
            let rows_y = |s: usize, e: usize| -> Option<(f32, f32)> {
                let (r0, r1) = geom::rows_span(galley, s, e)?;
                let y0 = galley.rows[r0].min_y() + v.g.shift(r0) + o.y;
                let y1 = galley.rows[r1].max_y() + v.g.shift(r1) + v.g.extra_after(r1) + o.y;
                Some((y0, y1))
            };
            let mut ds = Vec::new();
            for d in &v.g.laid.decorations {
                match d {
                    Decoration::Card { range, fill, stroke } => {
                        let (s, e) = chars(range);
                        if let Some((y0, y1)) = rows_y(s, e) {
                            let top = y0 - CARD_PAD_V - v.g.laid.header_h;
                            let rect = Rect::from_min_max(pos2(col_l + v.g.laid.card_x, top), pos2(col_r, y1 + CARD_PAD_V));
                            ds.push(DecoGeom::Card { rect, fill: *fill, stroke: *stroke });
                        }
                    }
                    Decoration::CodeHeader { range, lang, code } => {
                        let (s, e) = chars(range);
                        if let Some((y0, y1)) = rows_y(s, e) {
                            let hh = v.g.laid.header_h;
                            let card = Rect::from_min_max(pos2(col_l + v.g.laid.card_x, y0 - CARD_PAD_V - hh), pos2(col_r, y1 + CARD_PAD_V));
                            // Copy button: a square at the right end of the header row.
                            let side = (hh - 8.0).max(16.0);
                            let button = Rect::from_center_size(pos2(col_r - CARD_PAD_H + 4.0 - side / 2.0, card.min.y + hh / 2.0 + 2.0), vec2(side, side));
                            // A big block is split over several chunks: Copy takes all of it.
                            let whole = commands::block_at(&self.model.blocks, c.range.start).filter(|b| matches!(b.kind, mdedit_syntax::BlockKind::CodeBlock(_)));
                            let code_abs = whole.map_or_else(|| (c.range.start + code.start).min(c.range.end)..(c.range.start + code.end).min(c.range.end), |b| b.content.clone());
                            ds.push(DecoGeom::Header { card, lang: lang.clone(), code: code_abs, button });
                        }
                    }
                    Decoration::InlineBg { range, fill } => {
                        let (s, e) = chars(range);
                        let rects = geom::range_rects(galley, s, e, 0.0)
                            .into_iter()
                            .map(|(row, r)| to_c(row, r).expand2(vec2(2.0, 0.0)))
                            .collect();
                        ds.push(DecoGeom::InlineBg { rects, fill: *fill });
                    }
                    Decoration::QuoteBar { range, color, depth } => {
                        let (s, e) = chars(range);
                        if let Some((y0, y1)) = rows_y(s, e) {
                            // Cover the chunk's padding too, so the bar runs alongside a code
                            // card nested in the quote (its header and padding are space_before).
                            let (y0, y1) = (y0 - v.g.laid.space_before, y1 + v.g.laid.space_after);
                            let x = col_l + v.g.laid.quote_x + f32::from(depth.saturating_sub(1)) * QUOTE_STEP + 2.0;
                            ds.push(DecoGeom::Bar { rect: Rect::from_min_max(pos2(x, y0), pos2(x + 3.0, y1)), color: *color });
                        }
                    }
                    Decoration::Rule { range, color } => {
                        let (s, e) = chars(range);
                        if let Some((r0, _)) = geom::rows_span(galley, s, e) {
                            let row = &galley.rows[r0];
                            let thematic = c
                                .block
                                .and_then(|b| self.model.blocks.get(b))
                                .is_some_and(|b| b.kind == mdedit_syntax::BlockKind::ThematicBreak);
                            let base = o.y + v.g.shift(r0);
                            let y = if thematic { base + (row.min_y() + row.max_y()) / 2.0 } else { base + row.max_y() + 2.0 };
                            ds.push(DecoGeom::Rule { y, x0: col_l, x1: col_r, color: *color });
                        }
                    }
                    Decoration::Checkbox { range, checked } => {
                        let (s, e) = chars(range);
                        if let Some((row, r)) = geom::range_rects(galley, s, e.max(s + 1), 0.0).first().copied() {
                            let r = to_c(row, r);
                            let size = (theme.body_size * 0.85).max(8.0);
                            let x = if r.width() >= size * 0.6 { r.center().x - size / 2.0 } else { r.min.x - size - 4.0 };
                            let rect = Rect::from_min_size(pos2(x.max(col_l - size - 8.0), r.center().y - size / 2.0), Vec2::splat(size));
                            ds.push(DecoGeom::Checkbox { rect, checked: *checked, at: c.range.start + range.start });
                        }
                    }
                    Decoration::Bullet { range, depth } => {
                        let (s, e) = chars(range);
                        if let Some((row, r)) = geom::range_rects(galley, s, e.max(s + 1), 0.0).first().copied() {
                            let r = to_c(row, r);
                            let rad = (theme.body_size * 0.17).max(2.0);
                            // The marker is hidden: the bullet goes in the list indent.
                            let x = r.min.x - theme.body_size * 0.85;
                            ds.push(DecoGeom::Bullet { center: pos2(x, r.center().y), r: rad, depth: *depth });
                        }
                    }
                    Decoration::ListNumber { range, text } => {
                        let (s, e) = chars(range);
                        if let Some((row, r)) = geom::range_rects(galley, s, e.max(s + 1), 0.0).first().copied() {
                            let r = to_c(row, r);
                            ds.push(DecoGeom::Number { pos: pos2(r.min.x - theme.body_size * 0.4, r.center().y), text: text.clone() });
                        }
                    }
                    Decoration::Image { .. } => {}
                    Decoration::TableGrid { rows, color, .. } => {
                        ds.push(DecoGeom::Lines { lines: table_lines(galley, rows, &chars, &to_c), color: *color });
                    }
                    Decoration::Link { range, url } => {
                        let (s, e) = chars(range);
                        let rects = geom::range_rects(galley, s, e, 0.0).into_iter().map(|(row, r)| to_c(row, r)).collect();
                        ds.push(DecoGeom::Link { rects, url: url.clone() });
                    }
                }
            }
            // Images, in slot order.
            for slot in &v.g.images {
                let Some(row) = galley.rows.get(slot.row) else { continue };
                let y = o.y + row.max_y() + v.g.shift(slot.row) + 4.0;
                let rect = Rect::from_min_size(pos2(o.x, y), slot.size);
                ds.push(DecoGeom::Image { rect, slot: slot.clone() });
            }
            all.push(ds);
        }
        all
    }

    // ------------------------------------------------------------------ painting

    #[allow(clippy::too_many_arguments)]
    fn paint(&mut self, ui: &egui::Ui, env: &Env<'_>, doc: &Document, vis: &[Vis], decos: &[Vec<DecoGeom>], origin: Pos2, has_focus: bool, window_focus: bool) -> Option<Rect> {
        let theme = env.theme;
        let painter = ui.painter();
        let off = origin.to_vec2();
        let sel = doc.buffer.selection;
        let srange = sel.range();
        let text = &self.model.text;
        let (fa, fb) = self.focus_range(sel);
        let nl_w = theme.body_size * 0.35;
        let hover = ui.input(|i| i.pointer.hover_pos());
        let now = ui.input(|i| i.time);
        let mut caret_rect = None;

        // Background decorations.
        for (v, ds) in vis.iter().zip(decos) {
            for d in ds {
                match d {
                    DecoGeom::Card { rect, fill, stroke } => {
                        painter.rect(rect.translate(off), CARD_RADIUS, *fill, Stroke::new(1.0, *stroke), StrokeKind::Inside);
                    }
                    DecoGeom::InlineBg { rects, fill } => {
                        for r in rects {
                            painter.rect_filled(r.translate(off), 3.0, *fill);
                        }
                    }
                    DecoGeom::Bar { rect, color } => {
                        painter.rect_filled(rect.translate(off), 1.0, *color);
                    }
                    DecoGeom::Lines { lines, color } => {
                        for l in lines {
                            painter.line_segment([l[0] + off, l[1] + off], Stroke::new(1.0, *color));
                        }
                    }
                    DecoGeom::Rule { y, x0, x1, color } => {
                        painter.line_segment([pos2(*x0, *y) + off, pos2(*x1, *y) + off], Stroke::new(1.0, *color));
                    }
                    _ => {}
                }
            }
            let c = &self.model.chunks[v.i];
            let galley = &v.g.laid.galley;
            let o = self.galley_origin_at(v) + off;
            let paint_ranges = |range: Range<usize>, color: Color32| {
                if let Some(cr) = model::char_range_in_chunk(text, c, range) {
                    for (row, r) in geom::range_rects(galley, cr.start, cr.end, nl_w) {
                        let r = r.translate(o.to_vec2() + vec2(0.0, v.g.shift(row)));
                        painter.rect_filled(r, 0.0, color);
                    }
                }
            };
            // Search matches.
            let first = self.search_matches.partition_point(|m| m.end <= c.range.start);
            for (k, m) in self.search_matches.iter().enumerate().skip(first) {
                if m.start > c.range.end {
                    break;
                }
                let color = if Some(k) == self.search_current { theme.search_current } else { theme.search_match };
                paint_ranges(m.clone(), color);
            }
            // Selection.
            if !srange.is_empty() {
                paint_ranges(srange.clone(), theme.selection);
            }
        }

        // Text.
        for v in vis {
            let galley = &v.g.laid.galley;
            let o = self.galley_origin_at(v) + off;
            if v.g.images.is_empty() {
                painter.galley(o, galley.clone(), theme.text);
            } else {
                // Paint rows in segments separated by reserved image space.
                let mut seg_start = 0usize;
                let mut slots: Vec<usize> = v.g.images.iter().map(|s| s.row).collect();
                slots.sort_unstable();
                slots.dedup();
                slots.push(galley.rows.len().saturating_sub(1));
                for &end_row in &slots {
                    if seg_start >= galley.rows.len() || end_row < seg_start {
                        continue;
                    }
                    let y0 = galley.rows[seg_start].min_y();
                    let y1 = galley.rows[end_row.min(galley.rows.len() - 1)].max_y();
                    let sh = v.g.shift(seg_start);
                    let clip = Rect::from_min_max(pos2(f32::NEG_INFINITY, o.y + y0 + sh), pos2(f32::INFINITY, o.y + y1 + sh + 0.5))
                        .intersect(painter.clip_rect());
                    painter.with_clip_rect(clip).galley(o + vec2(0.0, sh), galley.clone(), theme.text);
                    seg_start = end_row + 1;
                }
            }
        }

        // Foreground decorations.
        for ds in decos {
            for d in ds {
                match d {
                    DecoGeom::Checkbox { rect, checked, .. } => {
                        let r = rect.translate(off);
                        let hot = hover.is_some_and(|p| r.contains(p));
                        let stroke = Stroke::new(1.5, if hot { theme.link } else { theme.muted });
                        if *checked {
                            painter.rect(r, 3.0, theme.link, Stroke::new(1.0, theme.link), StrokeKind::Inside);
                            let a = pos2(r.min.x + r.width() * 0.22, r.center().y);
                            let b = pos2(r.min.x + r.width() * 0.43, r.max.y - r.height() * 0.25);
                            let cc = pos2(r.max.x - r.width() * 0.2, r.min.y + r.height() * 0.25);
                            painter.line_segment([a, b], Stroke::new(2.0, theme.bg));
                            painter.line_segment([b, cc], Stroke::new(2.0, theme.bg));
                        } else {
                            painter.rect(r, 3.0, theme.bg, stroke, StrokeKind::Inside);
                        }
                    }
                    DecoGeom::Number { pos, text } => {
                        painter.text(*pos + off, egui::Align2::RIGHT_CENTER, text, egui::FontId::proportional(theme.body_size), theme.muted);
                    }
                    DecoGeom::Bullet { center, r, depth } => {
                        let c = *center + off;
                        match depth % 3 {
                            1 => {
                                painter.circle_filled(c, *r, theme.text);
                            }
                            2 => {
                                painter.circle_stroke(c, *r, Stroke::new(1.2, theme.text));
                            }
                            _ => {
                                painter.rect_filled(Rect::from_center_size(c, Vec2::splat(*r * 1.8)), 0.0, theme.text);
                            }
                        }
                    }
                    DecoGeom::Image { rect, slot } => {
                        let r = rect.translate(off);
                        if let Some(t) = &slot.texture {
                            painter.image(t.id, r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                        } else {
                            painter.rect(r, 4.0, theme.code_bg, Stroke::new(1.0, theme.code_border), StrokeKind::Inside);
                            painter.text(
                                r.left_center() + vec2(8.0, 0.0),
                                egui::Align2::LEFT_CENTER,
                                &slot.label,
                                egui::FontId::proportional((theme.body_size * 0.8).max(8.0)),
                                theme.muted,
                            );
                        }
                    }
                    DecoGeom::Header { card, lang, code, button } => {
                        // Header row, like ChatGPT: "</> lang" on the left, copy icon on the right.
                        let b = button.translate(off);
                        let mid_y = b.center().y;
                        let size = (theme.code_size * 0.95).max(9.0);
                        let icon = painter.text(
                            pos2(card.translate(off).min.x + CARD_PAD_H, mid_y),
                            egui::Align2::LEFT_CENTER,
                            "</>",
                            egui::FontId::monospace(size * 0.85),
                            theme.muted,
                        );
                        let label = if lang.is_empty() { "text" } else { lang.as_str() };
                        let family = if theme.font_families { egui::FontFamily::Name(mdedit_style::BOLD_FAMILY.into()) } else { egui::FontFamily::Proportional };
                        painter.text(pos2(icon.max.x + 8.0, mid_y), egui::Align2::LEFT_CENTER, label, egui::FontId::new(size, family), theme.text);
                        let hot = hover.is_some_and(|p| b.contains(p));
                        let copied = self.copied.is_some_and(|(at, t)| at == code.start && now - t < 1.5);
                        if hot {
                            painter.rect_filled(b, 6.0, theme.code_border);
                        }
                        if copied {
                            painter.text(pos2(b.max.x, mid_y), egui::Align2::RIGHT_CENTER, "Copied", egui::FontId::proportional(size * 0.9), theme.muted);
                            ui.ctx().request_repaint_after(std::time::Duration::from_millis(300));
                        } else {
                            // Two overlapping rounded squares.
                            let col = if hot { theme.text } else { theme.muted };
                            let stroke = Stroke::new(1.3, col);
                            let u = b.width() * 0.22;
                            let back = Rect::from_min_size(b.center() - vec2(u * 1.1, u * 1.1), vec2(u * 1.5, u * 1.5));
                            let front = back.translate(vec2(u * 0.7, u * 0.7));
                            painter.rect_stroke(back, 2.0, stroke, StrokeKind::Middle);
                            painter.rect_filled(front, 2.0, if hot { theme.code_border } else { theme.code_bg });
                            painter.rect_stroke(front, 2.0, stroke, StrokeKind::Middle);
                        }
                    }
                    _ => {}
                }
            }
        }

        // Focus mode: dim everything but the caret's chunk(s).
        if self.focus_mode {
            let dim = Color32::from_rgba_unmultiplied(theme.bg.r(), theme.bg.g(), theme.bg.b(), 165);
            for v in vis {
                if v.i < fa || v.i > fb {
                    let r = Rect::from_min_max(pos2(self.col_left - SIDE_PAD, v.y), pos2(self.col_left + self.col_w + SIDE_PAD, v.y + v.g.height));
                    painter.rect_filled(r.translate(off), 0.0, dim);
                }
            }
        }

        // Caret + IME preedit.
        let head = sel.head;
        let ci_chunk = chunk_at(&self.model.chunks, head);
        if let Some(v) = vis.iter().find(|v| v.i == ci_chunk) {
            let c = &self.model.chunks[v.i];
            let ci = char_in_chunk(text, c, head);
            let (r, row) = geom::caret_rect(&v.g.laid.galley, ci);
            let r = r.translate(self.galley_origin_at(v).to_vec2() + vec2(0.0, v.g.shift(row)));
            caret_rect = Some(r);
            if has_focus {
                let rs = r.translate(off);
                if let Some(pre) = &self.preedit {
                    let font = egui::FontId::proportional(theme.body_size);
                    let mut job = LayoutJob::default();
                    job.append(
                        pre,
                        0.0,
                        egui::TextFormat { font_id: font, color: theme.text, underline: Stroke::new(1.0, theme.text), ..Default::default() },
                    );
                    let g = ui.ctx().fonts_mut(|f| f.layout_job(job));
                    let pr = Rect::from_min_size(rs.min, g.size().max(vec2(1.0, rs.height())));
                    painter.rect_filled(pr.expand(1.0), 2.0, theme.bg);
                    painter.galley(pr.min, g, theme.text);
                    painter.rect_filled(Rect::from_min_size(pos2(pr.max.x, rs.min.y), vec2(2.0, rs.height())), 0.0, theme.caret);
                } else {
                    let elapsed = (now - self.last_activity).max(0.0);
                    let on = !window_focus || (elapsed % 1.0) < 0.5;
                    if on {
                        painter.rect_filled(Rect::from_min_size(rs.min - vec2(1.0, 0.0), vec2(2.0, rs.height())), 0.0, theme.caret);
                    }
                }
            }
        }
        caret_rect
    }

    fn galley_origin_at(&self, v: &Vis) -> Pos2 {
        pos2(self.col_left + v.g.laid.indent, v.y + v.g.laid.space_before)
    }

    // ------------------------------------------------------------------ mouse

    #[allow(clippy::too_many_arguments)]
    fn handle_mouse(&mut self, ui: &egui::Ui, env: &Env<'_>, doc: &mut Document, resp: &egui::Response, decos: &[Vec<DecoGeom>], origin: Pos2, screen_view: Rect, out: &mut EditorOutput) {
        let off = origin.to_vec2();
        let (pressed, down, pos, mods, now) =
            ui.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_down(), i.pointer.interact_pos(), i.modifiers, i.time));
        let hover_pos = ui.input(|i| i.pointer.hover_pos());

        // Hover feedback.
        if resp.hovered()
            && let Some(p) = hover_pos
        {
            let cp = p - off;
            let clickable = decos.iter().flatten().any(|d| match d {
                DecoGeom::Checkbox { rect, .. } => rect.contains(cp),
                DecoGeom::Header { button, .. } => button.contains(cp),
                DecoGeom::Link { rects, .. } => mods.command && rects.iter().any(|r| r.expand(1.0).contains(cp)),
                _ => false,
            });
            ui.ctx().set_cursor_icon(if clickable { egui::CursorIcon::PointingHand } else { egui::CursorIcon::Text });
        }

        if pressed
            && resp.hovered()
            && let Some(p) = pos
        {
            resp.request_focus();
            let cp = p - off;
            // Clickable decorations first.
            for d in decos.iter().flatten() {
                match d {
                    DecoGeom::Checkbox { rect, at, .. } if rect.expand(2.0).contains(cp) => {
                        doc.buffer.selection = Selection::caret(doc.buffer.clamp_offset(*at));
                        self.run_command(doc, &Command::ToggleTask);
                        self.touch(ui);
                        return;
                    }
                    DecoGeom::Header { button, code, .. } if button.contains(cp) => {
                        let t = self.model.text.get(code.clone()).unwrap_or_default().to_owned();
                        ui.ctx().copy_text(t);
                        self.copied = Some((code.start, now));
                        return;
                    }
                    DecoGeom::Link { rects, url } if mods.command && rects.iter().any(|r| r.expand(1.0).contains(cp)) => {
                        out.open_link = Some(url.clone());
                        return;
                    }
                    _ => {}
                }
            }
            // Click counting (double / triple click).
            let count = match self.last_click {
                Some((t, lp, n)) if now - t < 0.45 && lp.distance(p) < 6.0 => n % 3 + 1,
                _ => 1,
            };
            self.last_click = Some((now, p, count));
            let sel = doc.buffer.selection;
            let at = self.hit_test(env, sel, cp);
            let text = &self.model.text;
            doc.buffer.break_undo_group();
            if mods.shift && count == 1 {
                doc.buffer.selection = Selection { anchor: sel.anchor, head: at };
                self.drag = Some(Drag { unit: 1, origin: sel.anchor..sel.anchor });
            } else {
                let origin = match count {
                    2 => {
                        let w = mdedit_buffer::word_at(text, at);
                        if w.is_empty() { at..model::floor_boundary(text, mdedit_buffer::next_grapheme(text, at)) } else { w }
                    }
                    3 => line_range(text, at),
                    _ => at..at,
                };
                doc.buffer.selection = Selection { anchor: origin.start, head: origin.end };
                self.drag = Some(Drag { unit: count, origin });
            }
            self.desired_x = None;
            self.touch(ui);
            return;
        }

        if let Some(drag) = self.drag.clone() {
            if !down {
                self.drag = None;
                return;
            }
            let Some(p) = pos else { return };
            // Auto-scroll near the edges.
            let edge = 6.0;
            if p.y < screen_view.min.y + edge {
                self.anchor.dy -= ((screen_view.min.y + edge - p.y) * 0.4).clamp(2.0, 60.0);
                ui.ctx().request_repaint();
            } else if p.y > screen_view.max.y - edge {
                self.anchor.dy += ((p.y - screen_view.max.y + edge) * 0.4).clamp(2.0, 60.0);
                ui.ctx().request_repaint();
            }
            let cp = p - off;
            let cp = pos2(cp.x, cp.y.clamp(screen_view.min.y - off.y - 2.0, screen_view.max.y - off.y + 2.0));
            let sel = doc.buffer.selection;
            let at = self.hit_test(env, sel, cp);
            let text = &self.model.text;
            let new = match drag.unit {
                2 | 3 => {
                    let unit = if drag.unit == 2 { mdedit_buffer::word_at(text, at) } else { line_range(text, at) };
                    let unit = if unit.is_empty() { at..at } else { unit };
                    if unit.start < drag.origin.start {
                        Selection { anchor: drag.origin.end, head: unit.start }
                    } else {
                        Selection { anchor: drag.origin.start, head: unit.end.max(drag.origin.end) }
                    }
                }
                _ => Selection { anchor: sel.anchor, head: at },
            };
            if new != sel {
                doc.buffer.selection = new;
                self.touch(ui);
            }
        }
    }

    fn touch(&mut self, ui: &egui::Ui) {
        self.last_activity = ui.input(|i| i.time);
    }

    // ------------------------------------------------------------------ keyboard

    fn handle_events(&mut self, ui: &mut egui::Ui, env: &Env<'_>, doc: &mut Document, out: &mut EditorOutput) {
        let events = ui.input(|i| i.events.clone());
        let mut handled_keys = Vec::new();
        for ev in events {
            self.sync(doc);
            doc.buffer.selection = doc.buffer.clamp_selection(doc.buffer.selection);
            let ime_active = self.preedit.is_some();
            match &ev {
                Event::Text(t) => {
                    if ime_active {
                        continue;
                    }
                    let t: String = t.chars().filter(|c| !c.is_control() || *c == '\t').collect();
                    if !t.is_empty() {
                        self.type_text(doc, &t);
                        self.after_change(ui);
                    }
                }
                Event::Paste(t) => {
                    let t = t.replace("\r\n", "\n").replace('\r', "\n");
                    if !t.is_empty() {
                        doc.buffer.insert_at_selection(&t, EditKind::Other);
                        doc.buffer.break_undo_group();
                        self.after_change(ui);
                    }
                }
                Event::Copy => {
                    let r = doc.buffer.selection.range();
                    if !r.is_empty() {
                        ui.ctx().copy_text(doc.buffer.slice(r));
                    }
                }
                Event::Cut => {
                    let r = doc.buffer.selection.range();
                    if !r.is_empty() {
                        ui.ctx().copy_text(doc.buffer.slice(r.clone()));
                        doc.buffer.apply(vec![Edit::delete(r.clone())], Selection::caret(r.start), EditKind::Other);
                        self.after_change(ui);
                    }
                }
                Event::Ime(ime) => match ime {
                    ImeEvent::Preedit { text, .. } => {
                        // Some IMEs (IBus on Wayland) send an empty preedit every frame while idle;
                        // only a real composition may move the caret into view.
                        if text.is_empty() && !ime_active {
                            continue;
                        }
                        self.preedit = if text.is_empty() { None } else { Some(text.clone()) };
                        if self.preedit.is_some() && !doc.buffer.selection.is_empty() {
                            // Composition replaces the selection.
                            let r = doc.buffer.selection.range();
                            doc.buffer.apply(vec![Edit::delete(r.clone())], Selection::caret(r.start), EditKind::Deleting);
                        }
                        self.touch(ui);
                        self.scroll_req = Some(ScrollReq::Caret);
                    }
                    ImeEvent::Commit(t) => {
                        self.preedit = None;
                        let t: String = t.chars().filter(|c| !c.is_control()).collect();
                        if !t.is_empty() {
                            doc.buffer.insert_at_selection(&t, EditKind::Typing);
                            self.after_change(ui);
                        }
                    }
                    ImeEvent::DeleteSurrounding { before_chars, after_chars } => {
                        let text = &self.model.text;
                        let head = doc.buffer.selection.head;
                        let start = text[..head].char_indices().rev().nth(before_chars.saturating_sub(1)).map_or(0, |(b, _)| b);
                        let start = if *before_chars == 0 { head } else { start };
                        let end = text[head..].char_indices().nth(*after_chars).map_or(text.len(), |(b, _)| head + b);
                        if start < end {
                            doc.buffer.apply(vec![Edit::delete(start..end)], Selection::caret(start), EditKind::Deleting);
                            self.after_change(ui);
                        }
                    }
                    #[allow(deprecated)]
                    ImeEvent::Enabled | ImeEvent::Disabled => {}
                },
                Event::Key { key, pressed: true, modifiers, .. } => {
                    if ime_active {
                        continue;
                    }
                    if self.handle_key(ui, env, doc, *key, *modifiers) {
                        handled_keys.push(ev.clone());
                    }
                }
                _ => {}
            }
        }
        if !handled_keys.is_empty() {
            ui.input_mut(|i| i.events.retain(|e| !handled_keys.contains(e)));
        }
        let _ = out;
    }

    fn type_text(&mut self, doc: &mut Document, t: &str) {
        let mut chars = t.chars();
        if let (Some(ch), None) = (chars.next(), chars.next()) {
            let sel = to_sel(doc.buffer.selection);
            self.model.blocks_through(sel.anchor.max(sel.head));
            if let Some(r) = commands::on_char(&self.model.text, self.model.valid(), sel, ch) {
                self.apply_result(doc, r, EditKind::Typing);
                return;
            }
        }
        doc.buffer.insert_at_selection(t, EditKind::Typing);
    }

    fn apply_result(&mut self, doc: &mut Document, r: CommandResult, kind: EditKind) {
        let edits = r.edits.into_iter().map(|e| Edit { range: e.range, insert: e.insert }).collect();
        doc.buffer.apply(edits, Selection { anchor: r.selection.anchor, head: r.selection.head }, kind);
        if kind == EditKind::Other {
            doc.buffer.break_undo_group();
        }
        self.desired_x = None;
        self.scroll_req = Some(ScrollReq::Caret);
    }

    fn after_change(&mut self, ui: &egui::Ui) {
        self.desired_x = None;
        self.scroll_req = Some(ScrollReq::Caret);
        self.touch(ui);
    }

    fn move_to(&mut self, ui: &egui::Ui, doc: &mut Document, head: usize, extend: bool, keep_x: bool) {
        let head = doc.buffer.clamp_offset(head);
        let anchor = if extend { doc.buffer.selection.anchor } else { head };
        doc.buffer.selection = Selection { anchor, head };
        doc.buffer.break_undo_group();
        if !keep_x {
            self.desired_x = None;
        }
        self.scroll_req = Some(ScrollReq::Caret);
        self.touch(ui);
    }

    /// Returns true if the key was handled.
    fn handle_key(&mut self, ui: &egui::Ui, env: &Env<'_>, doc: &mut Document, key: Key, m: Modifiers) -> bool {
        let sel = doc.buffer.selection;
        let ctrl = m.command || m.ctrl;
        let shift = m.shift;
        if m.alt && !ctrl {
            return false;
        }
        let text_len = self.model.text.len();
        match key {
            Key::ArrowLeft | Key::ArrowRight => {
                let left = key == Key::ArrowLeft;
                let text = &self.model.text;
                let head = if !shift && !sel.is_empty() && !ctrl {
                    if left { sel.range().start } else { sel.range().end }
                } else if left {
                    if ctrl { mdedit_buffer::prev_word(text, sel.head) } else { prev_char_boundary(text, sel.head) }
                } else if ctrl {
                    mdedit_buffer::next_word(text, sel.head)
                } else {
                    next_char_boundary(text, sel.head)
                };
                self.move_to(ui, doc, head, shift, false);
            }
            Key::ArrowUp | Key::ArrowDown => {
                if ctrl {
                    return false;
                }
                let up = key == Key::ArrowUp;
                if !shift && !sel.is_empty() && self.desired_x.is_none() {
                    // Collapse first, then move from the selection edge.
                    let edge = if up { sel.range().start } else { sel.range().end };
                    doc.buffer.selection = Selection::caret(edge);
                }
                let head = self.vertical(env, doc, if up { -1 } else { 1 });
                self.move_to(ui, doc, head, shift, true);
            }
            Key::Home | Key::End => {
                let home = key == Key::Home;
                let head = if ctrl {
                    if home { 0 } else { text_len }
                } else {
                    self.row_edge(env, doc, home)
                };
                self.move_to(ui, doc, head, shift, false);
            }
            Key::PageUp | Key::PageDown => {
                let dir = if key == Key::PageUp { -1.0 } else { 1.0 };
                let (rect, _) = self.caret_content_rect(env, sel, sel.head);
                let x = self.desired_x.map_or(rect.min.x, |dx| dx + self.col_left);
                self.desired_x = Some(x - self.col_left);
                let page = (self.view_h - self.est_row_h * 2.0).max(self.est_row_h);
                let target = pos2(x, rect.center().y + dir * page);
                let head = if target.y < TOP_PAD {
                    0
                } else if target.y > TOP_PAD + self.heights.total() as f32 {
                    text_len
                } else {
                    self.hit_test(env, sel, target)
                };
                self.anchor.dy += dir * page;
                self.move_to(ui, doc, head, shift, true);
            }
            Key::Enter => {
                let s = to_sel(sel);
                self.model.blocks_through(sel.range().end);
                let r = if shift { None } else { commands::on_enter(&self.model.text, self.model.valid(), s) };
                match r {
                    Some(r) => self.apply_result(doc, r, EditKind::Other),
                    None => doc.buffer.insert_at_selection("\n", EditKind::Typing),
                }
                self.after_change(ui);
            }
            Key::Tab => {
                if ctrl {
                    return false;
                }
                self.model.blocks_through(sel.range().end);
                match commands::on_tab(&self.model.text, self.model.valid(), to_sel(sel), shift) {
                    Some(r) => self.apply_result(doc, r, EditKind::Other),
                    None if !shift => doc.buffer.insert_at_selection("\t", EditKind::Typing),
                    None => {}
                }
                self.after_change(ui);
            }
            Key::Backspace | Key::Delete => {
                let back = key == Key::Backspace;
                self.model.blocks_through(sel.range().end);
                let text = &self.model.text;
                if back
                    && !ctrl
                    && let Some(r) = commands::on_backspace(text, self.model.valid(), to_sel(sel))
                {
                    self.apply_result(doc, r, EditKind::Other);
                    self.after_change(ui);
                    return true;
                }
                let range = if !sel.is_empty() {
                    sel.range()
                } else if back {
                    let start = if ctrl { mdedit_buffer::prev_word(text, sel.head) } else { mdedit_buffer::prev_grapheme(text, sel.head) };
                    let start = if start >= sel.head { prev_char_boundary(text, sel.head) } else { start };
                    start..sel.head
                } else {
                    let end = if ctrl { mdedit_buffer::next_word(text, sel.head) } else { mdedit_buffer::next_grapheme(text, sel.head) };
                    let end = if end <= sel.head { next_char_boundary(text, sel.head) } else { end };
                    sel.head..end
                };
                if !range.is_empty() {
                    doc.buffer.apply(vec![Edit::delete(range.clone())], Selection::caret(range.start), EditKind::Deleting);
                }
                self.after_change(ui);
            }
            Key::A if ctrl => {
                let r = sel.range();
                self.model.blocks_through(sel.head);
                let code = commands::block_at(self.model.valid(), sel.head).and_then(|b| match &b.kind {
                    mdedit_syntax::BlockKind::CodeBlock(_) => Some(b.content.clone()),
                    _ => None,
                });
                let new = match code {
                    Some(c) if !c.is_empty() && r != c && c.end <= text_len => Selection { anchor: c.start, head: c.end },
                    _ => Selection { anchor: 0, head: text_len },
                };
                doc.buffer.selection = new;
                doc.buffer.break_undo_group();
                self.touch(ui);
            }
            Key::Z if ctrl => {
                if shift {
                    doc.buffer.redo();
                } else {
                    doc.buffer.undo();
                }
                self.after_change(ui);
            }
            Key::Y if ctrl => {
                doc.buffer.redo();
                self.after_change(ui);
            }
            Key::B if ctrl && !shift => {
                self.run_command(doc, &Command::Bold);
                self.touch(ui);
            }
            Key::I if ctrl && !shift => {
                self.run_command(doc, &Command::Italic);
                self.touch(ui);
            }
            Key::K if ctrl => {
                self.run_command(doc, if shift { &Command::CodeBlock } else { &Command::Link });
                self.touch(ui);
            }
            Key::Backtick if ctrl => {
                self.run_command(doc, &Command::InlineCode);
                self.touch(ui);
            }
            Key::Num0 | Key::Num1 | Key::Num2 | Key::Num3 | Key::Num4 | Key::Num5 | Key::Num6 if ctrl && !shift => {
                let level = match key {
                    Key::Num1 => 1,
                    Key::Num2 => 2,
                    Key::Num3 => 3,
                    Key::Num4 => 4,
                    Key::Num5 => 5,
                    Key::Num6 => 6,
                    _ => 0,
                };
                self.run_command(doc, &Command::Heading(level));
                self.touch(ui);
            }
            _ => return false,
        }
        true
    }

    /// New head for moving one visual row up (`dir < 0`) or down.
    fn vertical(&mut self, env: &Env<'_>, doc: &Document, dir: i32) -> usize {
        let sel = doc.buffer.selection;
        let head = sel.head;
        let n = self.model.chunks.len();
        let i = chunk_at(&self.model.chunks, head);
        let g = self.geom(env, sel, i);
        let ci = char_in_chunk(&self.model.text, &self.model.chunks[i], head);
        let (rect, row) = geom::caret_rect(&g.laid.galley, ci);
        let x = *self.desired_x.get_or_insert(rect.min.x + g.laid.indent);
        let rows = g.laid.galley.rows.len();
        let target = if dir < 0 {
            if row > 0 {
                Some((i, Some(row - 1)))
            } else if i > 0 {
                Some((i - 1, None))
            } else {
                None
            }
        } else if row + 1 < rows {
            Some((i, Some(row + 1)))
        } else if i + 1 < n {
            Some((i + 1, Some(0)))
        } else {
            None
        };
        let Some((j, trow)) = target else {
            return if dir < 0 { 0 } else { self.model.text.len() };
        };
        let gj = if j == i { g } else { self.geom(env, Selection::caret(self.model.chunks[j].range.start), j) };
        let galley = &gj.laid.galley;
        let trow = trow.unwrap_or(galley.rows.len().saturating_sub(1));
        let cj = geom::char_at_row_x(galley, trow, x - gj.laid.indent);
        byte_in_chunk(&self.model.text, &self.model.chunks[j], cj)
    }

    /// Start/end of the caret's visual row.
    fn row_edge(&mut self, env: &Env<'_>, doc: &Document, home: bool) -> usize {
        let sel = doc.buffer.selection;
        let i = chunk_at(&self.model.chunks, sel.head);
        let g = self.geom(env, sel, i);
        let c = &self.model.chunks[i];
        let ci = char_in_chunk(&self.model.text, c, sel.head);
        let row = geom::row_of_char(&g.laid.galley, ci);
        let (s, e) = geom::row_bounds(&g.laid.galley, row);
        byte_in_chunk(&self.model.text, c, if home { s } else { e })
    }
}

/// Fit an image size into `max_w` × IMAGE_MAX_H keeping the aspect ratio.
fn fit(size: Vec2, max_w: f32) -> Vec2 {
    let (w, h) = (size.x.max(1.0), size.y.max(1.0));
    let s = (max_w / w).min(IMAGE_MAX_H / h).min(1.0);
    vec2((w * s).max(1.0), (h * s).max(1.0))
}

/// The line around `at` including its `\n` (triple click).
fn line_range(text: &str, at: usize) -> Range<usize> {
    let at = model::floor_boundary(text, at);
    let start = text[..at].rfind('\n').map_or(0, |p| p + 1);
    let end = text[at..].find('\n').map_or(text.len(), |p| at + p + 1);
    start..end
}

fn prev_char_boundary(text: &str, at: usize) -> usize {
    let at = model::floor_boundary(text, at);
    text[..at].char_indices().next_back().map_or(0, |(b, _)| b)
}

fn next_char_boundary(text: &str, at: usize) -> usize {
    let at = model::floor_boundary(text, at);
    text[at..].chars().next().map_or(at, |c| at + c.len_utf8())
}

/// Plain job for a chunk (fallback when styling breaks the contract).
fn plain_job(text: &str, theme: &Theme, mode: Mode, width: f32) -> LayoutJob {
    let font = match mode {
        Mode::Source => egui::FontId::monospace(theme.code_size),
        Mode::LivePreview => egui::FontId::proportional(theme.body_size),
    };
    LayoutJob::simple(text.to_owned(), font, theme.text, width)
}

/// Table grid lines from cell ranges.
fn table_lines(
    galley: &Galley,
    rows: &[Vec<Range<usize>>],
    chars: &dyn Fn(&Range<usize>) -> (usize, usize),
    to_c: &dyn Fn(usize, Rect) -> Rect,
) -> Vec<[Pos2; 2]> {
    let mut row_rects: Vec<Vec<Rect>> = Vec::new();
    for cells in rows {
        let mut rr = Vec::new();
        for cell in cells {
            let (s, e) = chars(cell);
            let r = if e > s {
                geom::range_rects(galley, s, e, 0.0).first().map(|(row, r)| to_c(*row, *r))
            } else {
                let (r, row) = geom::caret_rect(galley, s);
                Some(to_c(row, r))
            };
            if let Some(r) = r {
                rr.push(r);
            }
        }
        if !rr.is_empty() {
            row_rects.push(rr);
        }
    }
    let mut lines = Vec::new();
    if row_rects.is_empty() {
        return lines;
    }
    let pad = CELL_GAP / 2.0;
    let x0 = row_rects.iter().flatten().map(|r| r.min.x).fold(f32::INFINITY, f32::min) - pad;
    let x1 = row_rects.iter().flatten().map(|r| r.max.x).fold(f32::NEG_INFINITY, f32::max) + pad;
    // Column borders: halfway between the widest cell of a column and the next column.
    let ncols = row_rects.iter().map(Vec::len).max().unwrap_or(0);
    let col = |j: usize, f: fn(&Rect) -> f32, fold: fn(f32, f32) -> f32, init: f32| {
        row_rects.iter().filter_map(|rr| rr.get(j)).map(f).fold(init, fold)
    };
    let borders: Vec<f32> = (1..ncols)
        .map(|j| (col(j - 1, |r| r.max.x, f32::max, f32::NEG_INFINITY) + col(j, |r| r.min.x, f32::min, f32::INFINITY)) / 2.0)
        .collect();
    for (k, rr) in row_rects.iter().enumerate() {
        let y0 = rr.iter().map(|r| r.min.y).fold(f32::INFINITY, f32::min) - 2.0;
        let y1 = rr.iter().map(|r| r.max.y).fold(f32::NEG_INFINITY, f32::max) + 2.0;
        if k == 0 {
            lines.push([pos2(x0, y0), pos2(x1, y0)]);
        }
        lines.push([pos2(x0, y1), pos2(x1, y1)]);
        lines.push([pos2(x0, y0), pos2(x0, y1)]);
        lines.push([pos2(x1, y0), pos2(x1, y1)]);
        for &x in borders.iter().take(rr.len().saturating_sub(1)) {
            lines.push([pos2(x, y0), pos2(x, y1)]);
        }
    }
    lines
}

/// List text indent of the first level and of each further level, in body-font sizes.
const LIST_INDENT: f32 = 1.6;
const LIST_STEP: f32 = 1.5;
/// Offset of a code card per enclosing block quote (clears the quote bar).
const QUOTE_STEP: f32 = 18.0;

/// Space between table columns (cell padding on both sides of a border).
const CELL_GAP: f32 = 28.0;

/// Line a live-preview table up in columns: add space before each cell (the section's
/// `leading_space`; the text itself is unchanged) so every cell of a column starts at the same
/// x, honouring the column's alignment. `None` when nothing needs to move.
fn align_table(galley: &Galley, job: &LayoutJob, rows: &[Vec<Range<usize>>], aligns: &[mdedit_syntax::Align]) -> Option<LayoutJob> {
    let text = &job.text;
    let valid = |r: &Range<usize>| r.start <= r.end && r.end <= text.len() && text.is_char_boundary(r.start) && text.is_char_boundary(r.end);
    let x = |b: usize| geom::caret_rect(galley, text[..b].chars().count()).0.min.x;
    let ncols = rows.iter().map(Vec::len).max()?;
    let mut shift = vec![0.0f32; rows.len()];
    let mut inserts: Vec<(usize, f32)> = Vec::new();
    let mut prev_end: Option<f32> = None;
    for j in 0..ncols {
        // (byte, start x after earlier shifts, width) of cell j in each row.
        let cells: Vec<Option<(usize, f32, f32)>> = rows
            .iter()
            .zip(&shift)
            .map(|(cells, sh)| {
                cells.get(j).filter(|c| valid(c)).map(|c| {
                    let x0 = x(c.start);
                    (c.start, x0 + sh, x(c.end) - x0)
                })
            })
            .collect();
        let width = cells.iter().flatten().map(|c| c.2).fold(0.0, f32::max);
        let mut col_x = cells.iter().flatten().map(|c| c.1).fold(f32::NEG_INFINITY, f32::max);
        if let Some(end) = prev_end {
            col_x = col_x.max(end + CELL_GAP);
        }
        if !col_x.is_finite() {
            continue;
        }
        let f = match aligns.get(j) {
            Some(mdedit_syntax::Align::Right) => 1.0,
            Some(mdedit_syntax::Align::Center) => 0.5,
            _ => 0.0,
        };
        for (r, cell) in cells.iter().enumerate() {
            let Some((b, start, w)) = *cell else { continue };
            let d = col_x - start + (width - w) * f;
            if d > 0.25 {
                inserts.push((b, d));
                shift[r] += d;
            }
        }
        prev_end = Some(col_x + width);
    }
    if inserts.is_empty() {
        return None;
    }
    let mut job = job.clone();
    for (at, space) in inserts {
        add_leading_space(&mut job, at, space);
    }
    Some(job)
}

/// While a list item or quote is edited its markers are shown as text. Pull them left into the
/// indent (negative leading space) so the text stays where it is. `None` if there are none.
fn hang_prefixes(galley: &Galley, job: &LayoutJob) -> Option<LayoutJob> {
    let text = &job.text;
    let mut out: Option<LayoutJob> = None;
    let mut ls = 0;
    for line in text.split_inclusive('\n') {
        let n = container_prefix_len(line);
        if n > 0 && n < line.trim_end().len() {
            let chars = text[..ls].chars().count();
            let width = geom::caret_rect(galley, chars + line[..n].chars().count()).0.min.x - geom::caret_rect(galley, chars).0.min.x;
            add_leading_space(out.get_or_insert_with(|| job.clone()), ls, -width);
        }
        ls += line.len();
    }
    out
}

/// Length of a line's indentation, `>` quote markers and list marker with its spaces.
fn container_prefix_len(line: &str) -> usize {
    let is_ws = |c: char| c == ' ' || c == '\t';
    let prefix = line.len() - line.trim_start_matches(|c| is_ws(c) || c == '>').len();
    let rest = &line[prefix..];
    let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    let marker = match rest.as_bytes().get(digits) {
        Some(b'.' | b')') if digits > 0 => digits + 1,
        Some(b'-' | b'*' | b'+') if digits == 0 => 1,
        _ => return prefix,
    };
    let after = &rest[marker..];
    if !after.starts_with(is_ws) {
        return prefix;
    }
    prefix + marker + after.len() - after.trim_start_matches(is_ws).len()
}

/// Add `space` before byte `at`, splitting the section that holds it if needed.
fn add_leading_space(job: &mut LayoutJob, at: usize, space: f32) {
    let at = egui::text::ByteIndex(at);
    let Some(i) = job.sections.iter().position(|s| s.byte_range.start <= at && at < s.byte_range.end) else { return };
    if job.sections[i].byte_range.start == at {
        job.sections[i].leading_space += space;
    } else {
        let mut tail = job.sections[i].clone();
        tail.byte_range.start = at;
        tail.leading_space = space;
        job.sections[i].byte_range.end = at;
        job.sections.insert(i + 1, tail);
    }
}

#[cfg(test)]
mod tests;

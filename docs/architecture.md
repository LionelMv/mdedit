# Architecture

## 1. Big decisions

### 1.1 Why not copy MarkText's architecture

MarkText = Electron (Chromium + Node) + Vue + **Muya**, an editor engine that turns
Markdown into a *block tree* (`packages/muya/src/block/*`: `paragraph`, `codeBlock`,
`table`, `math`, `diagram`, `frontmatter`, ...) and renders it into a
`contenteditable` DOM. The browser does the text layout, IME, selection and painting.
Without a browser we must do those ourselves, so we keep MarkText's **features and UX**
but not its internals.

### 1.2 Why not copy WordCraft's document model

WordCraft is a word processor. Its Markdown importer converts text into a
Word-like paragraph model (`Para` with `Kind::Code`, `Kind::Heading`, ...) and exports
it back. That loses anything the model can't hold (e.g. the code fence's language,
leading tabs; see `wordcraft-markdown-findings.md`). We **do** copy WordCraft's
*runtime stack* (pure Rust, egui/eframe, winit, wgpu/glow, `rfd`, `arboard`) because
that is where its speed comes from.

### 1.3 Our model: "text is truth, render is a projection"

```
 ┌───────────── file.md (UTF-8) ─────────────┐
 │                                           │
 ▼                                           │ save = write rope as-is
Rope buffer (ropey) ──edits──► Undo history  │
 │                                           │
 │ incremental re-parse (changed blocks only)│
 ▼                                           │
Block index: Vec<Block{kind, byte_range, ...}>
 │
 │ per-block layout (cached by content hash + width + theme)
 ▼
Visible-viewport painter (egui) ──► GPU
 ▲
 │ hit-testing: screen point ⇄ byte offset in the rope
Input (keys, IME, mouse, clipboard)
```

- The **rope** holds the exact Markdown. Every edit is a text edit on the rope.
- The **block index** is derived from the rope (never edited directly).
- **Live preview** (Typora/MarkText style): blocks render formatted; the block (or
  inline span) that holds the cursor reveals its Markdown markers (`**`, `#`, ```` ``` ````).
- **Source mode** is the same buffer drawn as plain text with syntax colouring.
  Switching modes is instant because nothing is converted.

Benefits: lossless round-trip, trivial undo (text diffs), simple autosave/crash
recovery, and correct handling of anything we don't render yet (it just shows as text).

## 2. Technology stack (all pure Rust unless noted)

| Concern | Choice | Why / notes |
|---------|--------|-------------|
| Window, events, IME | `eframe` + `winit` (Wayland + X11) | Same as WordCraft (`egui 0.37`). Native IME via winit's `Ime` events. |
| GPU painting | `egui` with `wgpu` backend (fallback `glow`) | Immediate-mode; we paint only visible blocks. |
| Text buffer | `ropey` | O(log n) edits, line/char/byte conversions, cheap snapshots for autosave. |
| Markdown parser | `pulldown-cmark` with `into_offset_iter()` | Fast, CommonMark-compliant, GFM tables/tasklists/strikethrough/footnotes, YAML front matter (`ENABLE_YAML_STYLE_METADATA_BLOCKS`), math (`ENABLE_MATH`), GFM alerts. Gives byte ranges for every event. Alternative: `comrak` (full AST + `sourcepos`) if we need more extensions. |
| Text shaping & layout | Phase 1: egui `LayoutJob` / `Galley`. Phase 2 (if needed): `parley` or `cosmic-text` | Hide behind a `TextLayout` trait so we can swap. Needed for complex scripts/BiDi and font fallback. |
| Fonts | `fontconfig` lookups via `fontdb` / `font-kit` | Use the system `monospace`, `sans-serif`, `serif` aliases; never hard-code "Courier New". |
| Code highlighting | `syntect` (feature `default-fancy`, pure-Rust regex) + `two-face` for extra grammars | Sublime grammars, line-by-line state ⇒ incremental. Alternative: `tree-sitter-highlight`. |
| Images | `image` 0.25, `resvg` for SVG, `egui_extras` loaders | Decode off the UI thread. |
| Math | Spike: `mitex` (LaTeX→Typst) + `typst` render to SVG → `resvg` | Fallback: show the TeX source in a styled box. |
| Mermaid | Optional: call `mmdc` (mermaid-cli) if installed, cache SVG | Fallback: show code. No mature pure-Rust Mermaid renderer yet; re-check later. |
| File dialogs | `rfd` (xdg-desktop-portal) | Works in Flatpak. |
| Clipboard | egui/winit built-in clipboard for text; `arboard` (`wayland-data-control`) for images/HTML | arboard's Wayland path fails on GNOME and falls back to X11 (see §5.0). |
| File watching | `notify` | Detect external changes, sidebar tree refresh. |
| Spell check | `spellbook` (Hunspell-compatible, pure Rust) using `/usr/share/hunspell/*.dic` | Skips code, URLs, math. |
| Config & paths | `serde` + `toml`, `directories` (XDG) | `~/.config/mdedit/config.toml`, `~/.local/state/mdedit/` for recovery. |
| HTML export | `pulldown-cmark::html` + `syntect` classed CSS | Self-contained HTML with inline CSS/images. |
| PDF export | Markdown → Typst markup → `typst` → PDF | Good typography + math for free. Alternative: render our own layout with `krilla` (what WordCraft does). |
| Logging | `log` + `env_logger` | |
| Tests | `insta` snapshots, `proptest`, `egui_kittest` (UI) | CommonMark spec tests for the parser layer. |

> Pin exact versions in `Cargo.lock`; check crates.io for current releases when you start.

## 3. Workspace layout

```
mdedit/
├── Cargo.toml                # [workspace], shared deps, lints (unsafe_code = "forbid")
├── apps/
│   └── mdedit/               # binary: main.rs, CLI args, eframe::run_native
├── crates/
│   ├── buffer/               # Rope wrapper, selections, transactions, undo/redo
│   ├── syntax/               # pulldown-cmark → Block index; incremental re-parse
│   ├── highlight/            # syntect wrapper, per-code-block caches, language aliases
│   ├── layout/               # Block → laid-out lines; TextLayout trait; caches
│   ├── render/               # paint blocks; images, math, diagrams, tables
│   ├── editor/               # the editor widget: input, IME, hit-testing, commands
│   ├── workspace/            # tabs, file tree, recent files, watcher, autosave/recovery
│   ├── export/               # HTML, PDF
│   ├── spell/                # spellbook integration
│   └── ui/                   # app shell: sidebar, toolbar, menus, prefs, themes, dialogs
└── docs/
```

Each crate depends only on crates above it in this list (buffer has no egui
dependency, so it is unit-testable without a window).

## 4. Core data types (sketch)

```rust
// crates/buffer
pub struct Buffer {
    rope: ropey::Rope,
    version: u64,                 // bumps on every edit; caches key off it
    selections: Vec<Selection>,   // byte offsets, anchor + head
    history: History,             // grouped transactions
}

pub struct Edit { pub range: std::ops::Range<usize>, pub insert: String }

// crates/syntax
pub struct Block {
    pub kind: BlockKind,
    pub range: std::ops::Range<usize>,   // byte range in the rope, incl. markers
    pub content: std::ops::Range<usize>, // e.g. code text without the fences
    pub hash: u64,                        // of the source slice, for layout caching
    pub children: Vec<Block>,             // list items, quote contents, table cells
}

pub enum BlockKind {
    Paragraph, Heading { level: u8 }, BlockQuote, ThematicBreak,
    List { ordered: bool, start: u64 }, ListItem { task: Option<bool> },
    CodeBlock(CodeBlockInfo), Table { aligns: Vec<Align> },
    Html, FrontMatter, MathBlock, Footnote { label: String }, Alert(AlertKind),
}

pub struct CodeBlockInfo {
    pub fence: Option<Fence>,    // None = indented code
    pub info: String,            // full info string, verbatim ("rust title=main.rs")
    pub lang: Option<String>,    // first word of info, normalized via alias table
    pub closed: bool,            // false if the fence runs to EOF
}
pub struct Fence { pub ch: char, pub len: usize, pub indent: usize }
```

## 5. Performance strategy (how we match WordCraft)

### 5.0 Measured WordCraft baseline (Flatpak binaries, this machine: 4 cores, 14 GB RAM, GNOME Wayland)

Test files: generated Markdown with headings, prose, lists, a Rust code block and a
table per section (1 MB ≈ 3 350 sections, 10 MB ≈ 33 000). One run per number, so
treat differences under ~20 % as noise.

| Measurement | 0.7.0 | 0.11.0 |
|---|---|---|
| `convert` 1 MB `.md` → `.txt` (parse + model build) | 0.34 s, 125 MB peak | 0.40 s, 169 MB peak |
| `convert` 10 MB | 9.6 s, 1.17 GB peak | 15.9 s, 1.58 GB peak |
| `info` 1 MB (adds full pagination/layout) | 3.5 s, 338 MB peak | 3.9 s, 358 MB peak |
| `info` 10 MB | 132 s, 3.2 GB peak | 227 s, 3.4 GB peak |
| GUI: open 1 MB file, RSS after settling | 455 MB, ~4 s to settle, then idle | 466 MB, ~5 s to settle, then ≈ 0.3 CPU-s/s while untouched |
| GUI: tiny file | 124 MB RSS | 130 MB RSS |
| GUI idle CPU (tiny file, window untouched) | ≈ 0.16 CPU-s per second | ≈ 0.15 CPU-s per second |

What this tells us:

- WordCraft is fast for normal documents but **scales badly**: in 0.11.0, 10× the
  input costs about 40× the parse time and 58× the layout time (0.7.0: 28× and 38×). The cause is that it builds and
  paginates a whole word-processor document up front.
- Our design (rope + incremental block parse + lay out only the visible blocks) should
  beat it on big files by a wide margin. **Targets for mdedit on this machine:**
  1 MB open < 150 ms and < 60 MB RSS; 10 MB open < 1 s and < 250 MB RSS; idle CPU ≈ 0.
- Re-run the same benchmark on mdedit at the end of M3 (scripts below).

```bash
# reproduce: CLI timings
/usr/bin/time -f "%e s, %M KB" wordcraft-cli convert big1m.md big1m.txt
# GUI: launch, sample /proc/<pid>/status VmRSS and /proc/<pid>/stat utime+stime every 0.25 s
```

Clipboard lesson: on GNOME Wayland `arboard` logs *"wayland data control protocol …
not supported by the compositor"* and falls back to the X11 clipboard (Mutter has no
`wlr-data-control`/`ext-data-control`). Use egui/winit's built-in clipboard
(smithay-clipboard on Wayland) for normal copy/paste, and use `arboard` only for extras such as image
paste, with the X11 fallback acceptable.

### 5.1 Techniques

1. **Native code, no JS/DOM.** Cold start target: < 150 ms to first frame on a
   mid-range laptop; 10 MB file opens in < 1 s (see §5.0).
2. **Incremental parse.** After an edit, find the enclosing top-level block(s),
   extend to the nearest stable boundary (blank line outside fences), re-parse only
   that slice with pulldown-cmark, and splice the block list. A fence opened or
   closed by the edit extends the re-parse to the next matching fence or EOF.
3. **Virtualized rendering.** Keep a prefix-sum of block heights (Fenwick tree).
   Paint only blocks intersecting the viewport (+1 screen of overscan). Unknown
   heights use an estimate until laid out.
4. **Caches keyed by (block hash, wrap width, theme, zoom).** Laid-out galleys,
   highlighted code spans, decoded images, rendered math/diagram textures.
5. **Background work.** Image decode, math/mermaid rendering, spell-check and
   highlighting of off-screen code run on a `rayon` pool / worker thread and post
   results back; the UI thread never blocks on I/O.
6. **Repaint only on change.** egui `request_repaint` only on input, cursor blink,
   or finished background work. Idle CPU ≈ 0.
7. **Measure.** `cargo bench` (criterion) on parse/layout of a 1 MB and 10 MB
   corpus; `puffin` profiler integration behind a feature flag.

## 6. Linux specifics

- Wayland and X11 through winit; test on GNOME (Mutter), KDE (KWin) and Sway.
- IME: fcitx5 and IBus via winit `Ime::Preedit/Commit`; draw preedit underlined at the caret.
- Fonts via fontconfig; honour system font scaling and `org.gnome.desktop.interface`
  text-scaling where available (winit gives scale factor; expose a zoom setting too).
- Dark mode: read the XDG portal `org.freedesktop.appearance color-scheme`.
- File chooser and "open with" via xdg-desktop-portal (`rfd`), so Flatpak sandboxing works.

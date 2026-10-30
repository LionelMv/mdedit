# Implementation plan

Ten milestones. Each one ends with something you can run. Rough effort is for one
experienced Rust developer; adjust as needed.

---

## M0 — Project setup (1–2 days)

1. Install toolchain: `rustup default stable` (edition 2024), `rustup component add clippy rustfmt`.
2. Linux build deps (Debian/Ubuntu names):
   `sudo apt install build-essential pkg-config libxkbcommon-dev libwayland-dev libx11-dev libxcursor-dev libxrandr-dev libxi-dev libgl1-mesa-dev libfontconfig1-dev libgtk-3-dev`
   (`libgtk-3-dev` only if you enable `rfd`'s GTK backend; the portal backend doesn't need it.)
   Fedora: `wayland-devel libxkbcommon-devel fontconfig-devel mesa-libGL-devel`.
3. `cargo new --vcs git mdedit` → convert to a workspace with the layout in
   `architecture.md §3`.
4. Workspace lints: `unsafe_code = "forbid"`, clippy `dbg_macro`/`todo = "warn"`.
   `rustfmt.toml`, `clippy.toml`.
5. CI (GitHub Actions/GitLab): `cargo fmt --check`, `cargo clippy -- -D warnings`,
   `cargo test`, build on Ubuntu LTS + Fedora containers.
6. Release profile: `opt-level = 3`, `lto = "thin"`, `codegen-units = 16`, `strip = true`.
   Dev profile: `opt-level = 1` and dependencies at `opt-level = 2` (WordCraft does this,
   keeps debug builds usable).

**Done when:** `cargo run` opens an empty eframe window on Wayland and X11.

---

## M1 — Buffer and plain-text editor (1–2 weeks)

The hardest piece of a non-web editor is the text widget. Build it before Markdown.

1. `buffer` crate: `ropey::Rope` wrapper, byte⇄char⇄(line, col) conversions,
   `apply(Edit)`, multiple selections, grapheme-aware movement (`unicode-segmentation`),
   word movement (`unicode-segmentation` word bounds).
2. Undo/redo: transactions of edits + selection before/after; group typing by time
   (≤ 1 s gap) and by word boundary.
3. `editor` crate: a custom egui widget (don't use `egui::TextEdit`; it re-lays out
   the whole text every frame):
   - layout per line, cache galleys by `(line hash, width)`;
   - virtualized painting of visible lines only;
   - caret, selection painting, blink;
   - mouse: click, drag, double-click word, triple-click line, shift-click;
   - keyboard: arrows, Home/End, PgUp/PgDn, Ctrl+arrows, Ctrl+Backspace/Delete;
   - **IME**: handle `egui::Event::Ime` preedit/commit; set IME cursor rect.
     Test with fcitx5 and IBus (Chinese/Japanese input);
   - clipboard via `arboard` (and egui's copy/cut/paste events).
4. File open/save (`rfd` dialogs), UTF-8 with BOM preservation, line-ending
   detection (LF/CRLF) preserved on save, atomic save (write temp + `rename`).
5. Benchmarks: open a 10 MB file, type at the end, scroll — record frame times.

**Done when:** you can edit a 10 MB plain text file at 60+ fps with IME working.

---

## M2 — Markdown parsing and source mode (1 week)

1. `syntax` crate: run `pulldown-cmark` with `Options::ENABLE_TABLES |
   ENABLE_FOOTNOTES | ENABLE_STRIKETHROUGH | ENABLE_TASKLISTS |
   ENABLE_YAML_STYLE_METADATA_BLOCKS | ENABLE_MATH | ENABLE_GFM |
   ENABLE_HEADING_ATTRIBUTES` and `into_offset_iter()`; build `Vec<Block>` with byte ranges.
2. Code-block info: implement `CodeBlockInfo` per `code-blocks.md` CB-1…CB-11
   (pulldown-cmark gives `CodeBlockKind::Fenced(info)`; derive fence char/len/indent
   from the source slice).
3. Inline spans per block (emphasis, strong, code, links, images, math) with ranges.
4. Incremental re-parse (architecture §5.2). Property test: incremental result ==
   full re-parse after random edits.
5. **Source mode**: colour Markdown syntax in the plain editor using the spans
   (headings bigger/bold, markers dimmed, code monospace). This is already a usable
   Markdown editor.

**Done when:** CommonMark spec tests pass via the block index, and incremental ==
full parse in 10 000 random edit runs.

---

## M3 — Live preview rendering (2–3 weeks)

1. `layout` crate: lay out each block type into a `BlockLayout` (galleys + child
   rects + height). Fonts: body (sans/serif setting), headings scale 2.0/1.5/1.25/1.1/1.0/0.9,
   monospace for code.
2. Hide Markdown markers in blocks without the caret; show them (dimmed) in the
   focused block / inline span (MarkText behaviour).
3. Block renderers: paragraph, headings, block quote (left bar), lists (bullets,
   numbers, nesting), thematic break, **code block card** (CB-20…CB-27),
   front matter (collapsible card), HTML blocks (shown as source, dimmed).
4. Inline renderers: bold/italic/strike, inline code pill (CB-30), links (Ctrl+click
   opens via `xdg-open` / portal), autolinks, hard/soft breaks, emoji shortcodes.
5. **Hit-testing**: map pointer → (block, galley, char) → rope byte offset, and back,
   including over hidden markers. This is the core of WYSIWYG; write many tests.
6. Height cache + Fenwick tree; scroll anchoring so content doesn't jump when
   off-screen heights change.

**Done when:** typing in live preview feels like MarkText on a 1 MB README, with no
frame over 16 ms.

---

## M4 — Code blocks done right (1–2 weeks)

Implement `code-blocks.md` fully:

1. `highlight` crate: `syntect` (`default-fancy`) + `two-face` syntaxes; language
   alias table; per-line state cache; theme mapping to the app theme.
2. Card UI: header with language label, language selector popup (fuzzy search),
   copy button, line numbers toggle.
3. Horizontal scroll inside the card; optional wrap setting; tab stops.
4. Editing behaviours CB-40…CB-49.
5. Exclusion of code from spell check (CB-26, used later in M7).
6. Round-trip property tests (CB-60) and the tricky-input snapshots.

**Done when:** every rule in `code-blocks.md` has a passing test.

---

## M5 — GFM and extensions (2–3 weeks)

1. **Tables**: grid rendering with alignment; Tab/Shift+Tab move between cells;
   Enter adds a row; toolbar for insert/delete row/column, alignment; table-size
   picker for new tables (MarkText "chessboard"). Edits rewrite the table's
   Markdown and re-pad columns (setting).
2. **Task lists**: clickable checkboxes toggling `[ ]`/`[x]` in the rope.
3. **Footnotes**: superscript references, hover preview, jump to definition.
4. **GitHub alerts** (`> [!NOTE]` …) with icons/colours.
5. **Images**: inline display (local paths relative to the file, `http(s)` with an
   opt-in setting, data URIs), async decode, resize handles writing
   `<img width>` or `![](x){width=}` per setting, image toolbar
   (replace/delete/alt text). Paste image from clipboard → save to `./assets/`
   (configurable) and insert a link.
6. **Math**: inline `$…$` and display `$$…$$` via the math renderer (spike first:
   `mitex`→`typst`→SVG→`resvg` texture; fallback styled TeX source).
7. **Mermaid / diagrams**: preview via `mmdc` if installed, cached by hash;
   otherwise code card only.
8. Emoji picker (`:` trigger) using an embedded emoji table.
9. Heading folding.

---

## M6 — Application shell (2 weeks)

1. Tabs (multiple documents), dirty markers, close-confirm dialog.
2. Sidebar: outline/TOC from headings (click to scroll), collapsible. (A folder tree
   and folder search were built and then removed as unnecessary.)
3. Find & replace in document (regex, case, whole word), highlights all matches.
4. Command palette (`Ctrl+Shift+P`) and quick-insert `/` menu (MarkText's
   `paragraphQuickInsertMenu`).
5. Inline format toolbar on selection (bold, italic, code, strike, link, highlight).
6. Paragraph front menu (drag handle ⋮⋮: turn into…, duplicate, delete, move up/down).
7. Modes: **Source**, **Live preview**, **Typewriter** (caret line stays vertically
   centred), **Focus** (dim all but current paragraph); full-screen (F11).
8. Menus + keyboard shortcuts (configurable keymap TOML), matching MarkText's
   defaults where sensible.
9. Status bar: word/char count, reading time, line:col, encoding, line endings.
10. CLI: `mdedit file.md`, single-instance over a private Unix socket so a second
    launch opens a tab in the running window (preference, on by default). The open
    files are reopened at the next start (preference, on by default).

---

## M7 — Preferences, themes, spell check, robustness (1–2 weeks)

1. Preferences window backed by `config.toml` (fonts, sizes, line height, editor
   width, tab width, autosave interval, image folder, code wrap, theme, keymap).
   Live-apply.
2. Themes: light/dark built-ins (e.g. a "Cadmium Light"-like and a "Material
   Dark"-like palette of our own), follow system dark mode via the XDG portal,
   user themes as TOML in `~/.config/mdedit/themes/`.
3. Spell check: `spellbook` with system Hunspell dictionaries, squiggles, context-menu
   suggestions, "add to dictionary"; skip code, URLs, math, front matter.
4. Autosave + crash recovery journal in `~/.local/state/mdedit/recovery/`; restore
   prompt on next launch.
5. External change detection: reload silently if not dirty, else prompt / show diff.
6. Large-file mode: > 20 MB → source mode, no highlighting, warning banner.

---

## M8 — Export (1 week)

1. HTML: standalone file with embedded CSS (theme), highlighted code (CB-62),
   images inlined or copied; optional TOC.
2. PDF: Markdown → Typst source → `typst` compile → PDF (page size, margins,
   header/footer, TOC options). Respect CB-29/CB-63.
3. Copy as HTML / copy as Markdown / copy as plain text.
4. Print via `xdg-open` of the generated PDF (or GTK print portal later).

---

## M9 — Packaging, docs, release (1 week)

See `linux-packaging.md`. Plus: user guide in `docs/user/`, sample document
(like MarkText's), CHANGELOG, versioned releases with AppImage + Flatpak + `.deb`
artifacts built in CI.

---

## Cross-cutting rules

- **Never mutate Markdown the user didn't touch.** Any feature that rewrites
  text (table re-pad, list renumber) only touches the edited block.
- **Every feature gets a round-trip test** (open → save → identical bytes).
- **UI thread budget: 8 ms/frame.** Anything slower moves to a worker.
- **Accessibility:** enable `accesskit` in eframe; keyboard-only operation for all
  commands.
- Keep a `docs/decisions/` folder of short ADRs when you change a technology choice.

## Suggested first week, concretely

1. M0 complete.
2. `buffer` crate with tests for edits/undo.
3. A minimal custom editor widget showing a rope with a caret and arrow-key movement.
4. Open/save a file.

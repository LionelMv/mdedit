# MarkText feature parity checklist

Source: MarkText README and `packages/muya/src` (block types and UI widgets),
`packages/desktop/src/main` (menus, preferences, spellchecker, file system).
"M#" = milestone in `implementation-plan.md`.

## Editing core

| MarkText feature | Where in MarkText | mdedit | M# |
|---|---|---|---|
| Realtime preview (WYSIWYG) | muya `block/*`, `inlineRenderer` | Live preview over rope | M3 |
| Source code mode | desktop `codeMirror` | Source mode (same buffer) | M2 |
| Typewriter mode | prefs | ✓ | M6 |
| Focus mode | prefs | ✓ | M6 |
| Undo/redo | muya `history` | Transaction history | M1 |
| Search & replace | muya `search` | ✓ regex | M6 |
| Inline style shortcuts | muya `ui/shortcuts.ts` | Keymap TOML | M6 |
| Inline format toolbar | `ui/inlineFormatToolbar` | ✓ | M6 |
| Quick insert menu | `ui/paragraphQuickInsertMenu` | `/` menu | M6 |
| Paragraph menu / drag | `ui/paragraphFrontMenu`, `paragraphFrontButton` | Drag handle | M6 |
| Heading fold | `block/folding.ts`, `headingFoldToggle` | ✓ | M5 |
| Copy heading link | `headingCopyLink` | ✓ | M6 |

## Markdown support

| Feature | MarkText | mdedit | M# |
|---|---|---|---|
| CommonMark | ✓ | pulldown-cmark | M2 |
| GFM tables (+ toolbars, drag) | `gfm/table`, `ui/table*` | ✓ | M5 |
| Task lists | `gfm/taskList*` | ✓ | M5 |
| Fenced/indented code + language selector | `commonMark/codeBlock`, `ui/codeBlockLanguageSelector` | ✓ per `code-blocks.md` | M4 |
| Math (KaTeX) | `extra/math` | Typst-based renderer | M5 |
| Diagrams (Mermaid, etc.) | `extra/diagram` | `mmdc` preview | M5 |
| Front matter | `extra/frontmatter` | ✓ | M3 |
| Footnotes | `extra/footnote`, `ui/footnoteTool` | ✓ | M5 |
| GitHub alerts | `block/githubAlerts.ts` | ✓ | M5 |
| Emoji | `ui/emojiSelector` | ✓ | M5 |
| HTML blocks | `commonMark/html` | Shown as source | M3 |
| Images: paste, picker, toolbar, resize | `ui/image*` | ✓ | M5 |
| Links tool | `ui/linkTools` | ✓ | M3/M6 |

## Application

| Feature | MarkText | mdedit | M# |
|---|---|---|---|
| Tabs, multiple windows | desktop `windows` | Tabs (windows later) | M6 |
| File tree sidebar, folder search | renderer sidebar | ✓ | M6 |
| Outline / TOC | ✓ | ✓ | M6 |
| Recent documents | `menu/recentDocuments.ts` | ✓ | M6 |
| Themes (Cadmium Light, Material Dark, …) | ✓ | Own palettes + user TOML | M7 |
| Preferences | `preferences/schema.json` | `config.toml` + window | M7 |
| Spell checker | `main/spellchecker` | spellbook | M7 |
| Autosave | ✓ | + crash recovery journal | M7 |
| Export HTML / PDF | ✓ | ✓ | M8 |
| Paste images from clipboard | ✓ | arboard | M5 |
| i18n | `i18n`, `locales` | `fluent` later | post-1.0 |
| CLI open | `main/cli` | ✓ + D-Bus single instance | M6 |

## Explicitly better than MarkText (goals)

- Native start-up and memory (no Chromium): target < 60 MB RSS for a small doc.
- Byte-exact round-trip of untouched content.
- Large files (10 MB+) stay responsive.
- Code blocks: tabs preserved, highlight with Sublime grammars, no page splits in PDF.

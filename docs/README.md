# mdedit — a native Rust Markdown editor for Linux

> Working name: **mdedit**. Rename freely; every doc uses it as a placeholder.

## Goal

A WYSIWYG ("live preview") Markdown editor with the feature set of
[MarkText](https://github.com/marktext/marktext), but:

- **No Electron, no web view.** One native binary written in Rust.
- **WordCraft-class performance.** Same stack family as
  [WordCraft](https://github.com/storytold/wordcraft) (pure Rust, `egui`/`eframe`
  on `winit`, GPU-painted, starts in milliseconds, no JS runtime).
- **Linux only (for now).** Wayland first, X11 supported, packaged as Flatpak /
  AppImage / `.deb`.
- **Markdown is the source of truth.** The file on disk round-trips byte for byte
  unless the user edits it. This is the main lesson from WordCraft's Markdown
  support (see [wordcraft-markdown-findings.md](wordcraft-markdown-findings.md)).

## Documents

| File | What it covers |
|------|----------------|
| [user-guide.md](user-guide.md) | How to use mdedit: files, tabs, outline, formatting, status bar, preferences, recovery |
| [issues.md](issues.md) | Known gaps and follow-ups to implement later |
| [architecture.md](architecture.md) | Technology choices, crate layout, data flow, performance strategy |
| [implementation-plan.md](implementation-plan.md) | Step-by-step phases (M0–M9), each with tasks and "done when" criteria |
| [feature-parity.md](feature-parity.md) | MarkText feature checklist mapped to the phase that delivers it |
| [code-blocks.md](code-blocks.md) | Full spec for fenced/indented code and inline code: parsing, display, editing, export |
| [wordcraft-markdown-findings.md](wordcraft-markdown-findings.md) | What's wrong with WordCraft's Markdown code blocks, with code references and a draft upstream issue |
| [linux-packaging.md](linux-packaging.md) | Desktop integration, MIME types, Flatpak/AppImage/deb |

## Reading order

1. `architecture.md` — understand the shape of the program.
2. `code-blocks.md` + `wordcraft-markdown-findings.md` — the part most worth getting right.
3. `implementation-plan.md` — then build it phase by phase.

## References studied

- MarkText `0.24.0-dev` (Electron + Vue; its editor engine is `packages/muya`).
- WordCraft `0.11.0`, commit `d522bfd` (2026-10-27), Markdown code in
  `crates/formats/src/markdown/`.

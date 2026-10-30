# Team rules for building mdedit (parallel agents)

Workspace: `/home/lionel/Documents/PROJECTS/markdown_editor`. Rust via `~/.cargo/bin/cargo`
(not on PATH in non-login shells). Read `docs/architecture.md`, `docs/code-blocks.md` and
the crate you own before writing code.

## Ownership — edit ONLY your own files

| Agent | Owns | Main deliverable |
|---|---|---|
| **buffer** | `crates/buffer`, `crates/workspace` | Rope buffer, undo/redo, movement, search, Document load/save, config, recent files, session, recovery, watcher |
| **syntax** | `crates/syntax`, `crates/export` | `parse()` block index with byte ranges, inline spans, outline, word count, editing commands, HTML export |
| **style** | `crates/style`, `crates/highlight` | syntect highlighting; live-preview + source-mode styling of chunks (LayoutJob + decorations); themes |
| **editor** | `crates/editor` | The egui editor widget: chunk layout cache, virtualized painting, caret/selection, keyboard, mouse, IME, clipboard, decorations painting, images, commands |
| **app** | `apps/mdedit` | Application shell: menus, tabs, file dialogs, outline sidebar, find/replace bar, preferences, status bar, export, autosave/recovery, CLI args, dark mode |
| **peer session** (`markdown-editor-a5`) | `packaging/`, `scripts/`, `tests/corpus/`, `benches/` docs | `.desktop`, icon, AppStream, install script, `.deb` via cargo-deb, test corpus, benchmark script |
| **integrator** (main session) | root `Cargo.toml`, `docs/` | Integration, final testing, fixes |

- **Public signatures marked STABLE are a contract.** Implement them; don't rename or
  remove them. You may *add* public items. If you truly need a contract change, make it
  backward compatible, or send the integrator a message saying exactly what you changed.
- Need a new dependency? Add it to **your crate's** `Cargo.toml` with an explicit version
  (don't edit the root `Cargo.toml`; tell the integrator if you think it should be shared).
- Until a dependency crate is implemented, its stubs compile but return empty/defaults.
  Code against the documented behaviour, not the stub. Write your own tests with real inputs.

## Building without fighting

All agents share one `target/` dir; cargo serializes builds with a lock ("Blocking waiting
for file lock" is normal: wait). Prefer narrow commands:

```bash
~/.cargo/bin/cargo test -p mdedit-<crate>          # your tests
~/.cargo/bin/cargo clippy -p mdedit-<crate> --all-targets
~/.cargo/bin/cargo build -p mdedit                  # app (only the app/editor agents need this)
```

Don't run `cargo clean`, don't `cargo update`, don't touch other agents' files, don't
commit (the integrator handles git). Don't launch the GUI unless you're the app or editor
agent; if you do, kill it after (`timeout 20 ...`).

## Quality bar

- `cargo clippy -p <crate> --all-targets` clean, `cargo test -p <crate>` green.
- `#![forbid(unsafe_code)]` is set workspace-wide.
- No panics on any input: no `unwrap()` on user data, clamp offsets, check char boundaries
  (`str::is_char_boundary`). Fuzz-ish `proptest` for parsers and buffer edits.
- Performance: nothing O(n) per frame on a 10 MB document unless cached by version.
- Security: never execute document content; links open only via the app (http/https/mailto
  through `open`, others refused); HTML export sanitized; files with user text mode 0600
  in state dirs; atomic saves; no remote fetches unless the user enabled them.

## Done = report back

When finished, reply with: what works, what's missing/known-buggy, test commands and results,
and any contract changes.

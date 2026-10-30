# mdedit

A fast, native Markdown editor for Linux, written in Rust (egui). Live preview like
MarkText, without Electron. The Markdown text is the source of truth: untouched parts of a
file are saved byte for byte.

## Build, install, run

```bash
./scripts/install.sh            # cargo build --release + install into ~/.local
mdedit [FILE]...                # or launch "mdedit" from the app menu
./scripts/uninstall.sh          # remove (keeps your settings)
./scripts/build-deb.sh          # .deb for Ubuntu 24.04+ (see packaging/deb/README.md)
```

How to use it: [docs/user-guide.md](docs/user-guide.md).

Settings: `~/.config/mdedit/config.toml`. Crash-recovery snapshots and the list of open files:
`~/.local/state/mdedit/`.
Set `MDEDIT_HOME=/some/dir` to keep everything in one folder (used by tests).

## Development

```bash
~/.cargo/bin/cargo test --workspace
~/.cargo/bin/cargo clippy --workspace --all-targets
MDEDIT_HOME=$(mktemp -d) MDEDIT_SHOT_DIR=/tmp/shots cargo test -p mdedit ui_tests -- --ignored   # whole-app checks (GPU)
RUST_LOG=mdedit_editor=debug mdedit big.md     # per-edit model rebuild timings
python3 -I scripts/gen-big.py /tmp/big && scripts/bench.sh ~/.local/bin/mdedit /tmp/big/big1m.md
```

Layout: `crates/{buffer,syntax,highlight,style,editor,workspace,export}` and `apps/mdedit`.
Design docs are in `docs/` (start with `docs/README.md`). Known gaps: `docs/issues.md`.

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.

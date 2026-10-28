# Building a `.deb`

mdedit uses [`cargo-deb`](https://github.com/kornelski/cargo-deb).

## 1. Metadata for `apps/mdedit/Cargo.toml`

Append this block to `apps/mdedit/Cargo.toml` (owned by the app agent; paths are
relative to `apps/mdedit/`):

```toml
[package.metadata.deb]
name = "mdedit"
maintainer = "Lionel <mvangicode@gmail.com>"
copyright = "2026, Lionel <mvangicode@gmail.com>"
license-file = []
extended-description = """\
mdedit is a fast, native Markdown editor with live preview, syntax-highlighted \
code blocks with one-click copy, tabs, a document outline, find/replace, sanitized \
HTML export, and crash recovery for unsaved changes."""
section = "editors"
priority = "optional"
depends = "$auto, libgl1, libegl1, libxkbcommon0, libwayland-client0, libwayland-egl1"
assets = [
    ["target/release/mdedit", "usr/bin/", "755"],
    ["../../packaging/linux/io.github.mdedit.mdedit.desktop", "usr/share/applications/", "644"],
    ["../../packaging/linux/io.github.mdedit.mdedit.metainfo.xml", "usr/share/metainfo/", "644"],
    ["../../packaging/linux/icons/io.github.mdedit.mdedit.svg", "usr/share/icons/hicolor/scalable/apps/io.github.mdedit.mdedit.svg", "644"],
    ["../../packaging/linux/icons/io.github.mdedit.mdedit-48.png", "usr/share/icons/hicolor/48x48/apps/io.github.mdedit.mdedit.png", "644"],
    ["../../packaging/linux/icons/io.github.mdedit.mdedit-128.png", "usr/share/icons/hicolor/128x128/apps/io.github.mdedit.mdedit.png", "644"],
    ["../../packaging/linux/icons/io.github.mdedit.mdedit-256.png", "usr/share/icons/hicolor/256x256/apps/io.github.mdedit.mdedit.png", "644"],
    ["../../packaging/linux/icons/io.github.mdedit.mdedit-512.png", "usr/share/icons/hicolor/512x512/apps/io.github.mdedit.mdedit.png", "644"],
    ["../../LICENSE-MIT", "usr/share/doc/mdedit/", "644"],
    ["../../LICENSE-APACHE", "usr/share/doc/mdedit/", "644"],
]
```

Notes:

- `target/release/mdedit` is special-cased by cargo-deb: it maps to the workspace's
  real target dir (also when `CARGO_TARGET_DIR` is set), so don't write `../../target`.
- `license-file = []` because the license is the SPDX expression `MIT OR Apache-2.0`
  from the workspace; cargo-deb writes it into `debian/copyright`. The full texts
  (`LICENSE-MIT`, `LICENSE-APACHE` at the repo root) are installed under
  `usr/share/doc/mdedit/`.
- `$auto` runs `dpkg-shlibdeps` on the binary (libc, and whatever system libs the build
  links). eframe/glow + winit load libGL/libEGL, libwayland and libxkbcommon with `dlopen`,
  so `$auto` misses them; they are listed explicitly.
  `rfd` with `xdg-portal` needs `xdg-desktop-portal` at runtime (present on GNOME/KDE).

## 2. Build

```bash
~/.cargo/bin/cargo install cargo-deb          # once
~/.cargo/bin/cargo deb -p mdedit              # builds release + writes target/debian/mdedit_0.1.0-1_amd64.deb
dpkg-deb -c target/debian/mdedit_*.deb        # check contents
sudo apt install ./target/debian/mdedit_*.deb
```

Validation after install:

```bash
desktop-file-validate /usr/share/applications/io.github.mdedit.mdedit.desktop
appstreamcli validate --no-net /usr/share/metainfo/io.github.mdedit.mdedit.metainfo.xml
```

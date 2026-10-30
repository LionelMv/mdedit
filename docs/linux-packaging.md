# Linux integration and packaging

## App ID and files

- App ID: `io.github.<you>.mdedit` (reverse-DNS you control).
- `packaging/linux/io.github.<you>.mdedit.desktop`:

```ini
[Desktop Entry]
Type=Application
Name=mdedit
Comment=Fast Markdown editor
Exec=mdedit %F
Icon=io.github.<you>.mdedit
Terminal=false
Categories=Office;TextEditor;Utility;
MimeType=text/markdown;text/x-markdown;text/plain;
StartupWMClass=mdedit
```

- `io.github.<you>.mdedit.metainfo.xml` (AppStream) — validate with `appstreamcli validate`.
- Icons: `hicolor/{48,128,256,512}x…/apps/` PNG + `scalable` SVG.
- Set the winit app id / X11 WM class to the App ID (`eframe::NativeOptions::viewport.with_app_id`)
  so Wayland compositors match the `.desktop` file and show the right icon.

## Build targets

| Format | Tool | Notes |
|---|---|---|
| `.deb` | `cargo-deb` | `[package.metadata.deb]` with assets (binary, desktop, icons, metainfo). |
| `.rpm` | `cargo-generate-rpm` | Same asset list. |
| AppImage | `linuxdeploy` or `cargo-appimage` | Build on the oldest supported distro (glibc baseline). |
| Flatpak | `flatpak-builder` + `flatpak-cargo-generator.py` for offline crates | Runtime `org.freedesktop.Platform`; permissions: `--socket=wayland --socket=fallback-x11 --device=dri --share=ipc`; file access through portals (`rfd`). |
| Arch | PKGBUILD in AUR | `cargo build --release --locked`. |

## Runtime checks

- Test matrix: Ubuntu LTS (GNOME/Wayland), Fedora (GNOME), Kubuntu/KDE (Wayland),
  Sway, an X11 session (e.g. Xfce).
- Verify: HiDPI fractional scaling, IME (fcitx5, IBus), drag-and-drop of files
  from the file manager, clipboard image paste, dark mode switching, portal dialogs in Flatpak.

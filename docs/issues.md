# Issues to implement later

Known gaps and follow-ups, newest first. Remove an entry when it is done.

## 1. Pasting and dropping images is not implemented

**Problem.** Preferences has an **Image folder** setting (`image_folder` in
`config.toml`, default `assets`), but nothing in the code uses it. Pasting an image from the
clipboard does nothing, and dropping an image file onto the window tries to open it as a
document.

**To do.**
- On paste (Ctrl+V) of clipboard image data, or on dropping an image file onto the editor:
  save the image into the image folder next to the document (create it if needed), and
  insert `![](assets/name.png)` at the caret.
- Pick a unique file name (for example `image-YYYYMMDD-HHMMSS.png`) and never overwrite an
  existing file.
- An unsaved document has no folder yet: ask the user to save first, or keep the image in
  memory until the first save.
- Keep the folder name sanitized (`prefs::sanitize_image_folder` already strips `..` and
  leading `/`).
- Document it in [user-guide.md](user-guide.md) under "Links and images", including the
  Preferences setting.

**Alternative.** If image pasting is not wanted, remove the setting from Preferences and
`Config` instead.

## 2. Design docs still describe the folder sidebar and folder search

**Problem.** The Files and Search sidebar panels, Open Folder, and folder command-line
arguments were removed; the sidebar is now outline-only. These docs still describe the old
design: [architecture.md](architecture.md), [feature-parity.md](feature-parity.md),
[implementation-plan.md](implementation-plan.md), [team.md](team.md).

**To do.** Update or strike the folder-tree and folder-search parts so the docs match the app.
[user-guide.md](user-guide.md) and the top-level `README.md` are already up to date.

## 3. Small buttons still have a shaded background

**Problem.** The ✕ close buttons are now plain (no shaded square), but the other small
buttons still show one: **«** (hide outline), **»** (show outline) and **+** (new tab).

**To do.** Decide whether these should match the ✕ buttons. If so, draw them with
`egui::Button::new(…).small().frame(false)` like the close buttons in `apps/mdedit/src/app.rs`.

# Issues to implement later

Known gaps and follow-ups. Remove an entry when it is done.

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

## 2. Small buttons still have a shaded background

**Problem.** The ✕ close buttons are now plain (no shaded square), but the other small
buttons still show one: **«** (hide outline), **»** (show outline) and **+** (new tab).

**To do.** Decide whether these should match the ✕ buttons. If so, draw them with
`egui::Button::new(…).small().frame(false)` like the close buttons in `apps/mdedit/src/app.rs`.

## 3. Change name of the Editor (caps?)

## 4. Long table cells don't wrap

**Problem.** In live preview, table columns are lined up by adding space before each cell
(`align_table` in `crates/editor/src/lib.rs`), and table rows never wrap: a wrapped row would
break out of the grid. A table wider than the text column (for example one with a long
sentence in a cell) runs past the right edge and is cut off by the window.

**To do.** Lay each cell out on its own (one galley per cell, wrapped to a column width that
fits the window), with caret movement and hit-testing mapped back to the source text. Simpler
stopgap: let wide tables scroll horizontally.

## 5. Files opened from outside don't bring the window to the front on Wayland

**Problem.** A file sent to the running window opens as a new tab, but the window stays in the
background and its dock icon blinks. winit's `focus_window` does nothing on Wayland, and
`request_user_attention` is what makes the icon blink.

**To do.** Wayland only raises a window given an activation token (`XDG_ACTIVATION_TOKEN`, set
by launchers for the process they start). The later `mdedit` would forward its token and the
window would call `xdg_activation_v1.activate(token, surface)` on its own Wayland connection.
winit 0.30 offers no API for that, and reaching its connection from outside needs `unsafe`, which
this project forbids. Revisit when winit exposes it.

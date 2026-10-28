# mdedit user guide

mdedit is a Markdown editor for Linux. It shows your document formatted as you type ("live
preview"), while the file on disk stays plain Markdown. Parts of a file you don't edit are
saved exactly as they were.

## Opening and saving files

- **Open:** File → Open… (Ctrl+O), File → Open Recent, or drag files onto the window. From a
  terminal: `mdedit notes.md todo.md`. A path that doesn't exist yet opens an empty document
  that will be saved there.
- **New document:** File → New (Ctrl+N), or the **+** button after the last tab.
- **Save:** Ctrl+S. **Save As:** Ctrl+Shift+S.
- **Export:** File → Export HTML… writes a standalone `.html` file. File → Copy as HTML puts
  the HTML on the clipboard, ready to paste into an email or web editor.
- **Very large files** (over 20 MB) open in Source mode for speed. Ctrl+E switches to live
  preview.

## Tabs

Each open document has a tab. The tab title ends with a dot (•) when there are unsaved
changes. Hover over a tab to see its full path.

- Switch tabs: click them, or Ctrl+Tab / Ctrl+Shift+Tab.
- Close a tab: the **✕** on the tab, Ctrl+W, or middle-click the tab. If it has unsaved
  changes, mdedit asks whether to save first.

## The outline sidebar

The sidebar on the left lists the headings of the current document. Click a heading to jump to
it. The heading you are currently in is highlighted.

- **Hide it:** the **«** button at the top of the outline, Ctrl+Shift+B, or View → Sidebar.
  It collapses into a thin strip.
- **Show it again:** the **»** button in that strip, or the same shortcut or menu item.
- Drag its right edge to make it wider or narrower.

mdedit remembers whether the outline was shown or hidden.

## Live preview and Source mode

- **Live** (the default) shows the document formatted. The Markdown symbols (`**`, `#`, …)
  appear only on the line you are editing, so you can still change them.
- **Source** shows the plain Markdown text with syntax colouring.

Switch with Ctrl+E, View → Source Mode, or by clicking **Live** / **Source** in the status bar.
To start new documents in Source mode, see [Preferences](#preferences).

## Writing and formatting

Type Markdown as usual, or use the menus and shortcuts:

| Shortcut | Does |
|---|---|
| Ctrl+B / Ctrl+I | Bold / italic |
| Ctrl+D | Strikethrough |
| Ctrl+` | Inline code |
| Ctrl+L | Link |
| Ctrl+Shift+I | Image |
| Ctrl+1 … Ctrl+6 | Heading 1 to 6 |
| Ctrl+Shift+0 | Turn a heading back into a paragraph |
| Ctrl+Shift+Q | Quote |
| Ctrl+Alt+U / Ctrl+Alt+O / Ctrl+Alt+X | Bullet / numbered / task list |
| Ctrl+Shift+K | Code block |
| Ctrl+Shift+M | Math block |
| Ctrl+Shift+T | Insert a table |
| Ctrl+Alt+- | Horizontal rule |

Some things happen as you type:

- **Enter** in a list starts the next item (numbers count up). Enter on an empty item ends
  the list.
- Typing ```` ``` ```` plus a language name (for example ```` ```bash ````) and pressing
  Enter adds the closing fence for you.
- Click a task list checkbox to tick it, or use Paragraph → Toggle Task.

Code blocks are shown as cards with syntax highlighting, the language name, and a copy button.

## Links and images

- **Ctrl+click** a link to follow it. Web links open in your browser. Links to other Markdown
  files open them in a new tab. Links to a heading (`#section`) jump to it.
- **Images** referenced with `![alt](path)` are shown in the document. Relative paths are
  resolved from the document's folder.
- **Web images** (`http://`, `https://`) are not loaded by default, for privacy: loading one
  reveals your IP address to the image's host. Turn them on in Preferences.

## Find and replace

- **Find:** Ctrl+F. **Replace:** Ctrl+H.
- Next / previous match: Enter / Shift+Enter in the find bar, or F3 / Shift+F3.
- Close the find bar: Esc or its **✕**.

## Reading and writing comfort

All in the **View** menu:

- **Typewriter mode** keeps the line you are typing in the middle of the window.
- **Focus mode** dims everything except the paragraph you are working on.
- **Zoom:** Ctrl++ and Ctrl+- make everything larger or smaller, and Ctrl+0 resets it.
- **Theme:** Light, Dark, or Follow System.
- **Full screen:** F11.

## The status bar

The bar at the bottom of the window shows, from left to right:

| Item | Meaning |
|---|---|
| `259 words · 1498 chars` | Word and character count of the document. |
| `Ln 12, Col 5` | Line and column of the text cursor. |
| `Live` / `Source` | The current view mode. **Click it** to switch (same as Ctrl+E). |
| `LF` / `CRLF` | The file's line endings. LF is the Linux and macOS standard, CRLF the Windows one. mdedit keeps whichever the file already uses when it saves. |
| `UTF-8` / `UTF-8 BOM` | The text encoding. Some Windows programs add an invisible "BOM" marker at the start of the file; mdedit keeps it if it was there. |
| `100%` | The zoom level. **Click it** to reset to 100%. Ctrl++ and Ctrl+- change it. |

## Preferences

File → Preferences… (Ctrl+,). Changes apply immediately.

| Setting | What it does |
|---|---|
| Theme | Light, Dark, or Follow system. |
| Body font size / Code font size | Text size for normal text and for code. |
| Line height | Space between lines. |
| Max text width | How wide the text column can get, or "Full width" to use the whole window. |
| Recovery snapshot every | How often unsaved changes are backed up (see below). 0 turns it off. |
| Remote images | Load images from the web (off by default). |
| New documents | Start new documents in Source mode. |

**Reset to defaults** restores every setting except whether the outline is shown.

Settings are stored in `~/.config/mdedit/config.toml`.

## Safety: crash recovery and outside changes

- **Crash recovery.** While a document has unsaved changes, mdedit regularly saves a backup
  copy (every 30 seconds by default) in `~/.local/state/mdedit/`. If mdedit or your computer
  crashes, the next start offers to restore those documents. This never touches your real
  files until you save.
- **Files changed by another program.** If a file you have open is changed on disk and you
  have no unsaved edits, mdedit reloads it and tells you. If you do have unsaved edits, a
  banner asks whether to **Reload** (take the disk version and lose your edits) or **Keep
  Mine**.

## Help inside the app

- Help → Keyboard Shortcuts lists every shortcut.
- Help → Markdown Cheatsheet is a quick Markdown syntax reference.

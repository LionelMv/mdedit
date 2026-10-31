# Manual checks

Things the automated tests can't cover, because they need a real desktop. Run these before a
release, and after changing what they name.

## Input method composition

An input method (IBus, fcitx5) lets you type characters your keyboard doesn't have: you type a
few keys, temporary underlined text (the *preedit*) appears at the caret, and you confirm or
cancel it. mdedit drops the empty preedits an idle input method sends, because they caused
nonstop redrawing on Wayland, but an empty preedit is also how a cancelled composition ends. The
filter's logic has a unit test with made-up events; only a real input method tests the whole path.

**Run after changing:** `drop_idle_preedits` in `apps/mdedit/src/app.rs`, or the IME handling in
`crates/editor/src/lib.rs`.

**Setup:** add an input method in the system's keyboard settings, such as Japanese (Mozc) or
Chinese (Pinyin), and switch to it (usually Super+Space). Test with IBus and with fcitx5, on
Wayland and on X11, as far as you can.

| Step | Expected |
|---|---|
| Type a few letters, then confirm with Enter or a candidate | The final characters appear once, at the caret |
| Type a few letters, then press Escape | The temporary text disappears; the document is unchanged |
| Type a few letters, then delete them all with Backspace | The temporary text disappears; the document is unchanged |
| Start composing with text selected | The selection is replaced once, and undo restores it |
| Leave the focused window alone for a few seconds | CPU stays near zero in `top` |

If you use dead keys or the compose key for accents like `é`, try one as well: they can go
through the input method too.

**Signs of trouble:** temporary text that stays after cancelling, characters doubled, or CPU use
while idle.

## Result log

| Date | Input method | Session | Result |
|---|---|---|---|
| | | | Not run yet |

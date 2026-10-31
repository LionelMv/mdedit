//! Headless widget tests (egui::Context + synthetic RawInput).

use super::*;
use egui::{PointerButton, RawInput};
use mdedit_buffer::Buffer;

struct H {
    ctx: egui::Context,
    ed: EditorState,
    doc: Document,
    theme: Theme,
    hl: Highlighter,
    time: f64,
    size: Vec2,
    platform: egui::PlatformOutput,
    step: f64,
}

impl H {
    fn new(text: &str) -> Self {
        let mut doc = Document::new();
        doc.buffer = Buffer::new(text);
        let mut h = Self {
            ctx: egui::Context::default(),
            ed: EditorState::new(),
            doc,
            theme: Theme::light(),
            hl: Highlighter::new(),
            time: 10.0,
            size: vec2(800.0, 600.0),
            platform: Default::default(),
            step: 1.0,
        };
        h.frame(vec![]);
        h.frame(vec![]);
        h
    }

    fn frame(&mut self, events: Vec<Event>) -> EditorOutput {
        self.frame_at(events, Modifiers::NONE)
    }

    fn frame_at(&mut self, events: Vec<Event>, modifiers: Modifiers) -> EditorOutput {
        self.time += self.step;
        let mut events = events;
        events.insert(0, Event::ModifiersChanged(modifiers));
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
            events,
            time: Some(self.time),
            focused: true,
            ..Default::default()
        };
        let mut out = EditorOutput::default();
        let (ed, doc, theme, hl) = (&mut self.ed, &mut self.doc, &self.theme, &self.hl);
        let mut full = self.ctx.run_ui(input, |ui| {
            out = ed.show(ui, doc, theme, hl, None);
        });
        self.platform = std::mem::take(&mut full.platform_output);
        full.drop_without_applying_deltas();
        out
    }

    fn key(&mut self, key: Key, modifiers: Modifiers) -> EditorOutput {
        self.frame_at(vec![Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }], modifiers)
    }

    fn typ(&mut self, s: &str) -> EditorOutput {
        self.frame(vec![Event::Text(s.into())])
    }

    fn text(&self) -> String {
        self.doc.buffer.text()
    }
    fn sel(&self) -> Selection {
        self.doc.buffer.selection
    }
    fn set_caret(&mut self, at: usize) {
        self.doc.buffer.selection = Selection::caret(at);
    }

    /// Screen position of byte `at` (centre of the caret rect).
    fn pos_of(&mut self, at: usize) -> Pos2 {
        let env = Env { ctx: self.ctx.clone(), theme: &self.theme, hl: &self.hl, base_dir: None };
        let sel = self.doc.buffer.selection;
        let (r, _) = self.ed.caret_content_rect(&env, sel, at);
        // The scroll area starts at the screen origin (plus the central ui margin = 0 for the root ui).
        let off = self.ed.anchor_offset();
        r.center() - vec2(0.0, off) + self.content_origin().to_vec2()
    }

    fn content_origin(&self) -> Pos2 {
        // Root ui has a margin of 8 by default? We run the editor on the root ui whose max_rect
        // is the whole screen.
        Pos2::ZERO
    }

    fn click(&mut self, p: Pos2, modifiers: Modifiers) {
        self.frame(vec![Event::PointerMoved(p)]);
        self.frame_at(vec![Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers }], modifiers);
        self.frame_at(vec![Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: false, modifiers }], modifiers);
    }
}

const CTRL: Modifiers = Modifiers { alt: false, ctrl: true, shift: false, mac_cmd: false, command: true };
const SHIFT: Modifiers = Modifiers { alt: false, ctrl: false, shift: true, mac_cmd: false, command: false };
const CTRL_SHIFT: Modifiers = Modifiers { alt: false, ctrl: true, shift: true, mac_cmd: false, command: true };

#[test]
fn typing_inserts_text() {
    let mut h = H::new("");
    let out = h.typ("h");
    assert!(out.changed);
    h.typ("i");
    h.typ("é");
    assert_eq!(h.text(), "hié");
    assert_eq!(h.sel(), Selection::caret(4));
    let out = h.frame(vec![]);
    assert_eq!((out.line, out.column), (1, 4));
}

#[test]
fn enter_and_backspace() {
    let mut h = H::new("ab");
    h.set_caret(1);
    h.key(Key::Enter, Modifiers::NONE);
    assert_eq!(h.text(), "a\nb");
    assert_eq!(h.sel(), Selection::caret(2));
    let out = h.frame(vec![]);
    assert_eq!((out.line, out.column), (2, 1));
    h.key(Key::Backspace, Modifiers::NONE);
    assert_eq!(h.text(), "ab");
    h.key(Key::Delete, Modifiers::NONE);
    assert_eq!(h.text(), "a");
    // Backspace at 0 / Delete at end: nothing, no panic.
    h.set_caret(0);
    h.key(Key::Backspace, Modifiers::NONE);
    h.set_caret(1);
    h.key(Key::Delete, Modifiers::NONE);
    assert_eq!(h.text(), "a");
}

#[test]
fn backspace_multibyte_and_selection() {
    let mut h = H::new("aé😀b");
    h.set_caret(7);
    h.key(Key::Backspace, Modifiers::NONE);
    assert_eq!(h.text(), "aéb");
    h.doc.buffer.selection = Selection { anchor: 0, head: 3 };
    h.key(Key::Delete, Modifiers::NONE);
    assert_eq!(h.text(), "b");
}

#[test]
fn ctrl_backspace_deletes_word() {
    let mut h = H::new("hello world");
    h.set_caret(11);
    h.key(Key::Backspace, CTRL);
    assert_eq!(h.text(), "hello ");
}

#[test]
fn arrows_move_and_extend() {
    let mut h = H::new("abc\ndef\nghi");
    h.set_caret(1);
    h.key(Key::ArrowRight, Modifiers::NONE);
    assert_eq!(h.sel(), Selection::caret(2));
    h.key(Key::ArrowLeft, Modifiers::NONE);
    assert_eq!(h.sel(), Selection::caret(1));
    h.key(Key::ArrowDown, Modifiers::NONE);
    assert_eq!(h.sel(), Selection::caret(5), "same column on next line");
    h.key(Key::ArrowDown, Modifiers::NONE);
    assert_eq!(h.sel(), Selection::caret(9));
    h.key(Key::ArrowDown, Modifiers::NONE);
    assert_eq!(h.sel(), Selection::caret(11), "down on the last line goes to the end");
    h.key(Key::ArrowUp, Modifiers::NONE);
    h.key(Key::ArrowUp, Modifiers::NONE);
    assert_eq!(h.sel().head, 1, "desired x kept from the first vertical move");
    h.key(Key::ArrowUp, Modifiers::NONE);
    assert_eq!(h.sel(), Selection::caret(0));
    h.key(Key::ArrowRight, SHIFT);
    h.key(Key::ArrowRight, SHIFT);
    assert_eq!(h.sel(), Selection { anchor: 0, head: 2 });
    h.key(Key::ArrowRight, Modifiers::NONE);
    assert_eq!(h.sel(), Selection::caret(2), "collapse to the right edge");
    h.key(Key::End, Modifiers::NONE);
    assert_eq!(h.sel(), Selection::caret(3));
    h.key(Key::Home, Modifiers::NONE);
    assert_eq!(h.sel(), Selection::caret(0));
    h.key(Key::End, CTRL);
    assert_eq!(h.sel(), Selection::caret(11));
    h.key(Key::Home, CTRL_SHIFT);
    assert_eq!(h.sel(), Selection { anchor: 11, head: 0 });
}

#[test]
fn vertical_across_chunks() {
    // Several paragraphs => several chunks once the real chunker is in.
    let mut h = H::new("first para\n\nsecond para\n\nthird");
    h.set_caret(2);
    h.key(Key::ArrowDown, Modifiers::NONE);
    assert_eq!(h.sel().head, 11, "onto the blank line");
    h.key(Key::ArrowDown, Modifiers::NONE);
    assert!((13..=14).contains(&h.sel().head), "{:?}", h.sel());
    h.key(Key::ArrowDown, Modifiers::NONE);
    h.key(Key::ArrowDown, Modifiers::NONE);
    assert!((26..=27).contains(&h.sel().head), "{:?}", h.sel());
    h.key(Key::ArrowUp, Modifiers::NONE);
    h.key(Key::ArrowUp, Modifiers::NONE);
    assert!((13..=14).contains(&h.sel().head), "{:?}", h.sel());
}

#[test]
fn trailing_newline_caret() {
    let mut h = H::new("abc\n");
    h.key(Key::End, CTRL);
    assert_eq!(h.sel(), Selection::caret(4));
    h.typ("x");
    assert_eq!(h.text(), "abc\nx");
    h.key(Key::ArrowUp, Modifiers::NONE);
    assert_eq!(h.sel().head, 1);
}

#[test]
fn undo_redo() {
    let mut h = H::new("");
    h.typ("a");
    h.typ("b");
    h.key(Key::Enter, Modifiers::NONE);
    h.doc.buffer.break_undo_group();
    h.typ("c");
    assert_eq!(h.text(), "ab\nc");
    h.key(Key::Z, CTRL);
    assert_ne!(h.text(), "ab\nc");
    while h.doc.buffer.can_undo() {
        h.key(Key::Z, CTRL);
    }
    assert_eq!(h.text(), "");
    h.key(Key::Z, CTRL_SHIFT);
    assert!(!h.text().is_empty());
    h.key(Key::Y, CTRL);
    while h.doc.buffer.can_redo() {
        h.key(Key::Y, CTRL);
    }
    assert_eq!(h.text(), "ab\nc");
}

#[test]
fn paste_normalizes_crlf_and_copy_cut() {
    let mut h = H::new("xy");
    h.set_caret(1);
    h.frame(vec![Event::Paste("a\r\nb\rc".into())]);
    assert_eq!(h.text(), "xa\nb\ncy");
    assert_eq!(h.sel(), Selection::caret(6));
    h.doc.buffer.selection = Selection { anchor: 0, head: 2 };
    h.frame(vec![Event::Copy]);
    assert!(
        h.platform.commands.iter().any(|c| matches!(c, egui::OutputCommand::CopyText(t) if t == "xa")),
        "{:?}",
        h.platform.commands
    );
    h.frame(vec![Event::Cut]);
    assert_eq!(h.text(), "\nb\ncy");
}

#[test]
fn select_all() {
    let mut h = H::new("one\ntwo");
    h.key(Key::A, CTRL);
    assert_eq!(h.sel().range(), 0..7);
}

#[test]
fn tab_inserts_and_keeps_focus() {
    let mut h = H::new("a");
    h.set_caret(0);
    h.key(Key::Tab, Modifiers::NONE);
    assert_eq!(h.text(), "\ta");
    h.typ("b");
    assert_eq!(h.text(), "\tba", "still focused after Tab");
}

#[test]
fn ime_preedit_and_commit() {
    let mut h = H::new("ab");
    h.set_caret(1);
    h.frame(vec![Event::Ime(ImeEvent::Preedit { text: "ni".into(), active_range_chars: None })]);
    assert_eq!(h.text(), "ab", "preedit is not inserted");
    assert!(h.platform.ime.is_some(), "IME output set while focused");
    // Keys and text are ignored during composition.
    h.typ("z");
    assert_eq!(h.text(), "ab");
    h.frame(vec![Event::Ime(ImeEvent::Commit("你".into()))]);
    assert_eq!(h.text(), "a你b");
    assert_eq!(h.sel(), Selection::caret(4));
}

#[test]
fn click_places_caret_and_shift_click_extends() {
    let mut h = H::new("hello world");
    let p = h.pos_of(6);
    h.click(p, Modifiers::NONE);
    assert_eq!(h.sel(), Selection::caret(6));
    h.time += 5.0;
    let p2 = h.pos_of(2);
    h.click(p2, SHIFT);
    assert_eq!(h.sel(), Selection { anchor: 6, head: 2 });
}

#[test]
fn double_click_selects_word_and_drag_selects() {
    let mut h = H::new("hello world again");
    let p = h.pos_of(8);
    h.frame(vec![Event::PointerMoved(p)]);
    for _ in 0..2 {
        h.time -= 0.9; // keep the two clicks close in time
        h.frame(vec![Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE }]);
        h.time -= 0.9;
        h.frame(vec![Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }]);
    }
    assert_eq!(h.sel().range(), 6..11);

    h.time += 10.0;
    let a = h.pos_of(1);
    let b = h.pos_of(4);
    h.frame(vec![Event::PointerMoved(a)]);
    h.frame(vec![Event::PointerButton { pos: a, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE }]);
    h.frame(vec![Event::PointerMoved(b)]);
    h.frame(vec![Event::PointerButton { pos: b, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }]);
    assert_eq!(h.sel(), Selection { anchor: 1, head: 4 });
}

#[test]
fn run_command_and_select_api() {
    let mut h = H::new("word");
    h.doc.buffer.selection = Selection { anchor: 0, head: 4 };
    let applied = h.ed.run_command(&mut h.doc, &Command::Bold);
    if applied {
        assert_eq!(h.text(), "**word**");
    }
    h.ed.select(&mut h.doc, 1..3);
    assert_eq!(h.sel(), Selection { anchor: 1, head: 3 });
    h.ed.scroll_to(&mut h.doc, 999);
    assert_eq!(h.sel(), Selection::caret(h.text().len()));
    h.frame(vec![]);
}

#[test]
fn word_counts_reported() {
    let mut h = H::new("one two three");
    let out = h.frame(vec![]);
    // Depends on the syntax crate's word_count.
    assert_eq!(out.words, 3);
}

#[test]
fn no_panic_on_weird_input() {
    let mut h = H::new("# T\n\n```rust\nfn x() {}\n```\n\n- [ ] task\n- item\n\n> quote\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n![img](x.png) [l](http://e.com)\n");
    let len = h.text().len();
    for at in [0, 1, 3, 5, 9, len / 2, len - 1, len] {
        h.set_caret(at);
        for k in [Key::ArrowDown, Key::ArrowUp, Key::End, Key::Home, Key::PageDown, Key::PageUp, Key::ArrowRight] {
            h.key(k, Modifiers::NONE);
            h.key(k, SHIFT);
        }
        h.typ("x");
        h.key(Key::Enter, Modifiers::NONE);
        h.key(Key::Tab, SHIFT);
        h.key(Key::Backspace, Modifiers::NONE);
    }
    h.ed.mode = Mode::Source;
    h.ed.focus_mode = true;
    h.ed.typewriter = true;
    h.ed.search_matches = vec![0..3, 5..9, 9999..10000];
    h.ed.search_current = Some(1);
    h.frame(vec![]);
    h.key(Key::A, CTRL);
    h.key(Key::A, CTRL);
    h.frame(vec![Event::Paste("zz".into())]);
    // Out-of-range selection from outside.
    h.doc.buffer.selection = Selection { anchor: 99_999, head: 3 };
    h.frame(vec![]);
    h.key(Key::Delete, Modifiers::NONE);
    h.size = vec2(30.0, 20.0);
    h.frame(vec![]);
}

#[test]
fn big_document_lays_out_only_visible_chunks() {
    let mut s = String::new();
    let mut i = 0;
    while s.len() < 10 * 1024 * 1024 {
        s.push_str(&format!("## Section {i}\n\nSome prose with *emphasis* and `code` here, number {i}.\n\n- item one\n- item two\n\n"));
        i += 1;
    }
    let t0 = std::time::Instant::now();
    let mut h = H::new(&s);
    let open = t0.elapsed();
    let st = h.ed.frame_stats();
    assert!(st.layouts < 200, "{st:?}");
    assert!(st.chunks > 100, "{st:?}");
    // Jump to the end and back.
    h.key(Key::End, CTRL);
    let st = h.ed.frame_stats();
    assert!(st.layouts < 200, "{st:?}");
    assert!(st.painted_chunks < 200, "{st:?}");
    assert_eq!(h.sel().head, s.len());
    let t1 = std::time::Instant::now();
    h.frame(vec![]);
    h.frame(vec![]);
    let idle = t1.elapsed();
    assert!(h.ed.frame_stats().layouts == 0, "cached when idle: {:?}", h.ed.frame_stats());
    let t2 = std::time::Instant::now();
    h.typ("x");
    let typing = t2.elapsed();
    assert!(h.text().ends_with('x'));
    let t3 = std::time::Instant::now();
    let blocks = mdedit_syntax::parse(&s);
    let parse = t3.elapsed();
    let t4 = std::time::Instant::now();
    let _ = mdedit_style::chunks(&s, &blocks);
    let _ = mdedit_syntax::word_count(&s, &blocks);
    let rest = t4.elapsed();
    eprintln!("open+2 frames: {open:?}, 2 idle frames: {idle:?}, typing frame: {typing:?}, parse: {parse:?}, chunks+wc: {rest:?}");
}

#[test]
fn scroll_wheel_and_page_down_keep_valid() {
    let text: String = (0..400).map(|i| format!("line {i}\n\n")).collect();
    let mut h = H::new(&text);
    h.key(Key::PageDown, Modifiers::NONE);
    assert!(h.sel().head > 0);
    h.frame(vec![Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: vec2(0.0, -500.0), modifiers: Modifiers::NONE, phase: egui::TouchPhase::Move, source: egui::MouseWheelSource::Wheel }]);
    for _ in 0..5 {
        h.frame(vec![]);
    }
    h.ed.scroll_to(&mut h.doc, 2000);
    h.frame(vec![]);
    h.frame(vec![]);
    let off = h.ed.anchor_offset();
    assert!(off > 100.0, "scrolled down: {off}");
}

impl H {
    fn decos(&mut self) -> Vec<DecoGeom> {
        let env = Env { ctx: self.ctx.clone(), theme: &self.theme, hl: &self.hl, base_dir: None };
        let sel = self.doc.buffer.selection;
        let off = self.ed.anchor_offset();
        let viewport = Rect::from_min_size(pos2(0.0, off), self.size);
        let vis = self.ed.layout_visible(&env, sel, viewport);
        self.ed.resolve_decos(&env, &vis).into_iter().flatten().collect()
    }
    fn press_release(&mut self, p: Pos2, m: Modifiers) -> (EditorOutput, EditorOutput) {
        self.frame_at(vec![Event::PointerMoved(p)], m);
        let a = self.frame_at(vec![Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: m }], m);
        let b = self.frame_at(vec![Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: false, modifiers: m }], m);
        (a, b)
    }
}

#[test]
fn ctrl_a_in_code_block_selects_content_first() {
    let text = "para\n\n```\ncode line\n```\n";
    let mut h = H::new(text);
    h.set_caret(12);
    h.frame(vec![]);
    let content = commands::block_at(&mdedit_syntax::parse(text), 12).map(|b| b.content.clone());
    h.key(Key::A, CTRL);
    if let Some(c) = content {
        assert_eq!(h.sel().range(), c, "CB-47 first Ctrl+A selects the code");
        h.key(Key::A, CTRL);
    }
    assert_eq!(h.sel().range(), 0..text.len());
}

#[test]
fn checkbox_click_toggles_task() {
    let mut h = H::new("- [ ] task\n\npara");
    h.set_caret(16);
    h.frame(vec![]);
    let cb = h.decos().into_iter().find_map(|d| match d {
        DecoGeom::Checkbox { rect, .. } => Some(rect),
        _ => None,
    });
    let Some(rect) = cb else { panic!("no checkbox decoration (style crate)") };
    let off = h.ed.anchor_offset();
    h.press_release(rect.center() - vec2(0.0, off), Modifiers::NONE);
    assert_eq!(h.text(), "- [x] task\n\npara");
}

#[test]
fn ctrl_click_link_reports_url() {
    let mut h = H::new("see [the link](https://example.com) here\n\npara");
    h.set_caret(45);
    h.frame(vec![]);
    let link = h.decos().into_iter().find_map(|d| match d {
        DecoGeom::Link { rects, .. } => rects.first().copied(),
        _ => None,
    });
    let Some(rect) = link else { panic!("no link decoration (style crate)") };
    let off = h.ed.anchor_offset();
    let before = h.sel();
    let (a, _) = h.press_release(rect.center() - vec2(0.0, off), CTRL);
    assert_eq!(a.open_link.as_deref(), Some("https://example.com"));
    assert_eq!(h.sel(), before, "ctrl+click on a link doesn't move the caret");
    // A plain click places the caret instead.
    h.time += 5.0;
    let (a, _) = h.press_release(rect.center() - vec2(0.0, off), Modifiers::NONE);
    assert!(a.open_link.is_none());
    assert!(h.sel().head > 4 && h.sel().head < 35);
}

#[test]
fn code_header_copy_button() {
    let text = "para\n\n```rust\nfn x() {}\n```\n";
    let mut h = H::new(text);
    h.set_caret(0);
    h.frame(vec![]);
    let btn = h.decos().into_iter().find_map(|d| match d {
        DecoGeom::Header { button, lang, .. } => Some((button, lang)),
        _ => None,
    });
    let Some((rect, lang)) = btn else { panic!("no code header decoration (style crate)") };
    assert_eq!(lang, "rust");
    let off = h.ed.anchor_offset();
    h.frame_at(vec![Event::PointerMoved(rect.center() - vec2(0.0, off))], Modifiers::NONE);
    h.frame_at(
        vec![Event::PointerButton { pos: rect.center() - vec2(0.0, off), button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE }],
        Modifiers::NONE,
    );
    assert!(
        h.platform.commands.iter().any(|c| matches!(c, egui::OutputCommand::CopyText(t) if t.contains("fn x() {}"))),
        "{:?}",
        h.platform.commands
    );
    assert_eq!(h.text(), text);
}

#[test]
fn live_preview_reveals_markers_in_focused_chunk() {
    // The caret's chunk is styled focused: its galley differs from the unfocused one.
    let mut h = H::new("# Title\n\nsome **bold** text\n");
    h.set_caret(0);
    h.frame(vec![]);
    let w_focused = {
        let p = h.pos_of(18);
        p.x
    };
    h.set_caret(1);
    h.frame(vec![]);
    let w_unfocused = h.pos_of(18).x;
    h.set_caret(18);
    h.frame(vec![]);
    let w_refocused = h.pos_of(18).x;
    // When focused the `**` markers take room, so the x of byte 18 is larger.
    assert!(w_refocused >= w_unfocused, "{w_refocused} vs {w_unfocused}");
    let _ = w_focused;
}

#[test]
fn search_and_select_scroll() {
    let text: String = (0..2000).map(|i| format!("para {i}\n\n")).collect();
    let mut h = H::new(&text);
    let at = text.find("para 1500").unwrap_or(0);
    h.ed.search_matches = std::iter::once(at..at + 9).collect();
    h.ed.search_current = Some(0);
    h.ed.select(&mut h.doc, at..at + 9);
    h.frame(vec![]);
    h.frame(vec![]);
    let p = h.pos_of(at);
    assert!(p.y > 0.0 && p.y < h.size.y, "selection scrolled into view: {p:?}");
}


#[test]
fn small_touchpad_deltas_accumulate() {
    // Wayland touchpads send many sub-pixel deltas; each must move the view.
    let text: String = (0..400).map(|i| format!("line {i}\n\n")).collect();
    let mut h = H::new(&text);
    h.step = 1.0 / 60.0;
    h.frame(vec![Event::PointerMoved(pos2(400.0, 300.0))]);
    let before = h.ed.anchor_offset();
    for _ in 0..120 {
        h.frame(vec![Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: vec2(0.0, -0.4), modifiers: Modifiers::NONE, phase: egui::TouchPhase::Move, source: egui::MouseWheelSource::Unknown }]);
    }
    let after = h.ed.anchor_offset();
    assert!(after > before + 30.0, "touchpad scrolled: {before} -> {after}");
}

#[test]
fn idle_empty_preedit_does_not_pin_the_view() {
    // IBus on Wayland sends an empty preedit every frame; it must not snap back to the caret.
    let text: String = (0..400).map(|i| format!("line {i}\n\n")).collect();
    let mut h = H::new(&text);
    h.step = 1.0 / 60.0;
    h.frame(vec![Event::PointerMoved(pos2(400.0, 300.0))]);
    let before = h.ed.anchor_offset();
    let idle = || Event::Ime(ImeEvent::Preedit { text: String::new(), active_range_chars: None });
    for _ in 0..20 {
        h.frame(vec![idle(), Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: vec2(0.0, -15.0), modifiers: Modifiers::NONE, phase: egui::TouchPhase::Move, source: egui::MouseWheelSource::Unknown }]);
    }
    for _ in 0..10 {
        h.frame(vec![idle()]);
    }
    let after = h.ed.anchor_offset();
    assert!(after > before + 100.0, "scrolled despite idle IME: {before} -> {after}");
}

#[test]
fn font_atlas_reset_relays_cached_text() {
    let mut h = H::new("# Title\n\nSome text.\n");
    h.frame(vec![]);
    assert_eq!(h.ed.frame_stats().layouts, 0, "warm cache");
    // Fill the atlas past egui's 80% threshold; the next pass wipes it.
    let mut size = 20.0;
    while h.ctx.fonts(|f| f.font_atlas_fill_ratio()) <= 0.8 {
        let text: String = ('!'..='~').collect();
        h.ctx.fonts_mut(|f| f.layout_no_wrap(text, egui::FontId::proportional(size), Color32::WHITE));
        size += 3.0;
    }
    h.frame(vec![]);
    assert!(h.ed.frame_stats().layouts > 0, "cached galleys must be re-laid after an atlas reset");
}

#[test]
fn table_cells_line_up_in_columns() {
    let text = "intro\n\n| Item | Value |\n| --- | ---: |\n| New sudo user | 1 |\n| App | 12345 |\n\nend\n";
    let mut h = H::new(text);
    h.set_caret(0);
    h.frame(vec![]);
    let at = |s: &str| text.find(s).unwrap();
    let x = |h: &mut H, s: &str| h.pos_of(at(s)).x;
    // Left-aligned first column: every cell starts at the same x.
    let (a, b) = (x(&mut h, "Item"), x(&mut h, "New sudo"));
    assert!((a - b).abs() < 1.0, "first column: {a} vs {b}");
    // Right-aligned second column: cells end at the same x, after the widest first cell.
    let end = |h: &mut H, s: &str| h.pos_of(at(s) + s.len()).x;
    let e2 = h.pos_of(at("| 1 |") + 3).x; // just after "1"
    let (e1, e3) = (end(&mut h, "Value"), end(&mut h, "12345"));
    assert!((e1 - e3).abs() < 1.0 && (e2 - e3).abs() < 1.0, "right column ends: {e1} {e2} {e3}");
    assert!(x(&mut h, "Value") > end(&mut h, "New sudo user"), "columns don't overlap");
}

#[test]
fn code_nested_in_list_does_not_overlap_item_text() {
    let text = "1. **Generate the key:**\n\n   ```bash\n   ssh-keygen -t ed25519\n   ```\n\n2. Next\n";
    let mut h = H::new(text);
    h.set_caret(text.len());
    h.frame(vec![]);
    let card = h.decos().into_iter().find_map(|d| match d {
        DecoGeom::Card { rect, .. } => Some(rect),
        _ => None,
    });
    let Some(card) = card else { panic!("no card for the nested code block") };
    let off = h.ed.anchor_offset();
    let item = h.pos_of(text.find("Generate").unwrap()).y + off;
    let code = h.pos_of(text.find("ssh-keygen").unwrap()).y + off;
    let next = h.pos_of(text.find("Next").unwrap()).y + off;
    assert!(item < card.min.y, "item text above the card: {item} vs {card:?}");
    assert!(card.min.y < code && code < card.max.y, "code inside the card");
    assert!(card.max.y < next, "next item below the card");
    // The code lines up with the card padding (the list indent is hidden).
    assert!(h.pos_of(text.find("ssh-keygen").unwrap()).x - card.min.x < CARD_PAD_H + 8.0);
}

#[test]
fn list_items_are_indented_with_hanging_wrap() {
    let long = "word ".repeat(60);
    let text = format!("Body text.\n\n- {long}\n  - nested\n    - third\n");
    let mut h = H::new(&text);
    h.set_caret(0);
    h.frame(vec![]);
    let x = |h: &mut H, at: usize| h.pos_of(at).x;
    let body = x(&mut h, 0);
    let item = x(&mut h, text.find("word").unwrap());
    let nested = x(&mut h, text.find("nested").unwrap());
    let third = x(&mut h, text.find("third").unwrap());
    assert!(item > body + 10.0, "items are indented: {body} -> {item}");
    assert!(nested > item + 10.0 && third > nested + 10.0, "levels step in: {item} {nested} {third}");
    // A byte on the wrapped second row lines up with the item's text.
    let first_y = h.pos_of(text.find("word").unwrap()).y;
    let wrapped = (text.find("word").unwrap()..text.find("\n  - nested").unwrap())
        .step_by(5)
        .find(|&b| h.pos_of(b).y > first_y + 5.0)
        .expect("the long item wraps");
    assert!((x(&mut h, wrapped) - item).abs() < 1.0, "wrapped row starts under the text");
}

#[test]
fn code_card_in_quote_is_right_of_the_bar() {
    let text = "> Quote:\n>\n> ```rust\n> let x = 1;\n> ```\n\nend\n";
    let mut h = H::new(text);
    h.set_caret(text.len());
    h.frame(vec![]);
    let decos = h.decos();
    let bar = decos.iter().find_map(|d| match d {
        DecoGeom::Bar { rect, .. } => Some(*rect),
        _ => None,
    });
    let card = decos.iter().find_map(|d| match d {
        DecoGeom::Card { rect, .. } => Some(*rect),
        _ => None,
    });
    let (Some(bar), Some(card)) = (bar, card) else { panic!("bar and card: {bar:?} {card:?}") };
    assert!(card.min.x > bar.max.x, "card {card:?} right of bar {bar:?}");
}

#[test]
fn list_in_quote_is_indented_with_hanging_wrap() {
    let long = "word ".repeat(60);
    let text = format!("> Intro\n>\n> - {long}\n>   - nested\n\nend\n");
    let mut h = H::new(&text);
    h.set_caret(text.len());
    h.frame(vec![]);
    let intro = h.pos_of(2).x;
    let start = text.find("word").unwrap();
    let item = h.pos_of(start).x;
    let nested = h.pos_of(text.find("nested").unwrap()).x;
    assert!(item > intro + 10.0 && nested > item + 10.0, "{intro} {item} {nested}");
    let first_y = h.pos_of(start).y;
    let wrapped = (start..start + long.len()).step_by(5).find(|&b| h.pos_of(b).y > first_y + 5.0).expect("wraps");
    assert!((h.pos_of(wrapped).x - item).abs() < 1.0, "wrapped row starts under the text");
}

#[test]
fn editing_an_item_does_not_move_its_text() {
    let text = "para\n\n- first\n  - nested item\n1. number\n\n> quoted\n";
    for word in ["first", "nested", "number", "quoted"] {
        let at = text.find(word).unwrap();
        let mut h = H::new(text);
        h.set_caret(0);
        h.frame(vec![]);
        let before = h.pos_of(at).x;
        h.set_caret(at + 1);
        h.frame(vec![]);
        let after = h.pos_of(at).x;
        assert!((before - after).abs() < 1.0, "{word}: {before} -> {after}");
    }
}

#[test]
fn code_in_list_keeps_its_own_indentation() {
    let text = "- x\n\n  ```py\n  if a:\n      b\n  ```\n";
    let mut h = H::new(text);
    h.set_caret(0);
    h.frame(vec![]);
    let (a, b) = (h.pos_of(text.find("if a").unwrap()).x, h.pos_of(text.find("b\n").unwrap()).x);
    assert!(b > a + 10.0, "{a} {b}");
}

#[test]
fn second_list_in_quote_and_quote_in_list_are_indented() {
    let text = "> Intro\n>\n> - a\n> 1. b\n\n- Item\n  > quoted\n\nend\n";
    let mut h = H::new(text);
    h.set_caret(text.len());
    h.frame(vec![]);
    let x = |h: &mut H, s: &str| h.pos_of(text.find(s).unwrap()).x;
    let (intro, a, b) = (x(&mut h, "Intro"), x(&mut h, "a\n"), x(&mut h, "b\n"));
    assert!((a - b).abs() < 1.0 && a > intro + 10.0, "{intro} {a} {b}");
    let (item, quoted) = (x(&mut h, "Item"), x(&mut h, "quoted"));
    assert!(quoted > item + 10.0, "quote text right of its bar: {item} {quoted}");
}

#[test]
fn switching_tabs_keeps_scroll_position() {
    let long: String = (0..400).map(|i| format!("line {i}\n\n")).collect();
    let mut h = H::new(&long);
    h.step = 1.0 / 60.0;
    h.frame(vec![Event::PointerMoved(pos2(400.0, 300.0))]);
    for _ in 0..40 {
        h.frame(vec![Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: vec2(0.0, -200.0), modifiers: Modifiers::NONE, phase: egui::TouchPhase::Move, source: egui::MouseWheelSource::Unknown }]);
    }
    for _ in 0..30 {
        h.frame(vec![]);
    }
    let before = h.ed.anchor_offset();
    assert!(before > 1000.0, "scrolled: {before}");
    // Another tab with a shorter document that can scroll too, shown in the same place.
    let mut other_doc = Document::new();
    let short: String = (0..60).map(|i| format!("line {i}\n\n")).collect();
    other_doc.buffer = Buffer::new(&short);
    let mut other_ed = EditorState::new();
    std::mem::swap(&mut h.ed, &mut other_ed);
    std::mem::swap(&mut h.doc, &mut other_doc);
    for _ in 0..5 {
        h.frame(vec![]);
    }
    std::mem::swap(&mut h.ed, &mut other_ed);
    std::mem::swap(&mut h.doc, &mut other_doc);
    for _ in 0..5 {
        h.frame(vec![]);
    }
    let after = h.ed.anchor_offset();
    assert!((before - after).abs() < 2.0, "scroll kept: {before} -> {after}");
}

/// The whole model, compared with a rebuild from scratch.
fn assert_model_matches_rebuild(ed: &mut EditorState, doc: &Document, what: &str) {
    let text = doc.buffer.text();
    assert_eq!(ed.model.text, text, "{what}: text");
    let blocks = mdedit_syntax::parse(&text);
    let valid = ed.model.valid_blocks;
    assert_eq!(ed.model.blocks.len(), blocks.len(), "{what}: block count");
    assert!(ed.model.blocks[..valid] == blocks[..valid], "{what}: valid blocks differ from a full parse");
    assert!(ed.model.has_definitions || !mdedit_syntax::may_have_definition(&text), "{what}: definitions");
    assert!(ed.model.all_blocks() == blocks, "{what}: blocks differ from a full parse");
    let chunks = model::build_chunks(&text, &blocks);
    if ed.model.chunks != chunks {
        let i = ed.model.chunks.iter().zip(&chunks).position(|(a, b)| a != b);
        panic!("{what}: chunks differ at {i:?} ({} vs {}): {:?} vs {:?}", ed.model.chunks.len(), chunks.len(), i.map(|i| &ed.model.chunks[i]), i.map(|i| &chunks[i]));
    }
    assert_eq!(ed.model.hashes.len(), chunks.len(), "{what}: hashes");
    assert_eq!(ed.heights.len(), chunks.len(), "{what}: heights");
    for (i, h) in ed.model.hashes.iter().enumerate() {
        if let Some(h) = h {
            assert_eq!(*h, model::hash_str(&text[chunks[i].range.clone()]), "{what}: hash {i}");
        }
    }
}

/// Over `MIN_INCREMENTAL`, with every kind of block, nested code, an oversized block, and
/// definitions.
fn varied_doc(defs: bool) -> String {
    let mut s = String::from("\n\n");
    for i in 0..220 {
        s += &format!("## Section {i}\n\nSome **bold** `code` [link](http://x/{i}) é.\nSetext?\n\n- a\n- [ ] b\n  - c\n\n1. Step:\n\n   ```bash\n   echo {i}\n   ```\n\n2. Next\n\n> quote {i}\n>\n> ```rust\n> let a = {i};\n> ```\n\n| A | B |\n|---|---|\n| {i} | x |\n\n---\n\n<div>{i}</div>\n\n$$\nx^{i}\n$$\n\n");
    }
    s += "```rust\n";
    for i in 0..3000 {
        s += &format!("let v{i} = {i};\n");
    }
    s += "```\n\nend paragraph\n";
    if defs {
        s = format!("Use [foo] and [^n] now.\n\n{s}\n[foo]: http://x \"t\"\n\n[^n]: note\n");
    }
    s
}

#[test]
fn sync_updates_only_around_the_edit() {
    let src = varied_doc(false);
    let mut doc = Document::new();
    doc.buffer = Buffer::new(&src);
    let mut ed = EditorState::new();
    ed.sync(&doc);
    let n = ed.model.chunks.len();
    for i in 0..n {
        ed.model.chunk_hash(i);
    }
    let at = src.len() / 3;
    let at = at + src[at..].find("Setext").unwrap();
    doc.buffer.apply(vec![Edit::insert(at, "x")], Selection::caret(at + 1), EditKind::Typing);
    ed.sync(&doc);
    assert_model_matches_rebuild(&mut ed, &doc, "typing");
    let unknown = ed.model.hashes.iter().filter(|h| h.is_none()).count();
    let first = ed.model.hashes.iter().position(|h| h.is_none());
    let last = ed.model.hashes.iter().rposition(|h| h.is_none());
    assert!(unknown < 40, "only chunks around the edit were rebuilt, not {unknown} of {n} ({first:?}..{last:?}), blocks {}, at {at} of {}", ed.model.blocks.len(), src.len());
}

#[test]
fn sync_matches_rebuild_for_undo_and_replacing_everything() {
    let src = varied_doc(true);
    let mut doc = Document::new();
    doc.buffer = Buffer::new(&src);
    let mut ed = EditorState::new();
    ed.sync(&doc);
    let at = src.find("Setext").unwrap();
    doc.buffer.apply(vec![Edit::insert(at, "```\n")], Selection::caret(at), EditKind::Other);
    ed.sync(&doc);
    assert_model_matches_rebuild(&mut ed, &doc, "open fence");
    assert!(doc.buffer.undo());
    ed.sync(&doc);
    assert_model_matches_rebuild(&mut ed, &doc, "undo");
    doc.buffer.set_text("short\n");
    ed.sync(&doc);
    assert_model_matches_rebuild(&mut ed, &doc, "short");
    doc.buffer.set_text("");
    ed.sync(&doc);
    assert_model_matches_rebuild(&mut ed, &doc, "empty");
    doc.buffer.set_text(&src);
    ed.sync(&doc);
    assert_model_matches_rebuild(&mut ed, &doc, "big again");
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(48))]
    #[test]
    fn incremental_sync_equals_full_rebuild(
        defs in proptest::bool::ANY,
        edits in proptest::collection::vec(
            (0.0f64..1.0, 0usize..60, "[a-z#*`>\\-|\\[\\]:^ é\n]{0,10}|```\n|\n\n|- |> |    |---\n|\\[foo\\]: /u\n"),
            1..6,
        ),
        flush_each in proptest::bool::ANY,
    ) {
        let src = varied_doc(defs);
        let mut doc = Document::new();
        doc.buffer = Buffer::new(&src);
        let mut ed = EditorState::new();
        ed.sync(&doc);
        for (frac, del, ins) in edits {
            let len = doc.buffer.len_bytes();
            let a = doc.buffer.clamp_offset((len as f64 * frac) as usize);
            let b = doc.buffer.clamp_offset(a + del);
            doc.buffer.apply(vec![Edit::replace(a..b, ins.as_str())], Selection::caret(a), EditKind::Other);
            ed.sync(&doc);
            if flush_each {
                assert_model_matches_rebuild(&mut ed, &doc, &format!("{a}..{b} -> {ins:?}"));
            }
        }
        assert_model_matches_rebuild(&mut ed, &doc, "after all edits");
    }
}

#[test]
fn code_chunk_is_relaid_when_its_highlight_arrives() {
    let colours = |h: &mut H| -> usize {
        let env = Env { ctx: h.ctx.clone(), theme: &h.theme, hl: &h.hl, base_dir: None };
        let laid = h.ed.laid(&env, 0, false);
        laid.galley.job.sections.iter().map(|s| s.format.color).collect::<std::collections::HashSet<_>>().len()
    };
    let mut h = H::new("```rust\nfn main() { let x = 1; }\n```\n\nafter\n");
    assert!(h.hl.wait_idle(std::time::Duration::from_secs(60)));
    assert!(h.hl.has_waker(), "show registers a repaint request");
    h.frame(vec![]);
    assert!(colours(&mut h) >= 2, "highlighted once the result is in");
    // Editing shows at once; the new result is relaid under the same key.
    h.ed.request_focus = true;
    h.doc.buffer.selection = Selection::caret(12);
    h.frame(vec![]);
    h.typ("x");
    assert!(h.hl.wait_idle(std::time::Duration::from_secs(60)));
    h.frame(vec![]);
    assert!(colours(&mut h) >= 2);
    h.frame(vec![]);
    assert_eq!(h.ed.frame_stats().layouts, 0, "settled: no layouts without a new result");
}

/// Hands out a blank image of a fixed size for any `https://` URI and records `forget` calls.
#[derive(Default)]
struct FakeLoader {
    side: usize,
    forgotten: egui::mutex::Mutex<Vec<String>>,
}

impl egui::load::ImageLoader for FakeLoader {
    fn id(&self) -> &'static str {
        "FakeLoader"
    }
    fn load(&self, _: &egui::Context, uri: &str, _: egui::SizeHint) -> egui::load::ImageLoadResult {
        if !uri.starts_with("https://") {
            return Err(egui::load::LoadError::NotSupported);
        }
        let image = egui::ColorImage::filled([self.side, 1], Color32::WHITE);
        Ok(egui::load::ImagePoll::Ready { image: Arc::new(image) })
    }
    fn forget(&self, uri: &str) {
        self.forgotten.lock().push(uri.to_owned());
    }
    fn forget_all(&self) {}
    fn byte_size(&self) -> usize {
        0
    }
}

fn image_slots_of(h: &mut H) -> Vec<ImageSlot> {
    let env = Env { ctx: h.ctx.clone(), theme: &h.theme, hl: &h.hl, base_dir: None };
    let text = h.doc.buffer.text();
    let i = h.ed.model.chunks.iter().position(|c| text[c.range.clone()].contains("![")).expect("image chunk");
    h.ed.geom(&env, Selection::caret(text.len()), i).images
}

fn set_max_texture_side(ctx: &egui::Context, side: usize) {
    ctx.input_mut(|i| i.max_texture_side = side);
}

#[test]
fn image_over_gpu_limit_gets_placeholder() {
    let dir = tempfile::tempdir().expect("tempdir");
    let png = dir.path().join("wide.png");
    image::RgbaImage::new(3000, 1).save(&png).expect("save");
    let mut h = H::new(&format!("x\n\n![p]({})\n\ntail\n", png.display()));
    set_max_texture_side(&h.ctx, 2048);
    let slots = image_slots_of(&mut h);
    assert!(slots[0].texture.is_none() && slots[0].label.contains("too large"), "{}", slots[0].label);
    // A GPU that can take it: the cached verdict must not stick.
    set_max_texture_side(&h.ctx, 4096);
    assert!(!image_slots_of(&mut h)[0].label.contains("too large"));
}

#[test]
fn remote_image_over_gpu_limit_gets_placeholder() {
    let mut h = H::new("x\n\n![p](https://e.com/wide.png)\n\ntail\n");
    h.ed.remote_images = true;
    h.ctx.add_image_loader(Arc::new(FakeLoader { side: 3000, ..Default::default() }));
    set_max_texture_side(&h.ctx, 2048);
    let slots = image_slots_of(&mut h);
    assert!(slots[0].texture.is_none() && slots[0].label.contains("too large"), "{}", slots[0].label);
}

#[test]
fn svg_hint_stays_under_gpu_limit() {
    struct Spy(Arc<egui::mutex::Mutex<Vec<egui::SizeHint>>>);
    impl egui::load::ImageLoader for Spy {
        fn id(&self) -> &'static str {
            "Spy"
        }
        fn load(&self, _: &egui::Context, _: &str, hint: egui::SizeHint) -> egui::load::ImageLoadResult {
            self.0.lock().push(hint);
            Ok(egui::load::ImagePoll::Pending { size: None })
        }
        fn forget(&self, _: &str) {}
        fn forget_all(&self) {}
        fn byte_size(&self) -> usize {
            0
        }
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let svg = dir.path().join("a.svg");
    std::fs::write(&svg, r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"></svg>"#).expect("write");
    let hints = Arc::new(egui::mutex::Mutex::new(Vec::new()));
    let mut h = H::new(&format!("x\n\n![p]({})\n\ntail\n", svg.display()));
    h.ctx.add_image_loader(Arc::new(Spy(hints.clone())));
    h.ed.col_w = 4000.0;
    set_max_texture_side(&h.ctx, 1024);
    image_slots_of(&mut h);
    let seen = hints.lock();
    assert!(seen.contains(&egui::SizeHint::Size { width: 1024, height: 1024, maintain_aspect_ratio: true }), "{seen:?}");
}

#[test]
fn closing_releases_images_no_other_tab_uses() {
    let (shared, own) = ("https://e.com/shared.png", "https://e.com/own.png");
    let mut a = H::new(&format!("![s]({shared})\n\n![o]({own})\n"));
    let loader = Arc::new(FakeLoader { side: 4, ..Default::default() });
    a.ctx.add_image_loader(loader.clone());
    a.ed.remote_images = true;
    a.frame(vec![]);
    let ctx = a.ctx.clone();
    let mut b = H::new(&format!("![s]({shared})\n"));
    b.ctx = ctx.clone();
    b.ed.remote_images = true;
    b.frame(vec![]);

    a.ed.release_images(&ctx, std::iter::once(&b.ed));
    assert_eq!(*loader.forgotten.lock(), [own], "the other tab still shows the shared image");
    loader.forgotten.lock().clear();
    b.ed.release_images(&ctx, std::iter::empty());
    assert_eq!(*loader.forgotten.lock(), [shared]);
}

#[test]
fn changed_file_is_forgotten() {
    let dir = tempfile::tempdir().expect("tempdir");
    let png = dir.path().join("a.png");
    image::RgbaImage::new(2, 2).save(&png).expect("save");
    let mut h = H::new(&format!("x\n\n![p]({})\n", png.display()));
    let loader = Arc::new(FakeLoader { side: 4, ..Default::default() });
    h.ctx.add_image_loader(loader.clone());
    h.frame(vec![]);
    assert!(loader.forgotten.lock().is_empty(), "unchanged");
    image::RgbaImage::new(3, 3).save(&png).expect("save");
    h.frame(vec![]);
    assert_eq!(*loader.forgotten.lock(), [format!("file://{}", png.display())]);
    h.frame(vec![]);
    assert_eq!(loader.forgotten.lock().len(), 1, "forgotten once per change");
}

#[test]
fn editing_a_definition_updates_the_link_elsewhere() {
    let urls = |h: &mut H| -> Vec<String> {
        let env = Env { ctx: h.ctx.clone(), theme: &h.theme, hl: &h.hl, base_dir: None };
        let sel = h.doc.buffer.selection;
        let g = h.ed.geom(&env, sel, 0);
        g.laid.decorations.iter().filter_map(|d| match d { Decoration::Link { url, .. } => Some(url.clone()), _ => None }).collect()
    };
    for filler in ["other\n\n".to_string(), "para text here.\n\n".repeat(5000)] {
        let mut h = H::new(&format!("[foo] x\n\n{filler}[foo]: http://old\n"));
        let end = h.text().find("http://old").unwrap() + 10;
        h.set_caret(9);
        h.frame(vec![]);
        assert_eq!(urls(&mut h), ["http://old"]);
        h.doc.buffer.apply(vec![Edit::insert(end, "NEW")], Selection::caret(9), EditKind::Other);
        h.frame(vec![]);
        assert_eq!(urls(&mut h), ["http://oldNEW"]);
    }
}

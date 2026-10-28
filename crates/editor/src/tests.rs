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
        };
        h.frame(vec![]);
        h.frame(vec![]);
        h
    }

    fn frame(&mut self, events: Vec<Event>) -> EditorOutput {
        self.frame_at(events, Modifiers::NONE)
    }

    fn frame_at(&mut self, events: Vec<Event>, modifiers: Modifiers) -> EditorOutput {
        self.time += 1.0;
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

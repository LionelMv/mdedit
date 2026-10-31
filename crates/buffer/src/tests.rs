use super::*;
use proptest::prelude::*;

fn typed(b: &mut Buffer, s: &str) {
    for c in s.chars() {
        b.insert_at_selection(c.encode_utf8(&mut [0; 4]), EditKind::Typing);
    }
}

#[test]
fn slice_and_line_col() {
    let b = Buffer::new("ab\nçd\n");
    assert_eq!(b.slice(0..2), "ab");
    assert_eq!(b.slice(3..5), "ç");
    assert_eq!(b.slice(4..100), "çd\n"); // 4 is mid-char: rounded down to 3
    assert_eq!(b.slice(Range { start: 5, end: 2 }), ""); // reversed
    assert_eq!(b.line_col(0), (0, 0));
    assert_eq!(b.line_col(3), (1, 0));
    assert_eq!(b.line_col(5), (1, 1));
    assert_eq!(b.line_col(6), (1, 2));
    assert_eq!(b.line_col(7), (2, 0));
    assert_eq!(b.line_col(999), (2, 0));
    assert_eq!(b.clamp_offset(4), 3);
}

#[test]
fn apply_multi_any_order_and_undo_redo() {
    let mut b = Buffer::new("hello world");
    b.selection = Selection { anchor: 0, head: 5 };
    let v0 = b.version();
    b.apply(
        vec![Edit::replace(6..11, "there"), Edit::insert(0, ">> "), Edit::delete(5..6)],
        Selection::caret(3),
        EditKind::Other,
    );
    assert_eq!(b.text(), ">> hellothere");
    assert_eq!(b.selection, Selection::caret(3));
    assert!(b.version() > v0);
    assert!(b.can_undo() && !b.can_redo());
    let v1 = b.version();
    assert!(b.undo());
    assert_eq!(b.text(), "hello world");
    assert_eq!(b.selection, Selection { anchor: 0, head: 5 });
    assert!(b.version() > v1);
    assert!(b.can_redo() && !b.can_undo());
    assert!(!b.undo());
    assert!(b.redo());
    assert_eq!(b.text(), ">> hellothere");
    assert_eq!(b.selection, Selection::caret(3));
    assert!(!b.redo());
}

#[test]
fn noop_edits_create_no_history() {
    let mut b = Buffer::new("abc");
    let v = b.version();
    b.apply(vec![Edit::insert(1, ""), Edit::replace(0..1, "a")], Selection::caret(2), EditKind::Other);
    assert_eq!(b.version(), v);
    assert!(!b.can_undo());
    assert_eq!(b.selection, Selection::caret(2));
    b.apply(vec![], Selection::caret(99), EditKind::Other);
    assert_eq!(b.selection, Selection::caret(3));
}

#[test]
fn clamps_out_of_range_and_mid_char() {
    let mut b = Buffer::new("é");
    b.apply(vec![Edit::insert(1, "x")], Selection::caret(1), EditKind::Other);
    assert_eq!(b.text(), "xé");
    b.apply(vec![Edit::replace(50..60, "!")], Selection::caret(1000), EditKind::Other);
    assert_eq!(b.text(), "xé!");
    assert_eq!(b.selection, Selection::caret(4));
    // Overlapping edits (contract violation) don't panic.
    b.apply(vec![Edit::delete(0..3), Edit::delete(1..4)], Selection::caret(0), EditKind::Other);
    assert_eq!(b.text(), "");
    b.undo();
    assert_eq!(b.text(), "xé!");
}

#[test]
fn typing_merges_and_breaks_on_whitespace() {
    let mut b = Buffer::new("");
    typed(&mut b, "hello world");
    assert_eq!(b.text(), "hello world");
    assert_eq!(b.undo_depth(), 2);
    b.undo();
    assert_eq!(b.text(), "hello");
    assert_eq!(b.selection, Selection::caret(5));
    b.undo();
    assert_eq!(b.text(), "");
    b.redo();
    b.redo();
    assert_eq!(b.text(), "hello world");
    assert_eq!(b.selection, Selection::caret(11));
}

#[test]
fn typing_not_adjacent_or_broken_starts_new_step() {
    let mut b = Buffer::new("");
    typed(&mut b, "ab");
    b.selection = Selection::caret(0);
    typed(&mut b, "x");
    assert_eq!(b.undo_depth(), 2);
    b.break_undo_group();
    typed(&mut b, "y");
    assert_eq!(b.undo_depth(), 3);
    assert_eq!(b.text(), "xyab");
    b.apply(vec![Edit::insert(4, "!")], Selection::caret(5), EditKind::Other);
    b.apply(vec![Edit::insert(5, "!")], Selection::caret(6), EditKind::Other);
    assert_eq!(b.undo_depth(), 5);
}

#[test]
fn typing_after_undo_does_not_merge() {
    let mut b = Buffer::new("");
    typed(&mut b, "ab");
    b.undo();
    typed(&mut b, "c");
    typed(&mut b, "d");
    assert_eq!(b.undo_depth(), 1);
    assert!(!b.can_redo());
    b.undo();
    assert_eq!(b.text(), "");
}

#[test]
fn typing_over_selection_then_merges() {
    let mut b = Buffer::new("hello");
    b.selection = Selection { anchor: 0, head: 5 };
    typed(&mut b, "bye");
    assert_eq!(b.text(), "bye");
    assert_eq!(b.undo_depth(), 1);
    b.undo();
    assert_eq!(b.text(), "hello");
    assert_eq!(b.selection, Selection { anchor: 0, head: 5 });
}

#[test]
fn deleting_merges_backspace_and_delete() {
    let mut b = Buffer::new("abcdef");
    b.selection = Selection::caret(4);
    for _ in 0..3 {
        let at = b.selection.head;
        b.apply(vec![Edit::delete(at - 1..at)], Selection::caret(at - 1), EditKind::Deleting);
    }
    assert_eq!(b.text(), "aef");
    assert_eq!(b.undo_depth(), 1);
    // Forward delete at the same point merges too.
    b.apply(vec![Edit::delete(1..2)], Selection::caret(1), EditKind::Deleting);
    assert_eq!(b.text(), "af");
    assert_eq!(b.undo_depth(), 1);
    // Typing never merges with deleting.
    typed(&mut b, "z");
    assert_eq!(b.undo_depth(), 2);
    b.undo();
    b.undo();
    assert_eq!(b.text(), "abcdef");
    assert_eq!(b.selection, Selection::caret(4));
}

#[test]
fn typing_merge_window_expires() {
    let mut b = Buffer::new("");
    typed(&mut b, "a");
    std::thread::sleep(history::MERGE_WINDOW + std::time::Duration::from_millis(100));
    typed(&mut b, "b");
    assert_eq!(b.undo_depth(), 2);
}

#[test]
fn history_capped() {
    let mut b = Buffer::new("");
    for i in 0..(history::MAX_STEPS + 50) {
        b.apply(vec![Edit::insert(i, "x")], Selection::caret(i + 1), EditKind::Other);
    }
    assert_eq!(b.undo_depth(), history::MAX_STEPS);
    while b.undo() {}
    assert_eq!(b.len_bytes(), 50);
}

#[test]
fn history_byte_cap() {
    let mut b = Buffer::new("");
    let chunk = "y".repeat(history::MAX_BYTES / 4);
    for _ in 0..6 {
        let end = b.len_bytes();
        b.apply(vec![Edit::insert(end, chunk.clone())], Selection::caret(end), EditKind::Other);
    }
    assert!(b.undo_depth() < 6 && b.undo_depth() >= 1);
}

#[test]
fn set_text_clears_history() {
    let mut b = Buffer::new("a");
    typed(&mut b, "b");
    let v = b.version();
    b.set_text("zzz");
    assert!(!b.can_undo());
    assert!(b.version() > v);
}

#[derive(Debug, Clone)]
enum Op {
    Apply(Vec<(usize, usize, String)>, EditKind),
    Undo,
    Redo,
    Break,
}

fn kind() -> impl Strategy<Value = EditKind> {
    prop_oneof![Just(EditKind::Typing), Just(EditKind::Deleting), Just(EditKind::Other)]
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        6 => (proptest::collection::vec((0usize..40, 0usize..6, "[a-z é👍\n]{0,4}"), 1..4), kind()).prop_map(|(e, k)| Op::Apply(e, k)),
        2 => Just(Op::Undo),
        1 => Just(Op::Redo),
        1 => Just(Op::Break),
    ]
}

/// Naive model: clamp + round like the buffer, drop overlaps, apply on a String from the back.
fn naive_apply(text: &str, edits: &[(usize, usize, String)]) -> String {
    let floor = |mut i: usize| {
        i = i.min(text.len());
        while !text.is_char_boundary(i) {
            i -= 1;
        }
        i
    };
    let mut es: Vec<(usize, usize, &str)> = edits
        .iter()
        .map(|(s, l, ins)| {
            let a = floor(*s);
            (a, floor(s + l).max(a), ins.as_str())
        })
        .collect();
    es.sort_by_key(|e| (e.0, e.1));
    let mut prev = 0;
    for e in &mut es {
        if e.0 < prev {
            e.0 = prev;
            e.1 = e.1.max(prev);
        }
        prev = e.1;
    }
    let mut out = text.to_owned();
    for (a, b, ins) in es.iter().rev() {
        out.replace_range(*a..*b, ins);
    }
    out
}

proptest! {
    #[test]
    fn random_ops_undo_all_restores(init in "[a-z é👍\n]{0,30}", ops in proptest::collection::vec(op(), 0..40)) {
        let mut b = Buffer::new(&init);
        let mut last_version = b.version();
        for o in ops {
            match o {
                Op::Apply(edits, k) => {
                    let before = b.text();
                    let expected = naive_apply(&before, &edits);
                    let es = edits.into_iter().map(|(s, l, t)| Edit::replace(s..s + l, t)).collect();
                    b.apply(es, Selection::caret(3), k);
                    prop_assert_eq!(b.text(), expected.clone());
                    if expected != before { prop_assert!(b.version() > last_version); }
                }
                Op::Undo => { if b.undo() { prop_assert!(b.version() > last_version); } }
                Op::Redo => { if b.redo() { prop_assert!(b.version() > last_version); } }
                Op::Break => b.break_undo_group(),
            }
            prop_assert!(b.text().is_char_boundary(b.selection.head.min(b.len_bytes())));
            prop_assert!(b.selection.head <= b.len_bytes() && b.selection.anchor <= b.len_bytes());
            last_version = b.version();
        }
        while b.undo() {}
        prop_assert_eq!(b.text(), init);
    }

    #[test]
    fn redo_all_restores_final(init in "[a-z é\n]{0,20}", ops in proptest::collection::vec(op(), 0..30)) {
        let mut b = Buffer::new(&init);
        for o in ops {
            if let Op::Apply(edits, k) = o {
                let es = edits.into_iter().map(|(s, l, t)| Edit::replace(s..s + l, t)).collect();
                b.apply(es, Selection::caret(0), k);
            }
        }
        let fin = b.text();
        let sel = b.selection;
        while b.undo() {}
        prop_assert_eq!(b.text(), init);
        while b.redo() {}
        prop_assert_eq!(b.text(), fin);
        prop_assert_eq!(b.selection, sel);
    }

    #[test]
    fn typing_chars_undo_restores(init in "[a-z ]{0,10}", s in "[a-zé👍 \n]{0,30}", at in 0usize..12) {
        let mut b = Buffer::new(&init);
        b.selection = Selection::caret(b.clamp_offset(at));
        typed(&mut b, &s);
        while b.undo() {}
        prop_assert_eq!(b.text(), init);
    }
}

#[test]
fn document_title_prefers_path_then_name() {
    let mut d = Document::new();
    assert_eq!(d.title(), "Untitled");
    d.name = Some("Markdown Cheatsheet".into());
    assert_eq!(d.title(), "Markdown Cheatsheet");
    d.path = Some("/tmp/notes.md".into());
    assert_eq!(d.title(), "notes.md");
}

#[test]
fn find_up_to_goes_past_the_painting_cap() {
    let big = "a".repeat(crate::MAX_MATCHES + 10);
    assert_eq!(crate::find_all(&big, "a", &crate::SearchOptions::default()).unwrap().len(), crate::MAX_MATCHES);
    assert_eq!(crate::find_up_to(&big, "a", &crate::SearchOptions::default(), usize::MAX).unwrap().len(), big.len());
}

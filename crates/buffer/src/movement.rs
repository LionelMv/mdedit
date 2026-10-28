//! Grapheme- and word-wise movement over `&str`. STABLE signatures; all return byte offsets.
//!
//! Every function accepts any `at`: it is clamped to `text.len()` and rounded down to a char
//! boundary first, so none of them panic.

use std::ops::Range;

use unicode_segmentation::{GraphemeCursor, UnicodeSegmentation};

/// Clamp `at` to the text and round down to a char boundary.
fn clamp(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Class {
    Space,
    Word,
    Punct,
}

fn class_of(grapheme: &str) -> Class {
    match grapheme.chars().next() {
        None => Class::Space,
        Some(c) if c.is_whitespace() => Class::Space,
        Some(c) if c.is_alphanumeric() || c == '_' => Class::Word,
        // Emoji and other symbols count as punctuation-like runs.
        Some(_) => Class::Punct,
    }
}

/// Byte offset of the next grapheme boundary after `at` (or `text.len()`).
pub fn next_grapheme(text: &str, at: usize) -> usize {
    let at = clamp(text, at);
    if at >= text.len() {
        return text.len();
    }
    let mut cur = GraphemeCursor::new(at, text.len(), true);
    match cur.next_boundary(text, 0) {
        Ok(Some(b)) => b,
        _ => text[at..].chars().next().map_or(text.len(), |c| at + c.len_utf8()),
    }
}

/// Byte offset of the previous grapheme boundary before `at` (or 0).
pub fn prev_grapheme(text: &str, at: usize) -> usize {
    let at = clamp(text, at);
    if at == 0 {
        return 0;
    }
    let mut cur = GraphemeCursor::new(at, text.len(), true);
    match cur.prev_boundary(text, 0) {
        Ok(Some(b)) => b,
        _ => text[..at].chars().next_back().map_or(0, |c| at - c.len_utf8()),
    }
}

/// Ctrl+Right: end of the next word. Skips whitespace, then a run of word characters or a run
/// of punctuation.
pub fn next_word(text: &str, at: usize) -> usize {
    let at = clamp(text, at);
    let mut iter = text[at..].grapheme_indices(true).skip_while(|(_, g)| class_of(g) == Class::Space).peekable();
    let Some(&(first_i, first)) = iter.peek() else { return text.len() };
    let class = class_of(first);
    let mut end = at + first_i;
    for (i, g) in iter {
        if class_of(g) != class {
            break;
        }
        end = at + i + g.len();
    }
    end
}

/// Ctrl+Left: start of the previous word. Skips whitespace backwards, then a run of word
/// characters or a run of punctuation.
pub fn prev_word(text: &str, at: usize) -> usize {
    let at = clamp(text, at);
    let mut iter = text[..at].grapheme_indices(true).rev().skip_while(|(_, g)| class_of(g) == Class::Space).peekable();
    let Some(&(_, first)) = iter.peek() else { return 0 };
    let class = class_of(first);
    let mut start = at;
    for (i, g) in iter {
        if class_of(g) != class {
            break;
        }
        start = i;
    }
    start
}

/// Extend from `at` over graphemes of `class` in both directions.
fn run_around(text: &str, at: usize, class: Class) -> Range<usize> {
    let mut end = at;
    for (i, g) in text[at..].grapheme_indices(true) {
        if class_of(g) != class {
            break;
        }
        end = at + i + g.len();
    }
    let mut start = at;
    for (i, g) in text[..at].grapheme_indices(true).rev() {
        if class_of(g) != class {
            break;
        }
        start = i;
    }
    start..end
}

/// The word around `at` (double-click). Empty range at `at` if none.
///
/// Prefers a word (alphanumeric/`_`) touching `at` on either side, then a punctuation run;
/// whitespace yields an empty range.
pub fn word_at(text: &str, at: usize) -> Range<usize> {
    let at = clamp(text, at);
    // Snap to a grapheme boundary so we never split a cluster.
    let at = if at == 0 || at == text.len() || next_grapheme(text, prev_grapheme(text, at)) == at {
        at
    } else {
        prev_grapheme(text, at)
    };
    let after = text[at..].graphemes(true).next().map(class_of);
    let before = text[..at].graphemes(true).next_back().map(class_of);
    for class in [Class::Word, Class::Punct] {
        if after == Some(class) || before == Some(class) {
            return run_around(text, at, class);
        }
    }
    at..at
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graphemes() {
        let s = "a\u{301}b👨‍👩‍👧c";
        assert_eq!(next_grapheme(s, 0), 3);
        assert_eq!(next_grapheme(s, 3), 4);
        let fam_end = 4 + "👨‍👩‍👧".len();
        assert_eq!(next_grapheme(s, 4), fam_end);
        assert_eq!(prev_grapheme(s, fam_end), 4);
        assert_eq!(prev_grapheme(s, 3), 0);
        assert_eq!(prev_grapheme(s, 0), 0);
        assert_eq!(next_grapheme(s, s.len()), s.len());
        assert_eq!(next_grapheme(s, 999), s.len());
        // Mid-char offset: rounded down, no panic.
        assert_eq!(next_grapheme("é", 1), 2);
        assert_eq!(prev_grapheme("é", 1), 0);
        assert_eq!(next_grapheme("\r\n", 0), 2);
    }

    #[test]
    fn words_forward() {
        let s = "foo  bar.baz, qux";
        assert_eq!(next_word(s, 0), 3);
        assert_eq!(next_word(s, 3), 8);
        assert_eq!(next_word(s, 8), 9);
        assert_eq!(next_word(s, 9), 12);
        assert_eq!(next_word(s, 12), 13);
        assert_eq!(next_word(s, 13), 17);
        assert_eq!(next_word(s, 17), 17);
        assert_eq!(next_word("   ", 0), 3);
        assert_eq!(next_word("a == b", 1), 4);
    }

    #[test]
    fn words_backward() {
        let s = "foo  bar.baz, qux";
        assert_eq!(prev_word(s, 17), 14);
        assert_eq!(prev_word(s, 14), 12);
        assert_eq!(prev_word(s, 12), 9);
        assert_eq!(prev_word(s, 9), 8);
        assert_eq!(prev_word(s, 8), 5);
        assert_eq!(prev_word(s, 5), 0);
        assert_eq!(prev_word(s, 0), 0);
        assert_eq!(prev_word("   ", 3), 0);
    }

    #[test]
    fn word_at_cases() {
        let s = "hello, wörld_x  **bold**";
        assert_eq!(word_at(s, 0), 0..5);
        assert_eq!(word_at(s, 3), 0..5);
        assert_eq!(word_at(s, 5), 0..5); // end of word preferred over punctuation
        assert_eq!(word_at(s, 6), 5..6); // between ',' and ' '
        let w = s.find('w').unwrap();
        assert_eq!(word_at(s, w + 2), w..w + "wörld_x".len());
        let sp = w + "wörld_x".len() + 1;
        assert_eq!(word_at(s, sp), sp..sp);
        assert_eq!(word_at(s, sp + 1), sp + 1..sp + 3);
        assert_eq!(word_at("", 0), 0..0);
        assert_eq!(word_at("é", 1), 0..2);
    }

    #[test]
    fn combining_marks_stay_in_words() {
        let s = "cafe\u{301} ok";
        assert_eq!(next_word(s, 0), "cafe\u{301}".len());
        assert_eq!(word_at(s, 1), 0.."cafe\u{301}".len());
    }

    proptest::proptest! {
        #[test]
        fn never_panics(s in "(\\PC|\u{200D}|\u{301}|👨‍👩‍👧|\r\n| ){0,24}", at in 0usize..200) {
            let n = next_grapheme(&s, at);
            proptest::prop_assert!(s.is_char_boundary(n));
            let p = prev_grapheme(&s, at);
            proptest::prop_assert!(s.is_char_boundary(p));
            let nw = next_word(&s, at);
            proptest::prop_assert!(s.is_char_boundary(nw) && nw >= at.min(s.len()).saturating_sub(3));
            let pw = prev_word(&s, at);
            proptest::prop_assert!(s.is_char_boundary(pw) && pw <= at.min(s.len()));
            let r = word_at(&s, at);
            proptest::prop_assert!(r.start <= r.end && s.is_char_boundary(r.start) && s.is_char_boundary(r.end));
            if at < s.len() && s.is_char_boundary(at) {
                proptest::prop_assert!(n > at);
            }
            if at > 0 && at <= s.len() && s.is_char_boundary(at) {
                proptest::prop_assert!(p < at);
            }
        }

        #[test]
        fn grapheme_walk_terminates(s in "(\\PC|\u{200D}|\u{301}){0,32}") {
            let mut at = 0;
            let mut steps = 0;
            while at < s.len() { at = next_grapheme(&s, at); steps += 1; }
            proptest::prop_assert_eq!(steps, s.graphemes(true).count());
            while at > 0 { at = prev_grapheme(&s, at); steps -= 1; }
            proptest::prop_assert_eq!(steps, 0);
        }
    }
}

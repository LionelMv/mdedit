//! Incremental re-parse (architecture §5.1.2): after an edit, re-parse only the top-level
//! blocks around the change and shift the others, falling back to a full [`parse`] whenever
//! the boundaries of the re-parsed region might not be stable, or the region could depend on a
//! link reference or footnote definition elsewhere.

use crate::{Block, BlockKind, parse};

/// Below this size a full parse is cheap enough that the bookkeeping isn't worth it.
const MIN_INCREMENTAL: usize = 64 * 1024;

/// Blocks of `new_text`, given the blocks of `old_text` (from [`parse`] or `reparse`).
/// Always equal to `parse(new_text)`.
pub fn reparse(old_text: &str, old: &[Block], new_text: &str) -> Vec<Block> {
    let mut blocks = old.to_vec();
    reparse_in_place(old_text, &mut blocks, new_text);
    blocks
}

/// [`reparse`] updating `blocks` (the blocks of `old_text`) in place, without copying the
/// unchanged blocks: only the re-parsed region is replaced and the tail shifted.
pub fn reparse_in_place(old_text: &str, blocks: &mut Vec<Block>, new_text: &str) {
    let (o, n) = (old_text.as_bytes(), new_text.as_bytes());
    let mut p = o.iter().zip(n).take_while(|(a, b)| a == b).count();
    while !new_text.is_char_boundary(p) || !old_text.is_char_boundary(p) {
        p -= 1;
    }
    let max_s = o.len().min(n.len()) - p;
    let mut s = o.iter().rev().zip(n.iter().rev()).take(max_s).take_while(|(a, b)| a == b).count();
    while !old_text.is_char_boundary(o.len() - s) || !new_text.is_char_boundary(n.len() - s) {
        s -= 1;
    }
    let (old_end, new_end) = (o.len() - s, n.len() - s);
    let removed_definition = touches_definition(old_text, p, old_end);
    let has_definitions = may_have_definition(new_text);
    let valid_blocks = blocks.len();
    let change = TextChange { start: p, old_end, new_end, old_len: o.len(), removed_definition, has_definitions, valid_blocks };
    reparse_change(blocks, new_text, &change);
}

/// One edit, as the byte range that differs. Lets a caller update its text in place and re-parse
/// without keeping (or scanning) a copy of the old text.
#[derive(Clone, Debug)]
pub struct TextChange {
    pub start: usize,
    /// End of the replaced text in the old text.
    pub old_end: usize,
    /// End of the replacement in the new text.
    pub new_end: usize,
    pub old_len: usize,
    /// [`touches_definition`] of the old text around the change.
    pub removed_definition: bool,
    /// [`may_have_definition`] of the document. A caller can keep this from one edit to the
    /// next: it only changes through an edit that [`touches_definition`], which re-parses fully.
    pub has_definitions: bool,
    /// Blocks before this index have correct ranges (they must reach two blocks past `old_end`);
    /// later ones are owed a shift by the caller and are only moved along, not read or shifted.
    pub valid_blocks: usize,
}

/// What [`reparse_change`] did to the blocks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reparsed {
    /// Nothing changed.
    Unchanged,
    /// All blocks were rebuilt.
    Full,
    /// `old` (indices before the edit) became `new_len` blocks. Later blocks only moved: the
    /// valid ones have been shifted, the caller owes the others the length change.
    Spliced { old: std::ops::Range<usize>, new_len: usize },
}

/// Whether the lines around `start..end` hold a `]:`, so the edit could change a link reference
/// or footnote definition (one line either side: a definition's title or destination can be on
/// its own line), which resolves links anywhere in the document.
pub fn touches_definition(text: &str, start: usize, end: usize) -> bool {
    let a = crate::line_start(text, start);
    let a = crate::line_start(text, a.saturating_sub(1));
    let b = crate::line_end(text, end);
    let b = crate::line_end(text, (b + 1).min(text.len()));
    text[a..b].contains("]:")
}

/// Whether `text` may hold a link reference or footnote definition: a `]:` that isn't obviously
/// something else like `a[0]: x`, whose line has text before its `[`.
pub fn may_have_definition(text: &str) -> bool {
    text.match_indices("]:").any(|(at, _)| {
        let line = &text[crate::line_start(text, at)..at];
        let lead = line.trim_start_matches([' ', '\t', '>', '-', '*', '+']);
        let lead = lead.trim_start_matches(|c: char| c.is_ascii_digit() || matches!(c, '.' | ')' | ' ' | '\t'));
        // A label can also run over several lines, ending on a line with no `[` at all.
        lead.starts_with('[') || !line.contains('[')
    })
}

/// [`reparse_in_place`] given the change: `blocks` are the blocks of the old text.
pub fn reparse_change(blocks: &mut Vec<Block>, new_text: &str, change: &TextChange) -> Reparsed {
    let full = |blocks: &mut Vec<Block>| {
        *blocks = parse(new_text);
        Reparsed::Full
    };
    if blocks.is_empty() || new_text.len() < MIN_INCREMENTAL || change.old_len < MIN_INCREMENTAL {
        return full(blocks);
    }
    let (p, old_end, new_end) = (change.start, change.old_end, change.new_end);
    if p == old_end && p == new_end {
        return Reparsed::Unchanged;
    }
    if change.removed_definition || touches_definition(new_text, p, new_end) {
        return full(blocks);
    }
    let delta = new_end as isize - old_end as isize;
    let valid = change.valid_blocks.min(blocks.len());

    // One block before the one holding the change (setext underlines, lazy lines merge
    // upwards) ...
    let i = blocks[..valid].partition_point(|b| b.range.start <= p).saturating_sub(2);
    // ... and one block after the last changed one, which must come back unchanged.
    let j = blocks[..valid].partition_point(|b| b.range.start <= old_end);
    if j + 1 >= valid && valid < blocks.len() {
        return full(blocks);
    }
    // Edits in the last blocks re-parse through to the end of the text.
    let to_end = j + 1 >= blocks.len();
    let start = if i == 0 { 0 } else { blocks[i].range.start };
    let end_new = if to_end { new_text.len() } else { (blocks[j + 1].range.start as isize + delta) as usize };
    if start >= end_new || !new_text.is_char_boundary(end_new) {
        return full(blocks);
    }
    // Parsed on its own, this region can't see definitions elsewhere in the document.
    if change.has_definitions && new_text[start..end_new].contains('[') {
        return full(blocks);
    }
    let mut mid = parse(&new_text[start..end_new]);
    for b in &mut mid {
        shift(b, start as isize);
    }
    let boundary_ok = to_end || {
        let mut expect = blocks[j].clone();
        shift(&mut expect, delta);
        mid.last() == Some(&expect)
    };
    let stable = boundary_ok && !(start > 0 && mid.iter().any(|b| matches!(b.kind, BlockKind::FrontMatter)));
    if !stable {
        return full(blocks);
    }
    let replaced = if to_end { i..blocks.len() } else { i..j + 1 };
    let new_len = mid.len();
    let tail_from = i + new_len;
    let tail_to = valid + new_len - replaced.len();
    blocks.splice(replaced.clone(), mid);
    for b in &mut blocks[tail_from..tail_to] {
        shift(b, delta);
    }
    Reparsed::Spliced { old: replaced, new_len }
}

fn sh(r: &mut std::ops::Range<usize>, d: isize) {
    r.start = (r.start as isize + d) as usize;
    r.end = (r.end as isize + d) as usize;
}

/// Move every byte range in `b` (recursively) by `d`.
pub fn shift(b: &mut Block, d: isize) {
    if d == 0 {
        return;
    }
    sh(&mut b.range, d);
    sh(&mut b.content, d);
    for s in &mut b.inlines {
        sh(&mut s.range, d);
        sh(&mut s.content, d);
    }
    match &mut b.kind {
        BlockKind::ListItem { checkbox: Some(c), .. } => sh(c, d),
        BlockKind::CodeBlock(info) => {
            if let Some(r) = &mut info.open_fence {
                sh(r, d);
            }
            if let Some(r) = &mut info.close_fence {
                sh(r, d);
            }
        }
        BlockKind::Table { rows, .. } => {
            for row in rows {
                sh(&mut row.range, d);
                for c in &mut row.cells {
                    sh(c, d);
                }
            }
        }
        _ => {}
    }
    for c in &mut b.children {
        shift(c, d);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn doc(n: usize) -> String {
        let mut s = String::new();
        for i in 0..n {
            s += &format!(
                "## Section {i}\n\nSome **bold** and `code` with a [link](http://x/{i}).\nSetext?\n\n- a\n- [ ] b\n\n```rust\nfn f() {{}}\n```\n\n| A | B |\n|---|---|\n| {i} | x |\n\n> quote {i}\n\n"
            );
        }
        s
    }

    fn check(old: &str, new: &str) {
        let ob = parse(old);
        let expect = parse(new);
        assert_eq!(reparse(old, &ob, new), expect, "edit diverged");
        let mut inplace = ob;
        reparse_in_place(old, &mut inplace, new);
        assert_eq!(inplace, expect, "in-place edit diverged");
    }

    #[test]
    fn typing_in_the_middle() {
        let d = doc(800);
        let at = d.len() / 2;
        let at = (at..).find(|&i| d.is_char_boundary(i)).unwrap();
        for ins in ["x", "\n", "\n\n", "**", "```", "```\n", "---\n", "# ", "- ", "> ", "|", "<!--", "$$\n", "    ", "\t", "é"] {
            let mut n = d.clone();
            n.insert_str(at, ins);
            check(&d, &n);
        }
    }

    #[test]
    fn deleting_ranges() {
        let d = doc(800);
        for (a, len) in [(10, 1), (500, 40), (d.len() / 2, 300), (d.len() - 50, 50), (0, 30)] {
            let mut n = d.clone();
            n.replace_range(a..a + len, "");
            check(&d, &n);
        }
    }

    #[test]
    fn typing_at_the_end() {
        let d = doc(800);
        for ins in ["x", "\n", "\n\n# h", "```", "```\ncode", "- a\n- b", "| a |\n|---|"] {
            let mut n = d.clone();
            n.push_str(ins);
            check(&d, &n);
            let mut m = d.clone();
            m.truncate(d.len() - 3);
            check(&d, &m);
        }
    }

    #[test]
    fn opening_a_fence_falls_back_correctly() {
        let d = doc(800);
        let mut n = d.clone();
        n.insert_str(1000, "\n```\n");
        check(&d, &n);
    }

    #[test]
    fn definitions_elsewhere_still_resolve() {
        // A link or footnote used in the edited block, defined far away.
        let tail = "[foo]: http://x\n\n[^n]: note\n";
        let d = format!("See [foo] and [^n] here.\n\n{}{tail}", doc(800));
        let mut n = d.clone();
        n.insert(4, 'x');
        check(&d, &n);
        // Editing a definition itself, away from its `]:`.
        let lab = d.find("[foo]: http").unwrap();
        for at in [lab + 2, lab + 12] {
            let mut n = d.clone();
            n.insert(at, 'x');
            check(&d, &n);
        }
    }

    #[test]
    fn splice_reports_what_changed() {
        let d = doc(800);
        let at = d.len() / 2;
        let at = at + d[at..].find("Setext").unwrap();
        let mut n = d.clone();
        n.insert(at, 'x');
        let mut b = parse(&d);
        let before = b.len();
        let change = TextChange { start: at, old_end: at, new_end: at + 1, old_len: d.len(), removed_definition: false, has_definitions: false, valid_blocks: before };
        match reparse_change(&mut b, &n, &change) {
            Reparsed::Spliced { old, new_len } => {
                assert!(old.len() < 10, "{old:?}");
                assert_eq!(before - old.len() + new_len, b.len());
            }
            other => panic!("not spliced: {other:?}"),
        }
        assert_eq!(b, parse(&n));
    }

    #[test]
    fn blocks_past_valid_are_left_for_the_caller() {
        let d = doc(800);
        let at = d.find("Setext").unwrap();
        let mut n = d.clone();
        n.insert(at, 'x');
        let mut b = parse(&d);
        let valid = 40;
        let change = TextChange { start: at, old_end: at, new_end: at + 1, old_len: d.len(), removed_definition: false, has_definitions: false, valid_blocks: valid };
        let Reparsed::Spliced { old, new_len } = reparse_change(&mut b, &n, &change) else { panic!("not spliced") };
        let valid = valid + new_len - old.len();
        let expect = parse(&n);
        assert_eq!(b[..valid], expect[..valid]);
        for x in &mut b[valid..] {
            shift(x, 1);
        }
        assert_eq!(b, expect);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]
        #[test]
        fn matches_full_parse(pos in 0usize..100_000, del in 0usize..200, ins in "[a-z#*`>\\-|\\[\\]:\n ]{0,12}") {
            let d = doc(800);
            let mut a = pos.min(d.len());
            while !d.is_char_boundary(a) { a -= 1; }
            let mut b = (a + del).min(d.len());
            while !d.is_char_boundary(b) { b -= 1; }
            let mut n = d.clone();
            n.replace_range(a..b, &ins);
            check(&d, &n);
        }
    }
}

#[cfg(test)]
mod regression {
    /// Found by the style crate's fuzzing: a bare `\r` puts the block start before its line's.
    #[test]
    fn code_lines_with_a_bare_cr_before_multibyte() {
        for t in ["日\r~~~", "\u{2028}\r```"] {
            for b in &crate::parse(t) {
                let _ = crate::code_lines(t, b);
            }
        }
    }

    /// Found by the style crate's fuzzing: an inline code span ending in `\r` before a
    /// multi-byte char made `parse` slice inside `é`.
    #[test]
    fn code_span_cr_before_multibyte() {
        for t in ["# ` `\ré\n", "` `\ré", "``a``\r€", "`x`\r\n`é`"] {
            let _ = crate::parse(t);
        }
    }
}

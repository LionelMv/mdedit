//! Incremental re-parse (architecture §5.1.2): after an edit, re-parse only the top-level
//! blocks around the change and shift the others, falling back to a full [`parse`] whenever
//! the boundaries of the re-parsed region might not be stable.

use crate::{Block, BlockKind, parse};

/// Below this size a full parse is cheap enough that the bookkeeping isn't worth it.
const MIN_INCREMENTAL: usize = 64 * 1024;

/// Blocks of `new_text`, given the blocks of `old_text` (from [`parse`] or `reparse`).
/// Always equal to `parse(new_text)` except for link reference definitions far from the
/// edit (handled by falling back to a full parse whenever the edit touches `]:`).
pub fn reparse(old_text: &str, old: &[Block], new_text: &str) -> Vec<Block> {
    let mut blocks = old.to_vec();
    reparse_in_place(old_text, &mut blocks, new_text);
    blocks
}

/// [`reparse`] updating `blocks` (the blocks of `old_text`) in place, without copying the
/// unchanged blocks: only the re-parsed region is replaced and the tail shifted.
pub fn reparse_in_place(old_text: &str, blocks: &mut Vec<Block>, new_text: &str) {
    let full = |blocks: &mut Vec<Block>| *blocks = parse(new_text);
    if blocks.is_empty() || new_text.len() < MIN_INCREMENTAL || old_text.len() < MIN_INCREMENTAL {
        return full(blocks);
    }
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
    if p == old_end && p == new_end {
        return;
    }
    // Reference definitions resolve links anywhere in the document.
    let near = |t: &str, a: usize, b: usize| {
        let a = crate::floor_boundary(t, a.saturating_sub(2));
        let b = crate::floor_boundary(t, (b + 2).min(t.len()));
        t[a..b].contains("]:")
    };
    if near(old_text, p, old_end) || near(new_text, p, new_end) {
        return full(blocks);
    }
    let delta = new_end as isize - old_end as isize;

    // One block before the one holding the change (setext underlines, lazy lines merge
    // upwards) ...
    let i = blocks.partition_point(|b| b.range.start <= p).saturating_sub(2);
    // ... and one block after the last changed one, which must come back unchanged.
    let j = blocks.partition_point(|b| b.range.start <= old_end);
    // Edits in the last blocks re-parse through to the end of the text.
    let to_end = j + 1 >= blocks.len();
    let start = if i == 0 { 0 } else { blocks[i].range.start };
    let end_new = if to_end { new_text.len() } else { (blocks[j + 1].range.start as isize + delta) as usize };
    if start >= end_new || !new_text.is_char_boundary(end_new) {
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
    let tail_from = i + mid.len();
    blocks.splice(replaced, mid);
    for b in &mut blocks[tail_from..] {
        shift(b, delta);
    }
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

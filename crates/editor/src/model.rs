//! Per-version document model: cached text, parsed blocks, display chunks, and the pure
//! byte ⇄ (chunk, char index) mapping. No egui in here (unit-testable).

use std::hash::{Hash, Hasher};
use std::ops::Range;

use mdedit_syntax::Block;

/// Chunks bigger than this are split at line boundaries for display, so a huge code block
/// (or a misbehaving chunker) never produces one gigantic galley.
pub(crate) const MAX_CHUNK_BYTES: usize = 32 * 1024;

/// A display chunk: a `mdedit_style::Chunk`, possibly a piece of a split oversized one, or the
/// virtual empty chunk after a final `\n`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DChunk {
    pub range: Range<usize>,
    pub block: Option<usize>,
}

impl DChunk {
    /// End of the text shown in the galley (excludes the chunk's final `\n`).
    pub fn text_end(&self, text: &str) -> usize {
        if self.range.end > self.range.start && text.as_bytes().get(self.range.end - 1) == Some(&b'\n') {
            self.range.end - 1
        } else {
            self.range.end
        }
    }
}

/// Round `at` down to a char boundary inside `text`.
pub(crate) fn floor_boundary(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while at > 0 && !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// Validate the style crate's chunks (contiguous cover, char boundaries) and turn them into
/// display chunks: oversized chunks are split, and a virtual empty chunk is appended after a
/// final `\n` so the caret has somewhere to live. Falls back to our own line-based chunking if
/// the input chunks are not a valid cover.
pub(crate) fn display_chunks(text: &str, style_chunks: &[mdedit_style::Chunk]) -> Vec<DChunk> {
    let valid = !style_chunks.is_empty()
        && style_chunks[0].range.start == 0
        && style_chunks.last().is_some_and(|c| c.range.end == text.len())
        && style_chunks.windows(2).all(|w| w[0].range.end == w[1].range.start)
        && style_chunks.iter().all(|c| {
            c.range.start <= c.range.end && text.is_char_boundary(c.range.start) && text.is_char_boundary(c.range.end)
        });
    let mut out = Vec::with_capacity(style_chunks.len() + 1);
    if valid {
        for c in style_chunks {
            if c.range.is_empty() && !text.is_empty() {
                continue;
            }
            if c.range.len() > MAX_CHUNK_BYTES {
                split_into(text, c.range.clone(), None, &mut out);
            } else {
                out.push(DChunk { range: c.range.clone(), block: c.block });
            }
        }
    } else if !text.is_empty() {
        split_into(text, 0..text.len(), None, &mut out);
    }
    if out.is_empty() {
        out.push(DChunk { range: 0..0, block: None });
    }
    if text.ends_with('\n') {
        out.push(DChunk { range: text.len()..text.len(), block: None });
    }
    out
}

/// Split `range` into pieces of at most ~MAX_CHUNK_BYTES ending after a `\n` where possible.
fn split_into(text: &str, range: Range<usize>, block: Option<usize>, out: &mut Vec<DChunk>) {
    let bytes = text.as_bytes();
    let mut start = range.start;
    while start < range.end {
        let mut end = (start + MAX_CHUNK_BYTES).min(range.end);
        if end < range.end {
            // Prefer ending just after a newline.
            if let Some(nl) = bytes[start..end].iter().rposition(|&b| b == b'\n') {
                end = start + nl + 1;
            } else {
                end = floor_boundary(text, end);
                if end <= start {
                    end = range.end.min(start + MAX_CHUNK_BYTES * 2);
                    while end < range.end && !text.is_char_boundary(end) {
                        end += 1;
                    }
                }
            }
        }
        out.push(DChunk { range: start..end, block });
        start = end;
    }
}

/// Index of the chunk holding byte `at`. The boundary rule: `at == chunk.end` belongs to the
/// next chunk (the final `\n` of a chunk is not in its galley), except at the end of the text.
pub(crate) fn chunk_at(chunks: &[DChunk], at: usize) -> usize {
    let i = chunks.partition_point(|c| c.range.end <= at);
    i.min(chunks.len().saturating_sub(1))
}

/// Char index inside chunk `c`'s galley for byte `at` (clamped to the galley text).
pub(crate) fn char_in_chunk(text: &str, c: &DChunk, at: usize) -> usize {
    let start = c.range.start.min(text.len());
    let end = c.text_end(text).max(start);
    let at = floor_boundary(text, at.clamp(start, end));
    text[start..at].chars().count()
}

/// Byte offset for galley char index `ci` in chunk `c` (clamped to the galley text end).
pub(crate) fn byte_in_chunk(text: &str, c: &DChunk, ci: usize) -> usize {
    let start = c.range.start.min(text.len());
    let end = c.text_end(text).max(start);
    match text[start..end].char_indices().nth(ci) {
        Some((off, _)) => start + off,
        None => end,
    }
}

/// Char range inside chunk `c` for an absolute byte range (may end at `count + 1` when the
/// range covers the chunk's final `\n`, which lets selection painting extend the last row).
pub(crate) fn char_range_in_chunk(text: &str, c: &DChunk, range: Range<usize>) -> Option<Range<usize>> {
    if range.end < c.range.start || range.start > c.range.end {
        return None;
    }
    if range.is_empty() || range.end <= c.range.start || range.start >= c.range.end {
        return None;
    }
    let s = char_in_chunk(text, c, range.start);
    let te = c.text_end(text);
    let mut e = char_in_chunk(text, c, range.end);
    if range.end > te {
        e += 1; // covers the chunk's newline
    }
    Some(s..e)
}

pub(crate) fn hash_str(s: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// Estimated height of a chunk that hasn't been laid out yet.
pub(crate) fn estimate_height(text: &str, row_h: f32, chars_per_row: f32) -> f32 {
    let cpr = chars_per_row.max(8.0);
    let mut rows = 0.0f32;
    let mut lines = 0usize;
    for line in text.split('\n') {
        lines += 1;
        let n = line.len() as f32; // bytes ≈ chars for an estimate
        rows += (n / cpr).ceil().max(1.0);
    }
    if text.ends_with('\n') && lines > 1 {
        rows -= 1.0;
    }
    rows.max(1.0) * row_h
}

/// A Fenwick tree of chunk heights (f64 to keep 10 MB documents precise).
#[derive(Clone, Debug, Default)]
pub(crate) struct Heights {
    vals: Vec<f64>,
    tree: Vec<f64>,
}

impl Heights {
    pub fn new(vals: Vec<f64>) -> Self {
        let n = vals.len();
        let mut tree = vec![0.0; n + 1];
        for i in 0..n {
            tree[i + 1] += vals[i];
            let j = (i + 1) + ((i + 1).isolate_lowest_one());
            if j <= n {
                tree[j] += tree[i + 1];
            }
        }
        Self { vals, tree }
    }
    pub fn len(&self) -> usize {
        self.vals.len()
    }
    pub fn get(&self, i: usize) -> f64 {
        self.vals.get(i).copied().unwrap_or(0.0)
    }
    /// Set height `i`; returns true if it changed noticeably.
    pub fn set(&mut self, i: usize, v: f64) -> bool {
        let Some(old) = self.vals.get(i).copied() else { return false };
        let d = v - old;
        if d.abs() < 0.01 {
            return false;
        }
        self.vals[i] = v;
        let mut k = i + 1;
        while k < self.tree.len() {
            self.tree[k] += d;
            k += k.isolate_lowest_one();
        }
        true
    }
    /// Sum of heights `[0, i)`.
    pub fn prefix(&self, i: usize) -> f64 {
        let mut k = i.min(self.vals.len());
        let mut s = 0.0;
        while k > 0 {
            s += self.tree[k];
            k -= k.isolate_lowest_one();
        }
        s
    }
    pub fn total(&self) -> f64 {
        self.prefix(self.vals.len())
    }
    /// Index of the chunk containing y (clamped to the last chunk).
    pub fn find(&self, y: f64) -> usize {
        let n = self.vals.len();
        if n == 0 {
            return 0;
        }
        if y <= 0.0 {
            return 0;
        }
        let mut pos = 0usize;
        let mut rem = y;
        let mut step = n.next_power_of_two();
        while step > 0 {
            let next = pos + step;
            if next <= n && self.tree[next] <= rem {
                pos = next;
                rem -= self.tree[next];
            }
            step >>= 1;
        }
        pos.min(n - 1)
    }
}

/// Everything derived from one buffer version.
#[derive(Default)]
pub(crate) struct Model {
    pub version: Option<(u64, usize)>,
    pub text: String,
    pub blocks: Vec<Block>,
    pub chunks: Vec<DChunk>,
    pub hashes: Vec<Option<u64>>,
    pub counts: (usize, usize),
}

impl Model {
    pub fn chunk_hash(&mut self, i: usize) -> u64 {
        if let Some(Some(h)) = self.hashes.get(i) {
            return *h;
        }
        let Some(c) = self.chunks.get(i) else { return 0 };
        let r = c.range.start.min(self.text.len())..c.range.end.min(self.text.len());
        let h = hash_str(&self.text[r]);
        if let Some(slot) = self.hashes.get_mut(i) {
            *slot = Some(h);
        }
        h
    }
}

/// Length of the common prefix and suffix (non-overlapping) of two strings, in bytes.
pub(crate) fn common_affixes(a: &str, b: &str) -> (usize, usize) {
    let (ab, bb) = (a.as_bytes(), b.as_bytes());
    let max = ab.len().min(bb.len());
    let mut p = ab.iter().zip(bb).take_while(|(x, y)| x == y).count().min(max);
    while p > 0 && !(a.is_char_boundary(p) && b.is_char_boundary(p)) {
        p -= 1;
    }
    let max_s = max - p;
    let mut s = ab.iter().rev().zip(bb.iter().rev()).take(max_s).take_while(|(x, y)| x == y).count();
    while s > 0 && !(a.is_char_boundary(ab.len() - s) && b.is_char_boundary(bb.len() - s)) {
        s -= 1;
    }
    (p, s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mdedit_style::Chunk;

    fn ch(r: Range<usize>) -> Chunk {
        Chunk { range: r, block: Some(0) }
    }

    #[test]
    fn boundary_rule() {
        let text = "# a\n\npara\n";
        let chunks = display_chunks(text, &[ch(0..5), ch(5..10)]);
        assert_eq!(chunks.len(), 3, "virtual trailing chunk");
        assert_eq!(chunks[2].range, 10..10);
        assert_eq!(chunk_at(&chunks, 0), 0);
        assert_eq!(chunk_at(&chunks, 4), 0);
        assert_eq!(chunk_at(&chunks, 5), 1, "chunk.end is the start of the next chunk");
        assert_eq!(chunk_at(&chunks, 9), 1);
        assert_eq!(chunk_at(&chunks, 10), 2);
        assert_eq!(chunk_at(&chunks, 999), 2);
        // Chunk 0 galley text is "# a\n" (final \n removed).
        assert_eq!(chunks[0].text_end(text), 4);
        assert_eq!(char_in_chunk(text, &chunks[0], 4), 4);
        assert_eq!(byte_in_chunk(text, &chunks[0], 4), 4);
        assert_eq!(byte_in_chunk(text, &chunks[0], 99), 4);
        assert_eq!(char_in_chunk(text, &chunks[1], 7), 2);
    }

    #[test]
    fn no_trailing_newline() {
        let text = "abc";
        let chunks = display_chunks(text, &[ch(0..3)]);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunk_at(&chunks, 3), 0);
        assert_eq!(char_in_chunk(text, &chunks[0], 3), 3);
    }

    #[test]
    fn empty_text() {
        let chunks = display_chunks("", &[Chunk { range: 0..0, block: None }]);
        assert_eq!(chunks, vec![DChunk { range: 0..0, block: None }]);
        assert_eq!(chunk_at(&chunks, 0), 0);
        assert_eq!(char_in_chunk("", &chunks[0], 5), 0);
        assert_eq!(display_chunks("", &[]).len(), 1);
    }

    #[test]
    fn multibyte_mapping() {
        let text = "héllo wörld";
        let chunks = display_chunks(text, &[ch(0..text.len())]);
        let c = &chunks[0];
        for (ci, (b, _)) in text.char_indices().enumerate() {
            assert_eq!(char_in_chunk(text, c, b), ci);
            assert_eq!(byte_in_chunk(text, c, ci), b);
        }
        // Inside a multi-byte char rounds down.
        assert_eq!(char_in_chunk(text, c, 2), 1);
    }

    #[test]
    fn invalid_chunks_fall_back() {
        let text = "a\nb\n";
        let chunks = display_chunks(text, &[ch(0..1)]);
        assert_eq!(chunks.first().map(|c| c.range.start), Some(0));
        assert_eq!(chunks.iter().rev().nth(1).map(|c| c.range.end), Some(4));
        let chunks = display_chunks("é", &[ch(0..1)]);
        assert_eq!(chunks[0].range, 0..2);
    }

    #[test]
    fn huge_chunks_split() {
        let line = "x".repeat(99) + "\n";
        let text = line.repeat(3000); // 300 KB
        let chunks = display_chunks(&text, &[ch(0..text.len())]);
        assert!(chunks.len() > 4);
        for w in chunks.windows(2) {
            assert_eq!(w[0].range.end, w[1].range.start);
        }
        assert!(chunks.iter().all(|c| c.range.len() <= MAX_CHUNK_BYTES));
        // A single giant line with multibyte chars.
        let text = "é".repeat(100_000);
        let chunks = display_chunks(&text, &[ch(0..text.len())]);
        assert!(chunks.iter().all(|c| text.is_char_boundary(c.range.start) && text.is_char_boundary(c.range.end)));
        assert_eq!(chunks.last().map(|c| c.range.end), Some(text.len()));
    }

    #[test]
    fn char_ranges() {
        let text = "ab\ncd\n";
        let chunks = display_chunks(text, &[ch(0..6)]);
        let c = &chunks[0];
        assert_eq!(char_range_in_chunk(text, c, 1..4), Some(1..4));
        assert_eq!(char_range_in_chunk(text, c, 0..6), Some(0..6), "covers final newline");
        assert_eq!(char_range_in_chunk(text, c, 6..7), None);
    }

    #[test]
    fn fenwick() {
        let mut h = Heights::new(vec![10.0, 20.0, 30.0, 40.0, 50.0]);
        assert_eq!(h.prefix(0), 0.0);
        assert_eq!(h.prefix(3), 60.0);
        assert_eq!(h.total(), 150.0);
        assert_eq!(h.find(0.0), 0);
        assert_eq!(h.find(9.9), 0);
        assert_eq!(h.find(10.0), 1);
        assert_eq!(h.find(59.0), 2);
        assert_eq!(h.find(1e9), 4);
        assert!(h.set(1, 25.0));
        assert_eq!(h.prefix(3), 65.0);
        assert_eq!(h.find(64.0), 2);
        assert_eq!(h.find(65.0), 3);
        let h = Heights::new(Vec::new());
        assert_eq!(h.find(5.0), 0);
        assert_eq!(h.total(), 0.0);
    }

    #[test]
    fn affixes() {
        assert_eq!(common_affixes("hello world", "hello brave world"), (6, 5));
        assert_eq!(common_affixes("aaa", "aaaa"), (3, 0));
        assert_eq!(common_affixes("é", "è"), (0, 0));
        assert_eq!(common_affixes("", "x"), (0, 0));
    }

    proptest::proptest! {
        #[test]
        fn mapping_roundtrip(s in "[a-zé\n]{0,60}", at in 0usize..80) {
            let chunks = display_chunks(&s, &[]);
            let i = chunk_at(&chunks, at);
            let c = &chunks[i];
            let ci = char_in_chunk(&s, c, at);
            let b = byte_in_chunk(&s, c, ci);
            proptest::prop_assert!(s.is_char_boundary(b));
            proptest::prop_assert!(b >= c.range.start && b <= c.range.end);
        }
    }
}

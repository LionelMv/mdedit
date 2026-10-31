//! Per-version document model: cached text, parsed blocks, display chunks, and the pure
//! byte ⇄ (chunk, char index) mapping. No egui in here (unit-testable).

use std::hash::{Hash, Hasher};
use std::ops::Range;

use mdedit_syntax::{Block, BlockKind};

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
            push_display_chunk(text, c, &mut out);
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

fn push_display_chunk(text: &str, c: &mdedit_style::Chunk, out: &mut Vec<DChunk>) {
    if c.range.is_empty() && !text.is_empty() {
        return;
    }
    if c.range.len() > MAX_CHUNK_BYTES {
        split_into(text, c.range.clone(), None, out);
    } else {
        out.push(DChunk { range: c.range.clone(), block: c.block });
    }
}

/// All display chunks of a document, from scratch.
pub(crate) fn build_chunks(text: &str, blocks: &[Block]) -> Vec<DChunk> {
    split_nested_code(text, blocks, display_chunks(text, &mdedit_style::chunks(text, blocks)))
}

/// How the display chunks change for one edit: `old` is replaced by `new`, and the chunks after
/// it keep their text and only move.
#[derive(Debug)]
pub(crate) struct ChunkSplice {
    pub old: Range<usize>,
    pub new: Vec<DChunk>,
    delta: isize,
    block_delta: isize,
}

impl ChunkSplice {
    /// Work out the change after the blocks `old_blocks` became `blocks[old_blocks.start..][..new_len]`,
    /// in `text` with bytes `start..new_end` new (`delta` = its change in length). `None` if the
    /// chunk boundaries around them can't be matched up: the caller rebuilds everything.
    #[allow(clippy::too_many_arguments)]
    pub fn plan(text: &str, blocks: &[Block], valid: usize, chunks: &[DChunk], old_blocks: &Range<usize>, new_len: usize, start: usize, new_end: usize, delta: isize) -> Option<Self> {
        if blocks.is_empty() || chunks.is_empty() {
            return None;
        }
        let ls = |i: usize| line_start(text, blocks[i].range.start);
        // From the block before (its chunk ends where the replaced ones start) ...
        let mut a = old_blocks.start.saturating_sub(1).min(blocks.len() - 1);
        while a > 0 && ls(a) <= ls(a - 1) {
            a -= 1;
        }
        // ... to the next block that starts its own chunk (blocks on one line share a chunk).
        let mut b = old_blocks.start + new_len;
        while b < valid && b > 0 && ls(b) <= ls(b - 1) {
            b += 1;
        }
        // Only blocks before `valid` have correct ranges.
        if b >= valid && valid < blocks.len() {
            return None;
        }
        let begin = if a == 0 { 0 } else { ls(a) };
        let end = if b < blocks.len() { ls(b) } else { text.len() };
        if begin > start || end < new_end || b <= a {
            return None;
        }
        let k0 = chunks.partition_point(|c| c.range.start < begin);
        if chunks.get(k0)?.range.start != begin {
            return None;
        }
        let k1 = if b < blocks.len() {
            let old_end = end as isize - delta;
            let k1 = chunks.partition_point(|c| (c.range.start as isize) < old_end);
            if chunks.get(k1)?.range.start as isize != old_end {
                return None;
            }
            k1
        } else {
            chunks.len()
        };
        let mut pieces = Vec::new();
        if a == 0 && ls(0) > 0 {
            pieces.push(mdedit_style::Chunk { range: 0..ls(0), block: None });
        }
        pieces.extend(mdedit_style::chunks_of_blocks(text, blocks, a..b, end));
        let mut display = Vec::with_capacity(pieces.len() + 1);
        for c in &pieces {
            push_display_chunk(text, c, &mut display);
        }
        let mut new = split_nested_code(text, blocks, display);
        if b == blocks.len() && text.ends_with('\n') {
            new.push(DChunk { range: text.len()..text.len(), block: None });
        }
        let block_delta = new_len as isize - old_blocks.len() as isize;
        Some(Self { old: k0..k1, new, delta, block_delta })
    }

    /// Apply to the chunks and their cached text hashes (those after the change stay valid).
    pub fn apply(self, chunks: &mut Vec<DChunk>, hashes: &mut Vec<Option<u64>>) {
        let tail_from = self.old.start + self.new.len();
        hashes.splice(self.old.clone(), std::iter::repeat_n(None, self.new.len()));
        chunks.splice(self.old, self.new);
        if self.delta != 0 || self.block_delta != 0 {
            for c in &mut chunks[tail_from..] {
                c.range.start = (c.range.start as isize + self.delta) as usize;
                c.range.end = (c.range.end as isize + self.delta) as usize;
                if let Some(b) = &mut c.block {
                    *b = (*b as isize + self.block_delta) as usize;
                }
            }
        }
    }
}

/// Split list and quote chunks for display: each list item starts a chunk (the editor indents
/// it by its depth, so wrapped lines line up with the item's text), and each code block nested
/// in a list or quote gets a chunk of its own (from its opening fence line to the next
/// non-blank line), so it gets a full code card instead of being laid out inside the text.
pub(crate) fn split_nested_code(text: &str, blocks: &[Block], chunks: Vec<DChunk>) -> Vec<DChunk> {
    let mut out = Vec::with_capacity(chunks.len());
    for c in chunks {
        let top = c.block.and_then(|i| blocks.get(i));
        let codes = top.map(nested_code).unwrap_or_default();
        let mut cuts: Vec<usize> = Vec::new();
        for r in codes {
            cuts.push(line_start(text, r.start));
            cuts.push(after_code(text, r.end));
        }
        if let Some(b) = top.filter(|b| matches!(b.kind, BlockKind::List { .. } | BlockKind::BlockQuote)) {
            nested_starts(text, b, &mut cuts);
        }
        cuts.retain(|&p| p > c.range.start && p < c.range.end);
        cuts.sort_unstable();
        cuts.dedup();
        let mut start = c.range.start;
        for p in cuts.into_iter().chain(std::iter::once(c.range.end)) {
            if p > start {
                out.push(DChunk { range: start..p, block: c.block });
                start = p;
            }
        }
        if start < c.range.end || c.range.is_empty() {
            out.push(DChunk { range: start..c.range.end, block: c.block });
        }
    }
    out
}

/// Whether chunk `c` is a code block nested in its list / quote (see [`split_nested_code`]).
pub(crate) fn is_nested_code(text: &str, blocks: &[Block], c: &DChunk) -> bool {
    c.block
        .and_then(|i| blocks.get(i))
        .is_some_and(|b| nested_code(b).iter().any(|r| line_start(text, r.start) == c.range.start))
}

/// Nesting of the containers around line start `at` inside top-level block `b`: (list depth,
/// quote depth, list depth outside the first quote). List depth counts the list items `at` is in.
pub(crate) fn container_depth(text: &str, b: &Block, at: usize) -> (u8, u8, u8) {
    let (mut lists, mut quotes, mut outer) = (0u8, 0u8, 0u8);
    let mut cur = b;
    loop {
        match cur.kind {
            BlockKind::ListItem { .. } => {
                lists = lists.saturating_add(1);
                if quotes == 0 {
                    outer = lists;
                }
            }
            BlockKind::BlockQuote => quotes = quotes.saturating_add(1),
            _ => {}
        }
        // Children start after their line's indentation / `> `: compare from the line start.
        // Last match: a child's range can run up to the next sibling's line.
        match cur.children.iter().rev().find(|c| line_start(text, c.range.start) <= at && at <= c.range.end) {
            Some(n) => cur = n,
            None => return (lists, quotes, outer),
        }
    }
}

/// Chunk cuts for the list items and quotes nested in `b`: each is indented by its own depth.
fn nested_starts(text: &str, b: &Block, cuts: &mut Vec<usize>) {
    for c in &b.children {
        match c.kind {
            BlockKind::ListItem { .. } => cuts.push(line_start(text, c.range.start)),
            BlockKind::BlockQuote => {
                cuts.push(line_start(text, c.range.start));
                cuts.push(after_code(text, c.range.end));
            }
            _ => {}
        }
        nested_starts(text, c, cuts);
    }
}

/// Ranges of the code blocks inside a list or block quote (any depth).
fn nested_code(b: &Block) -> Vec<Range<usize>> {
    fn walk(b: &Block, out: &mut Vec<Range<usize>>) {
        for c in &b.children {
            if matches!(c.kind, BlockKind::CodeBlock(_)) {
                out.push(c.range.clone());
            } else {
                walk(c, out);
            }
        }
    }
    let mut out = Vec::new();
    if matches!(b.kind, BlockKind::List { .. } | BlockKind::BlockQuote) {
        walk(b, &mut out);
    }
    out
}

fn line_start(text: &str, at: usize) -> usize {
    let at = floor_boundary(text, at);
    text.as_bytes()[..at].iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1)
}

/// After the line containing `end` and any blank lines that follow it.
fn after_code(text: &str, end: usize) -> usize {
    let bytes = text.as_bytes();
    let mut p = floor_boundary(text, end);
    p = bytes[p..].iter().position(|&b| b == b'\n').map_or(text.len(), |i| p + i + 1);
    while p < text.len() {
        let e = bytes[p..].iter().position(|&b| b == b'\n').map_or(text.len(), |i| p + i + 1);
        if !text[p..e].trim().is_empty() {
            break;
        }
        p = e;
    }
    p
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
    /// Replace heights `range` with `new`.
    pub fn splice(&mut self, range: Range<usize>, new: Vec<f64>) {
        if range.len() == new.len() {
            for (i, v) in range.zip(new) {
                self.set(i, v);
            }
        } else {
            let mut vals = std::mem::take(&mut self.vals);
            vals.splice(range, new);
            *self = Self::new(vals);
        }
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
    /// Only `blocks[..valid_blocks]` have correct ranges; the rest are owed `owed_shift`. Moving
    /// every block after the caret on each keystroke is what made typing cost the document's
    /// size, and only the blocks being drawn or edited are read. See [`Self::blocks_through`].
    pub blocks: Vec<Block>,
    pub valid_blocks: usize,
    pub owed_shift: isize,
    /// `mdedit_syntax::may_have_definition` of the text.
    pub has_definitions: bool,
    pub chunks: Vec<DChunk>,
    pub hashes: Vec<Option<u64>>,
    pub counts: (usize, usize),
}

/// Blocks past the position asked for that are made valid too: enough for the parser's context.
const VALID_MARGIN: usize = 3;

impl Model {
    /// Blocks with correct ranges, at least up to byte `at` and a few more.
    pub fn blocks_through(&mut self, at: usize) -> &[Block] {
        let owed = self.owed_shift;
        let n = self.blocks.len();
        let mut margin = VALID_MARGIN;
        while self.valid_blocks < n && margin > 0 {
            let b = &mut self.blocks[self.valid_blocks];
            if (b.range.start as isize + owed) as usize > at {
                margin -= 1;
            }
            mdedit_syntax::shift(b, owed);
            self.valid_blocks += 1;
        }
        if self.valid_blocks == n {
            self.owed_shift = 0;
        }
        &self.blocks[..self.valid_blocks]
    }

    /// Make the blocks valid just past byte `at` and no further, ready for an edit ending there:
    /// everything after it then only needs its owed shift to grow.
    pub fn prepare_edit(&mut self, at: usize) {
        self.blocks_through(at);
        let past = self.valid().partition_point(|b| b.range.start <= at);
        let keep = (past + VALID_MARGIN).min(self.valid_blocks);
        for b in &mut self.blocks[keep..self.valid_blocks] {
            mdedit_syntax::shift(b, -self.owed_shift);
        }
        self.valid_blocks = keep;
    }

    /// The blocks known to have correct ranges.
    pub fn valid(&self) -> &[Block] {
        &self.blocks[..self.valid_blocks]
    }

    /// All blocks, with correct ranges.
    pub fn all_blocks(&mut self) -> &[Block] {
        self.blocks_through(usize::MAX)
    }

    /// `blocks` just came from a full parse of `text`.
    pub fn blocks_parsed(&mut self) {
        self.valid_blocks = self.blocks.len();
        self.owed_shift = 0;
        self.has_definitions = mdedit_syntax::may_have_definition(&self.text);
    }

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

/// Length of the common prefix and suffix (non-overlapping, on char boundaries) of `old` and a
/// new text of `new_len` bytes. The new text is read through `chunks_from(at)`: its chunks from
/// the one holding byte `at`, with that chunk's start. Compares whole chunks with memcmp, so
/// megabytes per keystroke stay cheap, without copying the new text. Both ends are found walking
/// forward: a rope's reverse iterator is far slower.
pub(crate) fn chunk_affixes<'a, I: Iterator<Item = &'a str>>(
    old: &str,
    new_len: usize,
    chunks_from: impl Fn(usize) -> (I, usize),
    new_byte: impl Fn(usize) -> u8,
) -> (usize, usize) {
    let o = old.as_bytes();
    let new_boundary = |i: usize| i >= new_len || (new_byte(i) & 0xC0) != 0x80;
    let max = o.len().min(new_len);
    let mut p = 0;
    for c in chunks_from(0).0 {
        let c = c.as_bytes();
        let n = c.len().min(max - p);
        if o[p..p + n] != c[..n] {
            p += o[p..p + n].iter().zip(c).take_while(|(x, y)| x == y).count();
            break;
        }
        p += n;
        if p == max {
            break;
        }
    }
    while p > 0 && !(old.is_char_boundary(p) && new_boundary(p)) {
        p -= 1;
    }
    // Walk the part of the new text that can be in the suffix, comparing each byte with the one
    // the same distance from the end of `old`. The suffix starts after the last difference.
    let from = new_len - (max - p);
    let mut suffix_start = from;
    let (chunks, mut at) = chunks_from(from);
    for c in chunks {
        let c = c.as_bytes();
        let skip = from.saturating_sub(at);
        let new_part = &c[skip.min(c.len())..];
        let start = at + skip.min(c.len());
        let old_start = start + o.len() - new_len;
        if *new_part != o[old_start..old_start + new_part.len()]
            && let Some(i) = new_part.iter().zip(&o[old_start..]).rposition(|(x, y)| x != y)
        {
            suffix_start = start + i + 1;
        }
        at += c.len();
    }
    let mut s = new_len - suffix_start;
    while s > 0 && !(old.is_char_boundary(o.len() - s) && new_boundary(new_len - s)) {
        s -= 1;
    }
    (p, s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mdedit_style::Chunk;

    /// [`chunk_affixes`] over `b` cut into chunks of `size` bytes (on char boundaries).
    fn affixes_in_chunks(a: &str, b: &str, size: usize) -> (usize, usize) {
        let mut parts = Vec::new();
        let mut rest = b;
        while !rest.is_empty() {
            let mut n = size.min(rest.len());
            while !rest.is_char_boundary(n) {
                n += 1;
            }
            parts.push(&rest[..n]);
            rest = &rest[n..];
        }
        let chunks_from = |at: usize| {
            let mut start = 0;
            let mut k = 0;
            while k + 1 < parts.len() && start + parts[k].len() <= at {
                start += parts[k].len();
                k += 1;
            }
            (parts[k.min(parts.len())..].iter().copied(), start)
        };
        chunk_affixes(a, b.len(), chunks_from, |i| b.as_bytes()[i])
    }

    /// The obvious byte loop, as the reference.
    fn common_affixes(a: &str, b: &str) -> (usize, usize) {
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

    #[test]
    fn nested_code_gets_its_own_chunk() {
        let text = "1. Step:\n\n   ```bash\n   echo hi\n   ```\n\n2. Next\n";
        let blocks = mdedit_syntax::parse(text);
        let chunks = split_nested_code(text, &blocks, display_chunks(text, &mdedit_style::chunks(text, &blocks)));
        let pieces: Vec<&str> = chunks.iter().map(|c| &text[c.range.clone()]).collect();
        assert_eq!(pieces, ["1. Step:\n\n", "   ```bash\n   echo hi\n   ```\n\n", "2. Next\n", ""]);
        assert!(chunks.iter().take(3).all(|c| c.block == Some(0)), "pieces keep the list block");
        let flags: Vec<bool> = chunks.iter().map(|c| is_nested_code(text, &blocks, c)).collect();
        assert_eq!(flags, [false, true, false, false]);
    }

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
        for size in [1, 2, 3, 5, 100] {
            assert_eq!(affixes_in_chunks("hello world", "hello brave world", size), (6, 5));
            assert_eq!(affixes_in_chunks("aaa", "aaaa", size), (3, 0));
            assert_eq!(affixes_in_chunks("é", "è", size), (0, 0));
            assert_eq!(affixes_in_chunks("", "x", size), (0, 0));
            assert_eq!(affixes_in_chunks("x", "", size), (0, 0));
        }
    }

    proptest::proptest! {
        #[test]
        fn chunk_affixes_match_byte_loop(a in "[aéb\n]{0,40}", b in "[aéb\n]{0,40}", size in 1usize..12) {
            let joined = format!("{a}{b}");
            proptest::prop_assert_eq!(affixes_in_chunks(&a, &b, size), common_affixes(&a, &b));
            proptest::prop_assert_eq!(affixes_in_chunks(&joined, &b, size), common_affixes(&joined, &b));
            proptest::prop_assert_eq!(affixes_in_chunks(&a, &joined, size), common_affixes(&a, &joined));
        }

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

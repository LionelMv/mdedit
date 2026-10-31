//! Pure editing commands: given the text, the parsed blocks and the selection, return the
//! edits to apply and the selection afterwards. The editor applies them with
//! `Buffer::apply`. No I/O, no egui. STABLE signatures.
//!
//! Selections here are `(anchor, head)` byte offsets, same as `mdedit_buffer::Selection`
//! (this crate doesn't depend on the buffer crate, so it has its own types).
//!
//! Every function clamps offsets to the text and to char boundaries, never panics, and
//! returns edits against the *original* text, sorted and non-overlapping. The returned
//! selection is expressed in the text *after* the edits.

use std::ops::Range;

use crate::{
    Block, BlockKind, CodeBlockInfo, SpanKind, floor_boundary, item_marker_pos, line_end, line_end_no_cr,
    line_start, list_marker_len,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextEdit {
    pub range: Range<usize>,
    pub insert: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Sel {
    pub anchor: usize,
    pub head: usize,
}

impl Sel {
    pub fn caret(at: usize) -> Self {
        Sel { anchor: at, head: at }
    }
    pub fn new(anchor: usize, head: usize) -> Self {
        Sel { anchor, head }
    }
    pub fn start(&self) -> usize {
        self.anchor.min(self.head)
    }
    pub fn end(&self) -> usize {
        self.anchor.max(self.head)
    }
    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }
}

/// Result of a command: edits against the *original* text (non-overlapping) + new selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandResult {
    pub edits: Vec<TextEdit>,
    pub selection: Sel,
}

/// Apply a result's edits to `text` (helper for tests and simple callers).
pub fn apply(text: &str, res: &CommandResult) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    let mut pos = 0;
    let mut edits: Vec<&TextEdit> = res.edits.iter().collect();
    edits.sort_by_key(|e| (e.range.start, e.range.end));
    for e in edits {
        let s = e.range.start.clamp(pos, text.len());
        let end = e.range.end.clamp(s, text.len());
        out.push_str(&text[pos..s]);
        out.push_str(&e.insert);
        pos = end;
    }
    out.push_str(&text[pos..]);
    out
}

/// Formatting commands for menus/shortcuts. STABLE.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Bold,
    Italic,
    Strikethrough,
    InlineCode,
    /// `[sel](url)`; if the selection looks like a URL, `[](url)` with caret in the text.
    Link,
    Image,
    /// Set heading level of the caret's block; 0 = paragraph.
    Heading(u8),
    BlockQuote,
    BulletList,
    OrderedList,
    TaskList,
    /// Insert a fenced code block (CB-40); with a selection, wrap the selected lines.
    CodeBlock,
    MathBlock,
    HorizontalRule,
    /// Insert a `cols`×`rows` table skeleton.
    Table { cols: usize, rows: usize },
    /// Toggle the `[ ]`/`[x]` of the task item at the caret.
    ToggleTask,
}

// ---------------------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------------------

fn edit(range: Range<usize>, insert: impl Into<String>) -> TextEdit {
    TextEdit { range, insert: insert.into() }
}

fn clamp_sel(text: &str, sel: Sel) -> Sel {
    Sel { anchor: floor_boundary(text, sel.anchor), head: floor_boundary(text, sel.head) }
}

/// Map a position in the original text to the edited text. Insertions exactly at `pos`
/// push it forward; positions inside a replaced range move to the end of the replacement.
fn map_pos(pos: usize, edits: &[TextEdit]) -> usize {
    let mut delta: isize = 0;
    for e in edits {
        let d = e.insert.len() as isize - (e.range.end - e.range.start) as isize;
        if e.range.end <= pos {
            delta += d;
        } else if e.range.start < pos {
            // inside a replaced range: end of the replacement
            return (e.range.start as isize + delta + e.insert.len() as isize).max(0) as usize;
        } else {
            break;
        }
    }
    (pos as isize + delta).max(0) as usize
}

fn finish(text: &str, mut edits: Vec<TextEdit>, anchor: usize, head: usize) -> Option<CommandResult> {
    edits.sort_by_key(|e| (e.range.start, e.range.end));
    // sanity: non-overlapping, in bounds, char boundaries
    let mut last = 0;
    for e in &edits {
        if e.range.start < last
            || e.range.end < e.range.start
            || e.range.end > text.len()
            || !text.is_char_boundary(e.range.start)
            || !text.is_char_boundary(e.range.end)
        {
            return None;
        }
        last = e.range.end;
    }
    let new_text_len = (text.len() as isize
        + edits.iter().map(|e| e.insert.len() as isize - e.range.len() as isize).sum::<isize>())
    .max(0) as usize;
    Some(CommandResult {
        edits,
        selection: Sel { anchor: anchor.min(new_text_len), head: head.min(new_text_len) },
    })
}

/// Result with edits and a selection mapped from the original selection.
fn finish_mapped(text: &str, mut edits: Vec<TextEdit>, sel: Sel) -> Option<CommandResult> {
    edits.sort_by_key(|e| (e.range.start, e.range.end));
    let a = map_pos(sel.anchor, &edits);
    let h = map_pos(sel.head, &edits);
    finish(text, edits, a, h)
}

/// Path of blocks (outermost first) containing `at` (inclusive range end).
pub fn block_path(blocks: &[Block], at: usize) -> Vec<&Block> {
    let mut out = Vec::new();
    let mut level = blocks;
    loop {
        // last block starting at or before `at`
        let i = level.partition_point(|b| b.range.start <= at);
        if i == 0 {
            break;
        }
        let b = &level[i - 1];
        if at > b.range.end {
            break;
        }
        out.push(b);
        level = &b.children;
    }
    out
}

/// Innermost block (any depth) containing byte `at`.
pub fn block_at(blocks: &[Block], at: usize) -> Option<&Block> {
    block_path(blocks, at).pop()
}

fn code_block_at(blocks: &[Block], at: usize) -> Option<(&Block, &CodeBlockInfo)> {
    let b = block_at(blocks, at)?;
    match &b.kind {
        BlockKind::CodeBlock(info) => Some((b, info)),
        _ => None,
    }
}

/// Whether `at` lies on a content line of a code block (not on a fence line).
fn in_code_content(text: &str, blocks: &[Block], at: usize) -> bool {
    code_block_at(blocks, at).is_some_and(|(b, info)| is_code_content_pos(text, b, info, at))
}

fn is_code_content_pos(text: &str, b: &Block, info: &CodeBlockInfo, at: usize) -> bool {
    match (&info.open_fence, &info.close_fence) {
        (Some(of), close) => {
            let open_line_end = line_end(text, of.start);
            if at <= open_line_end {
                return false;
            }
            if let Some(cf) = close
                && at >= line_start(text, cf.start) {
                    return false;
                }
            at <= b.range.end
        }
        (None, _) => true,
    }
}

/// Byte ranges of the lines touched by `[s, e]`: (line start, line end without newline).
fn lines_in(text: &str, s: usize, e: usize) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut ls = line_start(text, s);
    // a selection ending at the very start of a line doesn't include that line
    let e = if e > s && e == line_start(text, e) { e - 1 } else { e };
    loop {
        let le = line_end_no_cr(text, ls);
        out.push(ls..le);
        let next = line_end(text, ls) + 1;
        if next > e || next > text.len() {
            break;
        }
        ls = next;
    }
    out
}

fn leading_ws(s: &str) -> usize {
    s.bytes().take_while(|&b| b == b' ' || b == b'\t').count()
}

/// Length of a blockquote prefix (`> `, `>> `, `  > > `) at the start of `line`.
fn quote_prefix_len(line: &str) -> usize {
    let b = line.as_bytes();
    let mut i = 0;
    let mut end = 0;
    loop {
        let sp = b[i..].iter().take(3).take_while(|&&c| c == b' ').count();
        if b.get(i + sp) == Some(&b'>') {
            i += sp + 1;
            if b.get(i) == Some(&b' ') {
                i += 1;
            }
            end = i;
        } else {
            break;
        }
    }
    end
}

/// How much of a line belongs to the containers around a block on it.
struct ContainerPrefix {
    /// Bytes from the line start; ends on a byte the containers don't own.
    len: usize,
    /// Columns of a tab just before `len` that lie past the containers (a straddling tab).
    extra: usize,
}

/// Next column after a tab at `col`.
fn tab_stop(col: usize) -> usize {
    (col / 4 + 1) * 4
}

/// Column just after `s` (spaces and tabs expand; other bytes are one column).
fn advance_col(s: &str, mut col: usize) -> usize {
    for b in s.bytes() {
        col = if b == b'\t' { tab_stop(col) } else { col + 1 };
    }
    col
}

/// Column where the item's content starts, measured on the line holding its marker.
fn item_content_col(text: &str, item: &Block) -> usize {
    let mpos = item_marker_pos(text, item);
    let le = line_end_no_cr(text, mpos);
    let col = advance_col(&text[line_start(text, mpos)..mpos], 0);
    let Some((mlen, wlen)) = list_marker_len(&text[mpos..le]) else { return col + 2 };
    if text[mpos + wlen..le].trim().is_empty() {
        return col + mlen + 1; // nothing after the marker: one space belongs to it
    }
    advance_col(&text[mpos..mpos + wlen], col)
}

/// Split the line `l` into the part owned by `containers` (quotes and list items, outermost
/// first) and the rest; `indent` columns past the containers also count as owned (indented
/// code). A quote marker missing from the line ends the walk.
fn container_prefix(text: &str, containers: &[&Block], l: &Range<usize>, indent: usize) -> ContainerPrefix {
    let line = &text[l.clone()];
    let b = line.as_bytes();
    let (mut pos, mut col, mut want) = (0, 0, 0);
    let skip_ws = |pos: &mut usize, col: &mut usize, want: usize| {
        while *col < want && matches!(b.get(*pos), Some(b' ' | b'\t')) {
            *col = advance_col(&line[*pos..*pos + 1], *col);
            *pos += 1;
        }
    };
    for c in containers {
        match c.kind {
            BlockKind::BlockQuote => {
                skip_ws(&mut pos, &mut col, want);
                let sp = b[pos..].iter().take(3).take_while(|&&x| x == b' ').count();
                if b.get(pos + sp) != Some(&b'>') {
                    return ContainerPrefix { len: pos, extra: col.saturating_sub(want) };
                }
                pos += sp + 1;
                col += sp + 1;
                want = col;
                if matches!(b.get(pos), Some(b' ' | b'\t')) {
                    want += 1;
                }
                skip_ws(&mut pos, &mut col, want);
            }
            BlockKind::ListItem { .. } => {
                let mpos = item_marker_pos(text, c);
                if (l.start..=l.end).contains(&mpos) {
                    let wlen = list_marker_len(&text[mpos..l.end]).map_or(0, |m| m.1);
                    col = advance_col(&line[..mpos - l.start + wlen], 0);
                    pos = mpos - l.start + wlen;
                    want = col;
                } else {
                    want = item_content_col(text, c);
                    skip_ws(&mut pos, &mut col, want);
                }
            }
            _ => {}
        }
    }
    want += indent;
    skip_ws(&mut pos, &mut col, want);
    ContainerPrefix { len: pos, extra: col.saturating_sub(want) }
}

fn is_thematic_break(line: &str) -> bool {
    let t = line.trim();
    let Some(c) = t.chars().next() else { return false };
    if !matches!(c, '-' | '*' | '_') {
        return false;
    }
    let n = t.chars().filter(|&x| x == c).count();
    n >= 3 && t.chars().all(|x| x == c || x == ' ' || x == '\t')
}

#[derive(Clone, Debug)]
struct LineMarker {
    /// Offsets relative to the line start.
    indent: usize,
    marker: Range<usize>,
    /// After marker + following spaces.
    after: usize,
    ordered: Option<(u64, char)>,
    /// Checkbox `[ ]` range and checked state.
    task: Option<(Range<usize>, bool)>,
    /// Content start (after checkbox + one space).
    content: usize,
}

/// Parse a list marker at the start of `line` (after optional indentation).
fn parse_line_marker(line: &str) -> Option<LineMarker> {
    if is_thematic_break(line) {
        return None;
    }
    let indent = leading_ws(line);
    let rest = &line[indent..];
    let (mlen, wlen) = list_marker_len(rest)?;
    let ordered = if rest.as_bytes()[0].is_ascii_digit() {
        let n: u64 = rest[..mlen - 1].parse().ok()?;
        Some((n, rest.as_bytes()[mlen - 1] as char))
    } else {
        None
    };
    let after = indent + wlen;
    let mut content = after;
    let mut task = None;
    let tb = line.as_bytes();
    if wlen > mlen
        && tb.get(after) == Some(&b'[')
        && matches!(tb.get(after + 1), Some(b' ' | b'x' | b'X'))
        && tb.get(after + 2) == Some(&b']')
        && matches!(tb.get(after + 3), None | Some(b' ' | b'\t' | b'\r' | b'\n'))
    {
        task = Some((after..after + 3, tb[after + 1] != b' '));
        content = after + 3;
        if matches!(tb.get(content), Some(b' ' | b'\t')) {
            content += 1;
        }
    }
    Some(LineMarker { indent, marker: indent..indent + mlen, after, ordered, task, content })
}

fn longest_run(s: &str, ch: char) -> usize {
    let mut best = 0;
    let mut cur = 0;
    for c in s.chars() {
        if c == ch {
            cur += 1;
            best = best.max(cur);
        } else {
            cur = 0;
        }
    }
    best
}

/// A fence of `ch` longer than any run of `ch` in `content`, minimum 3 (CB-61).
pub fn fence_for(content: &str, ch: char) -> String {
    let n = (longest_run(content, ch) + 1).max(3);
    std::iter::repeat_n(ch, n).collect()
}

fn looks_like_url(s: &str) -> bool {
    let s = s.trim();
    !s.is_empty()
        && !s.contains(char::is_whitespace)
        && (s.starts_with("http://") || s.starts_with("https://") || s.starts_with("www.") || s.starts_with("mailto:"))
}

/// Non-whitespace, non-`>` characters of a prefix turned into spaces (for continuation lines).
fn continuation_prefix(prefix: &str) -> String {
    prefix.chars().map(|c| if c == '>' || c == '\t' || c == ' ' { c } else { ' ' }).collect()
}

// ---------------------------------------------------------------------------------------
// run
// ---------------------------------------------------------------------------------------

/// Run `cmd`. `None` when it doesn't apply. Toggling: Bold on `**x**` removes the markers.
pub fn run(text: &str, blocks: &[Block], sel: Sel, cmd: &Command) -> Option<CommandResult> {
    let sel = clamp_sel(text, sel);
    match cmd {
        Command::Bold => toggle_inline(text, blocks, sel, "**", |k| matches!(k, SpanKind::Strong)),
        Command::Italic => toggle_inline(text, blocks, sel, "*", |k| matches!(k, SpanKind::Emphasis)),
        Command::Strikethrough => toggle_inline(text, blocks, sel, "~~", |k| matches!(k, SpanKind::Strikethrough)),
        Command::InlineCode => toggle_inline(text, blocks, sel, "`", |k| matches!(k, SpanKind::Code)),
        Command::Link => link(text, blocks, sel, false),
        Command::Image => link(text, blocks, sel, true),
        Command::Heading(n) => heading(text, blocks, sel, (*n).min(6)),
        Command::BlockQuote => line_prefix_cmd(text, blocks, sel, LineCmd::Quote),
        Command::BulletList => line_prefix_cmd(text, blocks, sel, LineCmd::Bullet),
        Command::OrderedList => line_prefix_cmd(text, blocks, sel, LineCmd::Ordered),
        Command::TaskList => line_prefix_cmd(text, blocks, sel, LineCmd::Task),
        Command::CodeBlock => fenced_block(text, blocks, sel, '`'),
        Command::MathBlock => fenced_block(text, blocks, sel, '$'),
        Command::HorizontalRule => horizontal_rule(text, sel),
        Command::Table { cols, rows } => table(text, sel, *cols, *rows),
        Command::ToggleTask => toggle_task(text, blocks, sel),
    }
}

fn host_spans(blocks: &[Block], at: usize) -> Vec<&crate::Span> {
    let path = block_path(blocks, at);
    let mut out = Vec::new();
    for b in path.iter().rev() {
        if !b.inlines.is_empty() {
            out.extend(b.inlines.iter());
            break;
        }
    }
    out
}

fn toggle_inline(
    text: &str,
    blocks: &[Block],
    sel: Sel,
    marker: &str,
    is_kind: fn(&SpanKind) -> bool,
) -> Option<CommandResult> {
    let (s, e) = (sel.start(), sel.end());
    if in_code_content(text, blocks, s) {
        return None;
    }
    // 1. existing span around the selection → remove its markers
    let spans = host_spans(blocks, s);
    let found = spans
        .iter()
        .filter(|sp| is_kind(&sp.kind) && sp.range.start <= s && e <= sp.range.end)
        .min_by_key(|sp| sp.range.len());
    if let Some(sp) = found {
        let edits = vec![edit(sp.range.start..sp.content.start, ""), edit(sp.content.end..sp.range.end, "")];
        // positions inside the opening marker map to content start
        let clamp_in = |p: usize| p.clamp(sp.content.start, sp.content.end);
        let a = map_pos(clamp_in(sel.anchor), &edits);
        let h = map_pos(clamp_in(sel.head), &edits);
        // an insertion-free removal: map_pos for p == content.start shifts by -open
        return finish(text, edits, a, h);
    }
    // 2. selection itself carries the markers (unparsed, e.g. `**` alone)
    let selected = &text[s..e];
    if selected.len() >= 2 * marker.len()
        && selected.starts_with(marker)
        && selected.ends_with(marker)
        && marker != "`"
        && !(marker == "*" && is_strong_only(selected))
    {
        let inner = &selected[marker.len()..selected.len() - marker.len()];
        let edits = vec![edit(s..e, inner)];
        return finish(text, edits, s, s + inner.len()).map(|r| keep_dir(r, sel));
    }
    // 3. wrap
    let (open, close) = if marker == "`" {
        let f = "`".repeat(longest_run(selected, '`') + 1);
        let pad = if selected.starts_with('`') || selected.ends_with('`') { " " } else { "" };
        (format!("{f}{pad}"), format!("{pad}{f}"))
    } else {
        (marker.to_string(), marker.to_string())
    };
    if s == e {
        let ins = format!("{open}{close}");
        let caret = s + open.len();
        return finish(text, vec![edit(s..s, ins)], caret, caret);
    }
    let edits = vec![edit(s..s, open.clone()), edit(e..e, close)];
    let ns = s + open.len();
    let ne = e + open.len();
    finish(text, edits, ns, ne).map(|r| keep_dir(r, sel))
}

/// `**x**`: the stars are bold's, so italic must wrap it rather than take one pair away.
fn is_strong_only(selected: &str) -> bool {
    let run = |b: &mut dyn Iterator<Item = u8>| b.take_while(|&c| c == b'*').count();
    run(&mut selected.bytes()) == 2 && run(&mut selected.bytes().rev()) == 2
}

/// Keep the selection direction of `orig` for a result whose selection is (start, end).
fn keep_dir(mut r: CommandResult, orig: Sel) -> CommandResult {
    if orig.anchor > orig.head {
        std::mem::swap(&mut r.selection.anchor, &mut r.selection.head);
    }
    r
}

fn link(text: &str, blocks: &[Block], sel: Sel, image: bool) -> Option<CommandResult> {
    let (s, e) = (sel.start(), sel.end());
    if in_code_content(text, blocks, s) {
        return None;
    }
    // toggle off: caret inside an existing link → keep only its text
    if !image {
        let spans = host_spans(blocks, s);
        if let Some(sp) = spans
            .iter()
            .filter(|sp| matches!(sp.kind, SpanKind::Link { .. }) && sp.range.start <= s && e <= sp.range.end)
            .min_by_key(|sp| sp.range.len())
        {
            let inner = text[sp.content.clone()].to_string();
            let edits = vec![edit(sp.range.clone(), inner.clone())];
            return finish(text, edits, sp.range.start, sp.range.start + inner.len());
        }
    }
    let bang = if image { "!" } else { "" };
    let selected = &text[s..e];
    if s == e {
        let ins = format!("{bang}[](url)");
        let caret = s + bang.len() + 1;
        return finish(text, vec![edit(s..s, ins)], caret, caret);
    }
    if looks_like_url(selected) {
        let url = selected.trim();
        let ins = format!("{bang}[]({url})");
        let caret = s + bang.len() + 1;
        return finish(text, vec![edit(s..e, ins)], caret, caret);
    }
    let ins = format!("{bang}[{selected}](url)");
    let us = s + bang.len() + 1 + selected.len() + 2;
    finish(text, vec![edit(s..e, ins)], us, us + 3)
}

fn heading(text: &str, blocks: &[Block], sel: Sel, level: u8) -> Option<CommandResult> {
    let c = sel.head;
    if in_code_content(text, blocks, c) {
        return None;
    }
    let path = block_path(blocks, c);
    // setext heading: remove the underline, then add `#`s
    if let Some(b) = path.last()
        && let BlockKind::Heading { level: cur } = b.kind {
            let first = &text[b.range.clone()];
            let pre = quote_prefix_len(first);
            if !first[pre..].trim_start().starts_with('#') {
                let underline_ls = line_start(text, b.range.end);
                if underline_ls > b.range.start {
                    let cut_from = line_end_no_cr(text, underline_ls.saturating_sub(1));
                    let mut edits = vec![edit(cut_from..b.range.end, "")];
                    let target = if cur == level { 0 } else { level };
                    if target > 0 {
                        let at = b.range.start;
                        edits.insert(0, edit(at..at, format!("{} ", "#".repeat(target as usize))));
                    }
                    return finish_mapped(text, edits, Sel::caret(c.min(cut_from)));
                }
            }
        }
    let ls = line_start(text, c);
    let le = line_end_no_cr(text, c);
    let line = &text[ls..le];
    let qp = quote_prefix_len(line);
    let after_q = &line[qp..];
    let indent = after_q.bytes().take_while(|&b| b == b' ').count();
    // The heading goes after a list marker, so the line stays a list item. A task item can't
    // hold a heading without losing its checkbox.
    let marker = list_marker_len(&after_q[indent..]).map(|m| m.1);
    if marker.is_some_and(|m| after_q[indent + m..].starts_with("[ ] ") || after_q[indent + m..].to_ascii_lowercase().starts_with("[x] ")) {
        return None;
    }
    let ws = match marker {
        Some(m) => indent + m,
        None => indent.min(3),
    };
    let rest = &after_q[ws..];
    let hashes = rest.bytes().take_while(|&b| b == b'#').count();
    let is_atx = (1..=6).contains(&hashes) && matches!(rest.as_bytes().get(hashes), None | Some(b' ' | b'\t'));
    let (cur, mstart, mend) = if is_atx {
        let sp = rest[hashes..].bytes().take_while(|&b| b == b' ' || b == b'\t').count();
        (hashes as u8, ls + qp + ws, ls + qp + ws + hashes + sp)
    } else {
        (0, ls + qp + ws, ls + qp + ws)
    };
    let target = if cur == level { 0 } else { level };
    if target == 0 && cur == 0 {
        return None;
    }
    let ins = if target == 0 { String::new() } else { format!("{} ", "#".repeat(target as usize)) };
    let edits = vec![edit(mstart..mend, ins)];
    let caret = if c < mend { mend } else { c };
    finish_mapped(text, edits, Sel::caret(caret))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LineCmd {
    Quote,
    Bullet,
    Ordered,
    Task,
}

fn line_prefix_cmd(text: &str, _blocks: &[Block], sel: Sel, cmd: LineCmd) -> Option<CommandResult> {
    let lines = lines_in(text, sel.start(), sel.end());
    let multi = lines.len() > 1;
    let targets: Vec<Range<usize>> = lines
        .into_iter()
        .filter(|l| !multi || !text[l.clone()].trim().is_empty())
        .collect();
    if targets.is_empty() {
        return None;
    }
    let mut edits = Vec::new();
    match cmd {
        LineCmd::Quote => {
            let all = targets.iter().all(|l| quote_prefix_len(&text[l.clone()]) > 0);
            for l in &targets {
                let line = &text[l.clone()];
                if all {
                    // remove one level: up to 3 spaces, `>`, optional space
                    let sp = line.bytes().take(3).take_while(|&b| b == b' ').count();
                    let mut end = sp + 1;
                    if line.as_bytes().get(end) == Some(&b' ') {
                        end += 1;
                    }
                    edits.push(edit(l.start..l.start + end, ""));
                } else {
                    edits.push(edit(l.start..l.start, "> "));
                }
            }
        }
        LineCmd::Bullet | LineCmd::Ordered | LineCmd::Task => {
            let has = |m: &Option<LineMarker>| match (cmd, m) {
                (LineCmd::Bullet, Some(m)) => m.ordered.is_none() && m.task.is_none(),
                (LineCmd::Ordered, Some(m)) => m.ordered.is_some(),
                (LineCmd::Task, Some(m)) => m.task.is_some(),
                _ => false,
            };
            let markers: Vec<(Range<usize>, usize, Option<LineMarker>)> = targets
                .iter()
                .map(|l| {
                    let line = &text[l.clone()];
                    let qp = quote_prefix_len(line);
                    (l.clone(), qp, parse_line_marker(&line[qp..]))
                })
                .collect();
            let all = markers.iter().all(|(_, _, m)| has(m));
            let mut n = 1u64;
            for (l, qp, m) in &markers {
                let base = l.start + qp;
                match m {
                    Some(m) if all => {
                        // toggle off: remove marker (and checkbox), keep indentation
                        edits.push(edit(base + m.indent..base + m.content, ""));
                    }
                    Some(m) => {
                        let new = match cmd {
                            LineCmd::Bullet => "- ".to_string(),
                            LineCmd::Ordered => {
                                let s = format!("{n}. ");
                                n += 1;
                                s
                            }
                            _ => {
                                let bullet = if m.ordered.is_some() { "- ".to_string() } else { text[base + m.marker.start..base + m.after].to_string() };
                                let bullet = if bullet.ends_with([' ', '\t']) { bullet } else { format!("{bullet} ") };
                                match &m.task {
                                    Some(_) => text[base + m.marker.start..base + m.content].to_string(),
                                    None => format!("{bullet}[ ] "),
                                }
                            }
                        };
                        edits.push(edit(base + m.indent..base + m.content, new));
                    }
                    None => {
                        let line = &text[base..l.end];
                        let ind = leading_ws(line);
                        let new = match cmd {
                            LineCmd::Bullet => "- ".to_string(),
                            LineCmd::Ordered => {
                                let s = format!("{n}. ");
                                n += 1;
                                s
                            }
                            _ => "- [ ] ".to_string(),
                        };
                        edits.push(edit(base + ind..base + ind, new));
                    }
                }
            }
        }
    }
    edits.retain(|e| !(e.range.is_empty() && e.insert.is_empty()));
    // drop edits that change nothing
    edits.retain(|e| text.get(e.range.clone()) != Some(e.insert.as_str()));
    if edits.is_empty() {
        return None;
    }
    finish_lines(text, edits, sel)
}

/// Like [`finish_mapped`], but a non-empty selection whose start is at a line start keeps
/// covering the whole line (insertions at the start don't push it).
fn finish_lines(text: &str, mut edits: Vec<TextEdit>, sel: Sel) -> Option<CommandResult> {
    if sel.is_empty() {
        return finish_mapped(text, edits, sel);
    }
    edits.sort_by_key(|e| (e.range.start, e.range.end));
    let (s, e) = (sel.start(), sel.end());
    let ns = if s == line_start(text, s) { map_pos_before(s, &edits) } else { map_pos(s, &edits) };
    let ne = map_pos(e, &edits);
    finish(text, edits, ns, ne).map(|r| keep_dir(r, sel))
}

/// Like [`map_pos`] but insertions exactly at `pos` don't push it.
fn map_pos_before(pos: usize, edits: &[TextEdit]) -> usize {
    let mut delta: isize = 0;
    for e in edits {
        let d = e.insert.len() as isize - (e.range.end - e.range.start) as isize;
        if e.range.end < pos || (e.range.end == pos && e.range.start < pos) {
            delta += d;
        } else if e.range.start < pos {
            return (e.range.start as isize + delta).max(0) as usize;
        } else {
            break;
        }
    }
    (pos as isize + delta).max(0) as usize
}

fn fenced_block(text: &str, blocks: &[Block], sel: Sel, ch: char) -> Option<CommandResult> {
    let (s, e) = (sel.start(), sel.end());
    // toggle off: caret in a fenced code block (for backticks) → remove its fences
    if ch == '`'
        && let Some((b, info)) = code_block_at(blocks, s) {
            let of = info.open_fence.clone()?;
            let mut edits = Vec::new();
            let content_start = b.content.start;
            // the fence's own prefix stays, so drop the first content line's copy of it
            let has_content = content_start > of.end
                && info.close_fence.as_ref().is_none_or(|cf| content_start < line_start(text, cf.start));
            let dup = if has_content {
                let first = content_start..line_end_no_cr(text, content_start);
                container_prefix(text, &block_path(blocks, s), &first, 0).len
            } else {
                0
            };
            let open_end = content_start.max(of.end) + dup;
            match &info.close_fence {
                Some(cf) => {
                    let cls = line_start(text, cf.start);
                    let ce = b.content.end.max(open_end);
                    edits.push(edit(of.start..open_end, ""));
                    // remove the newline before the closing fence line and the fence itself
                    if ce < cf.start && cls > content_start {
                        edits.push(edit(ce..cf.end, ""));
                    } else {
                        edits.push(edit(cf.start..cf.end, ""));
                    }
                }
                None => edits.push(edit(of.start..open_end, "")),
            }
            return finish_mapped(text, edits, sel);
        }
    let open_close = |content: &str| -> String {
        if ch == '$' { "$$".to_string() } else { fence_for(content, ch) }
    };
    if s == e {
        let ls = line_start(text, s);
        let le = line_end_no_cr(text, s);
        let line = &text[ls..le];
        let f = open_close("");
        if line.trim().is_empty() {
            let ins = format!("{f}\n\n{f}");
            let caret = ls + f.len() + 1;
            return finish(text, vec![edit(ls..le, ins)], caret, caret);
        }
        // insert on new lines after the current line
        let ins = format!("\n{f}\n\n{f}");
        let caret = le + 1 + f.len() + 1;
        return finish(text, vec![edit(le..le, ins)], caret, caret);
    }
    // wrap the selected lines
    let lines = lines_in(text, s, e);
    let first = lines.first()?.start;
    let last = lines.last()?.end;
    let content = &text[first..last];
    let f = open_close(content);
    let edits = vec![edit(first..first, format!("{f}\n")), edit(last..last, format!("\n{f}"))];
    let a = first + f.len() + 1;
    finish(text, edits, a, a + content.len()).map(|r| keep_dir(r, sel))
}

fn horizontal_rule(text: &str, sel: Sel) -> Option<CommandResult> {
    let c = sel.head;
    let ls = line_start(text, c);
    let le = line_end_no_cr(text, c);
    if text[ls..le].trim().is_empty() {
        let prev_blank = ls == 0 || text[line_start(text, ls - 1)..ls - 1].trim().is_empty();
        let ins = if prev_blank { "---".to_string() } else { "\n---".to_string() };
        let caret = ls + ins.len();
        return finish(text, vec![edit(ls..le, ins)], caret, caret);
    }
    let ins = "\n\n---\n".to_string();
    let caret = le + ins.len();
    finish(text, vec![edit(le..le, ins)], caret, caret)
}

fn table(text: &str, sel: Sel, cols: usize, rows: usize) -> Option<CommandResult> {
    let cols = cols.clamp(1, 64);
    let rows = rows.min(1000);
    let mut t = String::new();
    let header: Vec<String> = (1..=cols).map(|i| format!("Column {i}")).collect();
    t.push_str(&format!("| {} |\n", header.join(" | ")));
    t.push_str(&format!("|{}\n", " --- |".repeat(cols)));
    for _ in 0..rows {
        t.push_str(&format!("|{}\n", "  |".repeat(cols)));
    }
    t.pop(); // final newline
    let c = sel.end();
    let ls = line_start(text, c);
    let le = line_end_no_cr(text, c);
    let empty_line = text[ls..le].trim().is_empty();
    let (range, prefix) = if empty_line {
        let prev_blank = ls == 0 || text[line_start(text, ls - 1)..ls - 1].trim().is_empty();
        (ls..le, if prev_blank { "" } else { "\n" })
    } else {
        (le..le, "\n\n")
    };
    let first_cell = range.start + prefix.len() + 2;
    // A line right below would become a table row.
    let next = (le + 1).min(text.len());
    let suffix = if next > le && !text[next..line_end_no_cr(text, next)].trim().is_empty() { "\n" } else { "" };
    let ins = format!("{prefix}{t}{suffix}");
    finish(text, vec![edit(range, ins)], first_cell, first_cell + "Column 1".len())
}

fn toggle_task(text: &str, blocks: &[Block], sel: Sel) -> Option<CommandResult> {
    let c = sel.head;
    let path = block_path(blocks, c);
    let item = path.iter().rev().find(|b| matches!(b.kind, BlockKind::ListItem { .. }))?;
    if let BlockKind::ListItem { task: Some(checked), checkbox: Some(cb) } = &item.kind {
        let ins = if *checked { "[ ]" } else { "[x]" };
        if cb.end > text.len() {
            return None;
        }
        return finish_mapped(text, vec![edit(cb.clone(), ins)], sel);
    }
    // textual fallback (e.g. `- [ ]` with no text isn't a task for the parser)
    let mpos = item_marker_pos(text, item);
    let ls = line_start(text, mpos);
    let le = line_end_no_cr(text, mpos);
    let line = &text[ls..le];
    let m = parse_line_marker(&line[mpos - ls..]).map(|mut m| {
        let off = mpos - ls;
        m.marker = m.marker.start + off..m.marker.end + off;
        m.after += off;
        m.content += off;
        m.task = m.task.map(|(r, c)| (r.start + off..r.end + off, c));
        m
    })?;
    match m.task {
        Some((r, checked)) => {
            let ins = if checked { "[ ]" } else { "[x]" };
            finish_mapped(text, vec![edit(ls + r.start..ls + r.end, ins)], sel)
        }
        None => {
            let at = ls + m.after;
            let pad = if m.after == m.marker.end { " " } else { "" };
            finish_mapped(text, vec![edit(at..at, format!("{pad}[ ] "))], sel)
        }
    }
}

// ---------------------------------------------------------------------------------------
// keys
// ---------------------------------------------------------------------------------------

/// Enter key. Continues lists (`- `, `1. `→`2. `, `- [ ] `), ends the list on an empty item,
/// continues `> ` quotes, auto-indents inside code (CB-41), and completes a fence:
/// typing ```` ```lang ```` + Enter on a line by itself inserts the closing fence (CB-40).
/// `None` = plain newline.
pub fn on_enter(text: &str, blocks: &[Block], sel: Sel) -> Option<CommandResult> {
    let sel = clamp_sel(text, sel);
    if !sel.is_empty() {
        return None;
    }
    let c = sel.head;
    let path = block_path(blocks, c);
    // --- code blocks ---
    if let Some(b) = path.last()
        && let BlockKind::CodeBlock(info) = &b.kind {
            if let (Some(of), Some(f)) = (&info.open_fence, &info.fence) {
                let open_le = line_end_no_cr(text, of.start);
                if c >= of.start && c <= open_le {
                    // fence completion: caret after the fence chars, rest of line blank
                    if !info.closed && c >= of.start + f.len && text[c..open_le].trim().is_empty() {
                        let ls = line_start(text, of.start);
                        let prefix = continuation_prefix(&text[ls..of.start]);
                        let fence: String = std::iter::repeat_n(f.ch, f.len).collect();
                        let ins = format!("\n{prefix}\n{prefix}{fence}");
                        let caret = c + 1 + prefix.len();
                        return finish(text, vec![edit(c..c, ins)], caret, caret);
                    }
                    return None;
                }
            }
            if is_code_content_pos(text, b, info, c) {
                let ls = line_start(text, c);
                let line = &text[ls..c];
                let fence_prefix_has_quote = info
                    .open_fence
                    .as_ref()
                    .map(|of| text[line_start(text, of.start)..of.start].contains('>'))
                    .unwrap_or(false);
                let n = if fence_prefix_has_quote {
                    line.bytes().take_while(|&b| b == b' ' || b == b'\t' || b == b'>').count()
                } else {
                    leading_ws(line)
                };
                let ind = &line[..n];
                if let Some(r) = code_exit(text, b, info, c) {
                    return Some(r);
                }
                // between a bracket pair: open an indented line and put the closer below
                let prev = text[..c].chars().next_back();
                let next = text[c..].chars().next();
                if matches!((prev, next), (Some('{'), Some('}')) | (Some('('), Some(')')) | (Some('['), Some(']'))) {
                    let unit = if text[b.content.clone()].lines().any(|l| l.trim_start_matches(['>', ' ']).starts_with('\t')) {
                        "\t"
                    } else {
                        "    "
                    };
                    let ins = format!("\n{ind}{unit}\n{ind}");
                    let caret = c + 1 + ind.len() + unit.len();
                    return finish(text, vec![edit(c..c, ins)], caret, caret);
                }
                let ins = format!("\n{ind}");
                let caret = c + ins.len();
                return finish(text, vec![edit(c..c, ins)], caret, caret);
            }
            return None;
        }
    // --- innermost list item or quote ---
    let ci = path
        .iter()
        .rposition(|b| matches!(b.kind, BlockKind::ListItem { .. } | BlockKind::BlockQuote))?;
    let container = path[ci];
    let list = ci.checked_sub(1).map(|i| path[i]).filter(|b| matches!(b.kind, BlockKind::List { .. }));
    match container.kind {
        BlockKind::ListItem { .. } => enter_list(text, container, list, c),
        BlockKind::BlockQuote => enter_quote(text, container, c),
        _ => None,
    }
}

fn enter_list(text: &str, item: &Block, list: Option<&Block>, c: usize) -> Option<CommandResult> {
    let start = item_marker_pos(text, item);
    let ls = line_start(text, start);
    let le = line_end_no_cr(text, start);
    let m = parse_line_marker(&text[start..le])?;
    let content_start = start + m.content;
    if c < content_start.min(le) && c < start + m.after {
        return None;
    }
    let raw_prefix = &text[ls..start];
    let prefix = continuation_prefix(raw_prefix);
    let rest_of_item = &text[content_start.min(item.range.end)..item.range.end];
    // empty item → remove the marker and end the list
    if rest_of_item.trim().is_empty() && line_end_no_cr(text, c) == le {
        let from = if raw_prefix.trim().is_empty() { ls } else { start };
        let to = le.max(item.range.end.min(le));
        return finish(text, vec![edit(from..to, "")], from, from);
    }
    let marker_text = match m.ordered {
        Some((n, d)) => format!("{}{}", n.saturating_add(1), d),
        None => text[start + m.marker.start..start + m.marker.end].to_string(),
    };
    let spaces = &text[start + m.marker.end..start + m.after];
    let spaces = if spaces.is_empty() { " " } else { spaces };
    let task = if m.task.is_some() { "[ ] " } else { "" };
    // strip trailing spaces before the caret on the current line
    let ins = format!("\n{prefix}{marker_text}{spaces}{task}");
    let mut trim_from = c;
    while trim_from > content_start && text.as_bytes()[trim_from - 1] == b' ' {
        trim_from -= 1;
    }
    let caret = trim_from + ins.len();
    let mut edits = vec![edit(trim_from..c, ins)];
    // renumber the following items of the same list if they were sequential
    if let (Some((n, _)), Some(list)) = (m.ordered, list) {
        let after: Vec<&Block> = list.children.iter().filter(|b| b.range.start > item.range.start).collect();
        let mut renum = Vec::new();
        let mut expect = n.saturating_add(1);
        for sib in after {
            let ms = item_marker_pos(text, sib);
            let digits = text[ms..].bytes().take_while(|b| b.is_ascii_digit()).count();
            match text[ms..ms + digits].parse::<u64>() {
                Ok(k) if k == expect && digits > 0 => {
                    renum.push(edit(ms..ms + digits, expect.saturating_add(1).to_string()));
                    expect = expect.saturating_add(1);
                }
                _ => {
                    renum.clear();
                    break;
                }
            }
        }
        if renum.first().is_some_and(|e| e.range.start >= c) {
            edits.extend(renum);
        }
    }
    finish(text, edits, caret, caret)
}

/// CB-43: Enter on an empty last content line whose previous line is also empty exits the
/// block: the trailing empty lines are removed and the caret goes to an empty line after the
/// closing fence.
fn code_exit(text: &str, b: &Block, info: &CodeBlockInfo, c: usize) -> Option<CommandResult> {
    let cf = info.close_fence.as_ref()?;
    info.fence?;
    let blank = |r: Range<usize>| text[r].bytes().all(|x| matches!(x, b' ' | b'\t' | b'>' | b'\r'));
    let ls = line_start(text, c);
    let le = line_end_no_cr(text, c);
    let close_ls = line_start(text, cf.start);
    if line_end(text, c) + 1 != close_ls || !blank(ls..le) || ls <= b.content.start {
        return None;
    }
    let prev_ls = line_start(text, ls - 1);
    if !blank(prev_ls..ls - 1) {
        return None;
    }
    // walk back to the last non-blank content line
    let mut cut = prev_ls;
    loop {
        if cut <= b.content.start {
            return None; // whole block blank: keep normal behaviour
        }
        let pls = line_start(text, cut - 1);
        if !blank(pls..cut - 1) {
            break;
        }
        cut = pls;
    }
    // delete from the newline ending the last non-blank line up to the current line's end
    let del = (cut - 1)..le;
    let removed = del.len() as isize;
    let prefix = continuation_prefix(&text[close_ls..cf.start]);
    let prefix = prefix.trim_end_matches(' ').to_string();
    let after_fence = line_end(text, cf.start);
    let mut edits = vec![edit(del, "")];
    let caret_orig = if after_fence >= text.len() {
        edits.push(edit(text.len()..text.len(), format!("\n{prefix}")));
        text.len() as isize + 1 + prefix.len() as isize
    } else {
        let nls = after_fence + 1;
        let nle = line_end_no_cr(text, nls);
        if nls <= text.len() && blank(nls..nle) {
            nle as isize
        } else {
            edits.push(edit(after_fence..after_fence, format!("\n{prefix}")));
            after_fence as isize + 1 + prefix.len() as isize
        }
    };
    let caret = (caret_orig - removed).max(0) as usize;
    finish(text, edits, caret, caret)
}

fn enter_quote(text: &str, quote: &Block, c: usize) -> Option<CommandResult> {
    let ls = line_start(text, c);
    let le = line_end_no_cr(text, c);
    let line = &text[ls..le];
    let mut qp = quote_prefix_len(line);
    let prefix = if qp > 0 {
        let p = &line[..qp];
        if p.ends_with('>') { format!("{p} ") } else { p.to_string() }
    } else {
        // lazy continuation: use the quote's first-line prefix
        let qls = line_start(text, quote.range.start);
        let first = &text[qls..line_end_no_cr(text, quote.range.start)];
        let p = quote_prefix_len(first);
        qp = 0;
        let p = &first[..p];
        if p.ends_with('>') { format!("{p} ") } else { p.to_string() }
    };
    if c < ls + qp {
        return None;
    }
    if qp > 0 && line[qp..].trim().is_empty() {
        // empty quote line → end the quote
        return finish(text, vec![edit(ls..le, "")], ls, ls);
    }
    let ins = format!("\n{prefix}");
    let caret = c + ins.len();
    finish(text, vec![edit(c..c, ins)], caret, caret)
}

/// Tab / Shift+Tab: indent/outdent list items, indent lines inside code (CB-42),
/// move to the next/previous table cell. `None` = insert a literal tab (Tab) or nothing.
pub fn on_tab(text: &str, blocks: &[Block], sel: Sel, shift: bool) -> Option<CommandResult> {
    let sel = clamp_sel(text, sel);
    let path = block_path(blocks, sel.head);
    let leaf = *path.last()?;
    if let BlockKind::CodeBlock(info) = &leaf.kind {
        if !is_code_content_pos(text, leaf, info, sel.head) {
            return None;
        }
        let indent = if info.fence.is_some() { 0 } else { 4 };
        return tab_code(text, leaf, &path[..path.len() - 1], indent, sel, shift);
    }
    if let BlockKind::Table { rows, .. } = &leaf.kind {
        return tab_table(text, leaf, rows, sel, shift);
    }
    let items: Vec<&&Block> = path.iter().filter(|b| matches!(b.kind, BlockKind::ListItem { .. })).collect();
    if let Some(item) = items.last() {
        let parent = if items.len() >= 2 { Some(*items[items.len() - 2]) } else { None };
        return tab_list(text, path.as_slice(), item, parent, sel, shift);
    }
    None
}

/// Indentation only changes past `containers`' prefix (plus `indent` columns for indented
/// code), so the block stays inside its quotes and list items.
fn tab_code(text: &str, b: &Block, containers: &[&Block], indent: usize, sel: Sel, shift: bool) -> Option<CommandResult> {
    let uses_tabs = lines_in(text, b.content.start, b.content.end).iter().any(|l| {
        let p = container_prefix(text, containers, l, indent);
        p.extra > 0 || text[l.start + p.len..l.end].starts_with('\t')
    });
    let unit = if uses_tabs { "\t" } else { "    " };
    if sel.is_empty() && !shift {
        let ls = line_start(text, sel.head);
        let l = ls..line_end_no_cr(text, sel.head);
        let c = sel.head.max(ls + container_prefix(text, containers, &l, indent).len);
        let caret = c + unit.len();
        return finish(text, vec![edit(c..c, unit)], caret, caret);
    }
    let lines = lines_in(text, sel.start(), sel.end());
    let mut edits = Vec::new();
    for l in lines {
        if l.end < b.content.start || l.start > b.content.end {
            continue;
        }
        let line = &text[l.clone()];
        let p = container_prefix(text, containers, &l, indent);
        let at = l.start + p.len;
        if shift {
            let rest = &line[p.len..];
            let n = if rest.starts_with('\t') {
                1
            } else {
                rest.bytes().take(4).take_while(|&c| c == b' ').count()
            };
            if n > 0 {
                edits.push(edit(at..at + n, ""));
            } else if p.extra > 0 {
                // a tab straddles the container's column: keep just the columns it owns
                let before = advance_col(&line[..p.len - 1], 0);
                let owned = tab_stop(before) - before - p.extra;
                edits.push(edit(at - 1..at, " ".repeat(owned)));
            }
        } else if !line.is_empty() || l.start == line_start(text, sel.head) {
            edits.push(edit(at..at, unit));
        }
    }
    if edits.is_empty() {
        // consume the key (don't insert a literal tab over a selection)
        return finish(text, vec![], sel.anchor, sel.head);
    }
    finish_mapped(text, edits, sel)
}

fn tab_table(text: &str, table: &Block, rows: &[crate::TableRow], sel: Sel, shift: bool) -> Option<CommandResult> {
    let c = sel.head;
    let ri = rows.iter().position(|r| line_start(text, r.range.start) <= c && c <= line_end_no_cr(text, r.range.start))?;
    let row = &rows[ri];
    if row.cells.is_empty() {
        return None;
    }
    let ci = row.cells.iter().position(|cell| c <= cell.end).unwrap_or(row.cells.len() - 1);
    let target = if shift {
        if ci > 0 {
            Some((ri, ci - 1))
        } else if ri > 0 {
            let prev = &rows[ri - 1];
            prev.cells.len().checked_sub(1).map(|i| (ri - 1, i))
        } else {
            None
        }
    } else if ci + 1 < row.cells.len() {
        Some((ri, ci + 1))
    } else if ri + 1 < rows.len() {
        Some((ri + 1, 0))
    } else {
        None
    };
    match target {
        Some((r, i)) => {
            let cell = rows.get(r)?.cells.get(i)?.clone();
            finish(text, vec![], cell.start, cell.end)
        }
        None if shift => finish(text, vec![], sel.anchor, sel.head),
        None => {
            // Tab in the last cell: append a new empty row
            let cols = row.cells.len().max(1);
            let at = table.range.end;
            let ls = line_start(text, row.range.start);
            let prefix = continuation_prefix(&text[ls..row.range.start]);
            let ins = format!("\n{prefix}|{}", "  |".repeat(cols));
            let caret = at + 1 + prefix.len() + 2;
            finish(text, vec![edit(at..at, ins)], caret, caret)
        }
    }
}

fn tab_list(
    text: &str,
    _path: &[&Block],
    item: &Block,
    parent: Option<&Block>,
    sel: Sel,
    shift: bool,
) -> Option<CommandResult> {
    // lines to move: the selection's lines, or the whole item when the caret is in it
    let lines = if sel.is_empty() {
        lines_in(text, item.range.start, item.range.end)
    } else {
        lines_in(text, sel.start(), sel.end())
    };
    let mpos = item_marker_pos(text, item);
    let item_col = mpos - line_start(text, mpos);
    let width = list_marker_len(&text[mpos..]).map_or(2, |m| m.1).max(1);
    let mut edits = Vec::new();
    if shift {
        let amount = match parent {
            Some(p) => {
                let pm = item_marker_pos(text, p);
                let pcol = pm - line_start(text, pm);
                item_col.saturating_sub(pcol)
            }
            None => item_col,
        };
        if amount == 0 {
            return finish(text, vec![], sel.anchor, sel.head);
        }
        for l in lines {
            let line = &text[l.clone()];
            let qp = quote_prefix_len(line);
            let rest = &line[qp..];
            let n = if rest.starts_with('\t') {
                1
            } else {
                rest.bytes().take(amount).take_while(|&b| b == b' ').count()
            };
            if n > 0 {
                edits.push(edit(l.start + qp..l.start + qp + n, ""));
            }
        }
    } else {
        // indent by the content width of the previous sibling (or this item's marker width)
        let indent = " ".repeat(width.min(8));
        for l in lines {
            let line = &text[l.clone()];
            if line.trim().is_empty() {
                continue;
            }
            let qp = quote_prefix_len(line);
            edits.push(edit(l.start + qp..l.start + qp, indent.clone()));
        }
    }
    if edits.is_empty() {
        return finish(text, vec![], sel.anchor, sel.head);
    }
    finish_mapped(text, edits, sel)
}

/// Backspace at the start of a list item / quote / heading / empty code block removes the
/// marker (CB-44). `None` = normal backspace.
pub fn on_backspace(text: &str, blocks: &[Block], sel: Sel) -> Option<CommandResult> {
    let sel = clamp_sel(text, sel);
    if !sel.is_empty() {
        return None;
    }
    let c = sel.head;
    let path = block_path(blocks, c);
    let leaf = *path.last()?;
    match &leaf.kind {
        BlockKind::CodeBlock(info) => {
            // CB-44: empty fenced block, caret on its (only) content line → remove both fences
            if info.fence.is_some()
                && text[leaf.content.clone()].trim().is_empty()
                && c >= leaf.content.start
                && c <= leaf.content.start.max(line_start(text, c))
                && c > info.open_fence.as_ref().map_or(0, |of| of.end)
                && line_start(text, c) == leaf.content.start
            {
                let r = leaf.range.clone();
                return finish(text, vec![edit(r.clone(), "")], r.start, r.start);
            }
            return None;
        }
        BlockKind::Heading { .. } => {
            let ls = line_start(text, c);
            if ls == line_start(text, leaf.range.start) {
                let line = &text[ls..line_end_no_cr(text, c)];
                let qp = quote_prefix_len(line);
                let rest = &line[qp..];
                let ws = leading_ws(rest);
                let h = rest[ws..].bytes().take_while(|&b| b == b'#').count();
                if h > 0 {
                    let sp = rest[ws + h..].bytes().take_while(|&b| b == b' ' || b == b'\t').count();
                    let mend = ls + qp + ws + h + sp;
                    if c == mend {
                        let from = ls + qp + ws;
                        return finish(text, vec![edit(from..mend, "")], from, from);
                    }
                }
            }
        }
        _ => {}
    }
    // list item marker (innermost item whose first line holds the caret)
    let container = path
        .iter()
        .rev()
        .find(|b| matches!(b.kind, BlockKind::ListItem { .. } | BlockKind::BlockQuote));
    if let Some(item) = container
        && matches!(item.kind, BlockKind::ListItem { .. }) && line_start(text, c) == line_start(text, item.range.start) {
            let start = item_marker_pos(text, item);
            let le = line_end_no_cr(text, start);
            if let Some(m) = parse_line_marker(&text[start..le]) {
                if let Some((cb, _)) = &m.task
                    && c == start + m.content {
                        // first remove only the checkbox
                        let from = start + cb.start;
                        return finish(text, vec![edit(from..start + m.content, "")], from, from);
                    }
                if c == start + m.after || (m.task.is_none() && c == start + m.content) {
                    return finish(text, vec![edit(start..c, "")], start, start);
                }
            }
        }
    // quote marker right before the caret
    if path.iter().any(|b| matches!(b.kind, BlockKind::BlockQuote)) {
        let ls = line_start(text, c);
        let before = &text[ls..c];
        let qp = quote_prefix_len(before);
        if qp > 0 && qp == before.len() {
            // remove the last `>` (and the space after it)
            let gt = before.rfind('>')?;
            return finish(text, vec![edit(ls + gt..c, "")], ls + gt, ls + gt);
        }
    }
    None
}

/// Auto-pair: typing `(`, `[`, `{`, `"`, `` ` ``, `*`, `_` around a selection wraps it;
/// typing an opener at the caret may insert the closer. `None` = insert `ch` normally.
///
/// * With a selection, any of the pair characters wraps it (`*`/`_` not inside code).
/// * At a caret, `(`, `[`, `{` and `` ` `` (and `"` after whitespace) auto-close only when the
///   next character is whitespace or the end of the text. `*`/`_` never auto-close.
/// * Typing a closing character that is already the next character moves over it.
/// * Inside code blocks only brackets and `"` are paired (no `*`, `_`, `` ` ``).
pub fn on_char(text: &str, blocks: &[Block], sel: Sel, ch: char) -> Option<CommandResult> {
    let sel = clamp_sel(text, sel);
    let closer = match ch {
        '(' => ')',
        '[' => ']',
        '{' => '}',
        '"' => '"',
        '`' => '`',
        '*' => '*',
        '_' => '_',
        ')' | ']' | '}' => ch,
        _ => return None,
    };
    let in_code = code_block_at(blocks, sel.head).is_some_and(|(b, info)| is_code_content_pos(text, b, info, sel.head));
    if in_code && matches!(ch, '*' | '_' | '`') {
        return None;
    }
    let is_opener = matches!(ch, '(' | '[' | '{' | '"' | '`' | '*' | '_');
    if !sel.is_empty() {
        if !is_opener {
            return None;
        }
        let (s, e) = (sel.start(), sel.end());
        let edits = vec![edit(s..s, ch.to_string()), edit(e..e, closer.to_string())];
        let l = ch.len_utf8();
        return finish(text, edits, s + l, e + l).map(|r| keep_dir(r, sel));
    }
    let c = sel.head;
    let next = text[c..].chars().next();
    let prev = text[..c].chars().next_back();
    // type-over
    if next == Some(ch) && matches!(ch, ')' | ']' | '}' | '"' | '`') {
        let p = c + ch.len_utf8();
        return finish(text, vec![], p, p);
    }
    if !is_opener || matches!(ch, '*' | '_') {
        return None;
    }
    let next_ok = next.is_none_or(|n| n.is_whitespace());
    if !next_ok {
        return None;
    }
    if ch == '`' && prev == Some('`') {
        return None; // typing a fence: ``` must not become ````
    }
    if ch == '"' && !prev.is_none_or(|p| p.is_whitespace() || matches!(p, '(' | '[' | '{')) {
        return None;
    }
    let ins = format!("{ch}{closer}");
    let p = c + ch.len_utf8();
    finish(text, vec![edit(c..c, ins)], p, p)
}

/// Replace the info string of the code block containing `at` (language selector, CB-48).
///
/// Only the first word (the language) is replaced; attributes after it (`title=main.rs`)
/// are kept. An empty `lang` removes the language. Indented code blocks and languages that
/// would break the fence (a backtick in a backtick fence, whitespace) return `None`.
pub fn set_code_lang(text: &str, blocks: &[Block], at: usize, lang: &str) -> Option<CommandResult> {
    let at = floor_boundary(text, at);
    let (_, info) = code_block_at(blocks, at)?;
    let of = info.open_fence.clone()?;
    let f = info.fence?;
    let lang = lang.trim();
    if lang.contains(char::is_whitespace) || (f.ch == '`' && lang.contains('`')) {
        return None;
    }
    let after_fence = (of.start + f.len).min(of.end);
    let rest = &text[after_fence..of.end];
    let ws = leading_ws(rest);
    let word_start = after_fence + ws;
    let word_len = text[word_start..of.end].find(char::is_whitespace).unwrap_or(of.end - word_start);
    let word_end = word_start + word_len;
    let edits = if word_len == 0 {
        if lang.is_empty() {
            return None;
        }
        vec![edit(after_fence..word_start, lang.to_string())]
    } else if lang.is_empty() {
        // drop the word and the whitespace after it
        let trail = leading_ws(&text[word_end..of.end]);
        vec![edit(word_start..word_end + trail, "")]
    } else {
        vec![edit(word_start..word_end, lang.to_string())]
    };
    finish_mapped(text, edits, Sel::caret(at))
}

#[cfg(test)]
mod tests;

//! Text buffer for mdedit: a rope holding the exact Markdown, selections, undo/redo,
//! grapheme/word movement, search, and file load/save.
//!
//! All offsets in the public API are **byte offsets** into the UTF-8 text and always lie on
//! char boundaries. Internally the buffer always uses `\n` line endings; [`Document`] restores
//! the file's own line ending on save.
//!
//! OWNER: agent "buffer". Signatures marked STABLE are used by other crates: change them only
//! after telling the integrator.

use std::ops::Range;

mod document;
pub mod fsutil;
mod history;
mod movement;
mod search;

pub use document::{Document, LineEnding, MAX_FILE_SIZE, normalize_text};
pub use movement::{next_grapheme, next_word, prev_grapheme, prev_word, word_at};
pub use search::{MAX_MATCHES, SearchOptions, build_regex, find_all};

/// One replacement: bytes `range` of the text *before* the edit are replaced by `insert`.
/// STABLE.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub range: Range<usize>,
    pub insert: String,
}

impl Edit {
    pub fn insert(at: usize, text: impl Into<String>) -> Self {
        Self { range: at..at, insert: text.into() }
    }
    pub fn delete(range: Range<usize>) -> Self {
        Self { range, insert: String::new() }
    }
    pub fn replace(range: Range<usize>, text: impl Into<String>) -> Self {
        Self { range, insert: text.into() }
    }
}

/// A selection; `anchor == head` is a caret. STABLE.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
}

impl Selection {
    pub fn caret(at: usize) -> Self {
        Self { anchor: at, head: at }
    }
    pub fn range(&self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }
    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }
}

/// How an edit groups into undo history. STABLE.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditKind {
    /// Typing characters: merges with the previous typing step if close in time and position.
    Typing,
    /// Deleting with Backspace/Delete: merges like typing.
    Deleting,
    /// Anything else (paste, commands, formatting): always its own undo step.
    Other,
}

/// The rope plus selection and history. STABLE API.
pub struct Buffer {
    rope: ropey::Rope,
    version: u64,
    /// Primary selection (multi-cursor is not required for v1).
    pub selection: Selection,
    history: History,
}

use history::{Change, History};

impl Default for Buffer {
    fn default() -> Self {
        Self::new("")
    }
}

impl std::fmt::Debug for Buffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Buffer")
            .field("len_bytes", &self.rope.len_bytes())
            .field("version", &self.version)
            .field("selection", &self.selection)
            .finish_non_exhaustive()
    }
}

impl Buffer {
    /// A buffer holding `text` (line endings must already be `\n`).
    pub fn new(text: &str) -> Self {
        Self { rope: ropey::Rope::from_str(text), version: 0, selection: Selection::default(), history: History::default() }
    }
    pub fn rope(&self) -> &ropey::Rope {
        &self.rope
    }
    /// The whole text. O(n): callers that need it every frame should cache by [`Self::version`].
    pub fn text(&self) -> String {
        self.rope.to_string()
    }
    pub fn len_bytes(&self) -> usize {
        self.rope.len_bytes()
    }
    /// Bumps on every change (edit, undo, redo). Caches key off it.
    pub fn version(&self) -> u64 {
        self.version
    }
    /// Clamp `byte` to the text length and round it down to a char boundary. (Added API.)
    pub fn clamp_offset(&self, byte: usize) -> usize {
        let b = byte.min(self.rope.len_bytes());
        self.rope.char_to_byte(self.rope.byte_to_char(b))
    }
    /// Clamp both ends of a selection with [`Self::clamp_offset`]. (Added API.)
    pub fn clamp_selection(&self, sel: Selection) -> Selection {
        Selection { anchor: self.clamp_offset(sel.anchor), head: self.clamp_offset(sel.head) }
    }
    /// Text of `range` (clamped to the text and to char boundaries; reversed ranges are empty).
    pub fn slice(&self, range: Range<usize>) -> String {
        let start = self.clamp_offset(range.start);
        let end = self.clamp_offset(range.end).max(start);
        let (cs, ce) = (self.rope.byte_to_char(start), self.rope.byte_to_char(end));
        self.rope.slice(cs..ce).to_string()
    }
    pub fn line_count(&self) -> usize {
        self.rope.len_lines()
    }
    /// 0-based line containing `byte`.
    pub fn line_of_byte(&self, byte: usize) -> usize {
        self.rope.byte_to_line(byte.min(self.rope.len_bytes()))
    }
    /// Byte offset where 0-based `line` starts.
    pub fn line_start(&self, line: usize) -> usize {
        self.rope.line_to_byte(line.min(self.rope.len_lines()))
    }
    /// (line, column in chars), both 0-based.
    pub fn line_col(&self, byte: usize) -> (usize, usize) {
        let b = byte.min(self.rope.len_bytes());
        let ch = self.rope.byte_to_char(b);
        let line = self.rope.char_to_line(ch);
        (line, ch - self.rope.line_to_char(line))
    }

    /// Normalize edits: clamp, sort, drop overlaps and no-ops.
    fn normalize(&self, mut edits: Vec<Edit>) -> Vec<Edit> {
        for e in &mut edits {
            let start = self.clamp_offset(e.range.start);
            let end = self.clamp_offset(e.range.end).max(start);
            e.range = start..end;
        }
        edits.sort_by_key(|e| (e.range.start, e.range.end));
        let mut out: Vec<Edit> = Vec::with_capacity(edits.len());
        let mut prev_end = 0;
        for mut e in edits {
            // Overlapping edits violate the contract; trim instead of panicking.
            if e.range.start < prev_end {
                e.range.start = prev_end;
                e.range.end = e.range.end.max(prev_end);
            }
            prev_end = e.range.end;
            if e.range.is_empty() && e.insert.is_empty() {
                continue;
            }
            out.push(e);
        }
        out
    }

    /// Apply sorted, normalized edits to the rope; returns the inverse edits.
    fn apply_raw(&mut self, edits: &[Edit]) -> Vec<Edit> {
        let mut inverse = Vec::with_capacity(edits.len());
        let mut delta: isize = 0;
        for e in edits {
            let removed = self.slice(e.range.clone());
            let start = (e.range.start as isize + delta) as usize;
            inverse.push(Edit { range: start..start + e.insert.len(), insert: removed });
            delta += e.insert.len() as isize - e.range.len() as isize;
        }
        for e in edits.iter().rev() {
            let cs = self.rope.byte_to_char(e.range.start);
            let ce = self.rope.byte_to_char(e.range.end);
            if ce > cs {
                self.rope.remove(cs..ce);
            }
            if !e.insert.is_empty() {
                self.rope.insert(cs, &e.insert);
            }
        }
        inverse
    }

    /// Apply `edits` as one transaction. Edits are given against the text *before* the
    /// transaction, must not overlap, and may come in any order. `selection_after` becomes the
    /// selection; undo restores the selection from before. Offsets are clamped to char boundaries.
    ///
    /// Edits that change nothing are dropped; if nothing is left, only the selection is set
    /// (no history entry, version unchanged).
    pub fn apply(&mut self, edits: Vec<Edit>, selection_after: Selection, kind: EditKind) {
        let edits = self.normalize(edits);
        // Drop replacements that replace text with identical text.
        let edits: Vec<Edit> =
            edits.into_iter().filter(|e| e.range.len() != e.insert.len() || self.slice(e.range.clone()) != e.insert).collect();
        if edits.is_empty() {
            self.selection = self.clamp_selection(selection_after);
            return;
        }
        let sel_before = self.selection;
        let inverse = self.apply_raw(&edits);
        self.selection = self.clamp_selection(selection_after);
        self.version += 1;
        self.history.record(Change { forward: edits, inverse }, sel_before, self.selection, kind);
    }
    /// Replace the current selection with `text` (typing / paste), caret after it.
    pub fn insert_at_selection(&mut self, text: &str, kind: EditKind) {
        let sel = self.clamp_selection(self.selection);
        let range = sel.range();
        let caret = range.start + text.len();
        self.apply(vec![Edit::replace(range, text)], Selection::caret(caret), kind);
    }
    /// Undo one step; restores the text and the selection from before it.
    pub fn undo(&mut self) -> bool {
        let Some(step) = self.history.pop_undo() else { return false };
        for change in step.changes.iter().rev() {
            self.apply_raw(&change.inverse);
        }
        self.selection = self.clamp_selection(step.sel_before);
        self.version += 1;
        self.history.push_redo(step);
        true
    }
    /// Redo one step; restores the text and the selection from after it.
    pub fn redo(&mut self) -> bool {
        let Some(step) = self.history.pop_redo() else { return false };
        for change in &step.changes {
            self.apply_raw(&change.forward);
        }
        self.selection = self.clamp_selection(step.sel_after);
        self.version += 1;
        self.history.push_undo(step);
        true
    }
    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }
    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }
    /// Number of undo steps currently stored. (Added API.)
    pub fn undo_depth(&self) -> usize {
        self.history.undo_len()
    }
    /// Close the current undo group so the next typing starts a new step.
    pub fn break_undo_group(&mut self) {
        self.history.break_group();
    }
    /// Replace everything (e.g. external reload). Clears history.
    pub fn set_text(&mut self, text: &str) {
        self.rope = ropey::Rope::from_str(text);
        self.selection = Selection::default();
        self.history = History::default();
        self.version += 1;
    }
}

#[cfg(test)]
mod tests;

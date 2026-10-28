//! Undo/redo history: a stack of steps, each a list of reversible changes.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::{Edit, EditKind, Selection};

/// Maximum number of undo steps kept.
pub(crate) const MAX_STEPS: usize = 1000;
/// Maximum bytes of text stored in the undo stack (inserted + removed text).
pub(crate) const MAX_BYTES: usize = 64 * 1024 * 1024;
/// Typing/deleting within this interval merges into the previous step.
pub(crate) const MERGE_WINDOW: Duration = Duration::from_secs(1);

/// One applied transaction.
#[derive(Clone, Debug)]
pub(crate) struct Change {
    /// Sorted, non-overlapping edits against the text before the change.
    pub forward: Vec<Edit>,
    /// Sorted, non-overlapping edits against the text after the change that restore it.
    pub inverse: Vec<Edit>,
}

impl Change {
    fn bytes(&self) -> usize {
        self.forward.iter().chain(&self.inverse).map(|e| e.insert.len()).sum::<usize>()
            + (self.forward.len() + self.inverse.len()) * std::mem::size_of::<Edit>()
    }
}

/// One undo step (possibly several merged changes).
#[derive(Clone, Debug)]
pub(crate) struct Step {
    pub changes: Vec<Change>,
    pub sel_before: Selection,
    pub sel_after: Selection,
    pub kind: EditKind,
    pub last_time: Instant,
    bytes: usize,
}

#[derive(Default, Debug)]
pub(crate) struct History {
    undo: VecDeque<Step>,
    redo: Vec<Step>,
    undo_bytes: usize,
    /// When set, the next change never merges into the top step.
    broken: bool,
}

fn starts_with_ws(s: &str) -> bool {
    s.chars().next().is_some_and(char::is_whitespace)
}
fn ends_with_word(s: &str) -> bool {
    s.chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '_')
}

impl History {
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn break_group(&mut self) {
        self.broken = true;
    }
    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    /// Whether `change` (just applied) can merge into the top step.
    fn can_merge(&self, change: &Change, kind: EditKind, now: Instant) -> bool {
        if self.broken || kind == EditKind::Other {
            return false;
        }
        let Some(top) = self.undo.back() else { return false };
        if top.kind != kind || now.saturating_duration_since(top.last_time) > MERGE_WINDOW {
            return false;
        }
        let Some(prev) = top.changes.last() else { return false };
        if change.forward.len() != 1 || prev.forward.len() != 1 || prev.inverse.len() != 1 {
            return false;
        }
        let new = &change.forward[0];
        let prev_fwd = &prev.forward[0];
        let prev_inv = &prev.inverse[0];
        match kind {
            EditKind::Typing => {
                // Pure insert, starting where the previous insert ended.
                new.range.is_empty()
                    && !new.insert.is_empty()
                    && new.range.start == prev_inv.range.end
                    && !(starts_with_ws(&new.insert) && ends_with_word(&prev_fwd.insert))
            }
            EditKind::Deleting => {
                // Pure delete adjacent to the previous deletion point (backspace or delete).
                let at = prev_inv.range.start;
                prev_fwd.insert.is_empty()
                    && new.insert.is_empty()
                    && !new.range.is_empty()
                    && (new.range.end == at || new.range.start == at)
            }
            EditKind::Other => false,
        }
    }

    /// Record an applied change.
    pub fn record(&mut self, change: Change, sel_before: Selection, sel_after: Selection, kind: EditKind) {
        let now = Instant::now();
        self.redo.clear();
        let bytes = change.bytes();
        if self.can_merge(&change, kind, now)
            && let Some(top) = self.undo.back_mut()
        {
            top.changes.push(change);
            top.sel_after = sel_after;
            top.last_time = now;
            top.bytes += bytes;
            self.undo_bytes += bytes;
        } else {
            self.undo.push_back(Step { changes: vec![change], sel_before, sel_after, kind, last_time: now, bytes });
            self.undo_bytes += bytes;
        }
        self.broken = false;
        self.enforce_limits();
    }

    fn enforce_limits(&mut self) {
        while self.undo.len() > 1 && (self.undo.len() > MAX_STEPS || self.undo_bytes > MAX_BYTES) {
            if let Some(old) = self.undo.pop_front() {
                self.undo_bytes = self.undo_bytes.saturating_sub(old.bytes);
            }
        }
    }

    /// Pop the step to undo; caller applies it and passes it back via [`Self::push_redo`].
    pub fn pop_undo(&mut self) -> Option<Step> {
        let step = self.undo.pop_back()?;
        self.undo_bytes = self.undo_bytes.saturating_sub(step.bytes);
        self.broken = true;
        Some(step)
    }
    pub fn push_redo(&mut self, step: Step) {
        self.redo.push(step);
    }
    pub fn pop_redo(&mut self) -> Option<Step> {
        self.broken = true;
        self.redo.pop()
    }
    pub fn push_undo(&mut self, step: Step) {
        self.undo_bytes += step.bytes;
        self.undo.push_back(step);
        self.broken = true;
        self.enforce_limits();
    }
}

//! Find / replace bar for the active document.

use std::ops::Range;

use mdedit_buffer::{Edit, EditKind, SearchOptions, Selection, find_all};

use crate::tabs::Tab;

#[derive(Default)]
pub struct FindBar {
    pub open: bool,
    pub replace_mode: bool,
    pub query: String,
    pub replacement: String,
    pub opts: SearchOptions,
    matches: Vec<Range<usize>>,
    current: Option<usize>,
    error: Option<String>,
    /// (tab id, buffer version, query, options) the matches were computed for.
    key: Option<(u64, u64, String, SearchOptions)>,
    focus_query: bool,
    /// After a replace, select the next match once matches are recomputed.
    select_after_recompute: bool,
    /// Move keyboard focus to the replace field next frame (Tab from the query field).
    focus_replace: bool,
    /// One of our text fields had keyboard focus last frame.
    field_focused: bool,
    /// Tab whose editor currently shows our highlights.
    painted_tab: Option<u64>,
}

impl FindBar {
    pub fn open(&mut self, replace: bool, tab: &Tab) {
        self.open = true;
        self.replace_mode = replace;
        self.focus_query = true;
        // Seed the query with a single-line selection.
        let sel = tab.doc.buffer.selection.range();
        if !sel.is_empty() && sel.len() < 200 {
            let s = tab.doc.buffer.slice(sel);
            if !s.is_empty() && !s.contains('\n') {
                self.query = s;
            }
        }
    }

    pub fn close(&mut self, tabs: &mut [Tab]) {
        self.open = false;
        self.clear_highlights(tabs);
    }

    fn clear_highlights(&mut self, tabs: &mut [Tab]) {
        if let Some(id) = self.painted_tab.take()
            && let Some(t) = tabs.iter_mut().find(|t| t.id == id)
        {
            t.editor.search_matches.clear();
            t.editor.search_current = None;
        }
        self.key = None;
    }

    pub fn status_text(&self) -> String {
        if let Some(e) = &self.error {
            return e.clone();
        }
        if self.query.is_empty() {
            return String::new();
        }
        match (self.current, self.matches.len()) {
            (_, 0) => "No results".into(),
            (Some(c), n) => format!("{}/{n}", c + 1),
            (None, n) => format!("{n}"),
        }
    }

    /// Recompute matches if the query/options/document changed.
    pub fn refresh(&mut self, tabs: &mut [Tab], active: usize) {
        if !self.open {
            return;
        }
        let Some(tab_id) = tabs.get(active).map(|t| t.id) else { return };
        if self.painted_tab.is_some_and(|id| id != tab_id) {
            self.clear_highlights(tabs);
        }
        let Some(tab) = tabs.get_mut(active) else { return };
        let key = (tab.id, tab.doc.buffer.version(), self.query.clone(), self.opts.clone());
        if self.key.as_ref() == Some(&key) {
            return;
        }
        self.key = Some(key);
        let text = tab.doc.buffer.text();
        match find_all(&text, &self.query, &self.opts) {
            Ok(m) => {
                self.matches = m;
                self.error = None;
            }
            Err(e) => {
                self.matches.clear();
                self.error = Some(e);
            }
        }
        let caret = tab.doc.buffer.selection.range().start;
        self.current = if self.matches.is_empty() { None } else { Some(self.matches.iter().position(|r| r.start >= caret).unwrap_or(0)) };
        tab.editor.search_matches = self.matches.clone();
        tab.editor.search_current = self.current;
        self.painted_tab = Some(tab.id);
        if std::mem::take(&mut self.select_after_recompute)
            && let Some(c) = self.current
        {
            let r = self.matches[c].clone();
            tab.editor.select(&mut tab.doc, r);
        }
    }

    pub fn next(&mut self, tab: &mut Tab, forward: bool) {
        let n = self.matches.len();
        if n == 0 {
            return;
        }
        let sel = tab.doc.buffer.selection.range();
        let cur = match self.current {
            // If the current match is selected, step; otherwise go to the one at/after the caret.
            Some(c) if self.matches.get(c) == Some(&sel) => {
                if forward {
                    (c + 1) % n
                } else {
                    (c + n - 1) % n
                }
            }
            _ if forward => self.matches.iter().position(|r| r.start >= sel.end).unwrap_or(0),
            _ => self.matches.iter().rposition(|r| r.end <= sel.start).unwrap_or(n - 1),
        };
        self.current = Some(cur);
        tab.editor.search_current = Some(cur);
        tab.editor.select(&mut tab.doc, self.matches[cur].clone());
    }

    pub fn replace_one(&mut self, tab: &mut Tab) {
        let Some(c) = self.current else { return };
        let Some(m) = self.matches.get(c).cloned() else { return };
        if tab.doc.buffer.selection.range() != m {
            // First press selects the match, second press replaces it.
            tab.editor.select(&mut tab.doc, m);
            return;
        }
        let text = tab.doc.buffer.text();
        let (edits, caret) = replace_edits(&text, std::slice::from_ref(&m), &self.query, &self.replacement, &self.opts);
        tab.doc.buffer.apply(edits, Selection::caret(caret), EditKind::Other);
        self.select_after_recompute = true;
    }

    pub fn replace_all(&mut self, tab: &mut Tab) -> usize {
        if self.matches.is_empty() {
            return 0;
        }
        let text = tab.doc.buffer.text();
        let (edits, caret) = replace_edits(&text, &self.matches, &self.query, &self.replacement, &self.opts);
        let n = edits.len();
        tab.doc.buffer.apply(edits, Selection::caret(caret), EditKind::Other);
        n
    }

    /// Draw the bar. Returns a message for the toast area (e.g. "Replaced 3").
    pub fn ui(&mut self, ui: &mut egui::Ui, tabs: &mut [Tab], active: usize) -> Option<String> {
        let mut msg = None;
        let mut close = false;
        let tab = tabs.get_mut(active)?;
        // Undo/redo while typing in the bar act on the document, not on the query text.
        if self.field_focused {
            let (undo, redo) = ui.input_mut(|i| {
                let z = egui::Key::Z;
                let redo = i.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, z) || i.consume_key(egui::Modifiers::COMMAND, egui::Key::Y);
                (i.consume_key(egui::Modifiers::COMMAND, z), redo)
            });
            if undo {
                tab.doc.buffer.undo();
            }
            if redo {
                tab.doc.buffer.redo();
            }
        }
        let mut focused = false;
        ui.horizontal(|ui| {
            let resp = ui.add(egui::TextEdit::singleline(&mut self.query).hint_text("Find").desired_width(220.0).id_salt("find-query"));
            if std::mem::take(&mut self.focus_query) {
                resp.request_focus();
                if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), resp.id) {
                    let all = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(self.query.chars().count()));
                    state.cursor.set_char_range(Some(all));
                    state.store(ui.ctx(), resp.id);
                }
            }
            focused |= resp.has_focus();
            if resp.changed() {
                // Incremental search: jump to the first match at or after the caret.
                self.select_after_recompute = true;
            }
            if self.replace_mode && resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Tab) && !i.modifiers.shift) {
                self.focus_replace = true;
            }
            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                let back = ui.input(|i| i.modifiers.shift);
                self.next(tab, !back);
                resp.request_focus();
            }
            if resp.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                close = true;
            }
            ui.toggle_value(&mut self.opts.case_sensitive, "Aa").on_hover_text("Match case");
            ui.toggle_value(&mut self.opts.whole_word, "W").on_hover_text("Whole word");
            ui.toggle_value(&mut self.opts.regex, ".*").on_hover_text("Regular expression");
            let status = self.status_text();
            if self.error.is_some() {
                ui.colored_label(ui.visuals().error_fg_color, status).on_hover_text("Invalid regular expression");
            } else {
                ui.label(status);
            }
            if ui.button("⏶").on_hover_text("Previous (Shift+Enter / Shift+F3)").clicked() {
                self.next(tab, false);
            }
            if ui.button("⏷").on_hover_text("Next (Enter / F3)").clicked() {
                self.next(tab, true);
            }
            if !self.replace_mode && ui.button("Replace…").clicked() {
                self.replace_mode = true;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("✕").on_hover_text("Close (Esc)").clicked() {
                    close = true;
                }
            });
        });
        if self.replace_mode {
            ui.horizontal(|ui| {
                let resp = ui.add(egui::TextEdit::singleline(&mut self.replacement).hint_text(if self.opts.regex { "Replace ($1 for groups)" } else { "Replace" }).desired_width(220.0).id_salt("find-replace"));
                if std::mem::take(&mut self.focus_replace) {
                    resp.request_focus();
                }
                focused |= resp.has_focus();
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    self.replace_one(tab);
                    resp.request_focus();
                }
                if resp.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    close = true;
                }
                if ui.button("Replace").clicked() {
                    self.replace_one(tab);
                }
                if ui.button("Replace All").clicked() {
                    let n = self.replace_all(tab);
                    msg = Some(format!("Replaced {n} occurrence{}", if n == 1 { "" } else { "s" }));
                }
            });
        }
        self.field_focused = focused;
        if close {
            self.close(tabs);
            if let Some(t) = tabs.get_mut(active) {
                t.editor.request_focus = true;
            }
        }
        msg
    }
}

/// Edits replacing each match, plus the caret offset (in the new text) after the last one.
/// In regex mode `$1`/`${name}` in `replacement` expand to capture groups.
pub fn replace_edits(text: &str, matches: &[Range<usize>], query: &str, replacement: &str, opts: &SearchOptions) -> (Vec<Edit>, usize) {
    let re = if opts.regex { build_regex(query, opts) } else { None };
    let mut edits = Vec::with_capacity(matches.len());
    let mut delta: isize = 0;
    let mut caret = 0usize;
    for m in matches {
        if m.start > m.end || m.end > text.len() || !text.is_char_boundary(m.start) || !text.is_char_boundary(m.end) {
            continue;
        }
        let insert = match &re {
            Some(re) => match re.captures_at(text, m.start) {
                Some(caps) if caps.get(0).is_some_and(|g| g.range() == *m) => {
                    let mut s = String::new();
                    caps.expand(replacement, &mut s);
                    s
                }
                _ => replacement.to_string(),
            },
            None => replacement.to_string(),
        };
        let new_start = (m.start as isize + delta).max(0) as usize;
        caret = new_start + insert.len();
        delta += insert.len() as isize - m.len() as isize;
        edits.push(Edit::replace(m.clone(), insert));
    }
    (edits, caret)
}

fn build_regex(query: &str, opts: &SearchOptions) -> Option<regex::Regex> {
    let pat = if opts.whole_word { format!(r"\b(?:{query})\b") } else { query.to_string() };
    regex::RegexBuilder::new(&pat).case_insensitive(!opts.case_sensitive).multi_line(true).size_limit(10 << 20).build().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(text: &str, edits: &[Edit]) -> String {
        let mut sorted: Vec<&Edit> = edits.iter().collect();
        sorted.sort_by_key(|e| std::cmp::Reverse(e.range.start));
        let mut s = text.to_string();
        for e in sorted {
            s.replace_range(e.range.clone(), &e.insert);
        }
        s
    }

    #[test]
    fn replace_all_literal() {
        let text = "cat hat cat";
        let m = vec![0..3, 8..11];
        let (edits, caret) = replace_edits(text, &m, "cat", "dog!", &SearchOptions::default());
        let out = apply(text, &edits);
        assert_eq!(out, "dog! hat dog!");
        assert_eq!(caret, out.len());
        // Literal mode: "$1" is not expanded.
        let (edits, _) = replace_edits(text, &m, "cat", "$1", &SearchOptions::default());
        assert_eq!(apply(text, &edits), "$1 hat $1");
    }

    #[test]
    fn replace_all_regex_groups() {
        let text = "a=1, b=22";
        let opts = SearchOptions { regex: true, ..Default::default() };
        let m = vec![0..3, 5..9];
        let (edits, caret) = replace_edits(text, &m, r"(\w)=(\d+)", "$2:$1", &opts);
        let out = apply(text, &edits);
        assert_eq!(out, "1:a, 22:b");
        assert_eq!(caret, out.len());
    }

    #[test]
    fn replace_shrinking_caret_and_bad_ranges() {
        let text = "aaaa bb aaaa";
        let (edits, caret) = replace_edits(text, &[0..4, 8..12, 100..200], "aaaa", "x", &SearchOptions::default());
        assert_eq!(edits.len(), 2);
        let out = apply(text, &edits);
        assert_eq!(out, "x bb x");
        assert_eq!(caret, 6);
        // Non-char-boundary range skipped.
        let (edits, _) = replace_edits("é", std::slice::from_ref(&(0..1)), "x", "y", &SearchOptions::default());
        assert!(edits.is_empty());
    }

    #[test]
    fn whole_word_regex_case_insensitive() {
        let opts = SearchOptions { regex: true, whole_word: true, case_sensitive: false };
        let re = build_regex("ca(t)", &opts).unwrap();
        assert!(re.is_match("the CAT sat"));
        assert!(!re.is_match("concatenate"));
    }
}

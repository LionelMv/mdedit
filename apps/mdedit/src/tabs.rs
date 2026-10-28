//! Open documents (tabs), recovery ids, titles.

use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use mdedit_buffer::{Document, Selection};
use mdedit_editor::{EditorOutput, EditorState};
use mdedit_style::Mode;

/// Files larger than this open in Source mode with a banner.
pub const LARGE_FILE_BYTES: u64 = 20 * 1024 * 1024;

pub struct Tab {
    /// Stable id (survives reordering; background results refer to it).
    pub id: u64,
    pub doc: Document,
    pub editor: EditorState,
    /// Crash-recovery snapshot id.
    pub recovery_id: String,
    /// Buffer version last written to the recovery snapshot.
    pub snapshot_version: Option<u64>,
    /// Hash of the file content as we last loaded/saved it (to ignore our own writes).
    pub disk_hash: Option<u64>,
    /// "File changed on disk" banner (dirty doc changed externally).
    pub disk_changed: bool,
    /// Large-file banner (size in bytes) until dismissed.
    pub large_file: Option<u64>,
    /// Last editor output (status bar).
    pub status: EditorOutput,
}

impl Tab {
    pub fn new(id: u64, doc: Document, mode: Mode) -> Self {
        let mut editor = EditorState::new();
        editor.mode = mode;
        Self { id, doc, editor, recovery_id: new_recovery_id(), snapshot_version: None, disk_hash: None, disk_changed: false, large_file: None, status: EditorOutput::default() }
    }

    /// A pristine untitled document: no path, no edits, empty.
    pub fn is_pristine(&self) -> bool {
        self.doc.path.is_none() && !self.doc.is_dirty() && self.doc.buffer.len_bytes() == 0
    }

    pub fn base_dir(&self) -> Option<&Path> {
        self.doc.path.as_deref().and_then(Path::parent)
    }

    /// Put the caret at `at` clamped to the text and a char boundary.
    pub fn clamp_caret(&mut self, at: usize) {
        let rope = self.doc.buffer.rope();
        let len = rope.len_bytes();
        // byte_to_char gives the char containing the byte: snaps down to its start.
        let at = rope.char_to_byte(rope.byte_to_char(at.min(len)));
        self.doc.buffer.selection = Selection::caret(at);
    }
}

/// `"• name — mdedit"` (bullet when dirty).
pub fn window_title(name: &str, dirty: bool) -> String {
    if dirty { format!("• {name} — mdedit") } else { format!("{name} — mdedit") }
}

/// Tab label: name plus a dirty dot.
pub fn tab_label(name: &str, dirty: bool) -> String {
    if dirty { format!("{name} •") } else { name.to_string() }
}

static RECOVERY_COUNTER: AtomicU64 = AtomicU64::new(0);

/// `r<pid>-<unix millis>-<n>`: unique per process and matching `[A-Za-z0-9_-]+`.
pub fn new_recovery_id() -> String {
    let millis = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    format_recovery_id(std::process::id(), millis, RECOVERY_COUNTER.fetch_add(1, Ordering::Relaxed))
}

pub fn format_recovery_id(pid: u32, millis: u128, n: u64) -> String {
    format!("r{pid}-{millis}-{n}")
}

/// The pid embedded in a recovery id made by [`format_recovery_id`].
pub fn recovery_id_pid(id: &str) -> Option<u32> {
    id.strip_prefix('r')?.split('-').next()?.parse().ok()
}

/// Whether a snapshot belongs to another *running* mdedit (don't offer to restore those).
pub fn snapshot_owned_by_live_process(id: &str) -> bool {
    let Some(pid) = recovery_id_pid(id) else { return false };
    if pid == std::process::id() {
        return false;
    }
    std::fs::read_to_string(format!("/proc/{pid}/comm")).map(|c| c.trim() == "mdedit").unwrap_or(false)
}

pub fn hash_text(text: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles() {
        assert_eq!(window_title("notes.md", false), "notes.md — mdedit");
        assert_eq!(window_title("Untitled", true), "• Untitled — mdedit");
        assert_eq!(tab_label("a.md", true), "a.md •");
        assert_eq!(tab_label("a.md", false), "a.md");
    }

    #[test]
    fn recovery_ids_are_unique_and_valid() {
        let a = new_recovery_id();
        let b = new_recovery_id();
        assert_ne!(a, b);
        for id in [&a, &b] {
            assert!(id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'), "{id}");
        }
        assert_eq!(recovery_id_pid(&a), Some(std::process::id()));
        assert_eq!(format_recovery_id(42, 1000, 7), "r42-1000-7");
        assert_eq!(recovery_id_pid("r42-1000-7"), Some(42));
        assert_eq!(recovery_id_pid("garbage"), None);
        assert!(!snapshot_owned_by_live_process(&a)); // our own pid
        assert!(!snapshot_owned_by_live_process("x"));
    }

    #[test]
    fn clamp_caret_snaps_to_char_boundary() {
        let mut doc = Document::new();
        doc.buffer.set_text("héllo");
        let mut tab = Tab::new(1, doc, Mode::Source);
        let len = tab.doc.buffer.len_bytes();
        tab.clamp_caret(1000);
        assert_eq!(tab.doc.buffer.selection.head, len);
        if len == "héllo".len() {
            tab.clamp_caret(2); // inside 'é'
            assert_eq!(tab.doc.buffer.selection.head, 1);
        }
    }
}

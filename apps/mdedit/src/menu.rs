//! Menu bar, actions and keyboard shortcuts.

use std::path::PathBuf;

use egui::{Key, KeyboardShortcut, Modifiers};
use mdedit_editor::Command;

/// Everything the user can trigger from menus or shortcuts.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    New,
    Open,
    OpenFolder,
    OpenRecent(PathBuf),
    ClearRecent,
    Save,
    SaveAs,
    ExportHtml,
    CopyHtml,
    CloseTab,
    Quit,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
    Find,
    Replace,
    FindNext,
    FindPrev,
    Format(Command),
    InsertTableDialog,
    ToggleSource,
    ToggleSidebar,
    ToggleTypewriter,
    ToggleFocusMode,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    Theme(&'static str),
    Fullscreen,
    NextTab,
    PrevTab,
    Preferences,
    About,
    Cheatsheet,
    ShortcutsHelp,
}

const CMD: Modifiers = Modifiers::COMMAND;
const CMD_SHIFT: Modifiers = Modifiers { shift: true, ..Modifiers::COMMAND };
const CMD_ALT: Modifiers = Modifiers { alt: true, ..Modifiers::COMMAND };

/// App shortcuts, **most specific first** (egui's matching ignores extra Shift/Alt).
/// Zoom (Ctrl+= / Ctrl+- / Ctrl+0) is handled by egui itself.
pub fn shortcuts() -> Vec<(KeyboardShortcut, Action)> {
    use Action as A;
    use Command as C;
    let s = |m, k| KeyboardShortcut::new(m, k);
    vec![
        (s(CMD_SHIFT, Key::Tab), A::PrevTab),
        (s(CMD_SHIFT, Key::S), A::SaveAs),
        (s(CMD_SHIFT, Key::O), A::OpenFolder),
        (s(CMD_SHIFT, Key::B), A::ToggleSidebar),
        (s(CMD_SHIFT, Key::I), A::Format(C::Image)),
        (s(CMD_SHIFT, Key::Q), A::Format(C::BlockQuote)),
        (s(CMD_SHIFT, Key::K), A::Format(C::CodeBlock)),
        (s(CMD_SHIFT, Key::M), A::Format(C::MathBlock)),
        (s(CMD_SHIFT, Key::T), A::InsertTableDialog),
        (s(CMD_SHIFT, Key::Num0), A::Format(C::Heading(0))),
        (s(CMD_ALT, Key::O), A::Format(C::OrderedList)),
        (s(CMD_ALT, Key::U), A::Format(C::BulletList)),
        (s(CMD_ALT, Key::X), A::Format(C::TaskList)),
        (s(CMD_ALT, Key::Minus), A::Format(C::HorizontalRule)),
        (s(CMD, Key::Tab), A::NextTab),
        (s(CMD, Key::N), A::New),
        (s(CMD, Key::O), A::Open),
        (s(CMD, Key::S), A::Save),
        (s(CMD, Key::W), A::CloseTab),
        (s(CMD, Key::Q), A::Quit),
        (s(CMD, Key::F), A::Find),
        (s(CMD, Key::H), A::Replace),
        (s(CMD, Key::E), A::ToggleSource),
        (s(CMD, Key::Comma), A::Preferences),
        (s(CMD, Key::B), A::Format(C::Bold)),
        (s(CMD, Key::I), A::Format(C::Italic)),
        (s(CMD, Key::D), A::Format(C::Strikethrough)),
        (s(CMD, Key::Backtick), A::Format(C::InlineCode)),
        (s(CMD, Key::L), A::Format(C::Link)),
        (s(CMD, Key::Num1), A::Format(C::Heading(1))),
        (s(CMD, Key::Num2), A::Format(C::Heading(2))),
        (s(CMD, Key::Num3), A::Format(C::Heading(3))),
        (s(CMD, Key::Num4), A::Format(C::Heading(4))),
        (s(CMD, Key::Num5), A::Format(C::Heading(5))),
        (s(CMD, Key::Num6), A::Format(C::Heading(6))),
        (s(Modifiers::SHIFT, Key::F3), A::FindPrev),
        (s(Modifiers::NONE, Key::F3), A::FindNext),
        (s(Modifiers::NONE, Key::F11), A::Fullscreen),
    ]
}

/// Shortcut text for a menu item, if it has one.
pub fn shortcut_text(ctx: &egui::Context, action: &Action) -> Option<String> {
    let fixed = match action {
        Action::ZoomIn => Some("Ctrl++"),
        Action::ZoomOut => Some("Ctrl+-"),
        Action::ZoomReset => Some("Ctrl+0"),
        Action::Undo => Some("Ctrl+Z"),
        Action::Redo => Some("Ctrl+Shift+Z"),
        Action::Cut => Some("Ctrl+X"),
        Action::Copy => Some("Ctrl+C"),
        Action::Paste => Some("Ctrl+V"),
        Action::SelectAll => Some("Ctrl+A"),
        _ => None,
    };
    if let Some(f) = fixed {
        return Some(f.to_string());
    }
    shortcuts().into_iter().find(|(_, a)| a == action).map(|(s, _)| ctx.format_shortcut(&s))
}

/// Consume pressed app shortcuts (before the editor sees them).
pub fn consume_shortcuts(ctx: &egui::Context, out: &mut Vec<Action>) {
    let list = shortcuts();
    ctx.input_mut(|i| {
        for (sc, action) in &list {
            if i.consume_shortcut(sc) {
                out.push(action.clone());
            }
        }
    });
}

/// State the menus need to render check marks etc.
pub struct MenuState<'a> {
    pub recent: &'a [PathBuf],
    pub source_mode: bool,
    pub sidebar: bool,
    pub typewriter: bool,
    pub focus_mode: bool,
    pub theme: &'a str,
    pub fullscreen: bool,
    pub can_undo: bool,
    pub can_redo: bool,
}

fn item(ui: &mut egui::Ui, label: &str, action: Action, out: &mut Vec<Action>) {
    let mut b = egui::Button::new(label);
    if let Some(sc) = shortcut_text(ui.ctx(), &action) {
        b = b.shortcut_text(sc);
    }
    if ui.add(b).clicked() {
        out.push(action);
        ui.close();
    }
}

fn item_enabled(ui: &mut egui::Ui, enabled: bool, label: &str, action: Action, out: &mut Vec<Action>) {
    ui.add_enabled_ui(enabled, |ui| item(ui, label, action, out));
}

fn check(ui: &mut egui::Ui, on: bool, label: &str, action: Action, out: &mut Vec<Action>) {
    let mut b = egui::Button::selectable(on, label);
    if let Some(sc) = shortcut_text(ui.ctx(), &action) {
        b = b.shortcut_text(sc);
    }
    if ui.add(b).clicked() {
        out.push(action);
        ui.close();
    }
}

pub fn menu_bar(ui: &mut egui::Ui, st: &MenuState<'_>, out: &mut Vec<Action>) {
    use Action as A;
    use Command as C;
    egui::MenuBar::new().ui(ui, |ui| {
        ui.menu_button("File", |ui| {
            item(ui, "New", A::New, out);
            item(ui, "Open…", A::Open, out);
            item(ui, "Open Folder…", A::OpenFolder, out);
            ui.menu_button("Open Recent", |ui| {
                if st.recent.is_empty() {
                    ui.weak("No recent files");
                }
                for p in st.recent.iter().take(20) {
                    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string());
                    if ui.button(name).on_hover_text(p.display().to_string()).clicked() {
                        out.push(A::OpenRecent(p.clone()));
                        ui.close();
                    }
                }
                if !st.recent.is_empty() {
                    ui.separator();
                    item(ui, "Clear Recent", A::ClearRecent, out);
                }
            });
            ui.separator();
            item(ui, "Save", A::Save, out);
            item(ui, "Save As…", A::SaveAs, out);
            ui.separator();
            item(ui, "Export HTML…", A::ExportHtml, out);
            item(ui, "Copy as HTML", A::CopyHtml, out);
            ui.separator();
            item(ui, "Preferences…", A::Preferences, out);
            ui.separator();
            item(ui, "Close Tab", A::CloseTab, out);
            item(ui, "Quit", A::Quit, out);
        });
        ui.menu_button("Edit", |ui| {
            item_enabled(ui, st.can_undo, "Undo", A::Undo, out);
            item_enabled(ui, st.can_redo, "Redo", A::Redo, out);
            ui.separator();
            item(ui, "Cut", A::Cut, out);
            item(ui, "Copy", A::Copy, out);
            item(ui, "Paste", A::Paste, out);
            item(ui, "Select All", A::SelectAll, out);
            ui.separator();
            item(ui, "Find…", A::Find, out);
            item(ui, "Replace…", A::Replace, out);
            item(ui, "Find Next", A::FindNext, out);
            item(ui, "Find Previous", A::FindPrev, out);
        });
        ui.menu_button("Paragraph", |ui| {
            for level in 1..=6u8 {
                item(ui, &format!("Heading {level}"), A::Format(C::Heading(level)), out);
            }
            item(ui, "Paragraph", A::Format(C::Heading(0)), out);
            ui.separator();
            item(ui, "Quote", A::Format(C::BlockQuote), out);
            item(ui, "Bullet List", A::Format(C::BulletList), out);
            item(ui, "Ordered List", A::Format(C::OrderedList), out);
            item(ui, "Task List", A::Format(C::TaskList), out);
            item(ui, "Toggle Task", A::Format(C::ToggleTask), out);
            ui.separator();
            item(ui, "Code Block", A::Format(C::CodeBlock), out);
            item(ui, "Math Block", A::Format(C::MathBlock), out);
            item(ui, "Table…", A::InsertTableDialog, out);
            item(ui, "Horizontal Rule", A::Format(C::HorizontalRule), out);
        });
        ui.menu_button("Format", |ui| {
            item(ui, "Bold", A::Format(C::Bold), out);
            item(ui, "Italic", A::Format(C::Italic), out);
            item(ui, "Strikethrough", A::Format(C::Strikethrough), out);
            item(ui, "Inline Code", A::Format(C::InlineCode), out);
            ui.separator();
            item(ui, "Link", A::Format(C::Link), out);
            item(ui, "Image", A::Format(C::Image), out);
        });
        ui.menu_button("View", |ui| {
            check(ui, st.source_mode, "Source Mode", A::ToggleSource, out);
            check(ui, st.sidebar, "Sidebar", A::ToggleSidebar, out);
            check(ui, st.typewriter, "Typewriter Mode", A::ToggleTypewriter, out);
            check(ui, st.focus_mode, "Focus Mode", A::ToggleFocusMode, out);
            ui.separator();
            item(ui, "Zoom In", A::ZoomIn, out);
            item(ui, "Zoom Out", A::ZoomOut, out);
            item(ui, "Reset Zoom", A::ZoomReset, out);
            ui.separator();
            ui.menu_button("Theme", |ui| {
                check(ui, st.theme == "light", "Light", A::Theme("light"), out);
                check(ui, st.theme == "dark", "Dark", A::Theme("dark"), out);
                check(ui, st.theme != "light" && st.theme != "dark", "Follow System", A::Theme("system"), out);
            });
            check(ui, st.fullscreen, "Full Screen", A::Fullscreen, out);
            ui.separator();
            item(ui, "Next Tab", A::NextTab, out);
            item(ui, "Previous Tab", A::PrevTab, out);
        });
        ui.menu_button("Help", |ui| {
            item(ui, "Markdown Cheatsheet", A::Cheatsheet, out);
            item(ui, "Keyboard Shortcuts", A::ShortcutsHelp, out);
            ui.separator();
            item(ui, "About mdedit", A::About, out);
        });
    });
}

/// Human-readable list for the shortcuts window.
pub fn shortcut_help_rows(ctx: &egui::Context) -> Vec<(String, String)> {
    let mut rows: Vec<(String, String)> = shortcuts().into_iter().map(|(s, a)| (ctx.format_shortcut(&s), describe(&a))).collect();
    for (k, d) in [
        ("Ctrl++ / Ctrl+-", "Zoom in / out"),
        ("Ctrl+0", "Reset zoom"),
        ("Ctrl+Z / Ctrl+Shift+Z", "Undo / redo"),
        ("Ctrl+click", "Open link"),
        ("Enter / Shift+Enter", "Next / previous match (in find bar)"),
        ("Esc", "Close find bar"),
    ] {
        rows.push((k.into(), d.into()));
    }
    rows
}

fn describe(a: &Action) -> String {
    match a {
        Action::Format(c) => match c {
            Command::Heading(0) => "Paragraph".into(),
            Command::Heading(n) => format!("Heading {n}"),
            other => format!("{other:?}"),
        },
        Action::InsertTableDialog => "Insert table".into(),
        other => {
            let s = format!("{other:?}");
            // CamelCase → words.
            let mut out = String::new();
            for (i, ch) in s.chars().enumerate() {
                if i > 0 && ch.is_uppercase() {
                    out.push(' ');
                    out.extend(ch.to_lowercase());
                } else {
                    out.push(ch);
                }
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specific_shortcuts_come_first() {
        // egui's consume_shortcut ignores extra Shift/Alt, so a Ctrl+Shift+X entry must come
        // before Ctrl+X for the same key.
        let list = shortcuts();
        for (i, (a, _)) in list.iter().enumerate() {
            for (b, _) in &list[i + 1..] {
                if a.logical_key == b.logical_key {
                    let a_more = (a.modifiers.shift && !b.modifiers.shift) || (a.modifiers.alt && !b.modifiers.alt);
                    let b_more = (b.modifiers.shift && !a.modifiers.shift) || (b.modifiers.alt && !a.modifiers.alt);
                    assert!(!b_more || a_more, "{a:?} shadows {b:?}");
                }
            }
        }
    }

    #[test]
    fn no_duplicate_shortcuts() {
        let list = shortcuts();
        for (i, (a, _)) in list.iter().enumerate() {
            assert!(!list[i + 1..].iter().any(|(b, _)| b == a), "duplicate {a:?}");
        }
    }

    #[test]
    fn describe_actions() {
        assert_eq!(describe(&Action::ToggleSidebar), "Toggle sidebar");
        assert_eq!(describe(&Action::Format(Command::Heading(2))), "Heading 2");
    }
}

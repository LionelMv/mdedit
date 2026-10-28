//! Modal dialogs and small windows: unsaved changes, crash recovery, table size, about,
//! keyboard shortcuts.

use mdedit_workspace::recovery::Snapshot;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveChoice {
    Save,
    DontSave,
    Cancel,
}

/// "Save changes to X?" modal. Returns the choice once made.
pub fn unsaved_changes_modal(ctx: &egui::Context, name: &str) -> Option<SaveChoice> {
    let mut choice = None;
    let resp = egui::Modal::new(egui::Id::unique("mdedit-unsaved")).show(ctx, |ui| {
        ui.set_max_width(380.0);
        ui.heading("Unsaved changes");
        ui.add_space(6.0);
        ui.label(format!("Save changes to \"{name}\" before closing?"));
        ui.weak("Your changes will be lost if you don't save them.");
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui.button("Save").clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                choice = Some(SaveChoice::Save);
            }
            if ui.button("Don't Save").clicked() {
                choice = Some(SaveChoice::DontSave);
            }
            if ui.button("Cancel").clicked() {
                choice = Some(SaveChoice::Cancel);
            }
        });
    });
    if resp.should_close() && choice.is_none() {
        choice = Some(SaveChoice::Cancel);
    }
    choice
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryChoice {
    Restore,
    Discard,
    Later,
}

pub fn recovery_modal(ctx: &egui::Context, snaps: &[Snapshot]) -> Option<RecoveryChoice> {
    let mut choice = None;
    egui::Modal::new(egui::Id::unique("mdedit-recovery")).show(ctx, |ui| {
        ui.set_max_width(460.0);
        ui.heading("Recover unsaved documents?");
        ui.add_space(6.0);
        ui.label("mdedit did not close cleanly. These unsaved documents can be restored:");
        ui.add_space(4.0);
        egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
            for s in snaps {
                let name = s.path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "Untitled".into());
                let age = s.modified.elapsed().map(|d| human_age(d.as_secs())).unwrap_or_default();
                let words = s.text.split_whitespace().count();
                ui.label(format!("• {name} ({words} words{})", if age.is_empty() { String::new() } else { format!(", {age}") }));
            }
        });
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui.button("Restore").clicked() {
                choice = Some(RecoveryChoice::Restore);
            }
            if ui.button("Discard").on_hover_text("Delete the snapshots permanently").clicked() {
                choice = Some(RecoveryChoice::Discard);
            }
            if ui.button("Decide Later").on_hover_text("Keep the snapshots and ask again next time").clicked() {
                choice = Some(RecoveryChoice::Later);
            }
        });
    });
    choice
}

pub fn human_age(secs: u64) -> String {
    match secs {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", secs / 60),
        3600..=86_399 => format!("{} h ago", secs / 3600),
        _ => format!("{} days ago", secs / 86_400),
    }
}

/// Table size dialog state.
pub struct TableDialog {
    pub open: bool,
    pub cols: usize,
    pub rows: usize,
}

impl Default for TableDialog {
    fn default() -> Self {
        Self { open: false, cols: 3, rows: 3 }
    }
}

impl TableDialog {
    /// Returns `(cols, rows)` when the user confirms.
    pub fn show(&mut self, ctx: &egui::Context) -> Option<(usize, usize)> {
        if !self.open {
            return None;
        }
        let mut result = None;
        let mut close = false;
        let resp = egui::Modal::new(egui::Id::unique("mdedit-table")).show(ctx, |ui| {
            ui.heading("Insert table");
            ui.add_space(6.0);
            egui::Grid::new("table-grid").num_columns(2).show(ui, |ui| {
                ui.label("Columns");
                ui.add(egui::DragValue::new(&mut self.cols).range(1..=30));
                ui.end_row();
                ui.label("Rows");
                ui.add(egui::DragValue::new(&mut self.rows).range(1..=200));
                ui.end_row();
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Insert").clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    result = Some((self.cols.clamp(1, 30), self.rows.clamp(1, 200)));
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
        });
        if resp.should_close() || close || result.is_some() {
            self.open = false;
        }
        result
    }
}

pub fn about_window(ctx: &egui::Context, open: &mut bool) {
    egui::Window::new("About mdedit").open(open).collapsible(false).resizable(false).show(ctx, |ui| {
        ui.vertical_centered(|ui| {
            ui.heading("mdedit");
            ui.label(format!("Version {}", crate::cli::VERSION));
            ui.add_space(4.0);
            ui.label("A native Markdown editor for Linux.");
            ui.weak("Rust · egui · pulldown-cmark · syntect");
            ui.add_space(4.0);
            ui.weak("License: MIT OR Apache-2.0");
        });
    });
}

pub fn shortcuts_window(ctx: &egui::Context, open: &mut bool) {
    egui::Window::new("Keyboard Shortcuts").open(open).collapsible(false).default_width(420.0).show(ctx, |ui| {
        egui::ScrollArea::vertical().max_height(480.0).show(ui, |ui| {
            egui::Grid::new("shortcut-grid").num_columns(2).striped(true).spacing([24.0, 4.0]).show(ui, |ui| {
                for (k, d) in crate::menu::shortcut_help_rows(ctx) {
                    ui.monospace(k);
                    ui.label(d);
                    ui.end_row();
                }
            });
        });
    });
}

pub const CHEATSHEET: &str = r#"# Markdown Cheatsheet

## Headings

# Heading 1
## Heading 2
### Heading 3

## Emphasis

*italic* or _italic_, **bold**, ***bold italic***, ~~strikethrough~~, `inline code`

## Lists

- Bullet item
- Another item
  - Nested item

1. First
2. Second

- [ ] Task to do
- [x] Task done

## Links and images

[Link text](https://example.com)
<https://example.com>
![Alt text](image.png)

## Quotes

> A block quote.
> > Nested quote.

## Code

```rust
fn main() {
    println!("Hello, world!");
}
```

## Tables

| Left | Center | Right |
|:-----|:------:|------:|
| a    |   b    |     c |

## Math

Inline $E = mc^2$ and a block:

$$
\int_0^1 x^2\,dx = \frac{1}{3}
$$

## Footnotes

Here is a footnote reference.[^1]

[^1]: The footnote text.

## Horizontal rule

---
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ages() {
        assert_eq!(human_age(5), "just now");
        assert_eq!(human_age(120), "2 min ago");
        assert_eq!(human_age(7200), "2 h ago");
        assert_eq!(human_age(3 * 86_400), "3 days ago");
    }
}

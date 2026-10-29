//! Visual smoke test: renders the editor with wgpu and writes a PNG for manual inspection.
//! Ignored by default (needs a GPU/software adapter):
//! `MDEDIT_SHOT=/tmp/shot.png cargo test -p mdedit-editor --test render -- --ignored`

use egui_kittest::Harness;
use mdedit_buffer::{Buffer, Document, Selection};
use mdedit_editor::EditorState;
use mdedit_highlight::Highlighter;
use mdedit_style::Theme;

const SAMPLE: &str = "# Hello mdedit\n\nSome **bold**, *italic* and `inline code` text with a [link](https://e.com).\nA second line that is long enough to wrap around the column when the window is narrow enough to need it.\n\n- [ ] open task\n- [x] done task\n- bullet item\n  - nested item\n\n> A quote\n> over two lines\n\n```rust\nfn main() {\n\tprintln!(\"tab\");\n}\n```\n\n| col a | col b |\n|---|:---:|\n| 1 | two |\n\n---\n\n## Heading 2\n\nEnd paragraph.\n";

fn shot(name: &str, caret: usize, dark: bool) {
    let Some(dir) = std::env::var_os("MDEDIT_SHOT_DIR") else { return };
    let mut doc = Document::new();
    doc.buffer = Buffer::new(SAMPLE);
    doc.buffer.selection = Selection { anchor: caret, head: caret + 4 };
    let mut ed = EditorState::new();
    ed.search_matches = std::iter::once(8..14).collect();
    let hl = Highlighter::new();
    let theme = if dark { Theme::dark() } else { Theme::light() };
    let mut harness = Harness::builder().with_size(egui::vec2(900.0, 900.0)).build_ui(move |ui| {
        ui.painter().rect_filled(ui.max_rect(), 0.0, theme.bg);
        ed.show(ui, &mut doc, &theme, &hl, None);
    });
    harness.run_steps(4);
    let img = harness.render().expect("render");
    let path = std::path::Path::new(&dir).join(name);
    img.save(&path).expect("save");
    eprintln!("wrote {}", path.display());
}

#[test]
#[ignore]
fn render_screenshots() {
    shot("light_caret_top.png", 2, false);
    shot("light_caret_code.png", SAMPLE.find("println").unwrap_or(0), false);
    shot("dark_caret_list.png", SAMPLE.find("open task").unwrap_or(0), true);
    shot("dark_caret_table.png", SAMPLE.find("1 | two").unwrap_or(0), true);
    shot("dark_table.png", 2, true);
}

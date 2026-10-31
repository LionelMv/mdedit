//! Preferences window. Edits `Config` in place (live-applied by the app each frame).

use mdedit_workspace::Config;

/// Draw the window. Hand-edited values outside the widgets' ranges stay as they are. Returns `true` when the window was closed this frame.
pub fn preferences_window(ctx: &egui::Context, open: &mut bool, cfg: &mut Config) -> bool {
    let was_open = *open;
    let mut close_clicked = false;
    egui::Window::new("Preferences").open(open).collapsible(false).resizable(false).default_width(380.0).show(ctx, |ui| {
        egui::Grid::new("prefs-grid").num_columns(2).spacing([16.0, 8.0]).striped(false).show(ui, |ui| {
            ui.label("Theme");
            egui::ComboBox::from_id_salt("prefs-theme")
                .selected_text(match cfg.theme.as_str() {
                    "light" => "Light",
                    "dark" => "Dark",
                    _ => "Follow system",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut cfg.theme, "system".to_string(), "Follow system");
                    ui.selectable_value(&mut cfg.theme, "light".to_string(), "Light");
                    ui.selectable_value(&mut cfg.theme, "dark".to_string(), "Dark");
                });
            ui.end_row();

            ui.label("Body font size");
            ui.add(egui::Slider::new(&mut cfg.body_size, 10.0..=32.0).clamping(egui::SliderClamping::Never).step_by(0.5).suffix(" pt"));
            ui.end_row();

            ui.label("Code font size");
            ui.add(egui::Slider::new(&mut cfg.code_size, 9.0..=28.0).clamping(egui::SliderClamping::Never).step_by(0.5).suffix(" pt"));
            ui.end_row();

            ui.label("Line height");
            ui.add(egui::Slider::new(&mut cfg.line_height, 1.0..=2.5).clamping(egui::SliderClamping::Never).step_by(0.05));
            ui.end_row();

            ui.label("Max text width");
            ui.horizontal(|ui| {
                let mut full = cfg.max_width <= 0.0;
                if ui.checkbox(&mut full, "Full width").changed() {
                    cfg.max_width = if full { 0.0 } else { 820.0 };
                }
                if !full {
                    ui.add(egui::Slider::new(&mut cfg.max_width, 400.0..=2000.0).clamping(egui::SliderClamping::Never).step_by(10.0).suffix(" pt"));
                }
            });
            ui.end_row();

            ui.label("Recovery snapshot every");
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut cfg.autosave_secs).range(0..=3600).clamp_existing_to_range(false).suffix(" s"));
                ui.weak("(0 = off)");
            });
            ui.end_row();

            ui.label("Image folder");
            ui.add(egui::TextEdit::singleline(&mut cfg.image_folder).hint_text("assets").desired_width(180.0));
            ui.end_row();

            ui.label("Remote images");
            ui.checkbox(&mut cfg.remote_images, "Load http(s) images").on_hover_text("Off by default for privacy: loading reveals your IP to the image host.");
            ui.end_row();

            ui.label("Opening files");
            ui.checkbox(&mut cfg.single_window, "Open in the existing window").on_hover_text("Files opened from the file manager or terminal become new tabs. Off: each opens its own window.");
            ui.end_row();

            ui.label("Starting up");
            ui.checkbox(&mut cfg.restore_session, "Reopen files from last time");
            ui.end_row();

            ui.label("New documents");
            ui.checkbox(&mut cfg.source_mode, "Start in source mode");
            ui.end_row();
        });
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("Reset to defaults").clicked() {
                let keep_sidebar = cfg.show_sidebar;
                *cfg = Config { show_sidebar: keep_sidebar, ..Config::default() };
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Close").clicked() {
                    close_clicked = true;
                }
            });
        });
    });
    // Keep the image folder a simple relative name (no traversal).
    cfg.image_folder = sanitize_image_folder(&cfg.image_folder);
    if close_clicked {
        *open = false;
    }
    was_open && !*open
}

/// A relative folder without `..` or a leading `/` (keeps typing-in-progress strings intact
/// otherwise).
pub fn sanitize_image_folder(s: &str) -> String {
    let parts: Vec<&str> = s.split('/').filter(|p| *p != ".." && *p != ".").collect();
    let joined = parts.join("/");
    joined.trim_start_matches('/').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_folder_sanitizing() {
        assert_eq!(sanitize_image_folder("assets"), "assets");
        assert_eq!(sanitize_image_folder("/etc"), "etc");
        assert_eq!(sanitize_image_folder("../../x/./img"), "x/img");
        assert_eq!(sanitize_image_folder("a/"), "a/");
        assert_eq!(sanitize_image_folder(""), "");
    }
}

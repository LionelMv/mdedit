//! Non-blocking notifications in the bottom-right corner.

use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Info,
    Error,
}

struct Toast {
    text: String,
    level: Level,
    until: Instant,
}

#[derive(Default)]
pub struct Toasts {
    items: Vec<Toast>,
}

impl Toasts {
    pub fn info(&mut self, text: impl Into<String>) {
        self.push(text.into(), Level::Info, Duration::from_secs(4));
    }
    pub fn error(&mut self, text: impl Into<String>) {
        let text = text.into();
        log::warn!("{text}");
        self.push(text, Level::Error, Duration::from_secs(8));
    }
    fn push(&mut self, text: String, level: Level, dur: Duration) {
        // Collapse repeats.
        self.items.retain(|t| t.text != text);
        self.items.push(Toast { text, level, until: Instant::now() + dur });
        if self.items.len() > 6 {
            self.items.remove(0);
        }
    }

    /// Draw; returns how long until the next one expires (for `request_repaint_after`).
    pub fn show(&mut self, ctx: &egui::Context) -> Option<Duration> {
        let now = Instant::now();
        self.items.retain(|t| t.until > now);
        if self.items.is_empty() {
            return None;
        }
        let mut dismiss = None;
        egui::Area::new(egui::Id::unique("mdedit-toasts"))
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-12.0, -36.0))
            .order(egui::Order::Foreground)
            .interactable(true)
            .show(ctx, |ui| {
                ui.set_max_width(420.0);
                for (i, t) in self.items.iter().enumerate() {
                    let visuals = ui.visuals();
                    let (fill, stroke) = match t.level {
                        Level::Info => (visuals.extreme_bg_color, visuals.widgets.noninteractive.bg_stroke),
                        Level::Error => (visuals.extreme_bg_color, egui::Stroke::new(1.5, visuals.error_fg_color)),
                    };
                    egui::Frame::popup(ui.style()).fill(fill).stroke(stroke).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            if t.level == Level::Error {
                                ui.colored_label(ui.visuals().error_fg_color, "⚠");
                            }
                            ui.add(egui::Label::new(&t.text).wrap());
                            if ui.small_button("✕").clicked() {
                                dismiss = Some(i);
                            }
                        });
                    });
                    ui.add_space(4.0);
                }
            });
        if let Some(i) = dismiss {
            self.items.remove(i);
        }
        self.items.iter().map(|t| t.until.saturating_duration_since(now)).min()
    }
}

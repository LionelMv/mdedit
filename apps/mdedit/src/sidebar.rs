//! Sidebar: document outline.

use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use mdedit_syntax::Heading;

use crate::app::BgMsg;
use crate::tabs::Tab;

#[derive(Default)]
pub struct Sidebar {
    outline: Vec<Heading>,
    /// (tab id, version) of the outline.
    outline_key: Option<(u64, u64)>,
    outline_last: Option<Instant>,
    outline_pending: Option<(u64, u64)>,
    /// (tab id, row) the outline last scrolled to, so that it moves only when that changes and
    /// the wheel can still browse it.
    followed: Option<(u64, Option<usize>)>,
    /// Scroll offset and height of the outline last frame.
    view: (f32, f32),
}

const OUTLINE_THROTTLE: Duration = Duration::from_millis(400);
const BACKGROUND_OUTLINE_BYTES: usize = 512 * 1024;

impl Sidebar {
    /// Update the cached outline for `tab` (throttled while typing; big documents are parsed
    /// on a background thread). Returns a wake-up delay if an update is pending.
    fn refresh_outline(&mut self, tab: &Tab, tx: &Sender<BgMsg>, ctx: &egui::Context) -> Option<Duration> {
        let key = (tab.id, tab.doc.buffer.version());
        if self.outline_key == Some(key) || self.outline_pending == Some(key) {
            return None;
        }
        let now = Instant::now();
        let same_tab = self.outline_key.is_some_and(|(id, _)| id == tab.id);
        if self.outline_pending.is_some_and(|(id, _)| id == tab.id) {
            // A background parse for an older version is still running; retry later.
            return Some(OUTLINE_THROTTLE);
        }
        if same_tab && let Some(last) = self.outline_last {
            let next = last + OUTLINE_THROTTLE;
            if now < next {
                return Some(next - now);
            }
        }
        self.outline_last = Some(now);
        if !same_tab {
            self.outline.clear();
        }
        let text = tab.doc.buffer.text();
        if text.len() > BACKGROUND_OUTLINE_BYTES {
            self.outline_pending = Some(key);
            let tx = tx.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let blocks = mdedit_syntax::parse(&text);
                let headings = mdedit_syntax::outline(&text, &blocks);
                let _ = tx.send(BgMsg::Outline { tab_id: key.0, version: key.1, headings });
                ctx.request_repaint();
            });
        } else {
            let blocks = mdedit_syntax::parse(&text);
            self.outline = mdedit_syntax::outline(&text, &blocks);
            self.outline_key = Some(key);
        }
        None
    }

    pub fn outline_done(&mut self, tab_id: u64, version: u64, headings: Vec<Heading>) {
        if self.outline_pending == Some((tab_id, version)) {
            self.outline_pending = None;
            self.outline = headings;
            self.outline_key = Some((tab_id, version));
        }
    }

    /// Draw the outline; clearing `shown` collapses it. Returns (byte offset of the clicked
    /// heading, wake-up delay).
    pub fn ui(&mut self, ui: &mut egui::Ui, active: Option<&Tab>, tx: &Sender<BgMsg>, shown: &mut bool) -> (Option<usize>, Option<Duration>) {
        let mut clicked = None;
        let mut wake = None;
        ui.horizontal(|ui| {
            ui.strong("Outline");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("«").on_hover_text("Hide outline (Ctrl+Shift+B)").clicked() {
                    *shown = false;
                }
            });
        });
        ui.separator();
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        if let Some(tab) = active {
            wake = self.refresh_outline(tab, tx, ui.ctx());
            if self.outline.is_empty() {
                ui.weak("No headings");
            }
            let at = tab.editor.reading_at();
            let current = self.outline.iter().rposition(|h| h.at <= at);
            let row_h = ui.spacing().interact_size.y;
            let mut area = egui::ScrollArea::vertical().auto_shrink([false, false]);
            if self.followed != Some((tab.id, current)) {
                self.followed = Some((tab.id, current));
                let (offset, height) = self.view;
                if let Some(i) = current
                    && height > 0.0
                {
                    let top = i as f32 * (row_h + ui.spacing().item_spacing.y);
                    area = area.vertical_scroll_offset(offset.min(top).max(top + row_h - height));
                }
            }
            // Only the visible rows: an outline can have thousands, and this runs every frame.
            let shown = area.show_rows(ui, row_h, self.outline.len(), |ui, rows| {
                for i in rows {
                    let h = &self.outline[i];
                    ui.horizontal(|ui| {
                        ui.set_min_height(row_h);
                        ui.add_space(f32::from(h.level.saturating_sub(1)) * 12.0);
                        let text = if h.text.trim().is_empty() { "(empty heading)" } else { h.text.as_str() };
                        let rich = if h.level <= 1 { egui::RichText::new(text).strong() } else { egui::RichText::new(text) };
                        if ui.selectable_label(current == Some(i), rich).clicked() {
                            clicked = Some(h.at);
                        }
                    });
                }
            });
            self.view = (shown.state.clamped_offset().y, shown.inner_rect.height());
        }
        (clicked, wake)
    }
}

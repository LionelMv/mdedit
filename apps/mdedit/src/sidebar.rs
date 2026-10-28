//! Sidebar: folder file tree, document outline, folder search.

use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use mdedit_syntax::Heading;
use mdedit_workspace::{SearchHit, TreeEntry};

use crate::app::BgMsg;
use crate::tabs::Tab;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SidebarTab {
    #[default]
    Files,
    Outline,
    Search,
}

/// What the user clicked.
pub enum SidebarAction {
    OpenFile(PathBuf),
    /// Open (or switch to) the file and go to 0-based `line`.
    OpenAtLine(PathBuf, usize),
    ScrollTo(usize),
    OpenFolder,
}

#[derive(Default)]
pub struct Sidebar {
    pub tab: SidebarTab,
    pub folder: Option<PathBuf>,
    tree: Option<TreeEntry>,
    tree_gen: u64,
    scanning: bool,
    /// Rescan requested (watcher) at this time; done after a short quiet period.
    rescan_at: Option<Instant>,
    outline: Vec<Heading>,
    /// (tab id, version) of the outline.
    outline_key: Option<(u64, u64)>,
    outline_last: Option<Instant>,
    outline_pending: Option<(u64, u64)>,
    pub search_query: String,
    search_gen: u64,
    searching: bool,
    search_hits: Vec<SearchHit>,
    searched_for: String,
}

const OUTLINE_THROTTLE: Duration = Duration::from_millis(400);
const BACKGROUND_OUTLINE_BYTES: usize = 512 * 1024;
const RESCAN_DEBOUNCE: Duration = Duration::from_millis(700);

impl Sidebar {
    pub fn set_folder(&mut self, folder: PathBuf, tx: &Sender<BgMsg>, ctx: &egui::Context) {
        self.folder = Some(folder);
        self.tree = None;
        self.search_hits.clear();
        self.searched_for.clear();
        self.tab = SidebarTab::Files;
        self.rescan(tx, ctx);
    }

    pub fn rescan(&mut self, tx: &Sender<BgMsg>, ctx: &egui::Context) {
        let Some(folder) = self.folder.clone() else { return };
        self.tree_gen += 1;
        self.scanning = true;
        self.rescan_at = None;
        let tx = tx.clone();
        let ctx = ctx.clone();
        let generation = self.tree_gen;
        std::thread::spawn(move || {
            let tree = mdedit_workspace::scan_tree(&folder);
            let _ = tx.send(BgMsg::Tree { generation, tree });
            ctx.request_repaint();
        });
    }

    /// The watcher saw a change under the folder: rescan after a quiet period.
    pub fn schedule_rescan(&mut self) {
        self.rescan_at = Some(Instant::now() + RESCAN_DEBOUNCE);
    }

    /// Run due timers; returns time until the next one.
    pub fn tick(&mut self, tx: &Sender<BgMsg>, ctx: &egui::Context) -> Option<Duration> {
        let at = self.rescan_at?;
        let now = Instant::now();
        if now >= at {
            self.rescan(tx, ctx);
            None
        } else {
            Some(at - now)
        }
    }

    pub fn tree_done(&mut self, generation: u64, tree: TreeEntry) {
        if generation == self.tree_gen {
            self.tree = Some(tree);
            self.scanning = false;
        }
    }

    pub fn search_done(&mut self, generation: u64, hits: Vec<SearchHit>) {
        if generation == self.search_gen {
            self.search_hits = hits;
            self.searching = false;
        }
    }

    fn start_search(&mut self, tx: &Sender<BgMsg>, ctx: &egui::Context) {
        let Some(folder) = self.folder.clone() else { return };
        let q = self.search_query.trim().to_string();
        if q.is_empty() {
            return;
        }
        self.search_gen += 1;
        self.searching = true;
        self.searched_for = q.clone();
        let generation = self.search_gen;
        let tx = tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let hits = mdedit_workspace::search_folder(&folder, &q);
            let _ = tx.send(BgMsg::Search { generation, hits });
            ctx.request_repaint();
        });
    }

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

    /// Draw the sidebar contents. Returns (action, wake-up delay).
    pub fn ui(&mut self, ui: &mut egui::Ui, active: Option<&Tab>, tx: &Sender<BgMsg>) -> (Option<SidebarAction>, Option<Duration>) {
        let mut action = None;
        let mut wake = None;
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.tab, SidebarTab::Files, "Files");
            ui.selectable_value(&mut self.tab, SidebarTab::Outline, "Outline");
            ui.selectable_value(&mut self.tab, SidebarTab::Search, "Search");
        });
        ui.separator();
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        match self.tab {
            SidebarTab::Files => self.files_ui(ui, active, tx, &mut action),
            SidebarTab::Outline => {
                if let Some(tab) = active {
                    wake = self.refresh_outline(tab, tx, ui.ctx());
                    if self.outline.is_empty() {
                        ui.weak("No headings");
                    }
                    let caret = tab.doc.buffer.selection.head;
                    let current = self.outline.iter().rposition(|h| h.at <= caret);
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                        for (i, h) in self.outline.iter().enumerate() {
                            ui.horizontal(|ui| {
                                ui.add_space(f32::from(h.level.saturating_sub(1)) * 12.0);
                                let text = if h.text.trim().is_empty() { "(empty heading)" } else { h.text.as_str() };
                                let rich = if h.level <= 1 { egui::RichText::new(text).strong() } else { egui::RichText::new(text) };
                                if ui.selectable_label(current == Some(i), rich).clicked() {
                                    action = Some(SidebarAction::ScrollTo(h.at));
                                }
                            });
                        }
                    });
                }
            }
            SidebarTab::Search => self.search_ui(ui, tx, &mut action),
        }
        (action, wake)
    }

    fn files_ui(&mut self, ui: &mut egui::Ui, active: Option<&Tab>, tx: &Sender<BgMsg>, action: &mut Option<SidebarAction>) {
        let Some(folder) = self.folder.clone() else {
            ui.weak("No folder open");
            if ui.button("Open Folder…").clicked() {
                *action = Some(SidebarAction::OpenFolder);
            }
            return;
        };
        ui.horizontal(|ui| {
            let name = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| folder.display().to_string());
            ui.strong(name).on_hover_text(folder.display().to_string());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("⟳").on_hover_text("Refresh").clicked() {
                    self.rescan(tx, ui.ctx());
                }
                if self.scanning {
                    ui.spinner();
                }
            });
        });
        let active_path = active.and_then(|t| t.doc.path.clone());
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match &self.tree {
            Some(tree) if tree.children.is_empty() => {
                ui.weak("No Markdown files");
            }
            Some(tree) => {
                for child in &tree.children {
                    tree_node(ui, child, active_path.as_deref(), action);
                }
            }
            None => {
                ui.weak("Scanning…");
            }
        });
    }

    fn search_ui(&mut self, ui: &mut egui::Ui, tx: &Sender<BgMsg>, action: &mut Option<SidebarAction>) {
        if self.folder.is_none() {
            ui.weak("Open a folder to search in it");
            if ui.button("Open Folder…").clicked() {
                *action = Some(SidebarAction::OpenFolder);
            }
            return;
        }
        ui.horizontal(|ui| {
            let resp = ui.add(egui::TextEdit::singleline(&mut self.search_query).hint_text("Search in folder").desired_width(ui.available_width() - 30.0));
            let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if enter || ui.button("🔍").clicked() {
                self.start_search(tx, ui.ctx());
            }
        });
        if self.searching {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Searching…");
            });
        } else if !self.searched_for.is_empty() {
            let n = self.search_hits.len();
            ui.weak(format!("{n}{} result{} for \"{}\"", if n >= 1000 { "+" } else { "" }, if n == 1 { "" } else { "s" }, self.searched_for));
        }
        let root = self.folder.clone().unwrap_or_default();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let mut last_path: Option<&Path> = None;
            for hit in &self.search_hits {
                if last_path != Some(hit.path.as_path()) {
                    let rel = hit.path.strip_prefix(&root).unwrap_or(&hit.path);
                    ui.add_space(4.0);
                    ui.strong(rel.display().to_string());
                    last_path = Some(hit.path.as_path());
                }
                let mut line_text: String = hit.text.trim().chars().take(120).collect();
                if line_text.is_empty() {
                    line_text.push(' ');
                }
                let resp = ui.selectable_label(false, format!("{}: {}", hit.line + 1, line_text));
                if resp.clicked() {
                    *action = Some(SidebarAction::OpenAtLine(hit.path.clone(), hit.line));
                }
            }
        });
    }
}

fn tree_node(ui: &mut egui::Ui, e: &TreeEntry, active: Option<&Path>, action: &mut Option<SidebarAction>) {
    if e.is_dir {
        egui::CollapsingHeader::new(format!("📁 {}", e.name)).id_salt(&e.path).default_open(false).show(ui, |ui| {
            for c in &e.children {
                tree_node(ui, c, active, action);
            }
        });
    } else {
        let selected = active.is_some_and(|a| a == e.path);
        if ui.selectable_label(selected, format!("📄 {}", e.name)).on_hover_text(e.path.display().to_string()).clicked() {
            *action = Some(SidebarAction::OpenFile(e.path.clone()));
        }
    }
}

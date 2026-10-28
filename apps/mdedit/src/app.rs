//! The application: tabs, panels, actions, background work, close/quit flow.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use egui::ViewportCommand;
use mdedit_buffer::{Document, LineEnding, Selection};
use mdedit_highlight::{HlTheme, Highlighter};
use mdedit_style::Mode;
use mdedit_syntax::Heading;
use mdedit_workspace::recovery::{self, Snapshot};
use mdedit_workspace::{Config, RecentFiles, Watcher};

use crate::cli::StartupTargets;
use crate::dialogs::{self, RecoveryChoice, SaveChoice, TableDialog};
use crate::find::FindBar;
use crate::links::{self, LinkAction};
use crate::menu::{self, Action, MenuState};
use crate::sidebar::Sidebar;
use crate::tabs::{self, LARGE_FILE_BYTES, Tab};
use crate::theme::ThemeState;
use crate::toast::Toasts;

/// Results from background threads (file dialogs, parses).
pub enum BgMsg {
    OpenFiles(Vec<PathBuf>),
    SaveAs { tab_id: u64, path: PathBuf, then_close: bool },
    SaveAsCancelled { then_close: bool },
    ExportDone(Result<PathBuf, String>),
    DialogClosed,
    Outline { tab_id: u64, version: u64, headings: Vec<Heading> },
    RecoveryError(String),
}

#[derive(Clone, Copy, Debug)]
struct PendingClose {
    tab_id: u64,
    quit: bool,
}

/// How often to poll the file watcher while idle.
const WATCH_POLL: Duration = Duration::from_secs(2);

pub struct MdApp {
    config: Config,
    /// Config as last written, to save only on change.
    saved_config: Config,
    recent: RecentFiles,
    tabs: Vec<Tab>,
    active: usize,
    next_tab_id: u64,
    highlighter: Arc<Highlighter>,
    theme: ThemeState,
    sidebar: Sidebar,
    show_sidebar: bool,
    find: FindBar,
    prefs_open: bool,
    about_open: bool,
    shortcuts_open: bool,
    table: TableDialog,
    toasts: Toasts,
    watcher: Option<Watcher>,
    tx: Sender<BgMsg>,
    rx: Receiver<BgMsg>,
    /// A native file dialog is open (one at a time).
    dialog_busy: bool,
    pending_close: Option<PendingClose>,
    quitting: bool,
    allow_close: bool,
    recovery_offer: Vec<Snapshot>,
    last_autosave: Instant,
    typewriter: bool,
    focus_mode: bool,
    last_title: String,
    last_zoom: f32,
    actions: Vec<Action>,
    ctx: egui::Context,
}

impl MdApp {
    pub fn new(cc: &eframe::CreationContext<'_>, targets: StartupTargets) -> Self {
        let ctx = &cc.egui_ctx;
        egui_extras::install_image_loaders(ctx);
        ctx.set_fonts(crate::fonts::system_font_definitions());
        std::thread::spawn(Highlighter::preload);

        let config = Config::load();
        let (tx, rx) = channel();
        let watcher = match Watcher::new() {
            Ok(w) => Some(w),
            Err(e) => {
                log::warn!("file watching disabled: {e}");
                None
            }
        };
        let mut app = Self {
            saved_config: config.clone(),
            show_sidebar: config.show_sidebar,
            config,
            recent: RecentFiles::load(),
            tabs: Vec::new(),
            active: 0,
            next_tab_id: 1,
            highlighter: Arc::new(Highlighter::new()),
            theme: ThemeState::default(),
            sidebar: Sidebar::default(),
            find: FindBar::default(),
            prefs_open: false,
            about_open: false,
            shortcuts_open: false,
            table: TableDialog::default(),
            toasts: Toasts::default(),
            watcher,
            tx,
            rx,
            dialog_busy: false,
            pending_close: None,
            quitting: false,
            allow_close: false,
            recovery_offer: Vec::new(),
            last_autosave: Instant::now(),
            typewriter: false,
            focus_mode: false,
            last_title: String::new(),
            last_zoom: ctx.zoom_factor(),
            actions: Vec::new(),
            ctx: ctx.clone(),
        };

        for f in &targets.files {
            if f.exists() {
                app.open_path(f);
            } else {
                // New document that will be saved at this path.
                let mut doc = Document::new();
                doc.path = Some(f.clone());
                let idx = app.push_tab(doc);
                app.active = idx;
            }
        }
        if app.tabs.is_empty() {
            app.new_tab();
        }
        app.recovery_offer = recovery::list().into_iter().filter(|s| !tabs::snapshot_owned_by_live_process(&s.id)).collect();
        app
    }

    // ---------------------------------------------------------------- tabs

    fn push_tab(&mut self, doc: Document) -> usize {
        let mode = if self.config.source_mode { Mode::Source } else { Mode::LivePreview };
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        self.tabs.push(Tab::new(id, doc, mode));
        self.tabs.len() - 1
    }

    fn new_tab(&mut self) {
        let idx = self.push_tab(Document::new());
        self.activate(idx);
    }

    fn activate(&mut self, idx: usize) {
        if idx < self.tabs.len() {
            self.active = idx;
            self.tabs[idx].editor.request_focus = true;
        }
    }

    fn tab_index(&self, id: u64) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    fn find_tab_by_path(&self, path: &Path) -> Option<usize> {
        self.tabs.iter().position(|t| t.doc.path.as_deref().is_some_and(|p| same_file(p, path)))
    }

    /// Open a file (or switch to its tab).
    fn open_path(&mut self, path: &Path) -> Option<usize> {
        let path = crate::cli::absolutize(path);
        if path.is_dir() {
            self.toasts.error(format!("Can't open {}: it is a folder", path.display()));
            return None;
        }
        if let Some(idx) = self.find_tab_by_path(&path) {
            self.activate(idx);
            return Some(idx);
        }
        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let doc = match Document::open(&path) {
            Ok(d) => d,
            Err(e) => {
                self.recent.remove(&path);
                let msg = if e.kind() == std::io::ErrorKind::InvalidData { "not valid UTF-8 text".to_string() } else { e.to_string() };
                self.toasts.error(format!("Can't open {}: {msg}", path.display()));
                return None;
            }
        };
        let disk_hash = tabs::hash_text(&doc.buffer.text());
        // Replace a pristine untitled tab instead of piling up empty ones.
        let reuse = self.tabs.get(self.active).is_some_and(Tab::is_pristine);
        let idx = if reuse {
            let mode = if self.config.source_mode { Mode::Source } else { Mode::LivePreview };
            let id = self.tabs[self.active].id;
            let recovery_id = self.tabs[self.active].recovery_id.clone();
            let mut t = Tab::new(id, doc, mode);
            t.recovery_id = recovery_id;
            self.tabs[self.active] = t;
            self.active
        } else {
            self.push_tab(doc)
        };
        let tab = &mut self.tabs[idx];
        tab.disk_hash = Some(disk_hash);
        if size > LARGE_FILE_BYTES {
            tab.editor.mode = Mode::Source;
            tab.large_file = Some(size);
        }
        if let Some(w) = self.watcher.as_mut() {
            w.watch(&path);
        }
        self.recent.add(&path);
        self.activate(idx);
        Some(idx)
    }

    /// Close without asking.
    fn close_tab_now(&mut self, id: u64) {
        let Some(idx) = self.tab_index(id) else { return };
        let tab = self.tabs.remove(idx);
        if let Err(e) = recovery::remove(&tab.recovery_id) {
            log::debug!("recovery remove {}: {e}", tab.recovery_id);
        }
        if let (Some(w), Some(p)) = (self.watcher.as_mut(), tab.doc.path.as_deref())
            && !self.tabs.iter().any(|t| t.doc.path.as_deref() == Some(p))
        {
            w.unwatch(p);
        }
        if self.active > idx || self.active >= self.tabs.len() {
            self.active = self.active.saturating_sub(1);
        }
        if self.tabs.is_empty() && !self.quitting {
            self.new_tab();
        }
        if let Some(t) = self.tabs.get_mut(self.active) {
            t.editor.request_focus = true;
        }
    }

    fn request_close(&mut self, id: u64) {
        let Some(idx) = self.tab_index(id) else { return };
        if self.tabs[idx].doc.is_dirty() {
            self.activate(idx);
            self.pending_close = Some(PendingClose { tab_id: id, quit: false });
        } else {
            self.close_tab_now(id);
        }
    }

    fn request_quit(&mut self, ctx: &egui::Context) {
        self.quitting = true;
        self.advance_quit(ctx);
    }

    fn advance_quit(&mut self, ctx: &egui::Context) {
        if let Some(idx) = self.tabs.iter().position(|t| t.doc.is_dirty()) {
            let id = self.tabs[idx].id;
            self.activate(idx);
            self.pending_close = Some(PendingClose { tab_id: id, quit: true });
        } else {
            self.on_exit_cleanup();
            self.allow_close = true;
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
    }

    fn cancel_quit(&mut self) {
        self.quitting = false;
        self.pending_close = None;
        if self.tabs.is_empty() {
            self.new_tab();
        }
    }

    /// Remove snapshots of documents that are saved or discarded, save config.
    fn on_exit_cleanup(&mut self) {
        for t in &self.tabs {
            if !t.doc.is_dirty() {
                let _ = recovery::remove(&t.recovery_id);
            }
        }
        self.sync_config();
        self.save_config_if_changed();
    }

    fn sync_config(&mut self) {
        self.config.show_sidebar = self.show_sidebar;
    }

    fn save_config_if_changed(&mut self) {
        if self.config != self.saved_config {
            match self.config.save() {
                Ok(()) => self.saved_config = self.config.clone(),
                Err(e) => self.toasts.error(format!("Can't save preferences: {e}")),
            }
        }
    }

    // ---------------------------------------------------------------- saving

    /// Save tab `id`. Without a path, opens Save As. Returns `true` if saved now.
    fn save_tab(&mut self, id: u64, then_close: bool) -> bool {
        let Some(idx) = self.tab_index(id) else { return false };
        if self.tabs[idx].doc.path.is_none() {
            self.start_save_as(id, then_close);
            return false;
        }
        self.finish_save(idx, None)
    }

    fn finish_save(&mut self, idx: usize, new_path: Option<PathBuf>) -> bool {
        let tab = &mut self.tabs[idx];
        let old_path = tab.doc.path.clone();
        let res = match &new_path {
            Some(p) => tab.doc.save_as(p),
            None => tab.doc.save(),
        };
        match res {
            Ok(()) => {
                tab.disk_hash = Some(tabs::hash_text(&tab.doc.buffer.text()));
                tab.disk_changed = false;
                tab.snapshot_version = None;
                let _ = recovery::remove(&tab.recovery_id);
                if let Some(p) = tab.doc.path.clone() {
                    if let Some(w) = self.watcher.as_mut() {
                        if let Some(old) = old_path.as_deref().filter(|o| *o != p) {
                            w.unwatch(old);
                        }
                        w.watch(&p);
                    }
                    self.recent.add(&p);
                }
                true
            }
            Err(e) => {
                if new_path.is_some() {
                    tab.doc.path = old_path;
                }
                let name = tab.doc.title();
                self.toasts.error(format!("Can't save {name}: {e}"));
                false
            }
        }
    }

    fn start_save_as(&mut self, tab_id: u64, then_close: bool) {
        let Some(idx) = self.tab_index(tab_id) else { return };
        if self.dialog_busy {
            return;
        }
        self.dialog_busy = true;
        let doc = &self.tabs[idx].doc;
        let dir = doc.path.as_deref().and_then(Path::parent).map(Path::to_path_buf);
        let name = doc.path.as_deref().and_then(Path::file_name).map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| suggested_name(&doc.buffer.slice(0..doc.buffer.len_bytes().min(400))));
        let tx = self.tx.clone();
        let ctx = self.ctx_for_threads();
        std::thread::spawn(move || {
            let mut d = rfd::FileDialog::new().set_title("Save As").add_filter("Markdown", &["md", "markdown"]).add_filter("All files", &["*"]).set_file_name(name);
            if let Some(dir) = dir {
                d = d.set_directory(dir);
            }
            let msg = match d.save_file() {
                Some(path) => BgMsg::SaveAs { tab_id, path, then_close },
                None => BgMsg::SaveAsCancelled { then_close },
            };
            let _ = tx.send(msg);
            ctx.request_repaint();
        });
    }

    fn ctx_for_threads(&self) -> egui::Context {
        self.ctx.clone()
    }

    // ---------------------------------------------------------------- dialogs

    fn start_open_dialog(&mut self) {
        if self.dialog_busy {
            return;
        }
        self.dialog_busy = true;
        let dir = self.tabs.get(self.active).and_then(|t| t.base_dir().map(Path::to_path_buf));
        let tx = self.tx.clone();
        let ctx = self.ctx_for_threads();
        std::thread::spawn(move || {
            let mut d = rfd::FileDialog::new();
            if let Some(dir) = dir {
                d = d.set_directory(dir);
            }
            let msg = d.set_title("Open").add_filter("Markdown", &["md", "markdown", "mdown", "mkd", "txt"]).add_filter("All files", &["*"]).pick_files().map(BgMsg::OpenFiles);
            let _ = tx.send(msg.unwrap_or(BgMsg::DialogClosed));
            ctx.request_repaint();
        });
    }

    fn start_export_html(&mut self) {
        if self.dialog_busy {
            return;
        }
        let Some(tab) = self.tabs.get(self.active) else { return };
        self.dialog_busy = true;
        let text = tab.doc.buffer.text();
        let title = tab.doc.path.as_deref().and_then(Path::file_stem).map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Untitled".into());
        let dir = tab.base_dir().map(Path::to_path_buf);
        let hl_theme = if self.theme.current().dark { HlTheme::Dark } else { HlTheme::Light };
        let hl = Arc::clone(&self.highlighter);
        let tx = self.tx.clone();
        let ctx = self.ctx_for_threads();
        std::thread::spawn(move || {
            let mut d = rfd::FileDialog::new().set_title("Export HTML").add_filter("HTML", &["html", "htm"]).set_file_name(format!("{title}.html"));
            if let Some(dir) = dir {
                d = d.set_directory(dir);
            }
            let msg = match d.save_file() {
                Some(path) => {
                    let opts = mdedit_export::HtmlOptions { title, theme: hl_theme, allow_raw_html: false };
                    let html = mdedit_export::to_html(&text, &opts, &hl);
                    BgMsg::ExportDone(std::fs::write(&path, html).map(|()| path).map_err(|e| e.to_string()))
                }
                None => BgMsg::DialogClosed,
            };
            let _ = tx.send(msg);
            ctx.request_repaint();
        });
    }

    // ---------------------------------------------------------------- background results

    fn poll_background(&mut self, ctx: &egui::Context) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                BgMsg::OpenFiles(paths) => {
                    self.dialog_busy = false;
                    for p in paths {
                        self.open_path(&p);
                    }
                }
                BgMsg::SaveAs { tab_id, path, then_close } => {
                    self.dialog_busy = false;
                    let Some(idx) = self.tab_index(tab_id) else { continue };
                    if self.finish_save(idx, Some(path)) {
                        if then_close {
                            self.close_tab_now(tab_id);
                            if self.quitting {
                                self.advance_quit(ctx);
                            }
                        }
                    } else if self.quitting {
                        self.cancel_quit();
                    }
                }
                BgMsg::SaveAsCancelled { then_close } => {
                    self.dialog_busy = false;
                    if then_close && self.quitting {
                        self.cancel_quit();
                    }
                }
                BgMsg::ExportDone(res) => {
                    self.dialog_busy = false;
                    match res {
                        Ok(p) => self.toasts.info(format!("Exported {}", p.display())),
                        Err(e) => self.toasts.error(format!("Export failed: {e}")),
                    }
                }
                BgMsg::DialogClosed => self.dialog_busy = false,
                BgMsg::Outline { tab_id, version, headings } => self.sidebar.outline_done(tab_id, version, headings),
                BgMsg::RecoveryError(e) => self.toasts.error(format!("Can't write recovery snapshot: {e}")),
            }
        }
    }

    // ---------------------------------------------------------------- external changes

    fn poll_watcher(&mut self) {
        let Some(w) = self.watcher.as_mut() else { return };
        let changed = w.changed();
        if changed.is_empty() {
            return;
        }
        for path in changed {
            let open: Vec<usize> = (0..self.tabs.len()).filter(|&i| self.tabs[i].doc.path.as_deref().is_some_and(|p| p == path || same_file(p, &path))).collect();
            for idx in open {
                self.check_disk_change(idx);
            }
        }
    }

    fn check_disk_change(&mut self, idx: usize) {
        let tab = &mut self.tabs[idx];
        let Some(path) = tab.doc.path.clone() else { return };
        let Ok(disk) = Document::open(&path) else {
            // Deleted or unreadable (or mid-rename by another program): keep ours.
            return;
        };
        let text = disk.buffer.text();
        let hash = tabs::hash_text(&text);
        if tab.disk_hash == Some(hash) {
            return; // our own save or no real change
        }
        if tab.doc.is_dirty() {
            tab.disk_changed = true;
        } else {
            reload_tab(tab, disk, &text, hash);
            let name = tab.doc.title();
            self.toasts.info(format!("Reloaded {name} (changed on disk)"));
        }
    }

    // ---------------------------------------------------------------- autosave / recovery

    /// Write recovery snapshots of dirty documents when due. Returns the next wake-up.
    fn autosave_tick(&mut self) -> Option<Duration> {
        let secs = self.config.autosave_secs;
        if secs == 0 {
            return None;
        }
        let needs = |t: &Tab| t.doc.is_dirty() && t.snapshot_version != Some(t.doc.buffer.version());
        if !self.tabs.iter().any(needs) {
            return None;
        }
        let interval = Duration::from_secs(secs);
        let elapsed = self.last_autosave.elapsed();
        if elapsed < interval {
            return Some(interval - elapsed);
        }
        self.last_autosave = Instant::now();
        let mut jobs = Vec::new();
        for t in self.tabs.iter_mut().filter(|t| needs(t)) {
            t.snapshot_version = Some(t.doc.buffer.version());
            jobs.push((t.recovery_id.clone(), t.doc.path.clone(), t.doc.buffer.text()));
        }
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            for (id, path, text) in jobs {
                if let Err(e) = recovery::write(&id, path.as_deref(), &text) {
                    let _ = tx.send(BgMsg::RecoveryError(e.to_string()));
                }
            }
        });
        None
    }

    fn restore_snapshots(&mut self, ctx: &egui::Context) {
        let snaps = std::mem::take(&mut self.recovery_offer);
        for s in snaps {
            let existing = s.path.as_deref().and_then(|p| self.find_tab_by_path(p));
            let idx = match existing {
                Some(i) => {
                    // An open tab for the same file: take its text from the snapshot, drop its
                    // own (empty) snapshot id.
                    let _ = recovery::remove(&self.tabs[i].recovery_id);
                    i
                }
                None => {
                    let mut doc = match s.path.as_deref() {
                        Some(p) if p.is_file() => Document::open(p).unwrap_or_default(),
                        _ => Document::new(),
                    };
                    doc.path = s.path.clone();
                    let disk_hash = s.path.as_deref().filter(|p| p.is_file()).map(|_| tabs::hash_text(&doc.buffer.text()));
                    let i = if self.tabs.get(self.active).is_some_and(Tab::is_pristine) {
                        let _ = recovery::remove(&self.tabs[self.active].recovery_id);
                        let id = self.tabs[self.active].id;
                        let mode = self.tabs[self.active].editor.mode;
                        self.tabs[self.active] = Tab::new(id, doc, mode);
                        self.active
                    } else {
                        self.push_tab(doc)
                    };
                    self.tabs[i].disk_hash = disk_hash;
                    if let (Some(w), Some(p)) = (self.watcher.as_mut(), s.path.as_deref()) {
                        w.watch(p);
                    }
                    i
                }
            };
            let tab = &mut self.tabs[idx];
            // set_text bumps the version, so the document is dirty (unsaved) as it should be.
            tab.doc.buffer.set_text(&s.text);
            tab.recovery_id = s.id.clone();
            tab.snapshot_version = Some(tab.doc.buffer.version());
            tab.editor.invalidate();
            self.active = idx;
        }
        self.toasts.info("Restored unsaved documents");
        ctx.request_repaint();
    }

    // ---------------------------------------------------------------- links

    fn handle_link(&mut self, url: &str) {
        let base = self.tabs.get(self.active).and_then(|t| t.base_dir().map(Path::to_path_buf));
        match links::classify(url, base.as_deref()) {
            LinkAction::External(u) => {
                std::thread::spawn(move || {
                    if let Err(e) = open::that_detached(&u) {
                        log::warn!("open {u}: {e}");
                    }
                });
            }
            LinkAction::OpenDoc(p) => {
                self.open_path(&p);
            }
            LinkAction::Anchor(slug) => {
                let Some(tab) = self.tabs.get_mut(self.active) else { return };
                let text = tab.doc.buffer.text();
                let blocks = mdedit_syntax::parse(&text);
                let target = slug.to_lowercase();
                match mdedit_syntax::outline(&text, &blocks).into_iter().find(|h| links::slugify(&h.text) == target) {
                    Some(h) => tab.editor.scroll_to(&mut tab.doc, h.at),
                    None => self.toasts.error(format!("No heading \"#{slug}\" in this document")),
                }
            }
            LinkAction::Refuse(why) => self.toasts.error(why),
        }
    }

    // ---------------------------------------------------------------- actions

    fn run_action(&mut self, action: Action, ctx: &egui::Context) {
        use Action as A;
        match action {
            A::New => self.new_tab(),
            A::Open => self.start_open_dialog(),
            A::OpenRecent(p) => {
                if p.exists() {
                    self.open_path(&p);
                } else {
                    self.recent.remove(&p);
                    let _ = self.recent.save();
                    self.toasts.error(format!("{} no longer exists", p.display()));
                }
            }
            A::ClearRecent => {
                self.recent.files.clear();
                if let Err(e) = self.recent.save() {
                    self.toasts.error(format!("Can't save recent files: {e}"));
                }
            }
            A::Save => {
                if let Some(id) = self.tabs.get(self.active).map(|t| t.id) {
                    self.save_tab(id, false);
                }
            }
            A::SaveAs => {
                if let Some(id) = self.tabs.get(self.active).map(|t| t.id) {
                    self.start_save_as(id, false);
                }
            }
            A::ExportHtml => self.start_export_html(),
            A::CopyHtml => {
                if let Some(t) = self.tabs.get(self.active) {
                    // Rendering + sanitizing can take seconds on hostile input: do it off
                    // the UI thread and put the result on the clipboard from there.
                    let text = t.doc.buffer.text();
                    let ctx2 = ctx.clone();
                    let hl = Arc::clone(&self.highlighter);
                    std::thread::spawn(move || {
                        let html = mdedit_export::to_html_fragment(&text, &hl);
                        ctx2.copy_text(html);
                        ctx2.request_repaint();
                    });
                    self.toasts.info("Copying HTML to the clipboard…");
                }
            }
            A::CloseTab => {
                if let Some(id) = self.tabs.get(self.active).map(|t| t.id) {
                    self.request_close(id);
                }
            }
            A::Quit => self.request_quit(ctx),
            A::Undo | A::Redo | A::SelectAll | A::Format(_) | A::ToggleSource | A::FindNext | A::FindPrev | A::Find | A::Replace => self.tab_action(action),
            A::Cut => ctx.send_viewport_cmd(ViewportCommand::RequestCut),
            A::Copy => ctx.send_viewport_cmd(ViewportCommand::RequestCopy),
            A::Paste => ctx.send_viewport_cmd(ViewportCommand::RequestPaste),
            A::InsertTableDialog => self.table.open = true,
            A::ToggleSidebar => self.show_sidebar = !self.show_sidebar,
            A::ToggleTypewriter => self.typewriter = !self.typewriter,
            A::ToggleFocusMode => self.focus_mode = !self.focus_mode,
            A::ZoomIn => egui::gui_zoom::zoom_in(ctx),
            A::ZoomOut => egui::gui_zoom::zoom_out(ctx),
            A::ZoomReset => ctx.set_zoom_factor(1.0),
            A::Theme(t) => self.config.theme = t.to_string(),
            A::Fullscreen => {
                let fs = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
                ctx.send_viewport_cmd(ViewportCommand::Fullscreen(!fs));
            }
            A::NextTab | A::PrevTab => {
                let n = self.tabs.len();
                if n > 1 {
                    let next = if action == A::NextTab { (self.active + 1) % n } else { (self.active + n - 1) % n };
                    self.activate(next);
                }
            }
            A::Preferences => self.prefs_open = true,
            A::About => self.about_open = true,
            A::ShortcutsHelp => self.shortcuts_open = true,
            A::Cheatsheet => {
                const NAME: &str = "Markdown Cheatsheet";
                let open = self.tabs.iter().position(|t| t.doc.path.is_none() && t.doc.name.as_deref() == Some(NAME));
                let idx = open.unwrap_or_else(|| {
                    let mut doc = Document::from_text(dialogs::CHEATSHEET);
                    doc.name = Some(NAME.into());
                    self.push_tab(doc)
                });
                self.activate(idx);
            }
        }
    }

    /// Actions on the active tab's document.
    fn tab_action(&mut self, action: Action) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        tab.editor.request_focus = true;
        match action {
            Action::Undo => {
                tab.doc.buffer.undo();
            }
            Action::Redo => {
                tab.doc.buffer.redo();
            }
            Action::SelectAll => {
                tab.doc.buffer.selection = Selection { anchor: 0, head: tab.doc.buffer.len_bytes() };
            }
            Action::Format(cmd) => {
                if !tab.editor.run_command(&mut tab.doc, &cmd) {
                    log::debug!("command {cmd:?} did not apply");
                }
            }
            Action::ToggleSource => {
                tab.editor.mode = match tab.editor.mode {
                    Mode::Source => Mode::LivePreview,
                    Mode::LivePreview => Mode::Source,
                };
            }
            Action::Find | Action::Replace => {
                tab.editor.request_focus = false;
                self.find.open(action == Action::Replace, tab);
            }
            Action::FindNext | Action::FindPrev => {
                if !self.find.open {
                    self.find.open(false, tab);
                } else {
                    self.find.refresh(&mut self.tabs, self.active);
                    if let Some(tab) = self.tabs.get_mut(self.active) {
                        self.find.next(tab, action == Action::FindNext);
                    }
                }
            }
            _ => {}
        }
    }

    fn modal_open(&self) -> bool {
        self.pending_close.is_some() || !self.recovery_offer.is_empty() || self.table.open
    }

    // ---------------------------------------------------------------- UI pieces

    fn tab_bar(&mut self, ui: &mut egui::Ui) {
        let mut activate = None;
        let mut close = None;
        egui::ScrollArea::horizontal().auto_shrink([false, true]).scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(ui, |ui| {
            ui.horizontal(|ui| {
                for (i, t) in self.tabs.iter().enumerate() {
                    let label = tabs::tab_label(&t.doc.title(), t.doc.is_dirty());
                    let selected = i == self.active;
                    let frame = egui::Frame::new()
                        .inner_margin(egui::Margin::symmetric(6, 2))
                        .corner_radius(4.0)
                        .fill(if selected { ui.visuals().selection.bg_fill.gamma_multiply(0.35) } else { egui::Color32::TRANSPARENT });
                    frame.show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let hover = t.doc.path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "Not saved yet".into());
                            let r = ui.selectable_label(selected, label).on_hover_text(hover);
                            if r.clicked() {
                                activate = Some(i);
                            }
                            if r.middle_clicked() {
                                close = Some(t.id);
                            }
                            if ui.add(egui::Button::new("✕").small().frame(false)).on_hover_text("Close (Ctrl+W)").clicked() {
                                close = Some(t.id);
                            }
                        });
                    });
                }
                if ui.small_button("+").on_hover_text("New (Ctrl+N)").clicked() {
                    self.actions.push(Action::New);
                }
            });
        });
        if let Some(i) = activate {
            self.activate(i);
        }
        if let Some(id) = close {
            self.request_close(id);
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui, zoom: f32) {
        let Some(t) = self.tabs.get(self.active) else { return };
        ui.horizontal(|ui| {
            let s = &t.status;
            ui.label(format!("{} words · {} chars", s.words, s.chars));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let zoom_btn = ui.add(egui::Button::new(format!("{:.0}%", zoom * 100.0)).frame(false));
                if zoom_btn.on_hover_text("Zoom: Ctrl++ / Ctrl+-. Click to reset to 100%").clicked() {
                    self.actions.push(Action::ZoomReset);
                }
                ui.separator();
                ui.label(if t.doc.bom { "UTF-8 BOM" } else { "UTF-8" });
                ui.separator();
                ui.label(match t.doc.line_ending {
                    LineEnding::Lf => "LF",
                    LineEnding::CrLf => "CRLF",
                });
                ui.separator();
                let (mode, hint) = match t.editor.mode {
                    Mode::LivePreview => ("Live", "Live preview. Click to show Markdown source (Ctrl+E)"),
                    Mode::Source => ("Source", "Markdown source. Click to show live preview (Ctrl+E)"),
                };
                if ui.add(egui::Button::new(mode).frame(false)).on_hover_text(hint).clicked() {
                    self.actions.push(Action::ToggleSource);
                }
                ui.separator();
                ui.label(format!("Ln {}, Col {}", s.line.max(1), s.column.max(1)));
            });
        });
    }

    fn banners(&mut self, ui: &mut egui::Ui) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        if tab.disk_changed {
            banner(ui, |ui| {
                ui.label("⚠ This file changed on disk.");
                if ui.button("Reload").on_hover_text("Discard your changes and load the file from disk").clicked() {
                    if let Some(p) = tab.doc.path.clone()
                        && let Ok(disk) = Document::open(&p)
                    {
                        let text = disk.buffer.text();
                        let hash = tabs::hash_text(&text);
                        reload_tab(tab, disk, &text, hash);
                        let _ = recovery::remove(&tab.recovery_id);
                    }
                    tab.disk_changed = false;
                }
                if ui.button("Keep Mine").clicked() {
                    tab.disk_changed = false;
                    // Don't ask again until the file changes again.
                    if let Some(p) = tab.doc.path.clone()
                        && let Ok(disk) = Document::open(&p)
                    {
                        tab.disk_hash = Some(tabs::hash_text(&disk.buffer.text()));
                    }
                }
            });
        }
        if let Some(size) = tab.large_file {
            banner(ui, |ui| {
                ui.label(format!("Large file ({:.1} MB): opened in Source mode for speed. Ctrl+E switches to live preview.", size as f64 / 1_048_576.0));
                if ui.add(egui::Button::new("✕").small().frame(false)).clicked() {
                    tab.large_file = None;
                }
            });
        }
    }

    fn drop_overlay(ctx: &egui::Context) {
        let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());
        if !hovering {
            return;
        }
        let rect = ctx.content_rect();
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("drop-overlay")));
        painter.rect_filled(rect, 0.0, egui::Color32::from_black_alpha(140));
        painter.text(rect.center(), egui::Align2::CENTER_CENTER, "Drop files to open", egui::FontId::proportional(24.0), egui::Color32::WHITE);
    }
}

impl eframe::App for MdApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_background(&ctx);

        // Theme (light/dark/system) and zoom; invalidate editor caches on change.
        let zoom = ctx.zoom_factor();
        if self.theme.update(&ctx, &self.config) || (zoom - self.last_zoom).abs() > f32::EPSILON {
            self.last_zoom = zoom;
            for t in &mut self.tabs {
                t.editor.invalidate();
            }
        }
        let theme = self.theme.current();

        // Window close button / Alt+F4.
        if ctx.input(|i| i.viewport().close_requested()) && !self.allow_close {
            if self.tabs.iter().any(|t| t.doc.is_dirty()) {
                ctx.send_viewport_cmd(ViewportCommand::CancelClose);
                if !self.quitting {
                    self.request_quit(&ctx);
                }
            } else {
                self.on_exit_cleanup();
                self.allow_close = true;
            }
        }

        // Drag and drop.
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        for p in dropped {
            self.open_path(&p);
        }

        self.poll_watcher();

        let mut actions = std::mem::take(&mut self.actions);
        if !self.modal_open() {
            menu::consume_shortcuts(&ctx, &mut actions);
            // Escape closes the find bar.
            if self.find.open && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                self.find.close(&mut self.tabs);
                if let Some(t) = self.tabs.get_mut(self.active) {
                    t.editor.request_focus = true;
                }
            }
        }

        // Menu bar.
        egui::Panel::top("menu").show(ui, |ui| {
            let tab = self.tabs.get(self.active);
            let fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
            let st = MenuState {
                recent: &self.recent.files,
                source_mode: tab.is_some_and(|t| t.editor.mode == Mode::Source),
                sidebar: self.show_sidebar,
                typewriter: self.typewriter,
                focus_mode: self.focus_mode,
                theme: &self.config.theme,
                fullscreen,
                can_undo: tab.is_some_and(|t| t.doc.buffer.can_undo()),
                can_redo: tab.is_some_and(|t| t.doc.buffer.can_redo()),
            };
            menu::menu_bar(ui, &st, &mut actions);
        });
        self.actions = actions;

        egui::Panel::top("tabs").show(ui, |ui| self.tab_bar(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui, zoom));

        let mut wake: Vec<Option<Duration>> = Vec::new();
        if self.show_sidebar {
            let mut heading = None;
            egui::Panel::left("sidebar").resizable(true).default_size(240.0).size_range(160.0..=600.0).show(ui, |ui| {
                let (h, w) = self.sidebar.ui(ui, self.tabs.get(self.active), &self.tx, &mut self.show_sidebar);
                heading = h;
                wake.push(w);
            });
            if let Some(at) = heading
                && let Some(tab) = self.tabs.get_mut(self.active)
            {
                tab.editor.scroll_to(&mut tab.doc, at);
                tab.editor.request_focus = true;
            }
        } else {
            // Collapsed: a thin strip with a button that brings the outline back.
            egui::Panel::left("sidebar_collapsed").resizable(false).exact_size(28.0).show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    if ui.small_button("»").on_hover_text("Show outline (Ctrl+Shift+B)").clicked() {
                        self.show_sidebar = true;
                    }
                });
            });
        }

        // Central area: banners, find bar, editor.
        let remote_images = self.config.remote_images;
        let mut link = None;
        egui::CentralPanel::default().frame(egui::Frame::new().fill(theme.bg)).show(ui, |ui| {
            self.banners(ui);
            if self.find.open {
                self.find.refresh(&mut self.tabs, self.active);
                egui::Frame::new().inner_margin(egui::Margin::symmetric(8, 4)).fill(ui.visuals().panel_fill).show(ui, |ui| {
                    if let Some(msg) = self.find.ui(ui, &mut self.tabs, self.active) {
                        self.toasts.info(msg);
                    }
                });
            }
            let hl = Arc::clone(&self.highlighter);
            if let Some(tab) = self.tabs.get_mut(self.active) {
                tab.editor.typewriter = self.typewriter;
                tab.editor.focus_mode = self.focus_mode;
                let base = tab.doc.path.as_deref().and_then(Path::parent).map(Path::to_path_buf);
                tab.editor.remote_images = remote_images;
                let out = tab.editor.show(ui, &mut tab.doc, &theme, &hl, base.as_deref());
                link = out.open_link.clone();
                tab.status = out;
            }
        });
        if let Some(url) = link {
            self.handle_link(&url);
        }

        // Modals and windows.
        if let Some(pc) = self.pending_close {
            let name = self.tab_index(pc.tab_id).map(|i| self.tabs[i].doc.title());
            match name {
                None => {
                    self.pending_close = None;
                    if pc.quit {
                        self.advance_quit(&ctx);
                    }
                }
                Some(name) => match dialogs::unsaved_changes_modal(&ctx, &name) {
                    Some(SaveChoice::Save) => {
                        self.pending_close = None;
                        let has_path = self.tab_index(pc.tab_id).is_some_and(|i| self.tabs[i].doc.path.is_some());
                        if self.save_tab(pc.tab_id, true) {
                            self.close_tab_now(pc.tab_id);
                            if pc.quit {
                                self.advance_quit(&ctx);
                            }
                        } else if has_path && pc.quit {
                            self.cancel_quit(); // save failed
                        }
                    }
                    Some(SaveChoice::DontSave) => {
                        self.pending_close = None;
                        self.close_tab_now(pc.tab_id);
                        if pc.quit {
                            self.advance_quit(&ctx);
                        }
                    }
                    Some(SaveChoice::Cancel) => self.cancel_quit(),
                    None => {}
                },
            }
        } else if !self.recovery_offer.is_empty() {
            match dialogs::recovery_modal(&ctx, &self.recovery_offer) {
                Some(RecoveryChoice::Restore) => self.restore_snapshots(&ctx),
                Some(RecoveryChoice::Discard) => {
                    for s in std::mem::take(&mut self.recovery_offer) {
                        if let Err(e) = recovery::remove(&s.id) {
                            self.toasts.error(format!("Can't delete snapshot: {e}"));
                        }
                    }
                }
                Some(RecoveryChoice::Later) => self.recovery_offer.clear(),
                None => {}
            }
        }
        if let Some((cols, rows)) = self.table.show(&ctx) {
            self.tab_action(Action::Format(mdedit_editor::Command::Table { cols, rows }));
        }
        if self.prefs_open && crate::prefs::preferences_window(&ctx, &mut self.prefs_open, &mut self.config) {
            self.save_config_if_changed();
        }
        dialogs::about_window(&ctx, &mut self.about_open);
        dialogs::shortcuts_window(&ctx, &mut self.shortcuts_open);

        // Actions from menus, shortcuts and widgets.
        for a in std::mem::take(&mut self.actions) {
            self.run_action(a, &ctx);
        }
        // Persist preference changes made outside the window (theme menu, sidebar toggle)
        // when no preferences window is being edited.
        if !self.prefs_open {
            self.sync_config();
            self.save_config_if_changed();
        }

        wake.push(self.autosave_tick());
        wake.push(self.toasts.show(&ctx));
        Self::drop_overlay(&ctx);

        // Window title.
        let title = self.tabs.get(self.active).map(|t| tabs::window_title(&t.doc.title(), t.doc.is_dirty())).unwrap_or_else(|| "mdedit".into());
        if title != self.last_title {
            ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }

        // Idle: wake only for timers / watcher polling.
        if self.watcher.is_some() && self.tabs.iter().any(|t| t.doc.path.is_some()) {
            wake.push(Some(WATCH_POLL));
        }
        if let Some(d) = wake.into_iter().flatten().min() {
            ctx.request_repaint_after(d.max(Duration::from_millis(16)));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.sync_config();
        self.save_config_if_changed();
    }
}

fn banner(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new().inner_margin(egui::Margin::symmetric(10, 6)).fill(ui.visuals().warn_fg_color.gamma_multiply(0.18)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_wrapped(add);
    });
}

/// Replace a tab's text with the file's content, keeping the caret (clamped).
fn reload_tab(tab: &mut Tab, disk: Document, text: &str, hash: u64) {
    let caret = tab.doc.buffer.selection.head;
    tab.doc.buffer.set_text(text);
    tab.doc.line_ending = disk.line_ending;
    tab.doc.bom = disk.bom;
    tab.doc.mark_saved();
    tab.clamp_caret(caret);
    tab.disk_hash = Some(hash);
    tab.disk_changed = false;
    tab.snapshot_version = None;
    tab.editor.invalidate();
}

/// Whether two paths name the same file (canonicalized when possible).
pub fn same_file(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

/// File name for Save As from the first heading or line of the text.
pub fn suggested_name(text: &str) -> String {
    let first = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let first = first.trim_start_matches('#').trim();
    let mut name: String = first.chars().filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_')).take(50).collect();
    name = name.trim().to_string();
    if name.is_empty() { "Untitled.md".into() } else { format!("{name}.md") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggested_names() {
        assert_eq!(suggested_name("# My Notes: v2!\n\ntext"), "My Notes v2.md");
        assert_eq!(suggested_name("\n\n  hello/../world  "), "helloworld.md");
        assert_eq!(suggested_name(""), "Untitled.md");
        assert_eq!(suggested_name("###"), "Untitled.md");
    }

    #[test]
    fn same_file_via_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.md");
        std::fs::write(&a, "x").unwrap();
        assert!(same_file(&a, &a));
        assert!(same_file(&a, &dir.path().join("./a.md")));
        assert!(!same_file(&a, &dir.path().join("b.md")));
        #[cfg(unix)]
        {
            let l = dir.path().join("link.md");
            std::os::unix::fs::symlink(&a, &l).unwrap();
            assert!(same_file(&a, &l));
        }
    }
}

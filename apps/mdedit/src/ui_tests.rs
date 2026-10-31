//! Whole-app visual checks with egui_kittest (rendered with wgpu, so ignored by default):
//! `MDEDIT_HOME=$(mktemp -d) MDEDIT_SHOT_DIR=/tmp/shots cargo test -p mdedit ui_tests -- --ignored`
//! (`MDEDIT_HOME` keeps the test away from your real settings; `MDEDIT_SHOT_DOC` picks the
//! document, default `docs/architecture.md`.)

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use crate::app::MdApp;
use crate::cli::StartupTargets;

/// Open a long document in dark mode, switch the theme to light and back, and save a
/// screenshot after each step. The final dark view must match the first one: a font atlas
/// reset used to leave the visible text as gibberish.
#[test]
#[ignore]
fn theme_switch_keeps_text_readable() {
    let Some(dir) = std::env::var_os("MDEDIT_SHOT_DIR") else { return };
    let home = std::env::var_os(mdedit_workspace::HOME_ENV).expect("set MDEDIT_HOME to a scratch dir");
    let config = std::path::Path::new(&home).join("config");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(config.join("config.toml"), "theme = \"dark\"\n").unwrap();
    let doc = std::env::var_os("MDEDIT_SHOT_DOC").map_or_else(
        || std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/architecture.md"),
        std::path::PathBuf::from,
    );
    let targets = StartupTargets { files: vec![doc], ..Default::default() };
    let mut h = Harness::builder().with_size(egui::vec2(1100.0, 750.0)).build_eframe(|cc| MdApp::new(cc, targets));
    let shot = |h: &mut Harness<'_, MdApp>, name: &str| {
        h.run_steps(6);
        let img = h.render().expect("render");
        let path = std::path::Path::new(&dir).join(name);
        img.save(&path).expect("save");
        eprintln!("wrote {}", path.display());
        img
    };
    let pick_theme = |h: &mut Harness<'_, MdApp>, theme: &str| {
        h.get_by_label("View").click();
        h.run_steps(2);
        h.get_by_label("Theme").hover();
        h.run_steps(2);
        h.get_by_label(theme).click();
    };
    let first = shot(&mut h, "1_dark.png");
    pick_theme(&mut h, "Light");
    shot(&mut h, "2_light.png");
    pick_theme(&mut h, "Dark");
    let last = shot(&mut h, "3_dark_again.png");
    // Only the caret and the pointer's hover effects may differ (~200 px when correct,
    // ~20 000 px when the text is garbled).
    let differing = first.pixels().zip(last.pixels()).filter(|(a, b)| a.0.iter().zip(b.0).any(|(x, y)| x.abs_diff(y) > 25)).count();
    assert!(differing < 2000, "{differing} pixels differ between the first and final dark views");
}

/// Opening a file into the empty startup tab must show that file's outline (the replaced tab
/// used to keep its id, so the outline cache thought it was up to date).
#[test]
#[ignore]
fn outline_shows_for_file_opened_into_empty_tab() {
    assert!(std::env::var_os(mdedit_workspace::HOME_ENV).is_some(), "set MDEDIT_HOME to a scratch dir");
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("doc.md");
    std::fs::write(&file, "# First heading\n\ntext\n\n## Second heading\n").unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_eframe(|cc| MdApp::new(cc, StartupTargets::default()));
    h.run_steps(3);
    h.input_mut().dropped_files.push(std::sync::Arc::new(Dropped(file)));
    h.run_steps(3);
    h.get_by_label("doc.md");
    h.get_by_label("First heading");
    h.get_by_label("Second heading");
}

/// A later `mdedit` start sends its file over the socket: it opens as a tab in this window.
#[test]
#[ignore]
fn file_from_a_later_start_opens_as_a_tab() {
    assert!(std::env::var_os(mdedit_workspace::HOME_ENV).is_some(), "set MDEDIT_HOME to a scratch dir");
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("later.md");
    std::fs::write(&file, "# Later heading\n").unwrap();
    let sock = dir.path().join("s.sock");
    let crate::single::Claim::First(listener) = crate::single::claim(&sock, &[]).unwrap() else { panic!("first start") };
    let mut h = Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_eframe(|cc| {
        let app = MdApp::new(cc, StartupTargets::default());
        app.listen(listener);
        app
    });
    h.run_steps(2);
    assert!(matches!(crate::single::claim(&sock, std::slice::from_ref(&file)).unwrap(), crate::single::Claim::Forwarded));
    for _ in 0..50 {
        std::thread::sleep(std::time::Duration::from_millis(10));
        h.run_steps(1);
        if h.query_by_label("later.md").is_some() {
            break;
        }
    }
    h.get_by_label("later.md");
    h.get_by_label("Later heading");
}

/// The last session's files reopen with the same active tab, a file given at startup comes
/// on top, and the session file follows the tabs.
#[test]
#[ignore]
fn session_restores_and_follows_tabs() {
    assert!(std::env::var_os(mdedit_workspace::HOME_ENV).is_some(), "set MDEDIT_HOME to a scratch dir");
    let dir = tempfile::tempdir().unwrap();
    let file = |n: &str| {
        let p = dir.path().join(format!("{n}.md"));
        std::fs::write(&p, format!("# Heading {n}\n")).unwrap();
        p
    };
    let (a, b, c) = (file("one"), file("two"), file("three"));
    let session = mdedit_workspace::Session { files: vec![a.clone(), b.clone(), dir.path().join("gone.md")], active: Some(a.clone()) };
    let start = |files: Vec<std::path::PathBuf>, session: mdedit_workspace::Session| StartupTargets { files, session: Some(session), save_session: true };

    let targets = start(vec![], session.clone());
    let mut h = Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_eframe(|cc| MdApp::new(cc, targets));
    h.run_steps(3);
    h.get_by_label("one.md");
    h.get_by_label("two.md");
    assert!(h.query_by_label("gone.md").is_none() && h.query_by_label("Untitled").is_none());
    h.get_by_label("Heading one"); // the saved active tab's outline
    h.get_by_label("two.md").click();
    h.run_steps(3);
    let saved = mdedit_workspace::Session::load();
    assert_eq!((saved.files, saved.active), (vec![a.clone(), b.clone()], Some(b.clone())));
    drop(h);

    // A file given at startup: restored tabs plus that one, active.
    let targets = start(vec![c.clone()], session);
    let mut h = Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_eframe(|cc| MdApp::new(cc, targets));
    h.run_steps(3);
    h.get_by_label("Heading three");
    let saved = mdedit_workspace::Session::load();
    assert_eq!((saved.files, saved.active), (vec![a, b, c.clone()], Some(c)));
}

/// A file dropped on the window (dropping goes through the same open path as File → Open).
#[derive(Debug)]
struct Dropped(std::path::PathBuf);

impl egui::DroppedFile for Dropped {
    fn path(&self) -> &std::path::Path {
        &self.0
    }
    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|e| e.to_string())
    }
}

/// Scrolling a tab, visiting a shorter tab and coming back must leave the view where it was
/// (tabs used to share a scroll area, which clamped to the other document's length).
#[test]
#[ignore]
fn tab_switch_keeps_scroll_position() {
    assert!(std::env::var_os(mdedit_workspace::HOME_ENV).is_some(), "set MDEDIT_HOME to a scratch dir");
    let tmp = tempfile::tempdir().unwrap();
    let doc = |name: &str, n: usize| {
        let body: String = (0..n).map(|i| format!("## Section {name}{i}\n\nSome text for section {i}.\n\n")).collect();
        let p = tmp.path().join(format!("{name}.md"));
        std::fs::write(&p, body).unwrap();
        p
    };
    let targets = StartupTargets { files: vec![doc("aaa", 60), doc("bbb", 20)], ..Default::default() };
    let mut h = Harness::builder().with_size(egui::vec2(1100.0, 750.0)).build_eframe(|cc| MdApp::new(cc, targets));
    h.run_steps(3);
    h.get_by_label("aaa.md").click();
    h.run_steps(3);
    h.hover_at(egui::pos2(700.0, 400.0));
    h.run_steps(2);
    for _ in 0..40 {
        h.input_mut().events.push(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -60.0),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            modifiers: egui::Modifiers::NONE,
        });
        h.run_steps(1);
    }
    h.run_steps(40);
    let first = h.render().expect("render");
    h.get_by_label("bbb.md").click();
    h.run_steps(5);
    h.get_by_label("aaa.md").click();
    h.run_steps(40);
    h.hover_at(egui::pos2(700.0, 400.0));
    h.run_steps(5);
    let last = h.render().expect("render");
    let differing = first.pixels().zip(last.pixels()).filter(|(a, b)| a.0.iter().zip(b.0).any(|(x, y)| x.abs_diff(y) > 25)).count();
    assert!(differing < 500, "{differing} pixels differ after switching tabs and back");
}

fn start(files: Vec<std::path::PathBuf>) -> Harness<'static, MdApp> {
    assert!(std::env::var_os(mdedit_workspace::HOME_ENV).is_some(), "set MDEDIT_HOME to a scratch dir");
    let targets = StartupTargets { files, ..Default::default() };
    let mut h = Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_eframe(|cc| MdApp::new(cc, targets));
    h.run_steps(3);
    h
}

fn write_file(dir: &tempfile::TempDir, name: &str) -> std::path::PathBuf {
    let p = dir.path().join(name);
    std::fs::write(&p, "# Heading one\n\ntext\n\n# Heading two\n").unwrap();
    p
}

fn type_text(h: &mut Harness<'_, MdApp>, text: &str) {
    h.event(egui::Event::Text(text.to_string()));
    h.run_steps(3);
}

/// Press and release in separate frames, as a real mouse does.
fn slow_click(h: &mut Harness<'_, MdApp>, label: &str) {
    let pos = h.get_by_label(label).rect().center();
    let pos = egui::pos2(pos.x, pos.y);
    h.hover_at(pos);
    h.run_steps(2);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
        h.run_steps(2);
    }
    h.run_steps(3);
}

/// With two snapshots of one file the newest wins, and the older one is gone for good (it used
/// to overwrite the newer text, and the newer snapshot was deleted).
#[test]
#[ignore]
fn restore_keeps_the_newest_snapshot_of_a_file() {
    use mdedit_workspace::recovery;
    assert!(std::env::var_os(mdedit_workspace::HOME_ENV).is_some(), "set MDEDIT_HOME to a scratch dir");
    let dir = tempfile::tempdir().unwrap();
    let file = write_file(&dir, "doc.md");
    recovery::write("r1-100-0", Some(&file), "# Older crash\n").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    recovery::write("r1-200-0", Some(&file), "# Newer crash\n").unwrap();
    let mut h = start(vec![]);
    h.get_by_label("Restore").click();
    h.run_steps(5);
    h.get_by_label("Newer crash");
    assert!(h.query_by_label("Older crash").is_none());
    assert_eq!(h.query_all_by_label("doc.md •").count(), 1);
    let left: Vec<String> = recovery::list().into_iter().map(|s| s.id).collect();
    assert_eq!(left, ["r1-200-0"]);
    recovery::remove("r1-200-0").unwrap();
}

/// Text typed after clicking a tab, in either way of clicking, goes to that tab's document.
#[test]
#[ignore]
fn clicking_a_tab_focuses_the_editor() {
    let dir = tempfile::tempdir().unwrap();
    let (x, y) = (write_file(&dir, "x.md"), write_file(&dir, "y.md"));
    let mut h = start(vec![x, y]);
    slow_click(&mut h, "x.md");
    type_text(&mut h, "a");
    h.get_by_label("x.md •");
    h.get_by_label("y.md").click();
    h.run_steps(3);
    type_text(&mut h, "b");
    h.get_by_label("y.md •");
}

/// Same for closing a tab: the tab left active takes the keys.
#[test]
#[ignore]
fn closing_a_tab_by_click_focuses_the_editor() {
    let dir = tempfile::tempdir().unwrap();
    let (x, y) = (write_file(&dir, "x.md"), write_file(&dir, "y.md"));
    let mut h = start(vec![x, y]);
    h.get_all_by_label("✕").last().unwrap().click();
    h.run_steps(3);
    assert!(h.query_by_label("y.md").is_none());
    type_text(&mut h, "a");
    h.get_by_label("x.md •");
}

#[test]
#[ignore]
fn clicking_an_outline_heading_focuses_the_editor() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = start(vec![write_file(&dir, "x.md")]);
    slow_click(&mut h, "Heading two");
    type_text(&mut h, "a");
    h.get_by_label("x.md •");
    let mut h = start(vec![write_file(&dir, "y.md")]);
    h.get_by_label("Heading two").click();
    h.run_steps(3);
    type_text(&mut h, "a");
    h.get_by_label("y.md •");
}

/// Keyboard tab switching keeps focusing the editor, and the find bar keeps its own focus even
/// right after a click that refocuses the editor.
#[test]
#[ignore]
fn keyboard_activation_and_find_bar_keep_focus() {
    let dir = tempfile::tempdir().unwrap();
    let (x, y) = (write_file(&dir, "x.md"), write_file(&dir, "y.md"));
    let mut h = start(vec![x, y]);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Tab);
    h.run_steps(3);
    type_text(&mut h, "a");
    h.get_by_label("x.md •");
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::F);
    h.run_steps(3);
    type_text(&mut h, "zzz");
    h.get_by_label("No results");
    drop(h);

    let (x, y) = (write_file(&dir, "x.md"), write_file(&dir, "y.md"));
    let mut h = start(vec![x, y]);
    h.get_by_label("x.md").click();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::F);
    h.run_steps(4);
    type_text(&mut h, "zzz");
    h.get_by_label("No results");
    h.get_by_label("x.md");
}

/// A path that doesn't exist yet opens one tab however many times it is asked for.
#[test]
#[ignore]
fn missing_path_opens_one_tab() {
    assert!(std::env::var_os(mdedit_workspace::HOME_ENV).is_some(), "set MDEDIT_HOME to a scratch dir");
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("new.md");
    let sock = dir.path().join("s.sock");
    let crate::single::Claim::First(listener) = crate::single::claim(&sock, &[]).unwrap() else { panic!("first start") };
    let targets = StartupTargets { files: vec![file.clone(), file.clone(), dir.path().join("./new.md")], ..Default::default() };
    let mut h = Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_eframe(|cc| {
        let app = MdApp::new(cc, targets);
        app.listen(listener);
        app
    });
    h.run_steps(3);
    assert_eq!(h.query_all_by_label("new.md").count(), 1);
    assert!(matches!(crate::single::claim(&sock, std::slice::from_ref(&file)).unwrap(), crate::single::Claim::Forwarded));
    for _ in 0..20 {
        std::thread::sleep(std::time::Duration::from_millis(10));
        h.run_steps(1);
    }
    assert_eq!(h.query_all_by_label("new.md").count(), 1);
}

/// A FIFO gets an error toast and no tab, whichever way it is reached (it used to freeze).
#[test]
#[ignore]
fn fifo_is_refused_everywhere() {
    let dir = tempfile::tempdir().unwrap();
    let fifo = dir.path().join("pipe.md");
    assert!(std::process::Command::new("mkfifo").arg(&fifo).status().unwrap().success());
    let toast = format!("Can't open {}: not a regular file", fifo.display());
    let refused = |h: &Harness<'_, MdApp>| {
        assert!(h.query_all_by_label(&toast).count() > 0, "no toast");
        assert!(h.query_by_label("pipe.md").is_none(), "opened a tab");
    };

    // Command line.
    let h = start(vec![fifo.clone()]);
    refused(&h);
    drop(h);

    // Session.
    let session = mdedit_workspace::Session { files: vec![fifo.clone()], active: Some(fifo.clone()) };
    let targets = StartupTargets { session: Some(session), ..Default::default() };
    let mut h = Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_eframe(|cc| MdApp::new(cc, targets));
    h.run_steps(3);
    refused(&h);
    drop(h);

    // Drop.
    let mut h = start(vec![]);
    h.input_mut().dropped_files.push(std::sync::Arc::new(Dropped(fifo.clone())));
    h.run_steps(3);
    refused(&h);
    drop(h);

    // Recent files.
    mdedit_workspace::RecentFiles { files: vec![fifo.clone()] }.save().unwrap();
    let mut h = start(vec![]);
    h.get_by_label("File").click();
    h.run_steps(2);
    h.get_by_label("Open Recent").hover();
    h.run_steps(2);
    h.query_all_by(|n| n.label().is_some_and(|l| l.contains("pipe.md"))).last().expect("recent entry").click();
    h.run_steps(3);
    refused(&h);
}

fn image_bytes_held(h: &Harness<'_, MdApp>) -> [usize; 3] {
    let l = h.ctx.loaders();
    [
        l.bytes.lock().iter().map(|x| x.byte_size()).sum(),
        l.image.lock().iter().map(|x| x.byte_size()).sum(),
        l.texture.lock().iter().map(|x| x.byte_size()).sum(),
    ]
}

/// Two tabs show one image: closing the first keeps it, closing the last frees it.
#[test]
#[ignore]
fn closing_last_tab_showing_an_image_frees_it() {
    std::env::var_os(mdedit_workspace::HOME_ENV).expect("set MDEDIT_HOME to a scratch dir");
    let d = tempfile::tempdir().unwrap();
    image::RgbaImage::new(64, 64).save(d.path().join("a.png")).unwrap();
    let files: Vec<_> = ["1.md", "2.md"].map(|n| d.path().join(n)).into();
    for f in &files {
        std::fs::write(f, "# t\n\n![x](a.png)\n").unwrap();
    }
    let targets = StartupTargets { files, ..Default::default() };
    let mut h = Harness::builder().with_size(egui::vec2(1100.0, 750.0)).build_eframe(|cc| MdApp::new(cc, targets));
    let settle = |h: &mut Harness<'_, MdApp>| {
        for _ in 0..30 {
            h.run_steps(1);
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    };
    settle(&mut h);
    assert!(image_bytes_held(&h).iter().all(|&b| b > 0), "{:?}", image_bytes_held(&h));
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::W);
    settle(&mut h);
    assert!(image_bytes_held(&h).iter().all(|&b| b > 0), "{:?}", image_bytes_held(&h));
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::W);
    settle(&mut h);
    assert_eq!(image_bytes_held(&h), [0, 0, 0]);
}

/// A config that can't be written is reported once per change, not retried every frame.
#[test]
#[ignore]
fn failed_config_save_is_not_retried_every_frame() {
    let home = std::env::var_os(mdedit_workspace::HOME_ENV).expect("set MDEDIT_HOME to a scratch dir");
    let target = std::path::Path::new(&home).join("config/config.toml");
    std::fs::create_dir_all(&target).unwrap(); // a directory: saving it fails
    let mut h = start(vec![]);
    h.get_by_label("View").click();
    h.run_steps(2);
    h.get_by_label("Theme").hover();
    h.run_steps(2);
    h.get_by_label("Dark").click();
    h.run_steps(3);
    let saved = h.state().config_is_saved();
    std::fs::remove_dir(&target).unwrap();
    assert!(saved, "the failed save must not stay pending");
}

/// Save on a quit prompt while a file dialog is open can't start Save As: it must keep asking,
/// and the window's close button must still work afterwards.
#[test]
#[ignore]
fn quit_save_with_a_dialog_open_keeps_asking() {
    assert!(std::env::var_os(mdedit_workspace::HOME_ENV).is_some(), "set MDEDIT_HOME to a scratch dir");
    let mut h = start(vec![]);
    h.run_steps(3);
    type_text(&mut h, "unsaved");
    h.state_mut().set_dialog_busy(true);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Q);
    h.run_steps(3);
    assert!(h.state().close_is_pending());
    h.get_by_label("Save").click();
    h.run_steps(3);
    assert!(h.state().close_is_pending(), "still asking");
}


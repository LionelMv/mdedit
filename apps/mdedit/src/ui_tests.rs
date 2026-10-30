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
    let targets = StartupTargets { files: vec![doc] };
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
    let targets = StartupTargets { files: vec![doc("aaa", 60), doc("bbb", 20)] };
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

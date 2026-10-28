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

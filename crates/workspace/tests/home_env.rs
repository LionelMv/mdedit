//! End-to-end test of the default-location APIs under `MDEDIT_HOME`.
//!
//! `std::env::set_var` is `unsafe` in edition 2024 (and unsafe is forbidden), so the parent
//! test re-runs this test binary with `MDEDIT_HOME` set and only the child does the work.

use std::path::Path;
use std::process::Command;

use mdedit_workspace::{Config, HOME_ENV, RecentFiles, recovery};

#[test]
fn default_locations_honour_mdedit_home() {
    if let Some(home) = std::env::var_os(HOME_ENV) {
        child(Path::new(&home));
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let status = Command::new(std::env::current_exe().unwrap())
        .env(HOME_ENV, tmp.path())
        .args(["--exact", "default_locations_honour_mdedit_home", "--nocapture", "--test-threads=1"])
        .status()
        .unwrap();
    assert!(status.success());
    // The child wrote everything under the override.
    assert!(tmp.path().join("config/config.toml").is_file());
    assert!(tmp.path().join("state/recent.toml").is_file());
    assert!(tmp.path().join("state/recovery").is_dir());
}

fn child(home: &Path) {
    assert_eq!(mdedit_workspace::config_dir().unwrap(), home.join("config"));
    assert_eq!(mdedit_workspace::state_dir().unwrap(), home.join("state"));
    assert_eq!(mdedit_workspace::data_dir().unwrap(), home.join("data"));

    assert_eq!(Config::load(), Config::default());
    let c = Config { theme: "dark".into(), ..Config::default() };
    c.save().unwrap();
    assert_eq!(Config::load(), c);

    let doc = home.join("doc.md");
    std::fs::write(&doc, "x").unwrap();
    let mut r = RecentFiles::load();
    assert!(r.files.is_empty());
    r.add(&doc);
    assert_eq!(RecentFiles::load().files, vec![std::fs::canonicalize(&doc).unwrap()]);

    recovery::write("tab1", Some(&doc), "unsaved").unwrap();
    let l = recovery::list();
    assert_eq!(l.len(), 1);
    assert_eq!(l[0].text, "unsaved");
    recovery::remove("tab1").unwrap();
    assert!(recovery::list().is_empty());
    assert!(recovery::write("bad id", None, "").is_err());
}

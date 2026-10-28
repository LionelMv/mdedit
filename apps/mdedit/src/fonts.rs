//! System fonts through fontconfig's `fc-match` (no extra crates): sans, bold, italic,
//! monospace, CJK and colour emoji fallbacks. Falls back to egui's bundled fonts.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use egui::{FontData, FontDefinitions, FontFamily};

/// Family names the style crate uses for bold / italic runs.
pub use mdedit_style::{BOLD_FAMILY, BOLD_ITALIC_FAMILY, ITALIC_FAMILY};

const MAX_FONT_BYTES: u64 = 50 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slot {
    Sans,
    Bold,
    Italic,
    BoldItalic,
    Mono,
    Cjk,
    Emoji,
    Symbols,
    Symbols2,
}

const LOOKUPS: &[(Slot, &str)] = &[
    (Slot::Sans, "sans-serif"),
    (Slot::Bold, "sans-serif:bold"),
    (Slot::Italic, "sans-serif:italic"),
    (Slot::BoldItalic, "sans-serif:bold:italic"),
    (Slot::Mono, "monospace"),
    (Slot::Cjk, "Noto Sans CJK SC"),
    (Slot::Emoji, "Noto Color Emoji"),
    // UI symbols (✕ ⟳ ⚠ ⏶) that text fonts and colour emoji lack.
    (Slot::Symbols, "DejaVu Sans"),
    (Slot::Symbols2, "Noto Sans Symbols2"),
];

/// Run `fc-match -f "%{file}\n%{family}" <pattern>`; returns (file, family list).
fn fc_match(pattern: &str) -> Option<(PathBuf, String)> {
    let out = Command::new("fc-match").arg("-f").arg("%{file}\n%{family}").arg(pattern).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?;
    let mut lines = s.lines();
    let file = lines.next()?.trim();
    if file.is_empty() {
        return None;
    }
    Some((PathBuf::from(file), lines.next().unwrap_or("").to_string()))
}

fn load(path: &Path) -> Option<Vec<u8>> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_FONT_BYTES {
        log::info!("skipping font {} (missing or too big)", path.display());
        return None;
    }
    std::fs::read(path).ok()
}

/// Whether a fallback lookup really found what we asked for (fc-match always returns *something*).
fn fallback_ok(slot: Slot, family: &str) -> bool {
    let f = family.to_ascii_lowercase();
    match slot {
        Slot::Cjk => f.contains("cjk") || f.contains("han") || f.contains("wenquanyi") || f.contains("droid sans fallback"),
        Slot::Emoji => f.contains("emoji"),
        Slot::Symbols => f.contains("dejavu sans"),
        Slot::Symbols2 => f.contains("symbols"),
        _ => true,
    }
}

/// Build the font definitions. Never fails: missing fonts fall back to egui's defaults.
pub fn system_font_definitions() -> FontDefinitions {
    // Lookups in parallel: each fc-match spawn takes a few ms.
    let found: Vec<(Slot, Option<(PathBuf, String)>)> = std::thread::scope(|s| {
        let handles: Vec<_> = LOOKUPS.iter().map(|&(slot, pat)| (slot, s.spawn(move || fc_match(pat)))).collect();
        handles.into_iter().map(|(slot, h)| (slot, h.join().ok().flatten())).collect()
    });

    let mut defs = FontDefinitions::default();
    let default_prop = defs.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let default_mono = defs.families.get(&FontFamily::Monospace).cloned().unwrap_or_default();

    let mut names: Vec<(Slot, String)> = Vec::new();
    let mut loaded: Vec<PathBuf> = Vec::new();
    for (slot, hit) in found {
        let Some((path, family)) = hit else { continue };
        if !fallback_ok(slot, &family) {
            log::debug!("no {slot:?} font (fc-match gave {family})");
            continue;
        }
        let name = format!("system-{slot:?}").to_lowercase();
        // Reuse the bytes when two slots resolve to the same file (e.g. no real italic).
        if let Some(i) = loaded.iter().position(|p| p == &path) {
            let existing = names[i].1.clone();
            names.push((slot, existing));
            loaded.push(path);
            continue;
        }
        let Some(bytes) = load(&path) else { continue };
        log::debug!("font {slot:?}: {}", path.display());
        // .ttc collections: index 0 (FontData::index defaults to 0).
        defs.font_data.insert(name.clone(), Arc::new(FontData::from_owned(bytes)));
        names.push((slot, name));
        loaded.push(path);
    }
    let get = |slot: Slot| names.iter().find(|(s, _)| *s == slot).map(|(_, n)| n.clone());
    let fallbacks: Vec<String> = [get(Slot::Cjk), get(Slot::Emoji), get(Slot::Symbols), get(Slot::Symbols2)].into_iter().flatten().collect();

    let chain = |primary: Option<String>, tail: &[String]| -> Vec<String> {
        let mut v: Vec<String> = primary.into_iter().collect();
        for t in fallbacks.iter().chain(tail) {
            if !v.contains(t) {
                v.push(t.clone());
            }
        }
        v
    };
    let prop = chain(get(Slot::Sans), &default_prop);
    let mono = chain(get(Slot::Mono), &default_mono);
    let bold = chain(get(Slot::Bold).or_else(|| get(Slot::Sans)), &default_prop);
    let italic = chain(get(Slot::Italic).or_else(|| get(Slot::Sans)), &default_prop);
    defs.families.insert(FontFamily::Proportional, prop);
    defs.families.insert(FontFamily::Monospace, mono);
    defs.families.insert(FontFamily::Name(BOLD_FAMILY.into()), bold);
    let bold_italic = chain(get(Slot::BoldItalic).or_else(|| get(Slot::Bold)).or_else(|| get(Slot::Sans)), &default_prop);
    defs.families.insert(FontFamily::Name(ITALIC_FAMILY.into()), italic);
    defs.families.insert(FontFamily::Name(BOLD_ITALIC_FAMILY.into()), bold_italic);
    defs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_families_always_registered() {
        let defs = system_font_definitions();
        for fam in [FontFamily::Proportional, FontFamily::Monospace, FontFamily::Name(BOLD_FAMILY.into()), FontFamily::Name(ITALIC_FAMILY.into()), FontFamily::Name(BOLD_ITALIC_FAMILY.into())] {
            let list = defs.families.get(&fam).expect("family");
            assert!(!list.is_empty());
            for name in list {
                assert!(defs.font_data.contains_key(name), "{name} missing");
            }
        }
    }

    #[test]
    fn fallback_detection() {
        assert!(fallback_ok(Slot::Cjk, "Noto Sans CJK SC,Noto Sans CJK SC Regular"));
        assert!(!fallback_ok(Slot::Cjk, "DejaVu Sans"));
        assert!(fallback_ok(Slot::Emoji, "Noto Color Emoji"));
        assert!(!fallback_ok(Slot::Emoji, "DejaVu Sans"));
    }
}

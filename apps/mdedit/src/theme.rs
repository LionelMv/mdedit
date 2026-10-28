//! Light/dark/system theme resolution and the editor `Theme` with user size overrides.

use mdedit_style::Theme;
use mdedit_workspace::Config;

/// Whether the UI should be dark for the configured preference.
pub fn want_dark(pref: &str, system: Option<egui::Theme>) -> bool {
    match pref {
        "dark" => true,
        "light" => false,
        _ => system == Some(egui::Theme::Dark),
    }
}

/// `Theme::light()/dark()` with the sizes from the config (clamped to sane ranges).
pub fn build_theme(dark: bool, cfg: &Config) -> Theme {
    let mut t = if dark { Theme::dark() } else { Theme::light() };
    t.body_size = sane(cfg.body_size, 8.0, 48.0, t.body_size);
    t.code_size = sane(cfg.code_size, 8.0, 48.0, t.code_size);
    t.line_height = sane(cfg.line_height, 1.0, 3.0, t.line_height);
    // The app always registers the bold/italic families (fonts.rs), so the style crate may
    // use them; the name changes so editor caches keyed on it don't mix the two.
    t.font_families = true;
    t.name = format!("{}+fonts", t.name);
    t.max_width = if cfg.max_width.is_finite() && cfg.max_width <= 0.0 { 0.0 } else { sane(cfg.max_width, 300.0, 4000.0, t.max_width) };
    t
}

fn sane(v: f32, lo: f32, hi: f32, default: f32) -> f32 {
    if v.is_finite() { v.clamp(lo, hi) } else { default }
}

/// Tracks the applied theme so we only rebuild/invalidate on change.
#[derive(Default)]
pub struct ThemeState {
    pub theme: Option<Theme>,
    applied_pref: Option<egui::ThemePreference>,
}

impl ThemeState {
    /// Resolve for this frame. Returns `true` when the editor theme changed (invalidate caches).
    pub fn update(&mut self, ctx: &egui::Context, cfg: &Config) -> bool {
        let pref = match cfg.theme.as_str() {
            "dark" => egui::ThemePreference::Dark,
            "light" => egui::ThemePreference::Light,
            _ => egui::ThemePreference::System,
        };
        if self.applied_pref != Some(pref) {
            ctx.set_theme(pref);
            self.applied_pref = Some(pref);
        }
        // egui resolves "system" itself (falling back to dark when the platform doesn't say);
        // follow its decision so the editor matches the widget visuals.
        let dark = match pref {
            egui::ThemePreference::System => ctx.system_theme().map_or(ctx.theme() == egui::Theme::Dark, |t| t == egui::Theme::Dark),
            _ => want_dark(&cfg.theme, None),
        };
        let theme = build_theme(dark, cfg);
        if self.theme.as_ref() != Some(&theme) {
            self.theme = Some(theme);
            return true;
        }
        false
    }

    pub fn current(&self) -> Theme {
        self.theme.clone().unwrap_or_else(Theme::light)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preference_resolution() {
        assert!(want_dark("dark", Some(egui::Theme::Light)));
        assert!(!want_dark("light", Some(egui::Theme::Dark)));
        assert!(want_dark("system", Some(egui::Theme::Dark)));
        assert!(!want_dark("system", None));
        assert!(!want_dark("bogus", Some(egui::Theme::Light)));
    }

    #[test]
    fn overrides_are_clamped() {
        let cfg = Config { body_size: 200.0, code_size: f32::NAN, line_height: 0.1, max_width: 0.0, ..Config::default() };
        let t = build_theme(true, &cfg);
        assert!(t.dark);
        assert!(t.font_families);
        assert_eq!(t.name, "Dark+fonts");
        assert_eq!(t.body_size, 48.0);
        assert_eq!(t.code_size, Theme::dark().code_size);
        assert_eq!(t.line_height, 1.0);
        assert_eq!(t.max_width, 0.0);
    }
}

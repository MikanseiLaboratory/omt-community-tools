//! Application chrome colors for dark / light Studio Monitor themes.

use egui::Color32;
use suite_core::ThemePreference;

/// Resolved UI palette for the current theme preference.
#[derive(Debug, Clone, Copy)]
pub struct UiChrome {
    /// App / letterbox background.
    pub bg: Color32,
    /// Sidebar / stats panel background.
    pub panel: Color32,
    /// Slightly elevated surface (rows, buttons).
    pub surface: Color32,
    /// Selected / hover surface.
    pub surface_active: Color32,
    /// Hairline borders.
    pub border: Color32,
    /// Primary text.
    pub text: Color32,
    /// Secondary / muted text.
    pub text_muted: Color32,
    /// Accent (selection, live affordances).
    pub accent: Color32,
    /// Soft accent wash behind selected rows.
    pub accent_soft: Color32,
    /// Error / danger text.
    pub danger: Color32,
}

impl UiChrome {
    /// Resolve palette from preference + whether the OS/egui reports dark mode.
    pub fn resolve(pref: ThemePreference, system_dark: bool) -> Self {
        let dark = match pref {
            ThemePreference::Dark => true,
            ThemePreference::Light => false,
            ThemePreference::System => system_dark,
        };
        if dark { Self::dark() } else { Self::light() }
    }

    fn dark() -> Self {
        Self {
            bg: rgb(0x0e1116),
            panel: rgb(0x141a22),
            surface: rgb(0x1a222d),
            surface_active: rgb(0x243041),
            border: rgb(0x2a3340),
            text: rgb(0xedf2f7),
            text_muted: rgb(0x8b9aab),
            accent: rgb(0x3d9cf0),
            accent_soft: rgb(0x1a3048),
            danger: rgb(0xf46a6a),
        }
    }

    fn light() -> Self {
        Self {
            bg: rgb(0xf3f5f8),
            panel: rgb(0xffffff),
            surface: rgb(0xeef1f5),
            surface_active: rgb(0xe2e8f0),
            border: rgb(0xd0d7e0),
            text: rgb(0x1a2330),
            text_muted: rgb(0x5c6b7a),
            accent: rgb(0x1f6feb),
            accent_soft: rgb(0xdbe8f8),
            danger: rgb(0xc02b2b),
        }
    }

    /// Apply palette into egui visuals (keeps widget chrome close to the old look).
    pub fn apply_to_context(self, ctx: &egui::Context, dark: bool) {
        let mut visuals = if dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        visuals.panel_fill = self.panel;
        visuals.window_fill = self.panel;
        visuals.extreme_bg_color = self.bg;
        visuals.faint_bg_color = self.surface;
        visuals.widgets.noninteractive.bg_fill = self.surface;
        visuals.widgets.inactive.bg_fill = self.surface;
        visuals.widgets.hovered.bg_fill = self.surface_active;
        visuals.widgets.active.bg_fill = self.surface_active;
        visuals.widgets.noninteractive.fg_stroke.color = self.text;
        visuals.widgets.inactive.fg_stroke.color = self.text;
        visuals.widgets.hovered.fg_stroke.color = self.text;
        visuals.widgets.active.fg_stroke.color = self.text;
        visuals.selection.bg_fill = self.accent_soft;
        visuals.selection.stroke.color = self.accent;
        visuals.window_stroke = egui::Stroke::new(1.0, self.border);
        visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, self.border);
        ctx.set_visuals(visuals);
        // Match Test Patterns: UI chrome text should not be drag-selectable.
        // Slightly roomier default spacing so panels feel less cramped.
        ctx.all_styles_mut(|style| {
            style.interaction.selectable_labels = false;
            style.spacing.item_spacing = egui::vec2(10.0, 8.0);
            style.spacing.window_margin = egui::Margin::same(12);
        });
    }
}

fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb(
        ((hex >> 16) & 0xff) as u8,
        ((hex >> 8) & 0xff) as u8,
        (hex & 0xff) as u8,
    )
}

//! Clean feed output window: display discovery, target resolution, and drawing.
//!
//! The output is a second egui viewport that covers one display in borderless
//! fullscreen. The operator window keeps the controls; this module only knows
//! about plain display/snapshot data so it can be drawn from a deferred
//! viewport callback (which cannot borrow the application state).

use egui::{
    Align2, Color32, FontId, Painter, Pos2, Rect, Stroke, TextureId, Ui, Vec2, ViewportBuilder,
    ViewportClass, ViewportCommand, ViewportId, viewport::WindowLevel,
};
use omt_media::AudioLevels;
use parking_lot::Mutex;
use suite_core::{Language, OutputDisplayRef, OutputOverlays, t};
use winit::window::Window;

use crate::overlay;

/// A display reported by winit, in `available_monitors()` order.
///
/// The index is the value [`ViewportBuilder::with_monitor`] and
/// [`ViewportCommand::SetMonitor`] expect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DisplayInfo {
    /// Zero-based index in `available_monitors()` order.
    pub index: usize,
    /// OS-reported monitor name, if any.
    pub name: Option<String>,
    /// Current width in physical pixels.
    pub width: u32,
    /// Current height in physical pixels.
    pub height: u32,
    /// Current refresh rate in millihertz, if known.
    pub refresh_mhz: Option<u32>,
    /// Whether this is the OS primary display.
    pub is_primary: bool,
    /// Whether the operator window currently lives on this display.
    pub is_operator: bool,
}

impl DisplayInfo {
    /// Short label used in the toolbar and display rows.
    pub(crate) fn short_label(&self) -> String {
        format!("Display {}", self.index + 1)
    }

    /// Resolution and refresh rate, e.g. `1920×1080 · 60 Hz`.
    pub(crate) fn detail(&self) -> String {
        let mut text = format!("{}×{}", self.width, self.height);
        if let Some(mhz) = self.refresh_mhz {
            text.push_str(" · ");
            text.push_str(&format_refresh_hz(mhz));
        }
        text
    }
}

/// Describe the displays attached to `window`.
///
/// Returns the displays in `available_monitors()` order and the index of the
/// display the window currently occupies (if the OS reports it).
pub(crate) fn scan_displays(window: &Window) -> (Vec<DisplayInfo>, Option<usize>) {
    let current = window.current_monitor();
    let primary = window.primary_monitor();
    let mut operator = None;
    let mut displays = Vec::new();

    for (index, monitor) in window.available_monitors().enumerate() {
        let is_operator = current.as_ref() == Some(&monitor);
        if is_operator {
            operator = Some(index);
        }
        let size = monitor.size();
        let name = monitor
            .name()
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty());
        displays.push(DisplayInfo {
            index,
            name,
            width: size.width,
            height: size.height,
            refresh_mhz: monitor.refresh_rate_millihertz(),
            is_primary: primary.as_ref() == Some(&monitor),
            is_operator,
        });
    }

    (displays, operator)
}

/// Why an output window could not be started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutputError {
    /// Fewer than two usable displays (or the only display runs the app).
    NoSecondDisplay,
    /// The chosen display is the one running the operator window.
    SameAsOperator,
    /// The saved display disappeared.
    Missing,
}

impl OutputError {
    /// Localised explanation for the settings panel / toolbar tooltip.
    pub(crate) fn message(self, language: Language) -> &'static str {
        t(
            language,
            match self {
                Self::NoSecondDisplay => "monitor.output_no_second_display",
                Self::SameAsOperator => "monitor.output_same_display",
                Self::Missing => "monitor.output_missing_display",
            },
        )
    }
}

/// Resolve the display index to put the output window on.
///
/// A saved display name wins over its index (monitors can be reordered), then
/// the saved index, then automatic selection: the first display that is not
/// running the operator window.
pub(crate) fn resolve_target(
    displays: &[DisplayInfo],
    preferred: Option<&OutputDisplayRef>,
) -> Result<usize, OutputError> {
    if displays.is_empty() {
        return Err(OutputError::NoSecondDisplay);
    }

    if let Some(preferred) = preferred {
        let by_name = preferred.name.as_deref().and_then(|name| {
            displays
                .iter()
                .position(|display| display.name.as_deref() == Some(name))
        });
        let by_index = usize::try_from(preferred.index)
            .ok()
            .filter(|index| *index < displays.len());

        if let Some(index) = by_name.or(by_index) {
            if displays[index].is_operator {
                // The saved display is now showing the operator window. Use
                // another display instead of failing, so enabling the clean
                // feed still works after a window move.
                return displays
                    .iter()
                    .position(|display| !display.is_operator)
                    .ok_or(OutputError::SameAsOperator);
            }
            return Ok(index);
        }
        // The saved display is gone: fall through to automatic selection so a
        // replaced projector still produces a picture.
    }

    if displays.iter().any(|display| display.is_operator) {
        return displays
            .iter()
            .position(|display| !display.is_operator)
            .ok_or(OutputError::NoSecondDisplay);
    }

    // The OS did not tell us where the operator window is. Automatic selection
    // could cover the controls, so only pick a display when there are clearly
    // at least two and prefer the non-primary one (the window usually starts
    // on the primary display).
    if displays.len() >= 2 {
        let candidate = if displays[0].is_primary { 1 } else { 0 };
        return Ok(candidate);
    }
    Err(OutputError::NoSecondDisplay)
}

/// One video texture ready to paint on the output.
#[derive(Debug, Clone, Copy)]
pub(crate) struct OutputVideo {
    /// Decoded frame texture (shared with the operator preview).
    pub texture: TextureId,
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
}

/// Connection state shown by the optional tally and warning overlays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum OutputStatus {
    /// No source selected / stopped.
    #[default]
    Idle,
    /// Connecting to a source.
    Connecting,
    /// Frames are arriving.
    Live,
    /// Frames stopped arriving.
    Stalled,
    /// The receiver is reconnecting.
    Reconnecting,
}

impl OutputStatus {
    /// Tally border colour for this state (`None` = no border).
    pub(crate) fn tally_color(self) -> Option<Color32> {
        match self {
            Self::Live => Some(Color32::from_rgb(0xe5, 0x39, 0x35)),
            Self::Connecting | Self::Stalled | Self::Reconnecting => {
                Some(Color32::from_rgb(0xf0, 0xa0, 0x20))
            }
            Self::Idle => None,
        }
    }

    /// Warning text key, or `None` when the state needs no warning.
    pub(crate) fn warning_key(self) -> Option<&'static str> {
        match self {
            Self::Idle => Some("monitor.output_no_signal"),
            Self::Connecting => Some("monitor.output_connecting"),
            Self::Live => None,
            Self::Stalled => Some("monitor.stalled"),
            Self::Reconnecting => Some("monitor.output_reconnecting"),
        }
    }
}

/// Everything the deferred viewport callback needs to draw one frame.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct OutputSnapshot {
    /// Current video texture, if any.
    pub video: Option<OutputVideo>,
    /// Audio levels for the optional VU meter.
    pub audio: AudioLevels,
    /// Connection state for tally / warnings.
    pub status: OutputStatus,
    /// Localised warning text (empty when the state needs no warning).
    pub status_text: &'static str,
    /// UI language for the embedded-window title.
    pub language: Language,
    /// Overlay toggles for this output.
    pub overlays: OutputOverlays,
    /// Draw the mouse cursor on the clean feed window.
    pub show_cursor: bool,
    /// Set by the output viewport (Esc / F9 / close) and consumed by the app.
    pub close_requested: bool,
}

impl OutputSnapshot {
    /// Consume a pending close request.
    pub(crate) fn take_close_request(&mut self) -> bool {
        std::mem::take(&mut self.close_requested)
    }
}

/// Stable id of the output viewport.
pub(crate) fn viewport_id() -> ViewportId {
    ViewportId::from_hash_of("omt-studio-monitor-output")
}

/// Viewport attributes for the clean feed window.
pub(crate) fn viewport_builder(
    title: &str,
    monitor: usize,
    always_on_top: bool,
) -> ViewportBuilder {
    let level = if always_on_top {
        WindowLevel::AlwaysOnTop
    } else {
        WindowLevel::Normal
    };
    ViewportBuilder::default()
        .with_title(title)
        .with_monitor(monitor)
        .with_decorations(false)
        .with_resizable(false)
        .with_active(false)
        .with_taskbar(false)
        .with_has_shadow(false)
        .with_window_level(level)
}

/// Draw the output viewport.
///
/// Called from `Context::show_viewport_deferred`; the callback gets the full
/// content area as its `ui`.
pub(crate) fn draw_viewport(ui: &mut Ui, class: ViewportClass, shared: &Mutex<OutputSnapshot>) {
    let ctx = ui.ctx().clone();
    // Copy the snapshot so no lock is held while egui runs the pass.
    let snapshot = *shared.lock();

    if class == ViewportClass::Deferred {
        // Idempotent in winit: the OS cursor is only touched when the flag
        // changes.
        ctx.send_viewport_cmd(ViewportCommand::CursorVisible(snapshot.show_cursor));
        let close = ctx.input(|input| {
            input.key_pressed(egui::Key::Escape)
                || input.key_pressed(egui::Key::F9)
                || input.pointer.button_clicked(egui::PointerButton::Primary)
                || input.pointer.button_clicked(egui::PointerButton::Secondary)
                || input.viewport().close_requested()
        });
        if close {
            // Hide immediately so a click on a fullscreen clean feed always
            // reveals the operator window, even if the root pass is lagging.
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
            shared.lock().close_requested = true;
            ctx.request_repaint_of(ViewportId::ROOT);
        }
    }

    if class == ViewportClass::EmbeddedWindow {
        // Fallback for integrations without real viewports: show the same
        // picture in a normal window inside the operator viewport.
        let title = t(snapshot.language, "monitor.output_window_title");
        let mut open = true;
        egui::Window::new(title)
            .id(egui::Id::new("omt-output-embedded"))
            .open(&mut open)
            .collapsible(false)
            .default_size(Vec2::new(1280.0, 720.0))
            .show(&ctx, |ui| draw_contents(ui, &snapshot));
        if !open {
            shared.lock().close_requested = true;
            ctx.request_repaint_of(ViewportId::ROOT);
        }
        return;
    }

    draw_contents(ui, &snapshot);
}

/// Paint the picture area: black background, letterboxed video, overlays.
fn draw_contents(ui: &Ui, snapshot: &OutputSnapshot) {
    let full = ui.max_rect();
    let painter = ui.painter();
    painter.rect_filled(full, 0.0, Color32::BLACK);

    let scale = (full.height() / 1080.0).clamp(0.75, 3.0);

    if let Some(video) = snapshot.video {
        let video_rect = fit_rect(full, video.width, video.height);
        painter.image(
            video.texture,
            video_rect,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
        if snapshot.overlays.safe_area {
            overlay::paint_safe_area(painter, video_rect, full);
        }
        if snapshot.overlays.vu {
            paint_output_vu(ui, video_rect, full, snapshot.audio, scale);
        }
    }

    if snapshot.overlays.tally {
        paint_tally(painter, full, snapshot.status, snapshot.language, scale);
    }
    if snapshot.overlays.warnings
        && snapshot.status != OutputStatus::Live
        && !snapshot.status_text.is_empty()
    {
        paint_warning(painter, full, snapshot.status_text, scale);
    }
}

/// Fit `width × height` inside `bounds`, preserving the aspect ratio.
pub(crate) fn fit_rect(bounds: Rect, width: u32, height: u32) -> Rect {
    if width == 0 || height == 0 || bounds.width() <= 0.0 || bounds.height() <= 0.0 {
        return bounds;
    }
    let aspect = width as f32 / height as f32;
    let bounds_aspect = bounds.width() / bounds.height();
    let (w, h) = if bounds_aspect > aspect {
        (bounds.height() * aspect, bounds.height())
    } else {
        (bounds.width(), bounds.width() / aspect)
    };
    Rect::from_center_size(bounds.center(), Vec2::new(w.max(1.0), h.max(1.0)))
}

fn paint_output_vu(ui: &Ui, video_rect: Rect, clip: Rect, levels: AudioLevels, scale: f32) {
    let bar_w = (14.0 * scale).clamp(10.0, 40.0);
    let gap = (5.0 * scale).clamp(4.0, 14.0);
    let height = video_rect.height() * 0.5;
    let margin = (18.0 * scale).clamp(12.0, 48.0);
    let left = video_rect.max.x - bar_w * 2.0 - gap - margin;
    let top = video_rect.min.y + (video_rect.height() - height) * 0.5;

    let painter = ui.painter().with_clip_rect(clip);
    let pad = 8.0 * scale;
    let panel = Rect::from_min_max(
        Pos2::new(left - pad, top - pad * 2.0),
        Pos2::new(video_rect.max.x - margin + pad, top + height + pad),
    );
    painter.rect_filled(panel, 4.0, Color32::from_black_alpha(150));

    overlay::draw_meter(&painter, Pos2::new(left, top), bar_w, gap, height, levels);

    let font = FontId::proportional((11.0 * scale).clamp(10.0, 20.0));
    let label_color = Color32::from_gray(220);
    painter.text(
        Pos2::new(left + bar_w * 0.5, top - 4.0 * scale),
        Align2::CENTER_BOTTOM,
        "L",
        font.clone(),
        label_color,
    );
    painter.text(
        Pos2::new(left + bar_w + gap + bar_w * 0.5, top - 4.0 * scale),
        Align2::CENTER_BOTTOM,
        "R",
        font,
        label_color,
    );
}

fn paint_tally(
    painter: &Painter,
    full: Rect,
    status: OutputStatus,
    language: Language,
    scale: f32,
) {
    let Some(color) = status.tally_color() else {
        return;
    };
    let width = (full.height() * 0.006).clamp(4.0, 12.0);
    painter.rect_stroke(
        full.shrink(width * 0.5),
        0.0,
        Stroke::new(width, color),
        egui::StrokeKind::Inside,
    );

    let font = FontId::proportional((full.height() * 0.018).clamp(14.0, 30.0));
    let label = t(language, "monitor.output_pgm");
    let pos = full.min + Vec2::splat((24.0 * scale).clamp(12.0, 64.0));
    let badge = Rect::from_min_size(pos, Vec2::new(font.size * 3.1, font.size * 1.85));
    painter.rect_filled(badge, 4.0, color);
    painter.text(
        badge.center(),
        Align2::CENTER_CENTER,
        label,
        font,
        Color32::WHITE,
    );
}

fn paint_warning(painter: &Painter, full: Rect, text: &str, scale: f32) {
    let font = FontId::proportional((full.height() * 0.032).clamp(18.0, 52.0));
    let galley = painter.layout_no_wrap(text.to_owned(), font, Color32::WHITE);
    let padding = Vec2::new(24.0 * scale, 14.0 * scale);
    let panel = Rect::from_center_size(full.center(), galley.size() + padding * 2.0);
    painter.rect_filled(panel, 6.0, Color32::from_rgb(0x1b, 0x1f, 0x25));
    painter.rect_stroke(
        panel,
        6.0,
        Stroke::new(1.0, Color32::from_rgb(0x3a, 0x44, 0x52)),
        egui::StrokeKind::Inside,
    );
    painter.galley(panel.center() - galley.size() * 0.5, galley, Color32::WHITE);
}

fn format_refresh_hz(mhz: u32) -> String {
    if mhz.is_multiple_of(1000) {
        format!("{} Hz", mhz / 1000)
    } else {
        format!("{:.1} Hz", mhz as f32 / 1000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn display(index: usize, is_operator: bool) -> DisplayInfo {
        DisplayInfo {
            index,
            name: Some(format!("Display {index}")),
            width: 1920,
            height: 1080,
            refresh_mhz: Some(60_000),
            is_primary: index == 0,
            is_operator,
        }
    }

    #[test]
    fn resolves_saved_name_before_index() {
        let mut displays = vec![display(0, true), display(1, false), display(2, false)];
        displays[2].name = Some("Projector".into());
        let preferred = OutputDisplayRef {
            index: 1,
            name: Some("Projector".into()),
        };
        assert_eq!(
            resolve_target(&displays, Some(&preferred)),
            Ok(2),
            "name should win when monitors were reordered"
        );
    }

    #[test]
    fn saved_operator_display_falls_back() {
        let displays = vec![display(0, true), display(1, false)];
        let preferred = OutputDisplayRef {
            index: 0,
            name: None,
        };
        assert_eq!(resolve_target(&displays, Some(&preferred)), Ok(1));
    }

    #[test]
    fn saved_operator_display_errors_without_alternative() {
        let displays = vec![display(0, true)];
        let preferred = OutputDisplayRef {
            index: 0,
            name: None,
        };
        assert_eq!(
            resolve_target(&displays, Some(&preferred)),
            Err(OutputError::SameAsOperator)
        );
    }

    #[test]
    fn saved_operator_display_matched_by_name_falls_back() {
        let mut displays = vec![display(0, true), display(1, false)];
        displays[0].name = Some("Built-in".into());
        let preferred = OutputDisplayRef {
            index: 1,
            name: Some("Built-in".into()),
        };
        assert_eq!(resolve_target(&displays, Some(&preferred)), Ok(1));
    }

    #[test]
    fn automatic_skips_operator_display() {
        let displays = vec![display(0, true), display(1, false), display(2, false)];
        assert_eq!(resolve_target(&displays, None), Ok(1));
    }

    #[test]
    fn automatic_errors_with_single_display() {
        let displays = vec![display(0, true)];
        assert_eq!(
            resolve_target(&displays, None),
            Err(OutputError::NoSecondDisplay)
        );
        assert_eq!(resolve_target(&[], None), Err(OutputError::NoSecondDisplay));
    }

    #[test]
    fn missing_saved_display_falls_back_to_automatic() {
        let displays = vec![display(0, true), display(1, false)];
        let preferred = OutputDisplayRef {
            index: 7,
            name: Some("Gone".into()),
        };
        assert_eq!(resolve_target(&displays, Some(&preferred)), Ok(1));
    }

    #[test]
    fn unknown_operator_prefers_non_primary_display() {
        let mut displays = vec![display(0, false), display(1, false)];
        displays[0].is_primary = true;
        assert_eq!(resolve_target(&displays, None), Ok(1));
    }

    #[test]
    fn fits_picture_and_centers_it() {
        let bounds = Rect::from_min_size(Pos2::ZERO, Vec2::new(1920.0, 1080.0));
        let rect = fit_rect(bounds, 1920, 1080);
        assert!((rect.width() - 1920.0).abs() < 0.01);
        assert!((rect.height() - 1080.0).abs() < 0.01);
        assert!(rect.center().distance(bounds.center()) < 0.01);

        // 4:3 picture inside a 16:9 screen gets pillarboxes.
        let rect = fit_rect(bounds, 1024, 768);
        assert!((rect.height() - 1080.0).abs() < 0.01);
        assert!((rect.width() - 1440.0).abs() < 0.01);
        assert!(rect.center().distance(bounds.center()) < 0.01);

        // 16:9 picture inside a 4:3 screen gets letterboxes.
        let bounds = Rect::from_min_size(Pos2::ZERO, Vec2::new(1024.0, 768.0));
        let rect = fit_rect(bounds, 1920, 1080);
        assert!((rect.width() - 1024.0).abs() < 0.01);
        assert!((rect.height() - 576.0).abs() < 0.01);
    }

    #[test]
    fn tally_colors_follow_status() {
        assert_eq!(OutputStatus::Idle.tally_color(), None);
        assert_eq!(
            OutputStatus::Live.tally_color(),
            Some(Color32::from_rgb(0xe5, 0x39, 0x35))
        );
        assert!(OutputStatus::Stalled.tally_color().is_some());
        assert_eq!(OutputStatus::Live.warning_key(), None);
        assert_eq!(
            OutputStatus::Idle.warning_key(),
            Some("monitor.output_no_signal")
        );
        assert_eq!(
            OutputStatus::Connecting.warning_key(),
            Some("monitor.output_connecting")
        );
        assert_eq!(
            OutputStatus::Reconnecting.warning_key(),
            Some("monitor.output_reconnecting")
        );
        assert_eq!(OutputStatus::Stalled.warning_key(), Some("monitor.stalled"));
    }

    #[test]
    fn display_detail_includes_refresh() {
        assert_eq!(display(1, false).detail(), "1920×1080 · 60 Hz");
        let mut odd = display(1, false);
        odd.refresh_mhz = Some(59_940);
        assert_eq!(odd.detail(), "1920×1080 · 59.9 Hz");
    }

    #[test]
    fn draws_without_panicking() {
        let snapshot = OutputSnapshot {
            video: Some(OutputVideo {
                texture: TextureId::Managed(0),
                width: 1920,
                height: 1080,
            }),
            audio: AudioLevels {
                peak_l: 0.5,
                peak_r: 0.25,
                ..Default::default()
            },
            status: OutputStatus::Stalled,
            status_text: "SIGNAL LOST",
            language: Language::English,
            overlays: OutputOverlays {
                vu: true,
                safe_area: true,
                tally: true,
                warnings: true,
            },
            show_cursor: false,
            close_requested: false,
        };
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1920.0, 1080.0))),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| draw_contents(ui, &snapshot));
        assert!(!output.shapes.is_empty());
        // Font atlas deltas belong to a renderer; this test has none.
        output.textures_delta.clear();
    }
}

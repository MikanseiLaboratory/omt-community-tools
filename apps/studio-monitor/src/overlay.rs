//! Picture overlays shared by the operator preview and the program output.
//!
//! The preview draws compact guides; the output window draws the same guides
//! larger. Keeping the geometry and colours here stops the two views from
//! drifting apart.

use egui::{Color32, Painter, Pos2, Rect, Vec2};
use omt_media::AudioLevels;

/// SMPTE ST 2046-1 action-safe fraction of the picture.
pub(crate) const ACTION_SAFE_FRAC: f32 = 0.93;
/// SMPTE ST 2046-1 title-safe fraction of the picture.
pub(crate) const TITLE_SAFE_FRAC: f32 = 0.90;

/// Vertical level-meter geometry.
#[derive(Debug, Clone, Copy)]
pub(crate) struct VuStyle {
    /// Width of one channel bar in points.
    pub bar_w: f32,
    /// Gap between the L and R bars in points.
    pub gap: f32,
    /// Meter height as a fraction of the picture height.
    pub height_frac: f32,
    /// Distance from the picture's right edge in points.
    pub margin: f32,
}

/// Compact meter used by the operator preview.
pub(crate) const PREVIEW_VU: VuStyle = VuStyle {
    bar_w: 10.0,
    gap: 4.0,
    height_frac: 0.6,
    margin: 12.0,
};

/// Draw the action/title safe guides over `video_rect`, clipped to `clip`.
pub(crate) fn paint_safe_area(painter: &Painter, video_rect: Rect, clip: Rect) {
    let painter = painter.with_clip_rect(clip);
    for (frac, color) in [
        (ACTION_SAFE_FRAC, Color32::from_rgb(0xff, 0xff, 0xff)),
        (TITLE_SAFE_FRAC, Color32::from_rgb(0xff, 0xeb, 0x3b)),
    ] {
        let w = video_rect.width() * frac;
        let h = video_rect.height() * frac;
        let r = Rect::from_center_size(video_rect.center(), Vec2::new(w, h));
        painter.rect_stroke(
            r,
            0.0,
            egui::Stroke::new(1.0, color),
            egui::StrokeKind::Outside,
        );
    }
}

/// Draw a stereo level meter at the right edge of `video_rect`.
pub(crate) fn paint_vu(
    painter: &Painter,
    video_rect: Rect,
    clip: Rect,
    levels: AudioLevels,
    style: VuStyle,
) {
    let painter = painter.with_clip_rect(clip);
    draw_meter(
        &painter,
        meter_origin(video_rect, style),
        style.bar_w,
        style.gap,
        meter_height(video_rect, style),
        levels,
    );
}

/// Position of the L bar for `style`.
pub(crate) fn meter_origin(video_rect: Rect, style: VuStyle) -> Pos2 {
    let height = video_rect.height() * style.height_frac;
    let top = video_rect.min.y + (video_rect.height() - height) * 0.5;
    let left = video_rect.max.x - style.bar_w * 2.0 - style.gap - style.margin;
    Pos2::new(left, top)
}

/// Meter height for `style` and the picture it is drawn over.
pub(crate) fn meter_height(video_rect: Rect, style: VuStyle) -> f32 {
    video_rect.height() * style.height_frac
}

/// Draw the L/R bars starting at `origin`.
pub(crate) fn draw_meter(
    painter: &Painter,
    origin: Pos2,
    bar_w: f32,
    gap: f32,
    height: f32,
    levels: AudioLevels,
) {
    let l = peak_to_meter(levels.peak_l);
    let r = peak_to_meter(levels.peak_r);
    paint_vu_bar(painter, origin, bar_w, height, l);
    paint_vu_bar(
        painter,
        Pos2::new(origin.x + bar_w + gap, origin.y),
        bar_w,
        height,
        r,
    );
}

/// Draw one meter bar with its own background.
pub(crate) fn paint_vu_bar(painter: &Painter, origin: Pos2, width: f32, full_h: f32, level: f32) {
    let bg = Rect::from_min_size(origin, Vec2::new(width, full_h));
    painter.rect_filled(bg, 2.0, Color32::from_rgb(0x1b, 0x22, 0x2c));
    let h = (full_h * level).max(2.0);
    let db = -60.0 + level * 60.0;
    let color = if db > -3.0 {
        Color32::from_rgb(0xf4, 0x43, 0x36)
    } else if db > -9.0 {
        Color32::from_rgb(0xff, 0xeb, 0x3b)
    } else {
        Color32::from_rgb(0x4c, 0xaf, 0x50)
    };
    let fill = Rect::from_min_max(
        Pos2::new(origin.x, origin.y + full_h - h),
        Pos2::new(origin.x + width, origin.y + full_h),
    );
    painter.rect_filled(fill, 2.0, color);
}

/// Map a linear peak to a 0..=1 meter position with a −60 dB floor.
pub(crate) fn peak_to_meter(peak: f32) -> f32 {
    const FLOOR_DB: f32 = -60.0;
    if peak <= 1e-6 {
        return 0.0;
    }
    let db = (20.0 * peak.log10()).clamp(FLOOR_DB, 0.0);
    ((db - FLOOR_DB) / -FLOOR_DB).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peak_meter_is_bounded() {
        assert_eq!(peak_to_meter(0.0), 0.0);
        assert_eq!(peak_to_meter(-1.0), 0.0);
        assert_eq!(peak_to_meter(1.0), 1.0);
        assert!((0.0..=1.0).contains(&peak_to_meter(0.5)));
    }
}

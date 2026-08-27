//! Windowed / fullscreen chrome panels and layout splitters.

use eframe::egui::{self, Color32, Context, CursorIcon, Pos2, Rect, RichText, Sense, Ui, Vec2};
use omt_media::{AudioLevels, BufferUnit, DiscoveredSource};
use suite_core::{Language, t};

use super::{
    ACTION_SAFE_FRAC, LAYOUT_GAP, LOG_MIN_H, MonitorApp, PREVIEW_MIN_W, SIDEBAR_MIN_W, STATS_MIN_W,
    TITLE_SAFE_FRAC, TOOLBAR_H, clamp_log_h, clamp_sidebar_w, clamp_stats_w,
};
use crate::chrome::UiChrome;

impl MonitorApp {
    pub(crate) fn ui_fullscreen(&mut self, ui: &mut egui::Ui, ctx: &Context, chrome: UiChrome) {
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(Color32::BLACK))
            .show(ui, |ui| {
                let full = ui.max_rect();
                self.preview_w = full.width();
                self.preview_h = full.height();

                let resp = ui.interact(full, ui.id().with("fs-root"), Sense::click());
                if resp.clicked() || resp.secondary_clicked() {
                    self.exit_fullscreen(ctx);
                }

                let has_frame = self.texture.is_some() && self.frame_w > 0 && self.frame_h > 0;
                if has_frame {
                    let (dw, dh) = self.fit_display_in_viewport(full.width(), full.height());
                    let video_rect = Rect::from_center_size(full.center(), Vec2::new(dw, dh));
                    self.paint_video_stack(ui, chrome, video_rect, full);
                } else {
                    ui.painter().text(
                        full.center(),
                        egui::Align2::CENTER_CENTER,
                        t(self.language, "monitor.waiting"),
                        egui::FontId::proportional(16.0),
                        chrome.text_muted,
                    );
                }
            });
    }

    pub(crate) fn ui_windowed(&mut self, ui: &mut egui::Ui, chrome: UiChrome) {
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(chrome.bg))
            .show(ui, |ui| {
                let gap = LAYOUT_GAP;
                let total = ui.available_rect_before_wrap().shrink(gap);
                let log_h = clamp_log_h(self.log_h).min((total.height() * 0.45).max(LOG_MIN_H));
                let top_h = (total.height() - log_h - gap).max(100.0);

                let top = Rect::from_min_size(total.min, Vec2::new(total.width(), top_h));
                let bottom = Rect::from_min_size(
                    Pos2::new(total.min.x, total.min.y + top_h + gap),
                    Vec2::new(total.width(), log_h),
                );

                let sidebar_w = fit_sidebar_w(self.sidebar_w, top.width(), self.stats_w, gap);
                let stats_w = fit_stats_w(self.stats_w, top.width(), sidebar_w, gap);
                let sidebar = Rect::from_min_size(top.min, Vec2::new(sidebar_w, top.height()));
                let stats = Rect::from_min_max(Pos2::new(top.max.x - stats_w, top.min.y), top.max);
                let preview_col = Rect::from_min_max(
                    Pos2::new(sidebar.max.x + gap, top.min.y),
                    Pos2::new(stats.min.x - gap, top.max.y),
                );
                let toolbar =
                    Rect::from_min_size(preview_col.min, Vec2::new(preview_col.width(), TOOLBAR_H));
                let picture = Rect::from_min_max(
                    Pos2::new(preview_col.min.x, preview_col.min.y + TOOLBAR_H + gap),
                    preview_col.max,
                );

                // Layer 0 (bottom): video picture only — clipped so it never covers chrome.
                self.paint_picture_layer(ui, chrome, picture);

                // Layer 1+: chrome panels on top of video.
                ui.scope_builder(egui::UiBuilder::new().max_rect(sidebar), |ui| {
                    self.ui_sidebar(ui, chrome);
                });
                ui.scope_builder(egui::UiBuilder::new().max_rect(toolbar), |ui| {
                    self.ui_preview_toolbar(ui, chrome);
                });
                ui.scope_builder(egui::UiBuilder::new().max_rect(stats), |ui| {
                    self.ui_stats(ui, chrome);
                });
                ui.scope_builder(egui::UiBuilder::new().max_rect(bottom), |ui| {
                    self.ui_log(ui, chrome);
                });

                let sidebar_split = Rect::from_min_max(
                    Pos2::new(sidebar.max.x, top.min.y),
                    Pos2::new(sidebar.max.x + gap, top.max.y),
                );
                let stats_split = Rect::from_min_max(
                    Pos2::new(stats.min.x - gap, top.min.y),
                    Pos2::new(stats.min.x, top.max.y),
                );
                let log_split = Rect::from_min_max(
                    Pos2::new(top.min.x, top.max.y),
                    Pos2::new(top.max.x, bottom.min.y),
                );

                let mut persist = false;
                let (dx, stopped) = interact_h_split(ui, "split-sidebar", sidebar_split);
                if dx.abs() > 0.0 {
                    self.sidebar_w = clamp_sidebar_w(self.sidebar_w + dx);
                }
                persist |= stopped;
                let (dx, stopped) = interact_h_split(ui, "split-stats", stats_split);
                if dx.abs() > 0.0 {
                    self.stats_w = clamp_stats_w(self.stats_w - dx);
                }
                persist |= stopped;
                let (dy, stopped) = interact_v_split(ui, "split-log", log_split);
                if dy.abs() > 0.0 {
                    self.log_h = clamp_log_h(self.log_h - dy);
                }
                persist |= stopped;
                if persist {
                    self.persist_monitor_layout();
                }
            });
    }

    fn paint_picture_layer(&mut self, ui: &mut Ui, chrome: UiChrome, picture: Rect) {
        // Card chrome so the gap against neighboring panels is visible.
        let radius = 6.0;
        ui.painter().rect_filled(picture, radius, chrome.panel);
        ui.painter().rect_stroke(
            picture,
            radius,
            egui::Stroke::new(1.0, chrome.border),
            egui::StrokeKind::Inside,
        );
        // Inset the interactive / video region so content isn't flush to the card edge.
        let inset = 8.0;
        let content = picture.shrink(inset);
        self.preview_w = content.width();
        self.preview_h = content.height();

        let resp = ui.interact(content, ui.id().with("preview"), Sense::click_and_drag());
        if resp.double_clicked() {
            self.enter_fullscreen(ui.ctx());
        }
        if resp.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll.abs() > f32::EPSILON {
                self.adjust_zoom(scroll);
            }
        }

        let middle_down = ui.input(|i| i.pointer.button_down(egui::PointerButton::Middle));
        if middle_down {
            if let Some(pos) = ui.input(|i| i.pointer.latest_pos()) {
                if let Some(prev) = self.pan_drag {
                    self.pan_x += pos.x - prev.x;
                    self.pan_y += pos.y - prev.y;
                }
                self.pan_drag = Some(pos);
                ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
            }
        } else {
            self.pan_drag = None;
        }

        let has_frame = self.texture.is_some() && self.frame_w > 0 && self.frame_h > 0;
        if has_frame {
            let (dw, dh) = self.display_size();
            let origin = content.min + Vec2::new(self.pan_x, self.pan_y);
            let video_rect = Rect::from_min_size(origin, Vec2::new(dw, dh));
            self.paint_video_stack(ui, chrome, video_rect, content);
        } else {
            ui.painter().text(
                content.center(),
                egui::Align2::CENTER_CENTER,
                t(self.language, "monitor.waiting"),
                egui::FontId::proportional(16.0),
                chrome.text_muted,
            );
        }
    }

    fn ui_preview_toolbar(&mut self, ui: &mut Ui, chrome: UiChrome) {
        egui::Frame::NONE
            .fill(chrome.panel)
            .stroke(egui::Stroke::new(1.0, chrome.border))
            .corner_radius(6.0)
            .inner_margin(egui::Margin::symmetric(16, 12))
            .show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    ui.label(RichText::new(&self.stall_text).color(chrome.text));
                    if self.audio_unavailable() {
                        ui.label(
                            RichText::new(t(self.language, "monitor.audio_unavailable"))
                                .color(chrome.text),
                        );
                    }
                    ui.label(
                        RichText::new(format!("{:.0}%", self.zoom * 100.0)).color(chrome.text),
                    );
                    if chip(ui, chrome, t(self.language, "monitor.zoom_reset"), false) {
                        self.zoom_reset();
                    }
                    if chip(ui, chrome, t(self.language, "monitor.fullscreen"), false) {
                        self.enter_fullscreen(ui.ctx());
                    }
                    ui.label(
                        RichText::new(
                            "wheel = zoom · middle-drag = pan · F11 / double-click = fullscreen",
                        )
                        .small()
                        .color(chrome.text_muted),
                    );
                });
            });
    }

    fn ui_sidebar(&mut self, ui: &mut Ui, chrome: UiChrome) {
        egui::Frame::NONE
            .fill(chrome.panel)
            .stroke(egui::Stroke::new(1.0, chrome.border))
            .corner_radius(6.0)
            .inner_margin(egui::Margin::symmetric(14, 14))
            .show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(t(self.language, "monitor.sources"))
                            .strong()
                            .color(chrome.text),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if chip(ui, chrome, t(self.language, "monitor.refresh"), false) {
                            self.request_refresh(false);
                        }
                    });
                });
                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                egui::ScrollArea::vertical()
                    .id_salt("monitor_sources")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let none_sel = self.selected.is_none();
                        if source_row(ui, chrome, t(self.language, "monitor.none"), "", none_sel) {
                            self.disconnect_source();
                        }

                        if self.discovered.is_empty() {
                            ui.add_space(12.0);
                            ui.label(
                                RichText::new(t(self.language, "monitor.no_sources"))
                                    .color(chrome.text_muted)
                                    .small(),
                            );
                        } else {
                            let mut hosts: Vec<(String, Vec<DiscoveredSource>)> = Vec::new();
                            for s in &self.discovered {
                                if let Some((_, list)) =
                                    hosts.iter_mut().find(|(h, _)| h == &s.host)
                                {
                                    list.push(s.clone());
                                } else {
                                    hosts.push((s.host.clone(), vec![s.clone()]));
                                }
                            }
                            for (host, sources) in hosts {
                                ui.add_space(16.0);
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(host.to_uppercase())
                                            .size(11.0)
                                            .strong()
                                            .color(chrome.text_muted),
                                    );
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            ui.label(
                                                RichText::new(format!("{}", sources.len()))
                                                    .size(11.0)
                                                    .color(chrome.text_muted),
                                            );
                                        },
                                    );
                                });
                                ui.add_space(8.0);
                                for s in sources {
                                    let label = if s.source.is_empty() {
                                        s.name.clone()
                                    } else {
                                        s.source.clone()
                                    };
                                    let selected = self.selected.as_deref() == Some(s.url.as_str());
                                    if source_row(
                                        ui,
                                        chrome,
                                        &label,
                                        &format!(":{}", s.port),
                                        selected,
                                    ) {
                                        self.select(s.url.clone(), s.addresses.clone());
                                    }
                                }
                            }
                        }
                        ui.add_space(12.0);
                    });

                ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                    ui.add_space(12.0);
                    if prefs_button(
                        ui,
                        chrome,
                        t(self.language, "monitor.preferences"),
                        self.preferences_open,
                    ) {
                        self.open_preferences();
                    }
                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(4.0);
                });
            });
    }

    fn paint_video_stack(&mut self, ui: &mut Ui, _chrome: UiChrome, video_rect: Rect, clip: Rect) {
        let Some(tex) = &self.texture else {
            return;
        };
        if self.frame_w == 0 || self.frame_h == 0 {
            return;
        }
        let painter = ui.painter().with_clip_rect(clip);
        painter.image(
            tex.id(),
            video_rect,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
        if self.settings.safe_area {
            paint_safe_area_clipped(ui, video_rect, clip);
        }
        if self.settings.vu_meter {
            paint_vu_clipped(ui, video_rect, clip, self.audio_levels);
        }
    }

    fn ui_stats(&mut self, ui: &mut Ui, chrome: UiChrome) {
        egui::Frame::NONE
            .fill(chrome.panel)
            .stroke(egui::Stroke::new(1.0, chrome.border))
            .corner_radius(6.0)
            .inner_margin(egui::Margin::symmetric(16, 14))
            .show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                egui::ScrollArea::vertical()
                    .id_salt("monitor_stats")
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 10.0;
                        let source_fps = if self.fps_d > 0 {
                            self.fps_n as f32 / self.fps_d as f32
                        } else {
                            0.0
                        };
                        let buffer_label = format_buffer_stats(
                            self.language,
                            &self.settings.buffer,
                            self.video_buffer_delay_ms,
                            self.audio_buffer_delay_ms,
                            self.buffer_fps().0,
                            self.buffer_fps().1,
                        );

                        if collapsing_label(
                            ui,
                            chrome,
                            t(self.language, "monitor.stats"),
                            self.stats_video_open,
                        ) {
                            self.stats_video_open = !self.stats_video_open;
                            self.persist_monitor_layout();
                        }
                        if self.stats_video_open {
                            stat_row(
                                ui,
                                chrome,
                                "Display FPS",
                                format!("{:.1}", self.display_fps),
                            );
                            stat_row(ui, chrome, "Source FPS", format!("{source_fps:.2}"));
                            stat_row(
                                ui,
                                chrome,
                                "Presented",
                                format!("{}", self.frames_presented),
                            );
                            stat_row(
                                ui,
                                chrome,
                                "Source dropped",
                                format!("{}", self.source_dropped),
                            );
                            stat_row(
                                ui,
                                chrome,
                                "Render skipped",
                                format!("{}", self.frames_render_skipped),
                            );
                            stat_row(ui, chrome, "Net dropped", format!("{}", self.net_dropped));
                            stat_row(ui, chrome, "Decoded", format!("{}", self.frames_decoded));
                            stat_row(
                                ui,
                                chrome,
                                "Decode ms",
                                format!(
                                    "{:.2} (peak {:.2})",
                                    self.decode_ms_avg, self.decode_ms_peak
                                ),
                            );
                            stat_row(
                                ui,
                                chrome,
                                "Wire queue",
                                format!("{}", self.wire_queue_depth),
                            );
                            stat_row(ui, chrome, "Reconnects", format!("{}", self.reconnects));
                            stat_row(ui, chrome, "Session", format!("{:?}", self.session_state));
                        }

                        ui.add_space(8.0);
                        if collapsing_label(
                            ui,
                            chrome,
                            t(self.language, "monitor.audio"),
                            self.stats_audio_open,
                        ) {
                            self.stats_audio_open = !self.stats_audio_open;
                            self.persist_monitor_layout();
                        }
                        if self.stats_audio_open {
                            stat_row(
                                ui,
                                chrome,
                                t(self.language, "monitor.audio_output"),
                                if self.audio_unavailable() {
                                    t(self.language, "monitor.audio_unavailable").to_string()
                                } else {
                                    self.audio_output_device.clone().unwrap_or_else(|| {
                                        t(self.language, "monitor.audio_default").to_string()
                                    })
                                },
                            );
                            stat_row(
                                ui,
                                chrome,
                                "Audio packets",
                                format!("{}", self.audio_frames),
                            );
                            stat_row(ui, chrome, "L", format_dbfs(self.audio_levels.peak_l));
                            stat_row(ui, chrome, "R", format_dbfs(self.audio_levels.peak_r));
                            stat_row(
                                ui,
                                chrome,
                                "Format",
                                if self.audio_levels.sample_rate > 0 {
                                    format!(
                                        "{} Hz / {} ch",
                                        self.audio_levels.sample_rate, self.audio_levels.channels
                                    )
                                } else {
                                    "-".into()
                                },
                            );
                            stat_row(
                                ui,
                                chrome,
                                t(self.language, "monitor.av_buffer"),
                                buffer_label,
                            );
                        }

                        ui.add_space(8.0);
                        if collapsing_label(
                            ui,
                            chrome,
                            t(self.language, "monitor.source_info"),
                            self.stats_source_open,
                        ) {
                            self.stats_source_open = !self.stats_source_open;
                            self.persist_monitor_layout();
                        }
                        if self.stats_source_open {
                            stat_row(
                                ui,
                                chrome,
                                "Resolution",
                                if self.frame_w > 0 {
                                    format!("{}×{}", self.frame_w, self.frame_h)
                                } else {
                                    "-".into()
                                },
                            );
                            stat_row(ui, chrome, "Bitrate", format_bitrate(self.bitrate_bps));
                            stat_row(ui, chrome, "Bytes RX", format_bytes(self.bytes_received));
                            stat_row(
                                ui,
                                chrome,
                                "URL",
                                self.selected.clone().unwrap_or_else(|| "-".into()),
                            );
                        }
                        ui.add_space(8.0);
                        stat_row(
                            ui,
                            chrome,
                            t(self.language, "simd"),
                            self.simd_summary.clone(),
                        );
                    });
            });
    }

    fn ui_log(&mut self, ui: &mut Ui, chrome: UiChrome) {
        egui::Frame::NONE
            .fill(chrome.panel)
            .stroke(egui::Stroke::new(1.0, chrome.border))
            .corner_radius(6.0)
            .inner_margin(egui::Margin::symmetric(14, 12))
            .show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(t(self.language, "monitor.log"))
                            .strong()
                            .color(chrome.text),
                    );
                    ui.add_space(8.0);
                    if chip(ui, chrome, t(self.language, "monitor.clear_log"), false) {
                        self.log_lines.clear();
                    }
                });
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(8.0);
                egui::ScrollArea::vertical()
                    .id_salt("monitor_log")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        if self.log_lines.is_empty() {
                            ui.label(
                                RichText::new("XML / metadata will appear here")
                                    .color(chrome.text_muted)
                                    .italics()
                                    .small(),
                            );
                        } else {
                            for line in &self.log_lines {
                                ui.monospace(RichText::new(line).color(chrome.text_muted).small());
                            }
                        }
                    });
            });
    }
}

fn fit_sidebar_w(width: f32, top_w: f32, stats_w: f32, gap: f32) -> f32 {
    let max = (top_w - clamp_stats_w(stats_w) - gap * 2.0 - PREVIEW_MIN_W).max(SIDEBAR_MIN_W);
    clamp_sidebar_w(width).min(max)
}

fn fit_stats_w(width: f32, top_w: f32, sidebar_w: f32, gap: f32) -> f32 {
    let max = (top_w - sidebar_w - gap * 2.0 - PREVIEW_MIN_W).max(STATS_MIN_W);
    clamp_stats_w(width).min(max)
}

fn interact_h_split(ui: &Ui, id: &'static str, hit: Rect) -> (f32, bool) {
    let resp = ui.interact(hit, ui.id().with(id), Sense::drag());
    if resp.hovered() || resp.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
    }
    (
        if resp.dragged() {
            resp.drag_delta().x
        } else {
            0.0
        },
        resp.drag_stopped(),
    )
}

fn interact_v_split(ui: &Ui, id: &'static str, hit: Rect) -> (f32, bool) {
    let resp = ui.interact(hit, ui.id().with(id), Sense::drag());
    if resp.hovered() || resp.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeVertical);
    }
    (
        if resp.dragged() {
            resp.drag_delta().y
        } else {
            0.0
        },
        resp.drag_stopped(),
    )
}

fn collapsing_label(ui: &mut Ui, chrome: UiChrome, title: &str, open: bool) -> bool {
    let chevron = if open { "▾" } else { "▸" };
    let resp = ui.add(
        egui::Label::new(
            RichText::new(format!("{chevron}  {title}"))
                .strong()
                .color(chrome.text),
        )
        .sense(Sense::click()),
    );
    if resp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    resp.clicked()
}

fn paint_gear(painter: &egui::Painter, center: Pos2, size: f32, color: Color32) {
    let stroke_w = (size * 0.12).clamp(1.4, 2.2);
    let r_ring = size * 0.28;
    painter.circle_stroke(center, r_ring, egui::Stroke::new(stroke_w, color));
    let tooth_inner = r_ring + stroke_w * 0.3;
    let tooth_outer = size * 0.46;
    for i in 0..8 {
        let a = i as f32 * std::f32::consts::TAU / 8.0;
        let (sin, cos) = a.sin_cos();
        let dir = Vec2::new(cos, sin);
        painter.line_segment(
            [center + dir * tooth_inner, center + dir * tooth_outer],
            egui::Stroke::new(stroke_w * 1.15, color),
        );
    }
}

fn prefs_button(ui: &mut Ui, chrome: UiChrome, label: &str, active: bool) -> bool {
    let height = 36.0;
    let (rect, resp) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::click());
    let hovered = resp.hovered();
    let fill = if active {
        chrome.accent_soft
    } else if hovered {
        chrome.surface_active
    } else {
        chrome.surface
    };
    let stroke = if active { chrome.accent } else { chrome.border };
    let text_color = if active { chrome.accent } else { chrome.text };
    let icon_color = if active || hovered {
        chrome.accent
    } else {
        chrome.text_muted
    };

    ui.painter().rect_filled(rect, 6.0, fill);
    ui.painter().rect_stroke(
        rect,
        6.0,
        egui::Stroke::new(1.0, stroke),
        egui::StrokeKind::Inside,
    );

    let icon_center = Pos2::new(rect.min.x + 18.0, rect.center().y);
    paint_gear(ui.painter(), icon_center, 18.0, icon_color);
    ui.painter().text(
        Pos2::new(rect.min.x + 34.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(13.0),
        text_color,
    );

    if hovered {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    resp.clicked()
}

fn chip(ui: &mut Ui, chrome: UiChrome, label: &str, active: bool) -> bool {
    let fill = if active {
        chrome.accent_soft
    } else {
        chrome.surface
    };
    let text = if active { chrome.accent } else { chrome.text };
    ui.add(
        egui::Button::new(RichText::new(label).color(text).size(12.0))
            .fill(fill)
            .corner_radius(4.0),
    )
    .clicked()
}

fn source_row(ui: &mut Ui, chrome: UiChrome, title: &str, subtitle: &str, selected: bool) -> bool {
    let fill = if selected {
        chrome.accent_soft
    } else {
        chrome.surface
    };
    let stroke = if selected {
        chrome.accent
    } else {
        chrome.border
    };
    let title_color = if selected { chrome.accent } else { chrome.text };

    let frame = egui::Frame::NONE
        .fill(fill)
        .stroke(egui::Stroke::new(1.0, stroke))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::symmetric(14, 12));

    let inner = frame.show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.horizontal(|ui| {
            // Selection dot
            let (dot_rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
            ui.painter().circle_filled(
                dot_rect.center(),
                3.5,
                if selected {
                    chrome.accent
                } else {
                    chrome.text_muted
                },
            );
            ui.add_space(10.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.label(RichText::new(title).size(13.0).strong().color(title_color));
                if !subtitle.is_empty() {
                    ui.label(RichText::new(subtitle).size(11.0).color(chrome.text_muted));
                }
            });
        });
    });

    ui.add_space(10.0);
    inner.response.interact(Sense::click()).clicked()
}

fn stat_row(ui: &mut Ui, chrome: UiChrome, label: &str, value: String) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).small().color(chrome.text_muted));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(value).small().strong().color(chrome.text));
        });
    });
}

fn paint_safe_area_clipped(ui: &mut Ui, video_rect: Rect, clip: Rect) {
    let painter = ui.painter().with_clip_rect(clip);
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

fn paint_vu_clipped(ui: &mut Ui, video_rect: Rect, clip: Rect, levels: AudioLevels) {
    let painter = ui.painter().with_clip_rect(clip);
    let bar_w = 10.0f32;
    let gap = 4.0f32;
    let height = video_rect.height() * 0.6;
    let top = video_rect.min.y + (video_rect.height() - height) * 0.5;
    let left = video_rect.max.x - bar_w * 2.0 - gap - 12.0;
    let l = peak_to_meter(levels.peak_l);
    let r = peak_to_meter(levels.peak_r);
    paint_vu_bar_painter(&painter, Pos2::new(left, top), bar_w, height, l);
    paint_vu_bar_painter(
        &painter,
        Pos2::new(left + bar_w + gap, top),
        bar_w,
        height,
        r,
    );
}

fn paint_vu_bar_painter(
    painter: &egui::Painter,
    origin: Pos2,
    width: f32,
    full_h: f32,
    level: f32,
) {
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

fn peak_to_meter(peak: f32) -> f32 {
    const FLOOR_DB: f32 = -60.0;
    if peak <= 1e-6 {
        return 0.0;
    }
    let db = (20.0 * peak.log10()).clamp(FLOOR_DB, 0.0);
    ((db - FLOOR_DB) / -FLOOR_DB).clamp(0.0, 1.0)
}

fn format_dbfs(peak: f32) -> String {
    if peak <= 1e-6 {
        "-∞ dBFS".into()
    } else {
        format!("{:.1} dBFS", 20.0 * peak.log10())
    }
}

fn format_bitrate(bps: f64) -> String {
    if bps >= 1_000_000.0 {
        format!("{:.2} Mbps", bps / 1_000_000.0)
    } else if bps >= 1_000.0 {
        format!("{:.1} kbps", bps / 1_000.0)
    } else {
        format!("{bps:.0} bps")
    }
}

fn format_bytes(bytes: i64) -> String {
    let b = bytes.max(0) as f64;
    if b >= 1_000_000_000.0 {
        format!("{:.2} GB", b / 1_000_000_000.0)
    } else if b >= 1_000_000.0 {
        format!("{:.2} MB", b / 1_000_000.0)
    } else if b >= 1_000.0 {
        format!("{:.1} KB", b / 1_000.0)
    } else {
        format!("{b:.0} B")
    }
}

fn format_video_delay_frames(
    language: Language,
    buffer: &omt_media::BufferSettings,
    video_ms: u32,
    fps_n: i32,
    fps_d: i32,
) -> String {
    let frames = match buffer.video.unit {
        BufferUnit::Frames => buffer.video.amount,
        BufferUnit::Milliseconds => crate::preferences::ms_to_frames(video_ms, fps_n, fps_d),
    };
    let unit = if frames == 1 {
        t(language, "monitor.buffer_frame")
    } else {
        t(language, "monitor.buffer_frames")
    };
    format!(
        "{frames} {unit} ({}{video_ms} ms)",
        t(language, "monitor.buffer_equiv")
    )
}

fn format_buffer_stats(
    language: Language,
    buffer: &omt_media::BufferSettings,
    video_ms: u32,
    audio_ms: u32,
    fps_n: i32,
    fps_d: i32,
) -> String {
    let v = format_video_delay_frames(language, buffer, video_ms, fps_n, fps_d);
    let a = format!("{audio_ms} ms");
    if buffer.linked {
        format!("link · V {v} / A {a}")
    } else {
        format!("indep · V {v} / A {a}")
    }
}

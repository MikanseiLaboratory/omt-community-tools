//! Preferences modal — language, theme, viewer, audio, A/V buffer, version, license.

use egui::{Color32, Context, RichText, Sense, Ui, Vec2};
use omt_media::{AudioOutputDevice, BufferSettings};
use suite_core::{Language, SUITE_VERSION, ThemePreference, t};

use crate::chrome::UiChrome;
use crate::settings::{MonitorSettings, VideoQualityPreset};

const PANEL_W: f32 = 480.0;

/// Mutable text drafts for A/V buffer fields (owned by the app).
#[derive(Debug, Clone, Default)]
pub struct BufferEditState {
    /// Video delay in **frames** (source FPS aware).
    pub video_frames: String,
    /// Audio delay in milliseconds.
    pub audio_ms: String,
}

impl BufferEditState {
    pub fn sync_from(
        &mut self,
        buffer: BufferSettings,
        video_ms: u32,
        audio_ms: u32,
        fps_n: i32,
        fps_d: i32,
    ) {
        let frames = match buffer.video.unit {
            omt_media::BufferUnit::Frames => buffer.video.amount,
            omt_media::BufferUnit::Milliseconds => ms_to_frames(video_ms, fps_n, fps_d),
        };
        self.video_frames = frames.to_string();
        self.audio_ms = if buffer.linked {
            video_ms.to_string()
        } else {
            audio_ms.to_string()
        };
    }
}

/// Convert milliseconds to whole frames at the given rate.
pub fn ms_to_frames(ms: u32, fps_n: i32, fps_d: i32) -> u32 {
    let fps = fps_n.max(1) as f64 / fps_d.max(1) as f64;
    (ms as f64 * fps / 1000.0).round().clamp(0.0, 120.0) as u32
}

/// User actions emitted by the preferences overlay.
#[derive(Debug, Clone)]
pub enum PrefsAction {
    Close,
    SetLanguage(Language),
    SetTheme(ThemePreference),
    SetAudioDevice(Option<String>),
    SetVideoDelayFrames(u32),
    SetAudioDelayMs(u32),
    SetBufferLink(bool),
    SetBoost(i32),
    SetQuality(VideoQualityPreset),
    SetAlpha(bool),
    SetSafeArea(bool),
    SetVu(bool),
    EnterFullscreen,
    OpenHelp,
    OpenLicense,
    Exit,
}

/// Draw the preferences modal. Returns an action if the user interacted.
#[allow(clippy::too_many_arguments)]
pub fn show(
    ctx: &Context,
    language: Language,
    theme: ThemePreference,
    chrome: UiChrome,
    suite_version: &str,
    settings: &MonitorSettings,
    audio_devices: &[AudioOutputDevice],
    selected_audio: Option<&str>,
    audio_unavailable: bool,
    buffer: BufferSettings,
    video_delay_ms: u32,
    audio_delay_ms: u32,
    fps_n: i32,
    fps_d: i32,
    buffer_edit: &mut BufferEditState,
) -> Option<PrefsAction> {
    let mut action = None;

    let modal = egui::Modal::new(egui::Id::new("prefs-modal"))
        .backdrop_color(Color32::from_black_alpha(0x80))
        .frame(
            egui::Frame::NONE
                .fill(chrome.panel)
                .stroke(egui::Stroke::new(1.0, chrome.border))
                .corner_radius(8.0)
                .inner_margin(egui::Margin::symmetric(4, 8)),
        );

    let response = modal.show(ctx, |ui| {
        ui.set_width(PANEL_W);
        ui.set_max_height(ctx.content_rect().height() * 0.88);

        // Header
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(
                RichText::new(t(language, "monitor.preferences"))
                    .strong()
                    .size(16.0)
                    .color(chrome.text),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(12.0);
                if chip_button(ui, chrome, t(language, "back"), false) {
                    action = Some(PrefsAction::Close);
                }
                ui.add_space(8.0);
            });
        });
        ui.separator();

        egui::ScrollArea::vertical()
            .id_salt("prefs_body")
            .max_height(ctx.content_rect().height() * 0.75)
            .show(ui, |ui| {
                ui.add_space(12.0);
                ui.indent("prefs-body", |ui| {
                    // —— Language / Theme ——
                    section_title(ui, chrome, t(language, "language"));
                    ui.horizontal_wrapped(|ui| {
                        for lang in [Language::English, Language::Japanese] {
                            if chip_button(ui, chrome, lang.display_name(), language == lang) {
                                action = Some(PrefsAction::SetLanguage(lang));
                            }
                        }
                    });

                    ui.add_space(12.0);
                    section_title(ui, chrome, t(language, "theme"));
                    ui.horizontal_wrapped(|ui| {
                        for (label, pref) in [
                            (t(language, "theme.light"), ThemePreference::Light),
                            (t(language, "theme.dark"), ThemePreference::Dark),
                            (t(language, "theme.system"), ThemePreference::System),
                        ] {
                            if chip_button(ui, chrome, label, theme == pref) {
                                action = Some(PrefsAction::SetTheme(pref));
                            }
                        }
                    });

                    // —— Viewer (former context menu) ——
                    ui.add_space(12.0);
                    ui.separator();
                    section_title(ui, chrome, t(language, "monitor.video"));
                    if toggle_row(
                        ui,
                        chrome,
                        t(language, "monitor.alpha_mask"),
                        settings.show_alpha,
                    ) {
                        action = Some(PrefsAction::SetAlpha(!settings.show_alpha));
                    }
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(t(language, "monitor.quality"))
                            .small()
                            .color(chrome.text_muted),
                    );
                    ui.horizontal_wrapped(|ui| {
                        for (preset, key) in [
                            (VideoQualityPreset::Default, "monitor.quality_default"),
                            (VideoQualityPreset::Low, "monitor.quality_low"),
                            (VideoQualityPreset::Medium, "monitor.quality_medium"),
                            (VideoQualityPreset::High, "monitor.quality_high"),
                            (VideoQualityPreset::LowBandwidth, "monitor.quality_low_bw"),
                        ] {
                            if chip_button(ui, chrome, t(language, key), settings.quality == preset)
                            {
                                action = Some(PrefsAction::SetQuality(preset));
                            }
                        }
                    });

                    ui.add_space(12.0);
                    section_title(ui, chrome, t(language, "monitor.overlay"));
                    if toggle_row(
                        ui,
                        chrome,
                        t(language, "monitor.safe_area"),
                        settings.safe_area,
                    ) {
                        action = Some(PrefsAction::SetSafeArea(!settings.safe_area));
                    }
                    if toggle_row(
                        ui,
                        chrome,
                        t(language, "monitor.vu_meter"),
                        settings.vu_meter,
                    ) {
                        action = Some(PrefsAction::SetVu(!settings.vu_meter));
                    }

                    ui.add_space(12.0);
                    section_title(ui, chrome, t(language, "monitor.audio"));
                    ui.label(
                        RichText::new(t(language, "monitor.audio_boost"))
                            .small()
                            .color(chrome.text_muted),
                    );
                    ui.horizontal_wrapped(|ui| {
                        for db in [0, 6, 10, 20] {
                            let label = if db == 0 {
                                "0 dB".into()
                            } else {
                                format!("+{db} dB")
                            };
                            if chip_button(ui, chrome, &label, settings.audio_boost_db == db) {
                                action = Some(PrefsAction::SetBoost(db));
                            }
                        }
                    });

                    ui.add_space(8.0);
                    section_title(ui, chrome, t(language, "monitor.audio_output"));
                    egui::Frame::NONE
                        .fill(chrome.bg)
                        .stroke(egui::Stroke::new(1.0, chrome.border))
                        .inner_margin(6.0)
                        .show(ui, |ui| {
                            ui.set_min_height(100.0);
                            egui::ScrollArea::vertical()
                                .id_salt("prefs_audio_devices")
                                .max_height(120.0)
                                .show(ui, |ui| {
                                    let default_selected = selected_audio.is_none();
                                    if device_row(
                                        ui,
                                        chrome,
                                        t(language, "monitor.audio_default"),
                                        default_selected,
                                    ) {
                                        action = Some(PrefsAction::SetAudioDevice(None));
                                    }
                                    if audio_devices.is_empty() {
                                        ui.label(
                                            RichText::new(t(language, "monitor.audio_none"))
                                                .color(chrome.text_muted)
                                                .small(),
                                        );
                                    } else {
                                        for dev in audio_devices {
                                            let sel = selected_audio == Some(dev.name.as_str());
                                            if device_row(ui, chrome, &dev.name, sel) {
                                                action = Some(PrefsAction::SetAudioDevice(Some(
                                                    dev.name.clone(),
                                                )));
                                            }
                                        }
                                    }
                                });
                        });
                    if audio_unavailable {
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new(t(language, "monitor.audio_unavailable"))
                                .color(chrome.text),
                        );
                        ui.label(
                            RichText::new(t(language, "monitor.audio_unavailable_hint"))
                                .small()
                                .color(chrome.text_muted),
                        );
                    }

                    // —— A/V buffer (text fields) ——
                    ui.add_space(12.0);
                    ui.separator();
                    section_title(ui, chrome, t(language, "monitor.av_buffer"));
                    if toggle_row(
                        ui,
                        chrome,
                        t(language, "monitor.buffer_link"),
                        buffer.linked,
                    ) {
                        action = Some(PrefsAction::SetBufferLink(!buffer.linked));
                    }
                    ui.label(
                        RichText::new(t(language, "monitor.buffer_unlink_info"))
                            .small()
                            .color(chrome.text_muted),
                    );
                    ui.add_space(6.0);

                    buffer_frames_field(
                        ui,
                        chrome,
                        language,
                        t(language, "monitor.buffer_video"),
                        &mut buffer_edit.video_frames,
                        video_delay_ms,
                        fps_n,
                        fps_d,
                        &mut action,
                    );
                    ui.add_space(4.0);
                    buffer_ms_field(
                        ui,
                        chrome,
                        t(language, "monitor.buffer_audio"),
                        &mut buffer_edit.audio_ms,
                        if buffer.linked {
                            video_delay_ms
                        } else {
                            audio_delay_ms
                        },
                        &mut action,
                    );

                    // —— Window / help ——
                    ui.add_space(12.0);
                    ui.separator();
                    ui.horizontal_wrapped(|ui| {
                        if chip_button(ui, chrome, t(language, "monitor.fullscreen"), false) {
                            action = Some(PrefsAction::EnterFullscreen);
                        }
                        if chip_button(ui, chrome, t(language, "monitor.help"), false) {
                            action = Some(PrefsAction::OpenHelp);
                        }
                        if chip_button(ui, chrome, t(language, "monitor.exit"), false) {
                            action = Some(PrefsAction::Exit);
                        }
                    });

                    ui.add_space(12.0);
                    ui.separator();
                    section_title(ui, chrome, t(language, "version"));
                    ui.label(
                        RichText::new(format!("OMT Tools / Studio Monitor  v{suite_version}"))
                            .color(chrome.text),
                    );

                    ui.add_space(8.0);
                    section_title(ui, chrome, t(language, "license"));
                    ui.label(
                        RichText::new(format!(
                            "{}\n\n{}",
                            t(language, "monitor.license_spdx"),
                            t(language, "monitor.license_body")
                        ))
                        .color(chrome.text_muted)
                        .small(),
                    );
                    if ui
                        .add(
                            egui::Label::new(
                                RichText::new(t(language, "monitor.license_link"))
                                    .color(chrome.accent),
                            )
                            .sense(Sense::click()),
                        )
                        .clicked()
                    {
                        action = Some(PrefsAction::OpenLicense);
                    }
                    ui.label(
                        RichText::new(format!(
                            "{} · suite {}",
                            t(language, "tool.studio_monitor"),
                            SUITE_VERSION
                        ))
                        .color(chrome.text_muted)
                        .small(),
                    );
                    ui.add_space(12.0);
                });
            });
    });

    if action.is_some() {
        action
    } else if response.should_close() {
        Some(PrefsAction::Close)
    } else {
        None
    }
}

#[allow(clippy::too_many_arguments)]
fn buffer_frames_field(
    ui: &mut Ui,
    chrome: UiChrome,
    language: Language,
    label: &str,
    edit: &mut String,
    applied_ms: u32,
    fps_n: i32,
    fps_d: i32,
    action: &mut Option<PrefsAction>,
) {
    let fps = fps_n.max(1) as f64 / fps_d.max(1) as f64;
    let applied_frames = ms_to_frames(applied_ms, fps_n, fps_d);
    let unit = if applied_frames == 1 {
        t(language, "monitor.buffer_frame")
    } else {
        t(language, "monitor.buffer_frames")
    };
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).color(chrome.text));
        ui.label(
            RichText::new(format!(
                "({applied_frames} {unit} · {}{applied_ms} ms @ {fps:.2})",
                t(language, "monitor.buffer_equiv"),
            ))
            .small()
            .color(chrome.text_muted),
        );
    });
    ui.horizontal(|ui| {
        let resp = ui.add(
            egui::TextEdit::singleline(edit)
                .id(egui::Id::new("buf-video-frames"))
                .desired_width(120.0)
                .hint_text("frames"),
        );
        ui.label(RichText::new(t(language, "monitor.buffer_frames")).color(chrome.text_muted));
        let enter = resp.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if enter || resp.lost_focus() {
            if let Ok(frames) = edit.trim().parse::<u32>() {
                let frames = frames.min(120);
                *edit = frames.to_string();
                *action = Some(PrefsAction::SetVideoDelayFrames(frames));
            } else if !edit.trim().is_empty() {
                *edit = applied_frames.to_string();
            }
        }
    });
}

fn buffer_ms_field(
    ui: &mut Ui,
    chrome: UiChrome,
    label: &str,
    edit: &mut String,
    applied_ms: u32,
    action: &mut Option<PrefsAction>,
) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).color(chrome.text));
        ui.label(
            RichText::new(format!("({applied_ms} ms)"))
                .small()
                .color(chrome.text_muted),
        );
    });
    ui.horizontal(|ui| {
        let resp = ui.add(
            egui::TextEdit::singleline(edit)
                .id(egui::Id::new("buf-audio-ms"))
                .desired_width(120.0)
                .hint_text("ms"),
        );
        ui.label(RichText::new("ms").color(chrome.text_muted));
        let enter = resp.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if enter || resp.lost_focus() {
            if let Ok(ms) = edit.trim().parse::<u32>() {
                let ms = ms.min(2_000);
                *edit = ms.to_string();
                *action = Some(PrefsAction::SetAudioDelayMs(ms));
            } else if !edit.trim().is_empty() {
                *edit = applied_ms.to_string();
            }
        }
    });
}

fn section_title(ui: &mut Ui, chrome: UiChrome, title: &str) {
    ui.label(RichText::new(title).strong().color(chrome.text));
    ui.add_space(4.0);
}

fn chip_button(ui: &mut Ui, chrome: UiChrome, label: &str, active: bool) -> bool {
    let fill = if active {
        chrome.accent_soft
    } else {
        chrome.surface
    };
    let stroke = if active { chrome.accent } else { chrome.border };
    let text = if active { chrome.accent } else { chrome.text };
    let button = egui::Button::new(RichText::new(label).color(text).size(13.0))
        .fill(fill)
        .stroke(egui::Stroke::new(1.0, stroke))
        .corner_radius(4.0);
    ui.add(button).clicked()
}

fn toggle_row(ui: &mut Ui, chrome: UiChrome, label: &str, on: bool) -> bool {
    let mark = if on { "✓" } else { "○" };
    chip_button(ui, chrome, &format!("{mark}  {label}"), on)
}

fn device_row(ui: &mut Ui, chrome: UiChrome, label: &str, active: bool) -> bool {
    let fill = if active {
        chrome.accent_soft
    } else {
        chrome.surface
    };
    let text = if active { chrome.accent } else { chrome.text };
    ui.add(
        egui::Button::new(RichText::new(label).color(text).size(13.0))
            .fill(fill)
            .min_size(Vec2::new(ui.available_width(), 28.0))
            .corner_radius(4.0),
    )
    .clicked()
}

//! Preferences modal — language, theme, viewer, audio, A/V buffer, version, license.

use egui::{Color32, Context, RichText, Sense, Ui, Vec2};
use omt_media::{AudioOutputDevice, BufferSettings, PlayoutMode};
use suite_core::{Language, OutputDisplayRef, SUITE_VERSION, ThemePreference, t};

use crate::chrome::UiChrome;
use crate::output::{DisplayInfo, OutputError};
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
    SetPlayoutMode(PlayoutMode),
    SetBoost(i32),
    SetVolume(i32),
    SetQuality(VideoQualityPreset),
    SetVideoDecode(omt_media::VideoDecodePath),
    SetAlpha(bool),
    SetSafeArea(bool),
    SetVu(bool),
    SetOutputEnabled(bool),
    SetOutputDisplay(Option<OutputDisplayRef>),
    SetOutputVu(bool),
    SetOutputSafeArea(bool),
    SetOutputTally(bool),
    SetOutputWarnings(bool),
    RefreshDisplays,
    EnterFullscreen,
    OpenHelp,
    OpenLicense,
    Exit,
}

/// Everything the preferences modal needs to render.
///
/// Kept in one struct so adding a setting does not grow the `show` signature
/// into the dozens.
pub struct PrefsContext<'a> {
    /// UI language.
    pub language: Language,
    /// Theme preference.
    pub theme: ThemePreference,
    /// Resolved colours.
    pub chrome: UiChrome,
    /// Suite version string.
    pub suite_version: &'a str,
    /// Current viewer settings.
    pub settings: &'a MonitorSettings,
    /// Available audio output devices.
    pub audio_devices: &'a [AudioOutputDevice],
    /// Selected audio device name.
    pub selected_audio: Option<&'a str>,
    /// Audio device actually in use.
    pub active_audio: Option<&'a str>,
    /// Whether the system audio output is unavailable.
    pub audio_unavailable: bool,
    /// Whether the eframe wgpu renderer is available.
    pub gpu_available: Option<bool>,
    /// A/V buffer settings.
    pub buffer: BufferSettings,
    /// Current video delay in milliseconds.
    pub video_delay_ms: u32,
    /// Current audio delay in milliseconds.
    pub audio_delay_ms: u32,
    /// Source frame-rate numerator.
    pub fps_n: i32,
    /// Source frame-rate denominator.
    pub fps_d: i32,
    /// Displays found by the last scan.
    pub displays: &'a [DisplayInfo],
    /// Display currently holding the operator window, if known.
    pub operator_display: Option<usize>,
    /// Whether the program output window is running.
    pub output_running: bool,
    /// Display index the output is running on.
    pub output_monitor: Option<usize>,
    /// Last output start error.
    pub output_error: Option<OutputError>,
}

/// Draw the preferences modal. Returns an action if the user interacted.
pub fn show(
    ctx: &Context,
    prefs: PrefsContext<'_>,
    buffer_edit: &mut BufferEditState,
) -> Option<PrefsAction> {
    let PrefsContext {
        language,
        theme,
        chrome,
        suite_version,
        settings,
        audio_devices,
        selected_audio,
        active_audio,
        audio_unavailable,
        gpu_available,
        buffer,
        video_delay_ms,
        audio_delay_ms,
        fps_n,
        fps_d,
        displays,
        operator_display,
        output_running,
        output_monitor,
        output_error,
    } = prefs;
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

                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(t(language, "monitor.decode"))
                            .small()
                            .color(chrome.text_muted),
                    );
                    ui.horizontal_wrapped(|ui| {
                        if chip_button(
                            ui,
                            chrome,
                            t(language, "monitor.decode_cpu"),
                            settings.video_decode == omt_media::VideoDecodePath::Cpu,
                        ) {
                            action =
                                Some(PrefsAction::SetVideoDecode(omt_media::VideoDecodePath::Cpu));
                        }
                        if chip_button(
                            ui,
                            chrome,
                            t(language, "monitor.decode_gpu"),
                            settings.video_decode == omt_media::VideoDecodePath::Gpu,
                        ) {
                            action =
                                Some(PrefsAction::SetVideoDecode(omt_media::VideoDecodePath::Gpu));
                        }
                    });
                    if settings.video_decode == omt_media::VideoDecodePath::Gpu
                        && gpu_available == Some(false)
                    {
                        ui.label(
                            RichText::new(t(language, "monitor.decode_gpu_unavailable"))
                                .small()
                                .color(chrome.text_muted),
                        );
                    }

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

                    // —— Program output ——
                    ui.add_space(12.0);
                    ui.separator();
                    section_title(ui, chrome, t(language, "monitor.output"));
                    ui.label(
                        RichText::new(t(language, "monitor.output_description"))
                            .small()
                            .color(chrome.text_muted),
                    );
                    ui.add_space(6.0);
                    if toggle_row(
                        ui,
                        chrome,
                        t(language, "monitor.output_send"),
                        output_running,
                    ) {
                        action = Some(PrefsAction::SetOutputEnabled(!output_running));
                    }
                    if output_running {
                        let label = output_monitor
                            .and_then(|index| displays.get(index))
                            .map(DisplayInfo::short_label)
                            .unwrap_or_default();
                        ui.label(
                            RichText::new(format!(
                                "{} · {label}",
                                t(language, "monitor.output_running")
                            ))
                            .small()
                            .strong()
                            .color(chrome.accent),
                        );
                    }

                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(t(language, "monitor.output_display"))
                            .small()
                            .color(chrome.text_muted),
                    );
                    egui::Frame::NONE
                        .fill(chrome.bg)
                        .stroke(egui::Stroke::new(1.0, chrome.border))
                        .inner_margin(6.0)
                        .show(ui, |ui| {
                            egui::ScrollArea::vertical()
                                .id_salt("prefs_output_displays")
                                .max_height(160.0)
                                .show(ui, |ui| {
                                    let auto_selected = settings.output.display.is_none();
                                    if display_row(
                                        ui,
                                        chrome,
                                        t(language, "monitor.output_auto"),
                                        "",
                                        auto_selected,
                                        true,
                                    ) {
                                        action = Some(PrefsAction::SetOutputDisplay(None));
                                    }
                                    if displays.len() < 2
                                        || !displays.iter().any(|display| !display.is_operator)
                                    {
                                        ui.label(
                                            RichText::new(t(
                                                language,
                                                "monitor.output_no_second_display",
                                            ))
                                            .small()
                                            .color(chrome.text_muted),
                                        );
                                    }
                                    for display in displays {
                                        let selected = display_selected(
                                            settings.output.display.as_ref(),
                                            display,
                                        );
                                        let mut subtitle = display.name.clone().unwrap_or_default();
                                        let mut tags = Vec::new();
                                        if display.is_primary {
                                            tags.push(t(language, "monitor.output_primary"));
                                        }
                                        if display.is_operator {
                                            tags.push(t(language, "monitor.output_here"));
                                        }
                                        if !tags.is_empty() {
                                            if !subtitle.is_empty() {
                                                subtitle.push_str(" · ");
                                            }
                                            subtitle.push_str(&tags.join(" · "));
                                        }
                                        let title = format!(
                                            "{} · {}",
                                            display.short_label(),
                                            display.detail()
                                        );
                                        if display_row(
                                            ui,
                                            chrome,
                                            &title,
                                            &subtitle,
                                            selected,
                                            !display.is_operator,
                                        ) {
                                            action = Some(PrefsAction::SetOutputDisplay(Some(
                                                OutputDisplayRef {
                                                    index: display.index as u32,
                                                    name: display.name.clone(),
                                                },
                                            )));
                                        }
                                    }
                                });
                        });
                    ui.add_space(4.0);
                    if chip_button(
                        ui,
                        chrome,
                        t(language, "monitor.output_refresh_displays"),
                        false,
                    ) {
                        action = Some(PrefsAction::RefreshDisplays);
                    }
                    if let Some(error) = output_error {
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(error.message(language))
                                .small()
                                .color(chrome.danger),
                        );
                    }
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(t(language, "monitor.output_hint"))
                            .small()
                            .color(chrome.text_muted),
                    );
                    if operator_display.is_none() && !displays.is_empty() {
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(t(language, "monitor.output_operator_unknown"))
                                .small()
                                .color(chrome.text_muted),
                        );
                    }
                    if settings.show_alpha {
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(t(language, "monitor.output_alpha_note"))
                                .small()
                                .color(chrome.text_muted),
                        );
                    }

                    ui.add_space(12.0);
                    section_title(ui, chrome, t(language, "monitor.output_overlays"));
                    if toggle_row(
                        ui,
                        chrome,
                        t(language, "monitor.vu_meter"),
                        settings.output.overlays.vu,
                    ) {
                        action = Some(PrefsAction::SetOutputVu(!settings.output.overlays.vu));
                    }
                    if toggle_row(
                        ui,
                        chrome,
                        t(language, "monitor.safe_area"),
                        settings.output.overlays.safe_area,
                    ) {
                        action = Some(PrefsAction::SetOutputSafeArea(
                            !settings.output.overlays.safe_area,
                        ));
                    }
                    if toggle_row(
                        ui,
                        chrome,
                        t(language, "monitor.output_tally"),
                        settings.output.overlays.tally,
                    ) {
                        action = Some(PrefsAction::SetOutputTally(!settings.output.overlays.tally));
                    }
                    if toggle_row(
                        ui,
                        chrome,
                        t(language, "monitor.output_warnings"),
                        settings.output.overlays.warnings,
                    ) {
                        action = Some(PrefsAction::SetOutputWarnings(
                            !settings.output.overlays.warnings,
                        ));
                    }
                    ui.label(
                        RichText::new(t(language, "monitor.output_clean_hint"))
                            .small()
                            .color(chrome.text_muted),
                    );

                    ui.add_space(12.0);
                    section_title(ui, chrome, t(language, "monitor.audio"));
                    ui.label(
                        RichText::new(t(language, "monitor.audio_volume"))
                            .small()
                            .color(chrome.text_muted),
                    );
                    ui.horizontal(|ui| {
                        let mut volume = settings.audio_volume_pct;
                        let resp = ui.add(
                            egui::Slider::new(&mut volume, 0..=100)
                                .suffix("%")
                                .show_value(true),
                        );
                        if resp.changed() {
                            action = Some(PrefsAction::SetVolume(volume));
                        }
                    });
                    ui.add_space(8.0);
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
                    if let Some(active) = active_audio {
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(format!(
                                "{}: {active}",
                                t(language, "monitor.audio_active")
                            ))
                            .small()
                            .color(chrome.text_muted),
                        );
                    }
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
                    let immediate = buffer.mode == PlayoutMode::Immediate;
                    if toggle_row(
                        ui,
                        chrome,
                        t(language, "monitor.playout_immediate"),
                        immediate,
                    ) {
                        action = Some(PrefsAction::SetPlayoutMode(if immediate {
                            PlayoutMode::Synced
                        } else {
                            PlayoutMode::Immediate
                        }));
                    }
                    ui.label(
                        RichText::new(t(language, "monitor.playout_immediate_info"))
                            .small()
                            .color(chrome.text_muted),
                    );
                    ui.add_space(6.0);
                    ui.add_enabled_ui(!immediate, |ui| {
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
                    });

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

fn display_selected(target: Option<&OutputDisplayRef>, display: &DisplayInfo) -> bool {
    let Some(target) = target else {
        return false;
    };
    match target.name.as_deref() {
        Some(name) => display.name.as_deref() == Some(name),
        None => target.index as usize == display.index,
    }
}

fn display_row(
    ui: &mut Ui,
    chrome: UiChrome,
    title: &str,
    subtitle: &str,
    selected: bool,
    enabled: bool,
) -> bool {
    let fill = if selected {
        chrome.accent_soft
    } else if enabled {
        chrome.surface
    } else {
        chrome.bg
    };
    let stroke = if selected {
        chrome.accent
    } else {
        chrome.border
    };
    let title_color = if selected {
        chrome.accent
    } else if enabled {
        chrome.text
    } else {
        chrome.text_muted
    };

    let frame = egui::Frame::NONE
        .fill(fill)
        .stroke(egui::Stroke::new(1.0, stroke))
        .corner_radius(6.0)
        .inner_margin(egui::Margin::symmetric(12, 9));
    let inner = frame.show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.horizontal(|ui| {
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
            ui.add_space(8.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 3.0;
                ui.label(RichText::new(title).size(13.0).strong().color(title_color));
                if !subtitle.is_empty() {
                    ui.label(RichText::new(subtitle).size(11.0).color(chrome.text_muted));
                }
            });
        });
    });
    ui.add_space(6.0);

    if !enabled {
        return false;
    }
    inner.response.interact(Sense::click()).clicked()
}

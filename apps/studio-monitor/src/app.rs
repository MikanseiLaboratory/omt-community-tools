//! egui/eframe Studio Monitor application.

mod panels;

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use anyhow::Result;
use eframe::egui::{self, Context, Pos2, TextureHandle, TextureOptions};
use omt_media::{
    AudioLevels, AudioOutputDevice, AudioOutputStatus, BufferUnit, ConnectOptions, DelaySetting,
    DiscoveredSource, ReceiveWorker, SessionState, StallState, list_output_devices, spawn_discover,
};
use suite_core::{
    Language, SUITE_VERSION, SimdCapabilities, StudioMonitorConfig, ThemePreference,
    install_egui_cjk_fonts, load_config, load_studio_monitor_config, save_config,
    save_studio_monitor_config, t,
};

use crate::chrome::UiChrome;
use crate::frame_prep::{FramePrep, PrepControl, PreparedFrame};
use crate::preferences::{self, BufferEditState, PrefsAction};
use crate::settings::MonitorSettings;

type DiscoveryResult = Result<Vec<DiscoveredSource>, String>;

const ZOOM_MIN: f32 = 0.1;
const ZOOM_MAX: f32 = 8.0;
const SIDEBAR_W: f32 = 300.0;
const STATS_W: f32 = 280.0;
const LOG_H: f32 = 180.0;
const SIDEBAR_MIN_W: f32 = 200.0;
const SIDEBAR_MAX_W: f32 = 480.0;
const STATS_MIN_W: f32 = 200.0;
const STATS_MAX_W: f32 = 420.0;
const LOG_MIN_H: f32 = 80.0;
const LOG_MAX_H: f32 = 360.0;
const PREVIEW_MIN_W: f32 = 160.0;
/// Outer inset + gap between chrome panels (sidebar / preview / stats / log).
const LAYOUT_GAP: f32 = 14.0;
const TOOLBAR_H: f32 = 48.0;
const ACTION_SAFE_FRAC: f32 = 0.93;
const TITLE_SAFE_FRAC: f32 = 0.90;

/// Launch the egui Studio Monitor window.
pub fn run_eframe(
    title: String,
    language: Language,
    theme: ThemePreference,
    initial_url: Option<String>,
) -> Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1440.0, 860.0])
            .with_title(title.clone())
            .with_app_id(suite_core::ToolId::StudioMonitor.binary_name()),
        ..Default::default()
    };
    eframe::run_native(
        &title,
        options,
        Box::new(move |cc| {
            install_egui_cjk_fonts(&cc.egui_ctx);
            Ok(Box::new(MonitorApp::new(
                &cc.egui_ctx,
                language,
                theme,
                initial_url,
            )))
        }),
    )
    .map_err(|e| anyhow::anyhow!("eframe: {e}"))
}

struct MonitorApp {
    language: Language,
    theme: ThemePreference,
    suite_version: String,
    preferences_open: bool,
    audio_devices: Vec<AudioOutputDevice>,
    audio_output_device: Option<String>,
    worker: ReceiveWorker,
    /// Owns the prep thread; kept for Drop shutdown.
    #[allow(dead_code)]
    prep: FramePrep,
    prep_ctrl: Arc<PrepControl>,
    discovered: Vec<DiscoveredSource>,
    selected: Option<String>,
    /// Discovery-time IPs for the current selection (used on connect / reapply).
    selected_addresses: Vec<String>,
    status: String,
    frame_w: u32,
    frame_h: u32,
    fps_n: i32,
    fps_d: i32,
    display_fps: f32,
    frames_presented: u64,
    frames_render_skipped: u64,
    last_refresh: Instant,
    last_frame_at: Option<Instant>,
    window_fps_count: u32,
    window_fps_start: Instant,
    texture: Option<TextureHandle>,
    discovering: bool,
    discovery_rx: Option<Receiver<DiscoveryResult>>,
    refresh_silent: bool,
    zoom: f32,
    preview_w: f32,
    preview_h: f32,
    bitrate_bps: f64,
    last_bytes_received: i64,
    last_bitrate_at: Instant,
    source_dropped: u64,
    net_dropped: i64,
    frames_decoded: u64,
    audio_frames: u64,
    audio_levels: AudioLevels,
    video_buffer_delay_ms: u32,
    audio_buffer_delay_ms: u32,
    bytes_received: i64,
    decode_ms_avg: f64,
    decode_ms_peak: f64,
    reconnects: u64,
    wire_queue_depth: u32,
    session_state: SessionState,
    pan_x: f32,
    pan_y: f32,
    pan_drag: Option<Pos2>,
    log_lines: VecDeque<String>,
    log_last_unix_ms: u64,
    stall_text: String,
    settings: MonitorSettings,
    buffer_edit: BufferEditState,
    fullscreen: bool,
    last_theme_dark: Option<bool>,
    simd_summary: String,
    sidebar_w: f32,
    stats_w: f32,
    log_h: f32,
    stats_video_open: bool,
    stats_audio_open: bool,
    stats_source_open: bool,
}

impl MonitorApp {
    fn new(
        ctx: &Context,
        language: Language,
        theme: ThemePreference,
        initial_url: Option<String>,
    ) -> Self {
        let worker = ReceiveWorker::spawn();
        let settings = MonitorSettings::default();
        worker.set_buffer(settings.buffer);
        worker.set_audio_boost_db(settings.audio_boost_db);
        if let Some(url) = &initial_url {
            worker.connect(url.clone());
        }

        let prep_ctrl = PrepControl::new();
        if let Some(url) = &initial_url {
            *prep_ctrl.selected_url.lock() = Some(url.clone());
        }
        prep_ctrl.set_alpha(settings.show_alpha);
        prep_ctrl.set_repaint_context(ctx.clone());
        let prep = FramePrep::start(worker.latest(), Arc::clone(&prep_ctrl));

        let suite_version = std::env::var(suite_core::env::SUITE_VERSION)
            .unwrap_or_else(|_| SUITE_VERSION.to_string());
        let layout = load_studio_monitor_config().unwrap_or_default();

        let system_dark = matches!(ctx.system_theme(), Some(egui::Theme::Dark));
        let chrome = UiChrome::resolve(theme, system_dark);
        chrome.apply_to_context(ctx, system_dark || matches!(theme, ThemePreference::Dark));

        let mut app = Self {
            language,
            theme,
            suite_version,
            preferences_open: false,
            audio_devices: list_output_devices(),
            audio_output_device: None,
            worker,
            prep,
            prep_ctrl,
            discovered: Vec::new(),
            selected: initial_url,
            selected_addresses: Vec::new(),
            status: String::new(),
            frame_w: 0,
            frame_h: 0,
            fps_n: 0,
            fps_d: 1,
            display_fps: 0.0,
            frames_presented: 0,
            frames_render_skipped: 0,
            last_refresh: Instant::now() - Duration::from_secs(10),
            last_frame_at: None,
            window_fps_count: 0,
            window_fps_start: Instant::now(),
            texture: None,
            discovering: false,
            discovery_rx: None,
            refresh_silent: true,
            zoom: 1.0,
            preview_w: 0.0,
            preview_h: 0.0,
            bitrate_bps: 0.0,
            last_bytes_received: 0,
            last_bitrate_at: Instant::now(),
            source_dropped: 0,
            net_dropped: 0,
            frames_decoded: 0,
            audio_frames: 0,
            audio_levels: AudioLevels::default(),
            video_buffer_delay_ms: settings.buffer.video_delay_ms(30, 1),
            audio_buffer_delay_ms: settings.buffer.audio_delay_ms(30, 1),
            bytes_received: 0,
            decode_ms_avg: 0.0,
            decode_ms_peak: 0.0,
            reconnects: 0,
            wire_queue_depth: 0,
            session_state: SessionState::Stopped,
            pan_x: 0.0,
            pan_y: 0.0,
            pan_drag: None,
            log_lines: VecDeque::new(),
            log_last_unix_ms: 0,
            stall_text: t(language, "monitor.waiting").to_string(),
            settings,
            buffer_edit: BufferEditState::default(),
            fullscreen: false,
            last_theme_dark: None,
            simd_summary: SimdCapabilities::detect().summary(),
            sidebar_w: clamp_sidebar_w(layout.sidebar_w as f32),
            stats_w: clamp_stats_w(layout.stats_w as f32),
            log_h: clamp_log_h(layout.log_h as f32),
            stats_video_open: layout.stats_video_open,
            stats_audio_open: layout.stats_audio_open,
            stats_source_open: layout.stats_source_open,
        };
        app.buffer_edit.sync_from(
            app.settings.buffer,
            app.video_buffer_delay_ms,
            app.audio_buffer_delay_ms,
            app.fps_n.max(1),
            app.fps_d.max(1),
        );
        app.request_refresh(true);
        app
    }

    fn request_refresh(&mut self, silent: bool) {
        if self.discovering {
            return;
        }
        self.discovering = true;
        self.refresh_silent = silent;
        if !silent {
            self.status = t(self.language, "monitor.refresh").to_string();
        }
        self.discovery_rx = Some(spawn_discover(Duration::from_millis(1500)));
        self.last_refresh = Instant::now();
    }

    fn poll_discovery(&mut self) {
        let Some(rx) = self.discovery_rx.as_ref() else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(list)) => {
                self.discovered = list;
                if let Some(url) = self.selected.as_ref()
                    && let Some(src) = self.discovered.iter().find(|s| &s.url == url)
                {
                    self.selected_addresses = src.addresses.clone();
                }
                if !self.refresh_silent {
                    self.status = if self.discovered.is_empty() {
                        t(self.language, "monitor.no_sources").to_string()
                    } else {
                        format!("{} source(s)", self.discovered.len())
                    };
                }
                self.discovering = false;
                self.discovery_rx = None;
            }
            Ok(Err(err)) => {
                if !self.refresh_silent {
                    self.status = err;
                }
                self.discovering = false;
                self.discovery_rx = None;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                if !self.refresh_silent {
                    self.status = "discovery task ended".into();
                }
                self.discovering = false;
                self.discovery_rx = None;
            }
        }
    }

    fn ingest_logs(&mut self) {
        let entries = self.worker.latest().metadata_since(self.log_last_unix_ms);
        for entry in entries {
            self.log_last_unix_ms = self.log_last_unix_ms.max(entry.unix_ms);
            let line = format!(
                "[{}] {}: {}",
                entry.unix_ms % 100_000,
                entry.kind,
                entry.text
            );
            self.log_lines.clear();
            self.log_lines.push_back(line);
        }
    }

    fn ingest_prepared_frame(&mut self, ctx: &Context) -> bool {
        let Some(frame) = self.prep_ctrl.take_prepared() else {
            return false;
        };
        let frame = Arc::try_unwrap(frame).unwrap_or_else(|arc| PreparedFrame {
            image: arc.image.clone(),
            fps_n: arc.fps_n,
            fps_d: arc.fps_d,
        });
        self.frame_w = frame.image.width() as u32;
        self.frame_h = frame.image.height() as u32;
        self.fps_n = frame.fps_n;
        self.fps_d = frame.fps_d.max(1);
        self.window_fps_count += 1;
        self.last_frame_at = Some(Instant::now());

        let size = frame.image.size;
        match &mut self.texture {
            Some(tex) if tex.size() == size => {
                tex.set(frame.image, TextureOptions::LINEAR);
            }
            _ => {
                self.texture =
                    Some(ctx.load_texture("omt-video", frame.image, TextureOptions::LINEAR));
            }
        }
        self.frames_presented = self
            .prep_ctrl
            .presented
            .load(std::sync::atomic::Ordering::Relaxed);
        self.frames_render_skipped = self
            .prep_ctrl
            .skipped
            .load(std::sync::atomic::Ordering::Relaxed);
        true
    }

    fn on_tick(&mut self, ctx: &Context) -> bool {
        self.poll_discovery();
        if !self.discovering && self.last_refresh.elapsed() > Duration::from_secs(3) {
            self.request_refresh(true);
        }
        self.ingest_logs();
        let got_frame = self.ingest_prepared_frame(ctx);

        {
            let counters = *self.worker.latest().counters.lock();
            let audio_levels = *self.worker.latest().audio_levels.lock();
            let video_buffer_delay_ms = *self.worker.latest().video_buffer_delay_ms.lock();
            let audio_buffer_delay_ms = *self.worker.latest().audio_buffer_delay_ms.lock();
            let stats = *self.worker.latest().stats.lock();
            let session_state = *self.worker.latest().session_state.lock();

            self.frames_decoded = counters.frames_decoded;
            self.source_dropped = counters.frames_replaced;
            self.audio_frames = counters.audio_frames;
            self.audio_levels = audio_levels;
            self.video_buffer_delay_ms = video_buffer_delay_ms;
            self.audio_buffer_delay_ms = audio_buffer_delay_ms;
            self.session_state = session_state;
            if self.settings.buffer.linked {
                let (fps_n, fps_d) = self.buffer_fps();
                let before = self.settings.buffer;
                self.settings.buffer.resync_linked(fps_n, fps_d);
                if self.settings.buffer != before {
                    self.worker.set_buffer(self.settings.buffer);
                }
            }
            self.net_dropped = (stats.frames_dropped_wire + stats.frames_dropped_decode) as i64;
            self.bytes_received = stats.bytes_received as i64;
            self.reconnects = stats.reconnects;
            self.wire_queue_depth = stats.wire_queue_depth;
            self.decode_ms_avg = if stats.frames_decoded > 0 {
                (stats.codec_time_ns as f64 / stats.frames_decoded as f64) / 1_000_000.0
            } else {
                0.0
            };
            self.decode_ms_peak = stats.codec_time_ns_peak as f64 / 1_000_000.0;
            let elapsed = self.last_bitrate_at.elapsed().as_secs_f64().max(0.001);
            if elapsed >= 0.5 {
                let delta = (stats.bytes_received as i64 - self.last_bytes_received).max(0) as f64;
                self.bitrate_bps = delta * 8.0 / elapsed;
                self.last_bytes_received = stats.bytes_received as i64;
                self.last_bitrate_at = Instant::now();
            }
        }

        match self.session_state {
            SessionState::Connecting => {
                self.status = "Connecting…".into();
            }
            SessionState::Reconnecting => {
                self.status = format!("Reconnecting… ({})", self.reconnects);
            }
            SessionState::Connected => {
                if let Some(err) = self.worker.latest().error.lock().clone() {
                    if !err.is_empty() {
                        self.status = err;
                    }
                } else if self.status.starts_with("Connecting")
                    || self.status.starts_with("Reconnecting")
                {
                    self.status.clear();
                }
            }
            SessionState::Stopping | SessionState::Stopped => {
                if let Some(err) = self.worker.latest().error.lock().clone() {
                    self.status = err;
                }
            }
        }

        {
            let guard = self.worker.stall();
            let mut d = guard.lock();
            self.stall_text = match d.tick() {
                StallState::Waiting => t(self.language, "monitor.waiting").to_string(),
                StallState::Live => "LIVE".into(),
                StallState::Stalled => t(self.language, "monitor.stalled").to_string(),
            };
        }

        if self.window_fps_start.elapsed() >= Duration::from_secs(1) {
            self.display_fps =
                self.window_fps_count as f32 / self.window_fps_start.elapsed().as_secs_f32();
            self.window_fps_count = 0;
            self.window_fps_start = Instant::now();
        }
        got_frame
    }

    fn buffer_fps(&self) -> (i32, i32) {
        if self.fps_n > 0 {
            (self.fps_n, self.fps_d.max(1))
        } else {
            (30, 1)
        }
    }

    fn connect_url(&mut self, url: String, addresses: Vec<String>) {
        let (quality, preview) = self.settings.quality.to_connect_parts();
        *self.prep_ctrl.selected_url.lock() = Some(url.clone());
        // Drop any prepared image from the previous source immediately.
        self.prep_ctrl.slot.store(None);
        self.prep_ctrl.notify();
        self.worker.connect_with(ConnectOptions {
            url,
            addresses,
            quality,
            preview,
        });
    }

    fn disconnect_source(&mut self) {
        self.selected = None;
        self.selected_addresses.clear();
        *self.prep_ctrl.selected_url.lock() = None;
        self.prep_ctrl.slot.store(None);
        self.prep_ctrl.notify();
        self.worker.disconnect();
        self.frame_w = 0;
        self.frame_h = 0;
        self.texture = None;
        self.status = t(self.language, "monitor.none").to_string();
    }

    fn select(&mut self, url: String, addresses: Vec<String>) {
        self.selected = Some(url.clone());
        self.selected_addresses = addresses.clone();
        self.connect_url(url, addresses);
        self.frames_presented = 0;
        self.frames_render_skipped = 0;
        self.frame_w = 0;
        self.frame_h = 0;
        self.last_frame_at = None;
        self.audio_frames = 0;
        self.audio_levels = AudioLevels::default();
        self.log_lines.clear();
        self.log_last_unix_ms = 0;
        self.last_bytes_received = 0;
        self.bitrate_bps = 0.0;
        self.decode_ms_avg = 0.0;
        self.decode_ms_peak = 0.0;
        self.reconnects = 0;
        self.wire_queue_depth = 0;
        self.session_state = SessionState::Connecting;
        self.zoom = 1.0;
        self.pan_x = 0.0;
        self.pan_y = 0.0;
        self.pan_drag = None;
        self.texture = None;
        self.status.clear();
    }

    fn reapply_connection(&mut self) {
        if let Some(url) = self.selected.clone() {
            self.connect_url(url, self.selected_addresses.clone());
        }
    }

    fn set_audio_boost_db(&mut self, db: i32) {
        self.settings.audio_boost_db = db;
        self.worker.set_audio_boost_db(db);
    }

    fn set_video_delay(&mut self, delay: DelaySetting) {
        let (fps_n, fps_d) = self.buffer_fps();
        self.settings.buffer.set_video(delay, fps_n, fps_d);
        self.worker.set_buffer(self.settings.buffer);
    }

    fn set_audio_delay(&mut self, delay: DelaySetting) {
        let (fps_n, fps_d) = self.buffer_fps();
        self.settings.buffer.set_audio(delay, fps_n, fps_d);
        self.worker.set_buffer(self.settings.buffer);
    }

    fn set_buffer_link(&mut self, linked: bool) {
        let (fps_n, fps_d) = self.buffer_fps();
        self.settings.buffer.set_linked(linked, fps_n, fps_d);
        self.worker.set_buffer(self.settings.buffer);
    }

    fn persist_suite_prefs(&self) {
        let mut cfg = load_config().unwrap_or_default();
        cfg.language = self.language;
        cfg.theme = self.theme;
        let _ = save_config(&cfg);
    }

    fn fit_display_in_viewport(&self, vw: f32, vh: f32) -> (f32, f32) {
        if self.frame_w == 0 || self.frame_h == 0 {
            return (vw.max(1.0), vh.max(1.0));
        }
        let fw = self.frame_w as f32;
        let fh = self.frame_h as f32;
        if vw / vh.max(1.0) > fw / fh {
            let dh = vh;
            let dw = dh * fw / fh;
            (dw.max(1.0), dh.max(1.0))
        } else {
            let dw = vw;
            let dh = dw * fh / fw;
            (dw.max(1.0), dh.max(1.0))
        }
    }

    fn fit_scale(&self) -> f32 {
        let (pw, ph) = if self.fullscreen {
            (self.preview_w.max(1.0), self.preview_h.max(1.0))
        } else if self.preview_w > 1.0 && self.preview_h > 1.0 {
            (self.preview_w, self.preview_h)
        } else {
            (900.0, 500.0)
        };
        if self.frame_w == 0 || self.frame_h == 0 {
            return 1.0;
        }
        (pw / self.frame_w as f32)
            .min(ph / self.frame_h as f32)
            .max(0.01)
    }

    fn display_size(&self) -> (f32, f32) {
        let fit = self.fit_scale();
        let scale = fit * self.zoom;
        (
            (self.frame_w as f32 * scale).max(1.0),
            (self.frame_h as f32 * scale).max(1.0),
        )
    }

    fn adjust_zoom(&mut self, delta_y: f32) {
        // smooth_scroll_delta is in points; normalize so one wheel notch ≈ 6% zoom.
        let notches = (delta_y / 80.0).clamp(-4.0, 4.0);
        let factor = (1.0 + 0.06 * notches).clamp(0.88, 1.12);
        self.zoom = (self.zoom * factor).clamp(ZOOM_MIN, ZOOM_MAX);
        if (self.zoom - 1.0).abs() < 0.01 {
            self.pan_x = 0.0;
            self.pan_y = 0.0;
        }
    }

    fn zoom_reset(&mut self) {
        self.zoom = 1.0;
        self.pan_x = 0.0;
        self.pan_y = 0.0;
    }

    fn enter_fullscreen(&mut self, ctx: &Context) {
        self.fullscreen = true;
        self.preferences_open = false;
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
    }

    fn exit_fullscreen(&mut self, ctx: &Context) {
        if self.fullscreen {
            self.fullscreen = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
        }
    }

    fn audio_unavailable(&self) -> bool {
        matches!(self.worker.audio().status(), AudioOutputStatus::Unavailable)
    }

    fn open_preferences(&mut self) {
        self.preferences_open = true;
        self.audio_devices = list_output_devices();
        self.audio_output_device = self.worker.audio_output_device();
        self.buffer_edit.sync_from(
            self.settings.buffer,
            self.video_buffer_delay_ms,
            self.audio_buffer_delay_ms,
            self.buffer_fps().0,
            self.buffer_fps().1,
        );
    }

    fn apply_prefs_action(&mut self, action: PrefsAction, ctx: &Context) {
        match action {
            PrefsAction::Close => self.preferences_open = false,
            PrefsAction::SetLanguage(lang) => {
                if self.language != lang {
                    self.language = lang;
                    self.persist_suite_prefs();
                }
            }
            PrefsAction::SetTheme(theme) => {
                if self.theme != theme {
                    self.theme = theme;
                    self.persist_suite_prefs();
                    self.last_theme_dark = None;
                }
            }
            PrefsAction::SetAudioDevice(name) => {
                self.audio_output_device = name.clone();
                self.worker.set_audio_output_device(name);
            }
            PrefsAction::SetVideoDelayFrames(frames) => {
                self.set_video_delay(DelaySetting {
                    amount: frames.min(120),
                    unit: BufferUnit::Frames,
                });
                let (fps_n, fps_d) = self.buffer_fps();
                self.video_buffer_delay_ms = self.settings.buffer.video_delay_ms(fps_n, fps_d);
                self.audio_buffer_delay_ms = self.settings.buffer.audio_delay_ms(fps_n, fps_d);
                self.buffer_edit.sync_from(
                    self.settings.buffer,
                    self.video_buffer_delay_ms,
                    self.audio_buffer_delay_ms,
                    fps_n,
                    fps_d,
                );
            }
            PrefsAction::SetAudioDelayMs(ms) => {
                self.set_audio_delay(DelaySetting {
                    amount: ms.min(2_000),
                    unit: BufferUnit::Milliseconds,
                });
                let (fps_n, fps_d) = self.buffer_fps();
                self.video_buffer_delay_ms = self.settings.buffer.video_delay_ms(fps_n, fps_d);
                self.audio_buffer_delay_ms = self.settings.buffer.audio_delay_ms(fps_n, fps_d);
                self.buffer_edit.sync_from(
                    self.settings.buffer,
                    self.video_buffer_delay_ms,
                    self.audio_buffer_delay_ms,
                    fps_n,
                    fps_d,
                );
            }
            PrefsAction::SetBufferLink(linked) => {
                self.set_buffer_link(linked);
                let (fps_n, fps_d) = self.buffer_fps();
                self.video_buffer_delay_ms = self.settings.buffer.video_delay_ms(fps_n, fps_d);
                self.audio_buffer_delay_ms = self.settings.buffer.audio_delay_ms(fps_n, fps_d);
                self.buffer_edit.sync_from(
                    self.settings.buffer,
                    self.video_buffer_delay_ms,
                    self.audio_buffer_delay_ms,
                    fps_n,
                    fps_d,
                );
            }
            PrefsAction::SetBoost(db) => self.set_audio_boost_db(db),
            PrefsAction::SetQuality(preset) => {
                self.settings.quality = preset;
                self.reapply_connection();
            }
            PrefsAction::SetAlpha(v) => {
                self.settings.show_alpha = v;
                self.prep_ctrl.set_alpha(v);
            }
            PrefsAction::SetSafeArea(v) => self.settings.safe_area = v,
            PrefsAction::SetVu(v) => self.settings.vu_meter = v,
            PrefsAction::EnterFullscreen => self.enter_fullscreen(ctx),
            PrefsAction::OpenHelp => {
                ctx.open_url(egui::OpenUrl::new_tab(
                    "https://github.com/MikanseiLaboratory/omt-tools#readme",
                ));
            }
            PrefsAction::OpenLicense => {
                ctx.open_url(egui::OpenUrl::new_tab(
                    "https://github.com/MikanseiLaboratory/omt-tools/blob/main/LICENSE",
                ));
            }
            PrefsAction::Exit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
        }
    }

    fn apply_theme_if_needed(&mut self, ctx: &Context) {
        let system_dark = matches!(ctx.system_theme(), Some(egui::Theme::Dark));
        let dark = match self.theme {
            ThemePreference::Dark => true,
            ThemePreference::Light => false,
            ThemePreference::System => system_dark,
        };
        if self.last_theme_dark != Some(dark) {
            UiChrome::resolve(self.theme, system_dark).apply_to_context(ctx, dark);
            self.last_theme_dark = Some(dark);
        }
    }

    fn chrome(&self, ctx: &Context) -> UiChrome {
        let system_dark = matches!(ctx.system_theme(), Some(egui::Theme::Dark));
        UiChrome::resolve(self.theme, system_dark)
    }

    fn persist_monitor_layout(&self) {
        let cfg = StudioMonitorConfig {
            schema_version: 1,
            sidebar_w: clamp_sidebar_w(self.sidebar_w).round() as u32,
            stats_w: clamp_stats_w(self.stats_w).round() as u32,
            log_h: clamp_log_h(self.log_h).round() as u32,
            stats_video_open: self.stats_video_open,
            stats_audio_open: self.stats_audio_open,
            stats_source_open: self.stats_source_open,
        };
        let _ = save_studio_monitor_config(&cfg);
    }
}

impl eframe::App for MonitorApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.apply_theme_if_needed(&ctx);
        let got_frame = self.on_tick(&ctx);

        // Escape / F11 fullscreen handling
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if self.preferences_open {
                self.preferences_open = false;
            } else if self.fullscreen {
                self.exit_fullscreen(&ctx);
            }
        }
        if ctx.input(|i| i.key_pressed(egui::Key::F11)) {
            if self.fullscreen {
                self.exit_fullscreen(&ctx);
            } else {
                self.enter_fullscreen(&ctx);
            }
        }

        let chrome = self.chrome(&ctx);
        let connected = self.selected.is_some();

        if self.fullscreen {
            self.ui_fullscreen(ui, &ctx, chrome);
        } else {
            self.ui_windowed(ui, chrome);
        }

        if self.preferences_open
            && let Some(action) = preferences::show(
                &ctx,
                self.language,
                self.theme,
                chrome,
                &self.suite_version,
                &self.settings,
                &self.audio_devices,
                self.audio_output_device.as_deref(),
                self.audio_unavailable(),
                self.settings.buffer,
                self.video_buffer_delay_ms,
                self.audio_buffer_delay_ms,
                self.buffer_fps().0,
                self.buffer_fps().1,
                &mut self.buffer_edit,
            )
        {
            self.apply_prefs_action(action, &ctx);
        }

        // Repaint when a prepared frame arrives (prep thread also requests).
        // VU meters only need ~30 Hz — continuous full-rate paints starve the GPU path.
        if got_frame || self.preferences_open {
            ctx.request_repaint();
        } else if connected && self.settings.vu_meter {
            ctx.request_repaint_after(Duration::from_millis(33));
        } else if connected || self.fullscreen {
            ctx.request_repaint_after(Duration::from_millis(16));
        } else {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
}

fn clamp_sidebar_w(width: f32) -> f32 {
    if !width.is_finite() || width <= 0.0 {
        return SIDEBAR_W;
    }
    width.clamp(SIDEBAR_MIN_W, SIDEBAR_MAX_W)
}

fn clamp_stats_w(width: f32) -> f32 {
    if !width.is_finite() || width <= 0.0 {
        return STATS_W;
    }
    width.clamp(STATS_MIN_W, STATS_MAX_W)
}

fn clamp_log_h(height: f32) -> f32 {
    if !height.is_finite() || height <= 0.0 {
        return LOG_H;
    }
    height.clamp(LOG_MIN_H, LOG_MAX_H)
}

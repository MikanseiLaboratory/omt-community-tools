//! egui/eframe Studio Monitor application.

mod panels;

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use anyhow::Result;
use eframe::egui::{self, ColorImage, Context, Pos2, TextureHandle, TextureId, TextureOptions};
use eframe::egui_wgpu::{RenderState, WgpuSetup, wgpu};
use omt_media::{
    AudioLevels, AudioOutputDevice, AudioOutputStatus, BufferUnit, ConnectOptions, DelaySetting,
    DiscoveredSource, GpuVideoContext, ReceiveWorker, SessionState, StallState, VideoDecodePath,
    list_output_devices, spawn_discover,
};
use suite_core::{
    Language, OutputDisplayRef, SUITE_VERSION, SimdCapabilities, StudioMonitorConfig,
    ThemePreference, install_egui_cjk_fonts, load_config, load_studio_monitor_config, save_config,
    save_studio_monitor_config, t,
};

use crate::chrome::UiChrome;
use crate::frame_prep::{FramePrep, PrepControl};
use crate::output::{self, DisplayInfo, OutputError, OutputSnapshot, OutputStatus, OutputVideo};
use crate::preferences::{self, BufferEditState, PrefsAction};
use crate::settings::{self, MonitorSettings};

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

fn wgpu_options_with_bgra_storage() -> eframe::WgpuConfiguration {
    let mut config = eframe::WgpuConfiguration::default();
    if let WgpuSetup::CreateNew(create) = &mut config.wgpu_setup {
        let inner = Arc::clone(&create.device_descriptor);
        create.device_descriptor = Arc::new(move |adapter| {
            let mut desc = inner(adapter);
            if adapter
                .features()
                .contains(wgpu::Features::BGRA8UNORM_STORAGE)
            {
                desc.required_features |= wgpu::Features::BGRA8UNORM_STORAGE;
            }
            desc
        });
    }
    config
}

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
        wgpu_options: wgpu_options_with_bgra_storage(),
        ..Default::default()
    };
    eframe::run_native(
        &title,
        options,
        Box::new(move |cc| {
            install_egui_cjk_fonts(&cc.egui_ctx);
            Ok(Box::new(MonitorApp::new(cc, language, theme, initial_url)))
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
    audio_active_device: Option<String>,
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
    native_video_id: Option<TextureId>,
    held_gpu_texture: Option<wgpu::Texture>,
    wgpu_state: Option<RenderState>,
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
    /// Decode backend reported by the receive worker (`None` while idle).
    active_video_decode: Option<VideoDecodePath>,
    pan_x: f32,
    pan_y: f32,
    pan_drag: Option<Pos2>,
    log_lines: VecDeque<String>,
    log_last_unix_ms: u64,
    stall_text: String,
    settings: MonitorSettings,
    buffer_edit: BufferEditState,
    fullscreen: bool,
    /// eframe skips `ui` while minimized/occluded; ingest still runs in `logic`.
    window_hidden: bool,
    last_got_frame: bool,
    last_theme_dark: Option<bool>,
    simd_summary: String,
    sidebar_w: f32,
    stats_w: f32,
    log_h: f32,
    stats_video_open: bool,
    stats_audio_open: bool,
    stats_source_open: bool,
    /// Displays reported by the OS, refreshed periodically.
    displays: Vec<DisplayInfo>,
    /// Index of the display that currently holds the operator window.
    operator_display: Option<usize>,
    displays_refreshed_at: Instant,
    displays_dirty: bool,
    /// Whether the clean-feed viewport should be shown.
    output_open: bool,
    /// Resolved display index for the output window.
    output_monitor: Option<usize>,
    /// Display the output was started on, used to detect reordering / unplug.
    output_target: Option<OutputTarget>,
    output_error: Option<OutputError>,
    output_snapshot: Arc<parking_lot::Mutex<OutputSnapshot>>,
    /// Frames left to pull focus back to the operator window after the clean
    /// feed is created or destroyed.
    output_focus_reclaim: u8,
    /// Latest stall state, used by the output status overlay.
    stall_state: StallState,
    /// Row-padded staging for `queue.write_texture` (256-byte alignment).
    cpu_upload_pad: Vec<u8>,
}

/// Display the clean feed was started on.
#[derive(Debug, Clone, PartialEq, Eq)]
struct OutputTarget {
    index: usize,
    name: Option<String>,
}

impl MonitorApp {
    fn new(
        cc: &eframe::CreationContext<'_>,
        language: Language,
        theme: ThemePreference,
        initial_url: Option<String>,
    ) -> Self {
        let ctx = &cc.egui_ctx;
        let layout = load_studio_monitor_config().unwrap_or_default();
        let wgpu_state = cc.wgpu_render_state.clone();
        let gpu = wgpu_state.as_ref().map(|state| GpuVideoContext {
            device: Arc::new(state.device.clone()),
            queue: Arc::new(state.queue.clone()),
            gpu_lock: None,
        });
        let worker = ReceiveWorker::spawn();
        worker.set_gpu(gpu);
        let mut settings = MonitorSettings::default();
        settings.video_decode = settings::decode_path_from_config(layout.video_decode);
        settings.buffer.mode = settings::playout_mode_from_config(layout.playout_mode);
        settings.output = layout.output.clone();
        worker.set_buffer(settings.buffer);
        worker.set_audio_boost_db(settings.audio_boost_db);
        worker.set_audio_volume_pct(settings.audio_volume_pct);
        if let Some(url) = &initial_url {
            let (quality, preview) = settings.quality.to_connect_parts();
            worker.connect_with(ConnectOptions {
                url: url.clone(),
                addresses: Vec::new(),
                quality,
                preview,
                video_decode: settings.video_decode,
            });
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
            audio_active_device: None,
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
            native_video_id: None,
            held_gpu_texture: None,
            wgpu_state,
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
            active_video_decode: None,
            pan_x: 0.0,
            pan_y: 0.0,
            pan_drag: None,
            log_lines: VecDeque::new(),
            log_last_unix_ms: 0,
            stall_text: t(language, "monitor.waiting").to_string(),
            settings,
            buffer_edit: BufferEditState::default(),
            fullscreen: false,
            window_hidden: false,
            last_got_frame: false,
            last_theme_dark: None,
            simd_summary: SimdCapabilities::detect().summary(),
            sidebar_w: clamp_sidebar_w(layout.sidebar_w as f32),
            stats_w: clamp_stats_w(layout.stats_w as f32),
            log_h: clamp_log_h(layout.log_h as f32),
            stats_video_open: layout.stats_video_open,
            stats_audio_open: layout.stats_audio_open,
            stats_source_open: layout.stats_source_open,
            displays: Vec::new(),
            operator_display: None,
            displays_refreshed_at: Instant::now() - Duration::from_secs(10),
            displays_dirty: true,
            output_open: false,
            output_monitor: None,
            output_target: None,
            output_error: None,
            output_snapshot: Arc::new(parking_lot::Mutex::new(OutputSnapshot {
                language,
                ..Default::default()
            })),
            output_focus_reclaim: 0,
            stall_state: StallState::Waiting,
            cpu_upload_pad: Vec::new(),
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
            self.log_lines.push_back(line);
            while self.log_lines.len() > 128 {
                self.log_lines.pop_front();
            }
        }
    }

    fn ingest_prepared_frame(&mut self, ctx: &Context) -> bool {
        let Some(frame) = self.prep_ctrl.take_prepared() else {
            return false;
        };
        self.frame_w = frame.width;
        self.frame_h = frame.height;
        self.fps_n = frame.fps_n;
        self.fps_d = frame.fps_d.max(1);
        self.window_fps_count += 1;
        self.last_frame_at = Some(Instant::now());

        if self.upload_cpu_pixels_native(&frame.pixels, frame.width, frame.height, frame.rgba) {
            self.texture = None;
        } else {
            self.clear_native_video();
            let image =
                color_image_from_pixels(&frame.pixels, frame.width, frame.height, frame.rgba);
            let size = image.size;
            match &mut self.texture {
                Some(tex) if tex.size() == size => {
                    tex.set(image, TextureOptions::LINEAR);
                }
                _ => {
                    self.texture =
                        Some(ctx.load_texture("omt-video", image, TextureOptions::LINEAR));
                }
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

    fn upload_cpu_pixels_native(
        &mut self,
        pixels: &[u8],
        width: u32,
        height: u32,
        rgba: bool,
    ) -> bool {
        let Some(state) = self.wgpu_state.clone() else {
            return false;
        };
        if width == 0 || height == 0 {
            return false;
        }
        let expected = (width as usize)
            .saturating_mul(height as usize)
            .saturating_mul(4);
        if pixels.len() < expected {
            return false;
        }
        let format = if rgba {
            wgpu::TextureFormat::Rgba8Unorm
        } else {
            wgpu::TextureFormat::Bgra8Unorm
        };
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let recreate = match self.held_gpu_texture.as_ref() {
            Some(tex) => {
                tex.size().width != width || tex.size().height != height || tex.format() != format
            }
            None => true,
        };
        if recreate {
            self.held_gpu_texture = Some(state.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("omt-cpu-video"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            }));
        }
        let src_row = width.saturating_mul(4);
        let padded_row = padded_bytes_per_row(width);
        let src = &pixels[..expected];
        let padded = padded_row != src_row;
        if padded {
            let padded_len = padded_row as usize * height as usize;
            if self.cpu_upload_pad.len() != padded_len {
                self.cpu_upload_pad.resize(padded_len, 0);
            }
            for y in 0..height as usize {
                let src_off = y * src_row as usize;
                let dst_off = y * padded_row as usize;
                self.cpu_upload_pad[dst_off..dst_off + src_row as usize]
                    .copy_from_slice(&src[src_off..src_off + src_row as usize]);
            }
        }
        let view = {
            let Some(tex) = self.held_gpu_texture.as_ref() else {
                return false;
            };
            let data: &[u8] = if padded { &self.cpu_upload_pad } else { src };
            state.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(if padded { padded_row } else { src_row }),
                    rows_per_image: Some(height),
                },
                size,
            );
            tex.create_view(&wgpu::TextureViewDescriptor::default())
        };
        let mut renderer = state.renderer.write();
        if recreate {
            if let Some(id) = self.native_video_id.take() {
                renderer.free_texture(&id);
            }
            self.native_video_id = Some(renderer.register_native_texture(
                &state.device,
                &view,
                wgpu::FilterMode::Linear,
            ));
        } else if let Some(id) = self.native_video_id {
            renderer.update_egui_texture_from_wgpu_texture(
                &state.device,
                &view,
                wgpu::FilterMode::Linear,
                id,
            );
        } else {
            self.native_video_id = Some(renderer.register_native_texture(
                &state.device,
                &view,
                wgpu::FilterMode::Linear,
            ));
        }
        true
    }

    fn ingest_gpu_frame(&mut self) -> bool {
        let Some(frame) = self.worker.latest().take_gpu() else {
            return false;
        };
        let Some(state) = self.wgpu_state.as_ref() else {
            return false;
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        {
            let mut renderer = state.renderer.write();
            match self.native_video_id {
                Some(id) => renderer.update_egui_texture_from_wgpu_texture(
                    &state.device,
                    &view,
                    wgpu::FilterMode::Linear,
                    id,
                ),
                None => {
                    self.native_video_id = Some(renderer.register_native_texture(
                        &state.device,
                        &view,
                        wgpu::FilterMode::Linear,
                    ));
                }
            }
        }
        self.held_gpu_texture = Some(frame.texture);
        self.texture = None;
        self.frame_w = frame.width;
        self.frame_h = frame.height;
        self.fps_n = frame.frame_rate_n;
        self.fps_d = frame.frame_rate_d.max(1);
        self.window_fps_count += 1;
        self.last_frame_at = Some(Instant::now());
        self.frames_presented = self.frames_presented.saturating_add(1);
        true
    }

    fn clear_native_video(&mut self) {
        if let (Some(state), Some(id)) = (self.wgpu_state.as_ref(), self.native_video_id.take()) {
            state.renderer.write().free_texture(&id);
        }
        self.held_gpu_texture = None;
    }

    fn rebind_held_gpu_texture(&mut self) {
        let Some(tex) = self.held_gpu_texture.as_ref() else {
            return;
        };
        let Some(state) = self.wgpu_state.as_ref() else {
            return;
        };
        let Some(id) = self.native_video_id else {
            return;
        };
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        state
            .renderer
            .write()
            .update_egui_texture_from_wgpu_texture(
                &state.device,
                &view,
                wgpu::FilterMode::Linear,
                id,
            );
    }

    pub(crate) fn has_video(&self) -> bool {
        (self.native_video_id.is_some() || self.texture.is_some())
            && self.frame_w > 0
            && self.frame_h > 0
    }

    pub(crate) fn video_texture_id(&self) -> Option<TextureId> {
        self.native_video_id
            .or_else(|| self.texture.as_ref().map(|tex| tex.id()))
    }

    fn on_tick(&mut self, ctx: &Context, ingest_gpu: bool) -> bool {
        self.poll_discovery();
        if !self.discovering && self.last_refresh.elapsed() > Duration::from_secs(3) {
            self.request_refresh(true);
        }
        self.ingest_logs();
        let mut got_frame = false;
        if ingest_gpu {
            got_frame = self.ingest_gpu_frame();
            if got_frame {
                self.prep_ctrl.slot.store(None);
            }
        }
        if !got_frame {
            got_frame = self.ingest_prepared_frame(ctx);
        }

        {
            let counters = *self.worker.latest().counters.lock();
            let audio_levels = *self.worker.latest().audio_levels.lock();
            let video_buffer_delay_ms = *self.worker.latest().video_buffer_delay_ms.lock();
            let audio_buffer_delay_ms = *self.worker.latest().audio_buffer_delay_ms.lock();
            let stats = *self.worker.latest().stats.lock();
            let session_state = *self.worker.latest().session_state.lock();
            let video_decode = *self.worker.latest().video_decode.lock();

            self.frames_decoded = counters.frames_decoded;
            self.source_dropped = counters.frames_replaced;
            self.audio_frames = counters.audio_frames;
            self.audio_levels = audio_levels;
            self.video_buffer_delay_ms = video_buffer_delay_ms;
            self.audio_buffer_delay_ms = audio_buffer_delay_ms;
            self.session_state = session_state;
            self.active_video_decode = video_decode;
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
                self.status = match self.worker.latest().error.lock().clone() {
                    Some(err) if !err.is_empty() => {
                        format!("Reconnecting… ({}) — {err}", self.reconnects)
                    }
                    _ => format!("Reconnecting… ({})", self.reconnects),
                };
            }
            SessionState::Connected => {
                if self.status.starts_with("Connecting") || self.status.starts_with("Reconnecting")
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
            let state = d.tick();
            self.stall_state = state;
            self.stall_text = match state {
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
            video_decode: self.settings.video_decode,
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
        self.clear_native_video();
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
        self.active_video_decode = None;
        self.zoom = 1.0;
        self.pan_x = 0.0;
        self.pan_y = 0.0;
        self.pan_drag = None;
        self.texture = None;
        self.clear_native_video();
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

    fn set_audio_volume_pct(&mut self, pct: i32) {
        self.settings.audio_volume_pct = pct.clamp(0, 100);
        self.worker
            .set_audio_volume_pct(self.settings.audio_volume_pct);
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

    /// Re-scan the displays attached to `frame`'s window.
    fn refresh_displays(&mut self, frame: &eframe::Frame) {
        self.displays_refreshed_at = Instant::now();
        self.displays_dirty = false;
        let Some(window) = frame.winit_window() else {
            return;
        };
        let (displays, operator) = output::scan_displays(window);
        self.displays = displays;
        self.operator_display = operator;
    }

    /// Stop the output when its display disappears or becomes the operator
    /// display, and follow the display when the OS reorders monitors.
    fn revalidate_output_target(&mut self, ctx: &Context) {
        let Some(target) = self.output_target.clone() else {
            self.stop_output(ctx);
            return;
        };
        let found = self
            .displays
            .iter()
            .find(|display| match target.name.as_deref() {
                Some(name) => display.name.as_deref() == Some(name),
                None => display.index == target.index,
            });
        let Some(display) = found else {
            self.fail_output(OutputError::Missing, ctx);
            return;
        };
        if display.is_operator {
            self.fail_output(OutputError::SameAsOperator, ctx);
            return;
        }
        if display.index != target.index {
            self.output_target = Some(OutputTarget {
                index: display.index,
                name: display.name.clone(),
            });
            self.output_monitor = Some(display.index);
            ctx.request_repaint();
        }
    }

    fn output_status(&self) -> OutputStatus {
        match self.session_state {
            SessionState::Stopped | SessionState::Stopping => OutputStatus::Idle,
            SessionState::Connecting => OutputStatus::Connecting,
            SessionState::Reconnecting => OutputStatus::Reconnecting,
            SessionState::Connected => match self.stall_state {
                StallState::Waiting => OutputStatus::Idle,
                StallState::Live => OutputStatus::Live,
                StallState::Stalled => OutputStatus::Stalled,
            },
        }
    }

    /// Start the output window. Returns `false` when no usable display exists.
    fn start_output(&mut self, ctx: &Context) -> bool {
        if self.output_open {
            return true;
        }
        let resolved =
            output::resolve_target(&self.displays, self.settings.output.display.as_ref());
        let index = match resolved {
            Ok(index) => index,
            Err(error) => {
                self.output_error = Some(error);
                return false;
            }
        };
        let name = self
            .displays
            .get(index)
            .and_then(|display| display.name.clone());
        self.output_target = Some(OutputTarget { index, name });
        self.output_monitor = Some(index);
        self.output_open = true;
        self.output_error = None;
        // Creating a borderless-fullscreen viewport can activate it on some
        // platforms; pull the operator window back over the next few frames.
        self.output_focus_reclaim = 3;
        {
            let mut snapshot = self.output_snapshot.lock();
            snapshot.close_requested = false;
            snapshot.video = None;
        }
        if self.settings.show_alpha {
            // The alpha-mask diagnostic is baked into the shared frame texture,
            // so it would leak into the clean feed.
            self.settings.show_alpha = false;
            self.prep_ctrl.set_alpha(false);
        }
        ctx.request_repaint();
        true
    }

    fn stop_output(&mut self, ctx: &Context) {
        self.output_open = false;
        self.output_monitor = None;
        self.output_target = None;
        self.output_error = None;
        // The destroyed fullscreen window can hand focus to another app.
        self.output_focus_reclaim = 1;
        {
            let mut snapshot = self.output_snapshot.lock();
            snapshot.close_requested = false;
            snapshot.video = None;
        }
        // The output viewport hid the OS cursor while it was alive; make sure
        // the operator window asks for it back.
        ctx.send_viewport_cmd(egui::ViewportCommand::CursorVisible(true));
        ctx.request_repaint();
    }

    fn fail_output(&mut self, error: OutputError, ctx: &Context) {
        self.stop_output(ctx);
        self.output_error = Some(error);
    }

    fn toggle_output(&mut self, ctx: &Context) {
        if self.output_open {
            self.stop_output(ctx);
            return;
        }
        if !self.start_output(ctx) {
            self.displays_dirty = true;
            self.open_preferences();
        }
    }

    /// Apply a new output display choice, moving a running output window.
    fn set_output_display(&mut self, target: Option<OutputDisplayRef>, ctx: &Context) {
        match output::resolve_target(&self.displays, target.as_ref()) {
            Ok(index) => {
                self.settings.output.display = target;
                self.output_error = None;
                self.persist_monitor_layout();
                if self.output_open {
                    let name = self
                        .displays
                        .get(index)
                        .and_then(|display| display.name.clone());
                    self.output_target = Some(OutputTarget { index, name });
                    self.output_monitor = Some(index);
                }
                ctx.request_repaint();
            }
            Err(error) => {
                self.output_error = Some(error);
                self.displays_dirty = true;
                ctx.request_repaint();
            }
        }
    }

    /// Keep the output viewport registered and its snapshot up to date.
    fn sync_output(&mut self, ctx: &Context) {
        if !self.output_open {
            return;
        }
        if self.output_snapshot.lock().take_close_request() {
            self.stop_output(ctx);
            return;
        }

        self.revalidate_output_target(ctx);
        if !self.output_open {
            return;
        }
        let Some(monitor) = self.output_monitor else {
            self.stop_output(ctx);
            return;
        };

        let status = self.output_status();
        let status_text = status
            .warning_key()
            .map(|key| t(self.language, key))
            .unwrap_or("");
        {
            let mut snapshot = self.output_snapshot.lock();
            snapshot.video = self
                .video_texture_id()
                .filter(|_| self.frame_w > 0 && self.frame_h > 0)
                .map(|texture| OutputVideo {
                    texture,
                    width: self.frame_w,
                    height: self.frame_h,
                });
            snapshot.audio = self.audio_levels;
            snapshot.status = status;
            snapshot.status_text = status_text;
            snapshot.language = self.language;
            snapshot.overlays = self.settings.output.overlays;
        }

        let title = t(self.language, "monitor.output_window_title").to_string();
        let builder = output::viewport_builder(&title, monitor);
        let shared = Arc::clone(&self.output_snapshot);
        ctx.show_viewport_deferred(output::viewport_id(), builder, move |ui, class| {
            output::draw_viewport(ui, class, shared.as_ref());
        });
        ctx.request_repaint_of(output::viewport_id());
    }

    fn audio_unavailable(&self) -> bool {
        matches!(self.worker.audio().status(), AudioOutputStatus::Unavailable)
    }

    fn open_preferences(&mut self) {
        self.preferences_open = true;
        self.displays_dirty = true;
        self.audio_devices = list_output_devices();
        self.audio_output_device = self.worker.audio_output_device();
        self.audio_active_device = self.worker.audio().active_device_name();
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
            PrefsAction::SetPlayoutMode(mode) => {
                self.settings.buffer.mode = mode;
                self.worker.set_buffer(self.settings.buffer);
                self.persist_monitor_layout();
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
            PrefsAction::SetVolume(pct) => self.set_audio_volume_pct(pct),
            PrefsAction::SetQuality(preset) => {
                self.settings.quality = preset;
                self.reapply_connection();
            }
            PrefsAction::SetVideoDecode(path) => {
                if self.settings.video_decode != path {
                    self.settings.video_decode = path;
                    self.persist_monitor_layout();
                    self.reapply_connection();
                }
            }
            PrefsAction::SetAlpha(v) => {
                self.settings.show_alpha = v;
                self.prep_ctrl.set_alpha(v);
            }
            PrefsAction::SetSafeArea(v) => self.settings.safe_area = v,
            PrefsAction::SetVu(v) => self.settings.vu_meter = v,
            PrefsAction::SetOutputEnabled(enabled) => {
                if enabled {
                    if !self.start_output(ctx) {
                        self.displays_dirty = true;
                    }
                } else {
                    self.stop_output(ctx);
                }
            }
            PrefsAction::SetOutputDisplay(target) => self.set_output_display(target, ctx),
            PrefsAction::SetOutputVu(v) => {
                self.settings.output.overlays.vu = v;
                self.persist_monitor_layout();
            }
            PrefsAction::SetOutputSafeArea(v) => {
                self.settings.output.overlays.safe_area = v;
                self.persist_monitor_layout();
            }
            PrefsAction::SetOutputTally(v) => {
                self.settings.output.overlays.tally = v;
                self.persist_monitor_layout();
            }
            PrefsAction::SetOutputWarnings(v) => {
                self.settings.output.overlays.warnings = v;
                self.persist_monitor_layout();
            }
            PrefsAction::RefreshDisplays => {
                self.displays_dirty = true;
                ctx.request_repaint();
            }
            PrefsAction::EnterFullscreen => self.enter_fullscreen(ctx),
            PrefsAction::OpenHelp => {
                ctx.open_url(egui::OpenUrl::new_tab(
                    "https://github.com/MikanseiLaboratory/omt-community-tools#readme",
                ));
            }
            PrefsAction::OpenLicense => {
                ctx.open_url(egui::OpenUrl::new_tab(
                    "https://github.com/MikanseiLaboratory/omt-community-tools/blob/main/LICENSE",
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
            video_decode: settings::decode_path_to_config(self.settings.video_decode),
            playout_mode: settings::playout_mode_to_config(self.settings.buffer.mode),
            output: self.settings.output.clone(),
        };
        let _ = save_studio_monitor_config(&cfg);
    }
}

impl eframe::App for MonitorApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // While the window is occluded, eframe skips `ui` / present and only
        // calls this. Video ingest used to live in `ui`, so the picture froze
        // until the window was shown again (audio kept playing on its thread).
        self.apply_theme_if_needed(ctx);
        let hidden = ctx.input(|i| i.viewport().visible() == Some(false));
        let became_visible = self.window_hidden && !hidden;
        self.window_hidden = hidden;
        // Pause wgpu copies in the receive worker while occluded. The previous
        // 16ms hidden repaint still submitted GPU work with no present, which
        // wedged the shared device and made the OMT sockets flap (Reconnects).
        // Keep ingesting while the output window is visible, even if the
        // operator window is minimized.
        self.worker.set_gpu_ingest(!hidden || self.output_open);
        if became_visible {
            self.rebind_held_gpu_texture();
            ctx.request_repaint();
        }
        self.last_got_frame = self.on_tick(ctx, !hidden);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let got_frame = self.last_got_frame;

        // Displays are polled: winit 0.30 has no monitor hot-plug event.
        if self.displays_dirty || self.displays_refreshed_at.elapsed() >= Duration::from_secs(2) {
            self.refresh_displays(frame);
        }

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
        if !self.preferences_open && ctx.input(|i| i.key_pressed(egui::Key::F9)) {
            self.toggle_output(&ctx);
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
                preferences::PrefsContext {
                    language: self.language,
                    theme: self.theme,
                    chrome,
                    suite_version: &self.suite_version,
                    settings: &self.settings,
                    audio_devices: &self.audio_devices,
                    selected_audio: self.audio_output_device.as_deref(),
                    active_audio: self.audio_active_device.as_deref(),
                    audio_unavailable: self.audio_unavailable(),
                    gpu_available: self.worker.gpu_available(),
                    buffer: self.settings.buffer,
                    video_delay_ms: self.video_buffer_delay_ms,
                    audio_delay_ms: self.audio_buffer_delay_ms,
                    fps_n: self.buffer_fps().0,
                    fps_d: self.buffer_fps().1,
                    displays: &self.displays,
                    operator_display: self.operator_display,
                    output_running: self.output_open,
                    output_monitor: self.output_monitor,
                    output_error: self.output_error,
                },
                &mut self.buffer_edit,
            )
        {
            self.apply_prefs_action(action, &ctx);
        }

        // Keep the output viewport registered even while the operator window
        // is minimized: eframe runs the root UI while a visible child exists.
        self.sync_output(&ctx);

        // Reclaim the operator window after the clean feed viewport appears or
        // disappears. `Focus` brings the root window to the front on desktop.
        if self.output_focus_reclaim > 0 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            self.output_focus_reclaim = self.output_focus_reclaim.saturating_sub(1);
            ctx.request_repaint();
        }

        // New video already wakes the event loop from the prep thread.
        // A 16 ms connected timer was tessellating the full CJK chrome at
        // 60 Hz even when the picture did not change, which starved other apps.
        if got_frame {
            ctx.request_repaint();
        } else if self.preferences_open || connected || self.fullscreen || self.output_open {
            ctx.request_repaint_after(Duration::from_millis(50));
        } else {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }
}

fn padded_bytes_per_row(width: u32) -> u32 {
    let row = width.saturating_mul(4);
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    row.div_ceil(align) * align
}

fn color_image_from_pixels(pixels: &[u8], width: u32, height: u32, rgba: bool) -> ColorImage {
    let expected = (width as usize)
        .saturating_mul(height as usize)
        .saturating_mul(4);
    let src = if pixels.len() >= expected {
        &pixels[..expected]
    } else {
        pixels
    };
    let size = [width as usize, height as usize];
    if rgba {
        ColorImage::from_rgba_premultiplied(size, src)
    } else {
        let mut rgba_buf = vec![0u8; expected];
        omt_media::bgra_to_rgba_into(src, &mut rgba_buf);
        ColorImage::from_rgba_premultiplied(size, &rgba_buf)
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

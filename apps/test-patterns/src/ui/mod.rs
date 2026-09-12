//! GPUI Test Patterns UI — pattern grid, send settings, preview, host stats.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Result;
use gpui::{
    App, Application, Bounds, Context, CursorStyle, FocusHandle, Focusable, FontWeight,
    InteractiveElement, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    RenderImage, SharedString, Timer, Window, WindowBounds, WindowOptions, div, prelude::*, px,
    rgb, size,
};
use omt_media::{
    AudioToneConfig, MAX_VIDEO_FRAME_BUFFER_FRAMES, MIN_VIDEO_FRAME_BUFFER_FRAMES, SendSession,
    SendSessionConfig, SendStats, clamp_video_frame_buffer_frames, uniquify_local_source_name,
};
use openmediatransport::{Quality, uyvy_to_rgba};
use parking_lot::Mutex;
use pattern_generator::{PatternKind, fill_uyvy, scroll_uyvy, uyvy_from_image_path};
use suite_core::{
    Language, TestPatternsConfig, load_test_patterns_config, reveal_in_file_manager,
    save_test_patterns_config, t,
};

mod presets;
mod widgets;
use presets::*;
use widgets::*;

/// Frames for one full scroll cycle at ±100% animation speed.
const ANIM_BASE_CYCLE_FRAMES: f32 = 300.0;

/// Advance an independent scroll phase without resetting on speed changes.
fn advance_scroll_phase(phase: f32, speed_pct: f32) -> f32 {
    let step = speed_pct.clamp(-200.0, 200.0) / 100.0 / ANIM_BASE_CYCLE_FRAMES;
    (phase + step).rem_euclid(1.0)
}

/// Live pattern pixels / animation, shared with the send provider.
struct LivePattern {
    kind: PatternKind,
    image_uyvy: Option<Arc<Vec<u8>>>,
    animate: bool,
    speed_h: f32,
    speed_v: f32,
    phase_x: f32,
    phase_y: f32,
}

impl LivePattern {
    fn from_view(
        kind: PatternKind,
        animate: bool,
        speed_h_pct: i32,
        speed_v_pct: i32,
        image_uyvy: Option<Arc<Vec<u8>>>,
    ) -> Self {
        Self {
            kind,
            image_uyvy,
            animate,
            speed_h: speed_h_pct.clamp(-200, 200) as f32 / 100.0,
            speed_v: speed_v_pct.clamp(-200, 200) as f32 / 100.0,
            phase_x: 0.0,
            phase_y: 0.0,
        }
    }
}
fn persist_test_patterns_prefs(view: &PatternsView) {
    let cfg = TestPatternsConfig {
        schema_version: 1,
        custom_images: view
            .custom_images
            .iter()
            .map(|img| img.path.clone())
            .collect(),
        frame_buffer_frames: clamp_video_frame_buffer_frames(view.frame_buffer_frames) as u32,
        name: view.name.clone(),
        width: view.width,
        height: view.height,
        fps_n: view.frame_rate.n,
        fps_d: view.frame_rate.d,
        quality: quality_to_config(view.quality),
        animate: view.animate,
        anim_speed_h_pct: view.anim_speed_h_pct,
        anim_speed_v_pct: view.anim_speed_v_pct,
        tone_hz: view.tone_hz,
        level_dbfs: view.level_dbfs,
        side_panel_w: clamp_side_panel_w(view.side_panel_w).round() as u32,
        stats_open: view.stats_open,
        settings_open: view.settings_open,
    };
    let _ = save_test_patterns_config(&cfg.sanitized());
}

pub fn run_gpui(title: String, language: Language) -> Result<()> {
    Application::new().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1100.0), px(780.0)), cx);
        let title = SharedString::from(title);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some(title.clone()),
                    ..Default::default()
                }),
                app_id: Some(suite_core::ToolId::TestPatterns.binary_name().into()),
                ..Default::default()
            },
            move |_, cx| cx.new(|cx| PatternsView::new(cx, language)),
        )
        .expect("open GPUI Test Patterns window");
        cx.activate(true);
    });
    Ok(())
}

struct PatternsView {
    language: Language,
    name: String,
    /// Advertised name for the current send (`Name (2)` when the host already has `Name`).
    on_air_name: Option<String>,
    kind: PatternKind,
    width: i32,
    height: i32,
    frame_rate: FrameRate,
    quality: Quality,
    animate: bool,
    anim_speed_h_pct: i32,
    anim_speed_v_pct: i32,
    /// Tone frequency in Hz; `0` means mute.
    tone_hz: f32,
    level_dbfs: f32,
    sample_rate: i32,
    channels: i32,
    samples: i32,
    /// Prefetch depth for paced video sends (persisted).
    frame_buffer_frames: u32,
    custom_images: Vec<CustomImage>,
    selected_custom: Option<usize>,
    session: Option<SendSession>,
    live: Arc<Mutex<LivePattern>>,
    last_stats: SendStats,
    error: Option<SharedString>,
    thumbs: Vec<(PatternKind, Option<Arc<RenderImage>>)>,
    preview: Option<Arc<RenderImage>>,
    /// Preview-sized UYVY cache for the selected custom image (scroll source).
    preview_image_uyvy: Option<Arc<Vec<u8>>>,
    preview_phase_x: f32,
    preview_phase_y: f32,
    last_preview_at: Instant,
    window_title: SharedString,
    /// Open dropdown and its window-relative left anchor (survives control-bar wrap).
    open_menu: Option<(MenuKind, f32)>,
    /// Right-click menu: image index and window-relative anchor.
    custom_menu: Option<(usize, f32, f32)>,
    /// Pending native file dialog result (must not block the GPUI UI thread).
    image_pick_rx: Option<Receiver<Option<PathBuf>>>,
    focus_handle: FocusHandle,
    name_editing: bool,
    side_panel_w: f32,
    stats_open: bool,
    settings_open: bool,
    resize_drag_x: Option<f32>,
}

impl PatternsView {
    fn new(cx: &mut Context<Self>, language: Language) -> Self {
        let thumbs = PatternKind::builtins()
            .iter()
            .copied()
            .map(|kind| (kind, pattern_thumb(kind)))
            .collect();

        let saved_cfg = load_test_patterns_config().unwrap_or_default().sanitized();
        let frame_buffer_frames =
            clamp_video_frame_buffer_frames(saved_cfg.frame_buffer_frames) as u32;
        let saved = saved_cfg.custom_images.clone();
        let saved_count = saved.len();
        let mut custom_images = Vec::new();
        let mut kept_paths = Vec::new();
        for path in saved {
            if !path.is_file() {
                continue;
            }
            let thumb = rgba_image_from_path(&path, THUMB_W as u32, THUMB_H as u32).ok();
            kept_paths.push(path.clone());
            custom_images.push(CustomImage { path, thumb });
        }
        let pruned_images = kept_paths.len() != saved_count;

        let mut view = Self {
            language,
            name: saved_cfg.name.clone(),
            on_air_name: None,
            kind: PatternKind::SmpteColorBars,
            width: saved_cfg.width,
            height: saved_cfg.height,
            frame_rate: FrameRate {
                n: saved_cfg.fps_n,
                d: saved_cfg.fps_d,
            },
            quality: quality_from_config(saved_cfg.quality),
            animate: saved_cfg.animate,
            anim_speed_h_pct: saved_cfg.anim_speed_h_pct,
            anim_speed_v_pct: saved_cfg.anim_speed_v_pct,
            tone_hz: saved_cfg.tone_hz,
            level_dbfs: saved_cfg.level_dbfs,
            sample_rate: 48_000,
            channels: 2,
            samples: 480,
            frame_buffer_frames,
            custom_images,
            selected_custom: None,
            session: None,
            live: Arc::new(Mutex::new(LivePattern::from_view(
                PatternKind::SmpteColorBars,
                saved_cfg.animate,
                saved_cfg.anim_speed_h_pct,
                saved_cfg.anim_speed_v_pct,
                None,
            ))),
            last_stats: SendStats::default(),
            error: None,
            thumbs,
            preview: None,
            preview_image_uyvy: None,
            preview_phase_x: 0.0,
            preview_phase_y: 0.0,
            last_preview_at: Instant::now() - Duration::from_secs(1),
            window_title: SharedString::from(""),
            open_menu: None,
            custom_menu: None,
            image_pick_rx: None,
            focus_handle: cx.focus_handle(),
            name_editing: false,
            side_panel_w: clamp_side_panel_w(saved_cfg.side_panel_w as f32),
            stats_open: saved_cfg.stats_open,
            settings_open: saved_cfg.settings_open,
            resize_drag_x: None,
        };
        view.refresh_title();
        if pruned_images {
            persist_test_patterns_prefs(&view);
        }
        view.restart_session();
        view.refresh_preview(cx);
        view.schedule_tick(cx);
        view
    }

    fn schedule_tick(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            Timer::after(Duration::from_millis(16)).await;
            this.update(cx, |this, cx| {
                this.on_tick(cx);
                this.schedule_tick(cx);
            })
            .ok();
        })
        .detach();
    }

    fn on_tick(&mut self, cx: &mut Context<Self>) {
        self.poll_image_pick(cx);
        if let Some(session) = &self.session {
            self.last_stats = session.stats();
        }
        // Animate generated patterns and custom images at the selected output frame rate.
        // Static image stills skip the tick until Animate is enabled.
        if self.kind != PatternKind::Image || self.animate {
            let frame_interval = Duration::from_secs_f64(
                self.frame_rate.d.max(1) as f64 / self.frame_rate.n.max(1) as f64,
            );
            if self.last_preview_at.elapsed() >= frame_interval {
                if self.animate {
                    self.preview_phase_x =
                        advance_scroll_phase(self.preview_phase_x, self.anim_speed_h_pct as f32);
                    self.preview_phase_y =
                        advance_scroll_phase(self.preview_phase_y, self.anim_speed_v_pct as f32);
                }
                self.refresh_preview(cx);
            }
        }
        self.refresh_title();
        cx.notify();
    }

    fn refresh_title(&mut self) {
        let pattern = if self.kind == PatternKind::Image {
            self.selected_custom
                .and_then(|i| self.custom_images.get(i))
                .map(|img| {
                    img.path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("Image")
                        .to_string()
                })
                .unwrap_or_else(|| "Image".into())
        } else {
            pattern_label(self.language, self.kind).to_string()
        };
        let fps = self.frame_rate.label();
        let res = if self.height >= 2160 {
            "2160p"
        } else if self.height >= 1080 {
            "1080p"
        } else if self.height >= 720 {
            "720p"
        } else {
            "SD"
        };
        self.window_title =
            SharedString::from(format!("OMT Test Patterns - {pattern} ({res}{fps})"));
    }

    fn select_pattern(&mut self, kind: PatternKind, cx: &mut Context<Self>) {
        self.custom_menu = None;
        self.open_menu = None;
        if kind == PatternKind::Image || self.kind == kind {
            return;
        }
        self.kind = kind;
        self.selected_custom = None;
        self.preview_image_uyvy = None;
        self.push_live_content(!self.animate);
        self.refresh_preview(cx);
        cx.notify();
    }

    fn select_custom_image(&mut self, index: usize, cx: &mut Context<Self>) {
        self.custom_menu = None;
        self.open_menu = None;
        if self.kind == PatternKind::Image && self.selected_custom == Some(index) {
            return;
        }
        let Some(entry) = self.custom_images.get(index) else {
            return;
        };
        let path = entry.path.clone();
        let preview_uyvy = match uyvy_from_image_path(&path, PREVIEW_W, PREVIEW_H) {
            Ok(buf) => Arc::new(buf),
            Err(e) => {
                self.error = Some(SharedString::from(e.to_string()));
                cx.notify();
                return;
            }
        };
        self.preview_image_uyvy = Some(preview_uyvy);
        self.selected_custom = Some(index);
        self.kind = PatternKind::Image;
        self.error = None;
        self.push_live_content(true);
        self.refresh_preview(cx);
        cx.notify();
    }

    fn request_pick_image(&mut self, _cx: &mut Context<Self>) {
        if self.image_pick_rx.is_some() {
            return;
        }
        self.custom_menu = None;
        let (tx, rx) = mpsc::channel();
        self.image_pick_rx = Some(rx);
        thread::Builder::new()
            .name("omt-pick-image".into())
            .spawn(move || {
                let path = rfd::FileDialog::new()
                    .add_filter("Images", &["png", "jpg", "jpeg", "bmp"])
                    .pick_file();
                let _ = tx.send(path);
            })
            .ok();
    }

    fn poll_image_pick(&mut self, cx: &mut Context<Self>) {
        let result = match self.image_pick_rx.as_ref() {
            Some(rx) => match rx.try_recv() {
                Ok(path) => Some(Ok(path)),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(Err(())),
            },
            None => None,
        };
        match result {
            Some(Ok(Some(path))) => {
                self.image_pick_rx = None;
                self.add_custom_image(path, cx);
            }
            Some(Ok(None)) => {
                // Cancelled — leave the previous pattern running.
                self.image_pick_rx = None;
                self.error = None;
            }
            Some(Err(())) => {
                self.image_pick_rx = None;
            }
            None => {}
        }
    }

    fn add_custom_image(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.error = None;
        if let Some(existing) = self.custom_images.iter().position(|img| img.path == path) {
            self.select_custom_image(existing, cx);
            return;
        }

        let thumb = match rgba_image_from_path(&path, THUMB_W as u32, THUMB_H as u32) {
            Ok(img) => img,
            Err(e) => {
                self.error = Some(SharedString::from(e));
                return;
            }
        };

        self.custom_images.push(CustomImage {
            path,
            thumb: Some(thumb),
        });
        self.persist_images();
        let index = self.custom_images.len() - 1;
        self.select_custom_image(index, cx);
    }

    fn remove_custom_image(&mut self, index: usize, cx: &mut Context<Self>) {
        self.custom_menu = None;
        if index >= self.custom_images.len() {
            return;
        }
        let removed = self.custom_images.remove(index);
        if let Some(thumb) = removed.thumb {
            cx.drop_image(thumb, None);
        }
        self.persist_images();

        let was_selected =
            self.kind == PatternKind::Image && self.selected_custom.is_some_and(|i| i == index);
        if let Some(sel) = self.selected_custom.as_mut()
            && *sel > index
        {
            *sel -= 1;
        }
        if was_selected {
            self.selected_custom = None;
            self.preview_image_uyvy = None;
            self.kind = PatternKind::SmpteColorBars;
            self.push_live_content(!self.animate);
            self.refresh_preview(cx);
        }
        cx.notify();
    }

    fn persist_images(&self) {
        persist_test_patterns_prefs(self);
    }

    fn nudge_frame_buffer(&mut self, delta: i32, cx: &mut Context<Self>) {
        let next = (self.frame_buffer_frames as i32 + delta).clamp(
            MIN_VIDEO_FRAME_BUFFER_FRAMES as i32,
            MAX_VIDEO_FRAME_BUFFER_FRAMES as i32,
        ) as u32;
        if next == self.frame_buffer_frames {
            cx.notify();
            return;
        }
        self.frame_buffer_frames = next;
        self.persist_images();
        if let Some(session) = self.session.as_ref() {
            session.set_frame_buffer_frames(next);
        }
        cx.notify();
    }

    fn open_custom_menu(&mut self, index: usize, x: f32, y: f32, cx: &mut Context<Self>) {
        self.open_menu = None;
        self.custom_menu = Some((index, x, y));
        cx.notify();
    }

    fn reveal_custom_image(&mut self, index: usize, cx: &mut Context<Self>) {
        self.custom_menu = None;
        if let Some(entry) = self.custom_images.get(index)
            && let Err(e) = reveal_in_file_manager(&entry.path)
        {
            self.error = Some(SharedString::from(e.to_string()));
        }
        cx.notify();
    }

    fn close_overlays(&mut self, cx: &mut Context<Self>) {
        self.open_menu = None;
        self.custom_menu = None;
        self.blur_name(cx);
    }

    fn blur_name(&mut self, cx: &mut Context<Self>) {
        if !self.name_editing {
            cx.notify();
            return;
        }
        self.name_editing = false;
        if self.name.trim().is_empty() {
            self.name = "Test Pattern".into();
        }
        cx.notify();
    }

    fn begin_edit_name(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.session.is_some() {
            cx.notify();
            return;
        }
        self.open_menu = None;
        self.custom_menu = None;
        self.name_editing = true;
        self.focus_handle.focus(window);
        cx.notify();
    }

    fn on_name_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        if !self.name_editing {
            return;
        }
        let key = event.keystroke.key.as_str();
        if key == "backspace" {
            self.name.pop();
            self.persist_images();
            cx.notify();
            return;
        }
        if key == "enter" || key == "escape" {
            self.blur_name(cx);
            return;
        }
        if let Some(ch) = event.keystroke.key_char.as_deref()
            && ch.chars().all(|c| !c.is_control())
            && self.name.len() + ch.len() <= 64
        {
            self.name.push_str(ch);
            self.persist_images();
            cx.notify();
        }
    }

    fn set_tone(&mut self, tone: TonePreset, cx: &mut Context<Self>) {
        self.open_menu = None;
        let hz = tone.hz();
        if (self.tone_hz - hz).abs() < f32::EPSILON {
            cx.notify();
            return;
        }
        self.tone_hz = hz;
        self.push_live_audio();
        self.persist_images();
        cx.notify();
    }

    fn set_frame_rate(&mut self, frame_rate: FrameRate, cx: &mut Context<Self>) {
        self.open_menu = None;
        if self.session.is_some() {
            cx.notify();
            return;
        }
        if self.frame_rate == frame_rate {
            cx.notify();
            return;
        }
        self.frame_rate = frame_rate;
        self.persist_images();
        cx.notify();
    }

    fn set_resolution(&mut self, resolution: Resolution, cx: &mut Context<Self>) {
        self.open_menu = None;
        if self.session.is_some() {
            cx.notify();
            return;
        }
        if self.width == resolution.width && self.height == resolution.height {
            cx.notify();
            return;
        }
        self.width = resolution.width;
        self.height = resolution.height;
        self.refresh_title();
        self.persist_images();
        cx.notify();
    }

    fn nudge_width(&mut self, delta: i32, cx: &mut Context<Self>) {
        if self.session.is_some() {
            cx.notify();
            return;
        }
        let next = (self.width + delta).clamp(64, 7680);
        // Keep even width for UYVY.
        let next = next - (next % 2);
        if next == self.width {
            cx.notify();
            return;
        }
        self.width = next;
        self.refresh_title();
        self.persist_images();
        cx.notify();
    }

    fn nudge_height(&mut self, delta: i32, cx: &mut Context<Self>) {
        if self.session.is_some() {
            cx.notify();
            return;
        }
        let next = (self.height + delta).clamp(64, 4320);
        if next == self.height {
            cx.notify();
            return;
        }
        self.height = next;
        self.refresh_title();
        self.persist_images();
        cx.notify();
    }

    fn toggle_animate(&mut self, cx: &mut Context<Self>) {
        self.animate = !self.animate;
        if !self.animate {
            self.preview_phase_x = 0.0;
            self.preview_phase_y = 0.0;
        }
        self.push_live_content(true);
        self.refresh_preview(cx);
        self.persist_images();
        cx.notify();
    }

    fn nudge_anim_speed_h(&mut self, delta: i32, cx: &mut Context<Self>) {
        let next = (self.anim_speed_h_pct + delta).clamp(-200, 200);
        if next == self.anim_speed_h_pct {
            cx.notify();
            return;
        }
        self.anim_speed_h_pct = next;
        self.push_live_speeds();
        self.persist_images();
        cx.notify();
    }

    fn nudge_anim_speed_v(&mut self, delta: i32, cx: &mut Context<Self>) {
        let next = (self.anim_speed_v_pct + delta).clamp(-200, 200);
        if next == self.anim_speed_v_pct {
            cx.notify();
            return;
        }
        self.anim_speed_v_pct = next;
        self.push_live_speeds();
        self.persist_images();
        cx.notify();
    }

    fn set_level(&mut self, level: LevelPreset, cx: &mut Context<Self>) {
        self.open_menu = None;
        let dbfs = level.dbfs();
        if (self.level_dbfs - dbfs).abs() < f32::EPSILON {
            cx.notify();
            return;
        }
        self.level_dbfs = dbfs;
        self.push_live_audio();
        self.persist_images();
        cx.notify();
    }

    fn toggle_menu(&mut self, menu: MenuKind, anchor_x: f32, cx: &mut Context<Self>) {
        self.custom_menu = None;
        self.name_editing = false;
        if self.session.is_some() && matches!(menu, MenuKind::Resolution | MenuKind::Fps) {
            if self.open_menu.map(|(kind, _)| kind) == Some(menu) {
                self.open_menu = None;
            }
            cx.notify();
            return;
        }
        self.open_menu = if self.open_menu.map(|(kind, _)| kind) == Some(menu) {
            None
        } else {
            Some((menu, anchor_x))
        };
        cx.notify();
    }

    fn set_quality(&mut self, quality: Quality, cx: &mut Context<Self>) {
        if self.quality == quality {
            return;
        }
        self.quality = quality;
        if let Some(session) = self.session.as_ref() {
            session.set_quality(quality);
        }
        self.persist_images();
        cx.notify();
    }

    fn start_sending(&mut self, cx: &mut Context<Self>) {
        self.open_menu = None;
        self.custom_menu = None;
        self.name_editing = false;
        if self.name.trim().is_empty() {
            self.name = "Test Pattern".into();
        }
        self.persist_images();
        self.restart_session();
        cx.notify();
    }

    fn stop_sending(&mut self, cx: &mut Context<Self>) {
        self.open_menu = None;
        self.custom_menu = None;
        self.stop();
        cx.notify();
    }

    fn current_image_path(&self) -> Option<&Path> {
        self.selected_custom
            .and_then(|i| self.custom_images.get(i))
            .map(|img| img.path.as_path())
    }

    fn audio_config(&self) -> AudioToneConfig {
        AudioToneConfig {
            sample_rate: self.sample_rate,
            channels: self.channels,
            tone_hz: self.tone_hz,
            level_dbfs: self.level_dbfs,
            samples: self.samples,
        }
    }

    fn load_image_uyvy(&self) -> std::result::Result<Arc<Vec<u8>>, SharedString> {
        match self.current_image_path() {
            Some(path) => match uyvy_from_image_path(path, self.width, self.height) {
                Ok(buf) => Ok(Arc::new(buf)),
                Err(e) => Err(SharedString::from(e.to_string())),
            },
            None => Err(SharedString::from("Select an image file first")),
        }
    }

    /// Push pattern / image / animation into the shared provider state.
    fn push_live_content(&mut self, invalidate_still: bool) -> bool {
        let image_uyvy = if self.kind == PatternKind::Image {
            match self.load_image_uyvy() {
                Ok(buf) => Some(buf),
                Err(e) => {
                    self.error = Some(e);
                    return false;
                }
            }
        } else {
            None
        };
        let animate = self.animate;
        {
            let mut live = self.live.lock();
            live.kind = self.kind;
            live.image_uyvy = image_uyvy;
            live.animate = animate;
            live.speed_h = self.anim_speed_h_pct.clamp(-200, 200) as f32 / 100.0;
            live.speed_v = self.anim_speed_v_pct.clamp(-200, 200) as f32 / 100.0;
            if !animate {
                live.phase_x = 0.0;
                live.phase_y = 0.0;
            }
        }
        if let Some(session) = self.session.as_ref() {
            session.set_animate(animate);
            if invalidate_still || !animate {
                session.invalidate_content();
            }
        }
        true
    }

    fn push_live_speeds(&self) {
        let mut live = self.live.lock();
        live.speed_h = self.anim_speed_h_pct.clamp(-200, 200) as f32 / 100.0;
        live.speed_v = self.anim_speed_v_pct.clamp(-200, 200) as f32 / 100.0;
    }

    fn push_live_audio(&self) {
        if let Some(session) = self.session.as_ref() {
            session.update_audio(self.audio_config());
        }
    }

    fn restart_session(&mut self) {
        self.stop();
        self.error = None;
        {
            let mut live = self.live.lock();
            live.phase_x = 0.0;
            live.phase_y = 0.0;
        }
        if !self.push_live_content(true) {
            return;
        }

        let width = self.width;
        let height = self.height;
        let live = Arc::clone(&self.live);
        let provider: Arc<dyn Fn(u64) -> Vec<u8> + Send + Sync> = Arc::new(move |_idx| {
            let (kind, phase_x, phase_y, image) = {
                let mut state = live.lock();
                let kind = state.kind;
                let image = state.image_uyvy.clone();
                let (phase_x, phase_y) = if state.animate {
                    let px = state.phase_x;
                    let py = state.phase_y;
                    state.phase_x =
                        (state.phase_x + state.speed_h / ANIM_BASE_CYCLE_FRAMES).rem_euclid(1.0);
                    state.phase_y =
                        (state.phase_y + state.speed_v / ANIM_BASE_CYCLE_FRAMES).rem_euclid(1.0);
                    (px, py)
                } else {
                    (0.0, 0.0)
                };
                (kind, phase_x, phase_y, image)
            };
            if let Some(still) = image {
                if phase_x == 0.0 && phase_y == 0.0 {
                    return still.as_ref().clone();
                }
                let mut buf = vec![0u8; (width as usize) * 2 * (height as usize)];
                scroll_uyvy(still.as_ref(), &mut buf, width, height, phase_x, phase_y);
                return buf;
            }
            let mut buf = vec![0u8; (width as usize) * 2 * (height as usize)];
            fill_uyvy(kind, &mut buf, width, height, phase_x, phase_y);
            buf
        });

        let advertised = uniquify_local_source_name(&self.name);
        self.on_air_name = Some(advertised.clone());

        let config = SendSessionConfig {
            name: advertised,
            width: self.width,
            height: self.height,
            fps_n: self.frame_rate.n,
            fps_d: self.frame_rate.d,
            quality: self.quality,
            animate: self.animate,
            frame_buffer_frames: self.frame_buffer_frames,
            audio: self.audio_config(),
        };

        match SendSession::start(config, provider) {
            Ok(session) => self.session = Some(session),
            Err(e) => {
                self.on_air_name = None;
                self.error = Some(SharedString::from(e.to_string()));
            }
        }
    }

    fn stop(&mut self) {
        if let Some(mut session) = self.session.take() {
            session.stop();
        }
        self.on_air_name = None;
    }

    fn refresh_preview(&mut self, cx: &mut Context<Self>) {
        self.last_preview_at = Instant::now();
        let (phase_x, phase_y) = if self.animate {
            (self.preview_phase_x, self.preview_phase_y)
        } else {
            (0.0, 0.0)
        };
        let uyvy = if self.kind == PatternKind::Image {
            let Some(src) = self.preview_image_uyvy.as_ref() else {
                return;
            };
            if phase_x == 0.0 && phase_y == 0.0 {
                src.as_ref().clone()
            } else {
                let mut buf = vec![0u8; (PREVIEW_W as usize) * 2 * (PREVIEW_H as usize)];
                scroll_uyvy(
                    src.as_ref(),
                    &mut buf,
                    PREVIEW_W,
                    PREVIEW_H,
                    phase_x,
                    phase_y,
                );
                buf
            }
        } else {
            let mut buf = vec![0u8; (PREVIEW_W as usize) * 2 * (PREVIEW_H as usize)];
            fill_uyvy(self.kind, &mut buf, PREVIEW_W, PREVIEW_H, phase_x, phase_y);
            buf
        };
        let rgba = uyvy_to_rgba(&uyvy, PREVIEW_W as u32, PREVIEW_H as u32);
        if let Some(image) = rgba_to_render_image(rgba, PREVIEW_W as u32, PREVIEW_H as u32) {
            if let Some(old) = self.preview.take() {
                cx.drop_image(old, None);
            }
            self.preview = Some(image);
        }
    }
}

impl Drop for PatternsView {
    fn drop(&mut self) {
        self.stop();
    }
}

impl Focusable for PatternsView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for PatternsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let language = self.language;
        let kind = self.kind;
        let tone_hz = self.tone_hz;
        let frame_rate = self.frame_rate;
        let quality = self.quality;
        let open_menu = self.open_menu;
        let custom_menu = self.custom_menu;
        let thumbs = self.thumbs.clone();
        let custom_images: Vec<(usize, PathBuf, Option<Arc<RenderImage>>)> = self
            .custom_images
            .iter()
            .enumerate()
            .map(|(i, img)| (i, img.path.clone(), img.thumb.clone()))
            .collect();
        let selected_custom = self.selected_custom;
        let preview = self.preview.clone();
        let stats = self.last_stats.clone();
        let error = self.error.clone();
        let sending = self.session.is_some();
        let title = self.window_title.clone();
        let name = self
            .on_air_name
            .clone()
            .unwrap_or_else(|| self.name.clone());
        let name_editing = self.name_editing;
        let width = self.width;
        let height = self.height;
        let animate = self.animate;
        let speed_h = self.anim_speed_h_pct;
        let speed_v = self.anim_speed_v_pct;
        let frame_buffer_frames = self.frame_buffer_frames;
        let level_dbfs = self.level_dbfs;
        let side_panel_w = self.side_panel_w;
        let stats_open = self.stats_open;
        let settings_open = self.settings_open;

        let mut root = div()
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .font(ui_font())
            .bg(rgb(0x1a1d23))
            .text_color(rgb(0xedf2f7))
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                this.on_name_key(event, cx);
            }))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                let Some(last_x) = this.resize_drag_x else {
                    return;
                };
                let x: f32 = event.position.x.into();
                this.side_panel_w = clamp_side_panel_w(this.side_panel_w + (last_x - x));
                this.resize_drag_x = Some(x);
                cx.notify();
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                    if this.resize_drag_x.take().is_some() {
                        persist_test_patterns_prefs(this);
                        cx.notify();
                    }
                }),
            )
            // Title strip
            .child(
                div()
                    .px_4()
                    .py_2()
                    .border_b_1()
                    .border_color(rgb(0x2a3340))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .min_w_0()
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_sm()
                            .min_w_0()
                            .flex_1()
                            .truncate()
                            .child(title),
                    )
                    .child(
                        div()
                            .text_xs()
                            .opacity(0.7)
                            .flex_shrink_0()
                            .child(if sending {
                                SharedString::from(t(language, "patterns.sending"))
                            } else {
                                SharedString::from(t(language, "patterns.idle"))
                            }),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .flex()
                    .flex_row()
                    // Pattern grid
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .id("pattern-grid")
                            .overflow_y_scroll()
                            .p_4()
                            .child(pattern_grid(
                                cx,
                                language,
                                kind,
                                selected_custom,
                                thumbs,
                                custom_images,
                            )),
                    )
                    // Drag handle + stats / settings side panel
                    .child(
                        div()
                            .id("side-splitter")
                            .w(px(SPLITTER_HIT))
                            .h_full()
                            .flex_shrink_0()
                            .cursor(CursorStyle::ResizeLeftRight)
                            .bg(rgb(0x2a3340))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                                    let x: f32 = event.position.x.into();
                                    this.resize_drag_x = Some(x);
                                    cx.stop_propagation();
                                }),
                            ),
                    )
                    .child(
                        div()
                            .w(px(side_panel_w))
                            .min_w(px(SIDE_PANEL_MIN_W))
                            .max_w_full()
                            .flex_shrink()
                            .h_full()
                            .min_h_0()
                            .id("side-panel")
                            .overflow_y_scroll()
                            .p_3()
                            .gap_2()
                            .flex()
                            .flex_col()
                            .bg(rgb(0x14181f))
                            .child(section_header(
                                cx,
                                "stats-section",
                                t(language, "patterns.stats"),
                                stats_open,
                                |this, cx| {
                                    this.stats_open = !this.stats_open;
                                    persist_test_patterns_prefs(this);
                                    cx.notify();
                                },
                            ))
                            .children(
                                stats_open.then(|| stats_block(language, &stats, error.clone())),
                            )
                            .child(div().mt_2().mb_1().h(px(1.0)).bg(rgb(0x2a3340)))
                            .child(section_header(
                                cx,
                                "settings-section",
                                t(language, "patterns.settings"),
                                settings_open,
                                |this, cx| {
                                    this.settings_open = !this.settings_open;
                                    persist_test_patterns_prefs(this);
                                    cx.notify();
                                },
                            ))
                            .children(settings_open.then(|| {
                                settings_block(
                                    cx,
                                    language,
                                    &name,
                                    name_editing,
                                    sending,
                                    animate,
                                    speed_h,
                                    speed_v,
                                )
                            })),
                    ),
            )
            // Bottom control bar — wrap to the viewport; scroll horizontally if still tight.
            .child(
                div()
                    .id("bottom-bar")
                    .px_3()
                    .pt_1()
                    .pb(px(PREVIEW_BOTTOM_MARGIN))
                    .gap_3()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .min_w_0()
                    .overflow_x_scroll()
                    .border_t_1()
                    .border_color(rgb(0x2a3340))
                    .bg(rgb(0x12161c))
                    .child(resolution_control(
                        cx,
                        language,
                        width,
                        height,
                        open_menu.map(|(kind, _)| kind) == Some(MenuKind::Resolution),
                        sending,
                    ))
                    .child(tone_control(
                        cx,
                        language,
                        tone_hz,
                        open_menu.map(|(kind, _)| kind) == Some(MenuKind::Tone),
                    ))
                    .child(fps_control(
                        cx,
                        language,
                        frame_rate,
                        open_menu.map(|(kind, _)| kind) == Some(MenuKind::Fps),
                        sending,
                    ))
                    .child(frame_buffer_control(cx, language, frame_buffer_frames))
                    .child(quality_control(cx, language, quality))
                    .child(level_control(
                        cx,
                        language,
                        level_dbfs,
                        open_menu.map(|(kind, _)| kind) == Some(MenuKind::Level),
                    ))
                    .child(transport_controls(cx, language, sending))
                    .child(div().flex_1().min_w(px(0.0)))
                    .child(output_preview(language, preview)),
            );

        // Root-level overlays so dropdowns / context menus paint above the main grid.
        if open_menu.is_some() || custom_menu.is_some() {
            root = root.child(overlay_layer(
                cx,
                language,
                open_menu,
                tone_hz,
                frame_rate,
                level_dbfs,
                width,
                height,
                custom_menu,
            ));
        }

        root
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anim_speed_change_does_not_jump_phase() {
        let start = 0.42;
        let next_slow = advance_scroll_phase(start, 50.0);
        let next_fast = advance_scroll_phase(start, 200.0);
        let delta_slow = (next_slow - start).rem_euclid(1.0);
        let delta_fast = (next_fast - start).rem_euclid(1.0);
        assert!(delta_fast > delta_slow);
        assert!(delta_fast < 0.02);
        assert!((next_slow - start).abs() < 0.01);
    }
}

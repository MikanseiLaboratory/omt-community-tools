//! Off-UI frame handoff for CPU decode.
//!
//! Default path forwards the decoder's BGRA `Arc` unchanged. The UI uploads
//! those bytes to a reused wgpu texture. Alpha-mask preview still converts
//! here so the UI thread stays a blit.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use arc_swap::ArcSwapOption;
use egui::Context;
use omt_media::{LatestVideo, bgra_alpha_mask};
use parking_lot::{Condvar, Mutex};

/// Prepared frame ready for GPU upload (no egui [`egui::ColorImage`] copy).
pub struct PreparedFrame {
    /// Tightly packed pixels (`BGRA` unless [`Self::rgba`]).
    pub pixels: Arc<[u8]>,
    /// Pixel width.
    pub width: u32,
    /// Pixel height.
    pub height: u32,
    /// Declared frame rate numerator.
    pub fps_n: i32,
    /// Declared frame rate denominator.
    pub fps_d: i32,
    /// `true` when [`Self::pixels`] are RGBA (alpha-mask preview).
    pub rgba: bool,
}

/// Shared controls / output for the prep engine.
pub struct PrepControl {
    pub selected_url: Mutex<Option<String>>,
    pub show_alpha: AtomicBool,
    pub running: AtomicBool,
    /// Bumped when alpha toggles so the last frame can be re-prepared.
    pub alpha_epoch: AtomicU64,
    pub presented: AtomicU64,
    pub skipped: AtomicU64,
    /// Latest-wins prepared image (Arc for cheap handoff).
    pub slot: ArcSwapOption<PreparedFrame>,
    /// egui context used to request a repaint as soon as a frame is ready.
    repaint_ctx: Mutex<Option<Context>>,
    /// Wakes the prep thread for alpha / shutdown without a new video frame.
    wake: Condvar,
    wake_mutex: Mutex<()>,
}

impl PrepControl {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            selected_url: Mutex::new(None),
            show_alpha: AtomicBool::new(false),
            running: AtomicBool::new(true),
            alpha_epoch: AtomicU64::new(0),
            presented: AtomicU64::new(0),
            skipped: AtomicU64::new(0),
            slot: ArcSwapOption::empty(),
            repaint_ctx: Mutex::new(None),
            wake: Condvar::new(),
            wake_mutex: Mutex::new(()),
        })
    }

    pub fn take_prepared(&self) -> Option<Arc<PreparedFrame>> {
        self.slot.swap(None)
    }

    /// Bind the UI context so prepared frames can wake the event loop promptly.
    pub fn set_repaint_context(&self, ctx: Context) {
        *self.repaint_ctx.lock() = Some(ctx);
    }

    pub fn set_alpha(&self, show: bool) {
        self.show_alpha.store(show, Ordering::Relaxed);
        self.alpha_epoch.fetch_add(1, Ordering::Relaxed);
        self.wake_prep();
    }

    /// Wake the prep thread (URL change, disconnect, etc.).
    pub fn notify(&self) {
        self.wake_prep();
    }

    fn wake_prep(&self) {
        let _guard = self.wake_mutex.lock();
        self.wake.notify_all();
    }

    fn request_repaint(&self) {
        if let Some(ctx) = self.repaint_ctx.lock().as_ref() {
            ctx.request_repaint();
        }
    }
}

/// Join handle wrapper so Drop can signal shutdown.
pub struct FramePrep {
    ctrl: Arc<PrepControl>,
    join: Option<JoinHandle<()>>,
}

impl FramePrep {
    pub fn start(latest: Arc<LatestVideo>, ctrl: Arc<PrepControl>) -> Self {
        let ctrl_thread = Arc::clone(&ctrl);
        let join = thread::Builder::new()
            .name("omt-prep".into())
            .spawn(move || prep_loop(latest, ctrl_thread))
            .expect("spawn omt-prep");
        Self {
            ctrl,
            join: Some(join),
        }
    }
}

impl Drop for FramePrep {
    fn drop(&mut self) {
        self.ctrl.running.store(false, Ordering::SeqCst);
        self.ctrl.wake_prep();
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

fn prep_loop(latest: Arc<LatestVideo>, ctrl: Arc<PrepControl>) {
    type LastRawFrame = (Arc<[u8]>, u32, u32, i32, i32);
    let mut last_raw: Option<LastRawFrame> = None;
    let mut applied_epoch = 0u64;

    while ctrl.running.load(Ordering::Relaxed) {
        let url_ok = {
            let selected = ctrl.selected_url.lock();
            let worker_url = latest.url.lock();
            match (selected.as_ref(), worker_url.as_ref()) {
                (Some(sel), Some(url)) => sel == url,
                _ => false,
            }
        };

        let mut did_work = false;
        if url_ok {
            // Block until a frame arrives (or short timeout for alpha/shutdown).
            if let Some(frame) = latest.wait_take(Duration::from_millis(4)) {
                last_raw = Some((
                    frame.bgra,
                    frame.width,
                    frame.height,
                    frame.fps_n,
                    frame.fps_d.max(1),
                ));
                applied_epoch = ctrl.alpha_epoch.load(Ordering::Relaxed);
                if let Some(raw) = last_raw.as_ref() {
                    publish_from_raw(&ctrl, raw);
                    did_work = true;
                }
            } else {
                let epoch = ctrl.alpha_epoch.load(Ordering::Relaxed);
                if epoch != applied_epoch {
                    if let Some(raw) = last_raw.as_ref() {
                        applied_epoch = epoch;
                        publish_from_raw(&ctrl, raw);
                        did_work = true;
                    } else {
                        applied_epoch = epoch;
                    }
                }
            }
        } else {
            last_raw = None;
            ctrl.slot.store(None);
            // Idle until URL selection / shutdown.
            let mut g = ctrl.wake_mutex.lock();
            let _ = ctrl.wake.wait_for(&mut g, Duration::from_millis(50));
            did_work = true;
        }

        if !did_work {
            // Tiny park so alpha toggles / shutdown are responsive without 1ms spin.
            let mut g = ctrl.wake_mutex.lock();
            let _ = ctrl.wake.wait_for(&mut g, Duration::from_millis(2));
        }
    }
}

fn publish_from_raw(ctrl: &PrepControl, raw: &(Arc<[u8]>, u32, u32, i32, i32)) {
    let (bgra, width, height, fps_n, fps_d) = raw;
    if *width == 0 || *height == 0 {
        ctrl.skipped.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let expected = (*width as usize)
        .saturating_mul(*height as usize)
        .saturating_mul(4);
    if bgra.len() < expected {
        ctrl.skipped.fetch_add(1, Ordering::Relaxed);
        return;
    }

    let (pixels, rgba) = if ctrl.show_alpha.load(Ordering::Relaxed) {
        (Arc::<[u8]>::from(bgra_alpha_mask(bgra)), true)
    } else if bgra.len() == expected {
        (Arc::clone(bgra), false)
    } else {
        (Arc::from(&bgra[..expected]), false)
    };

    ctrl.slot.store(Some(Arc::new(PreparedFrame {
        pixels,
        width: *width,
        height: *height,
        fps_n: *fps_n,
        fps_d: *fps_d,
        rgba,
    })));
    ctrl.presented.fetch_add(1, Ordering::Relaxed);
    ctrl.request_repaint();
}

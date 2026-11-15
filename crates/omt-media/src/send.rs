//! OMT send session with paced video + OS-thread tone audio.

use std::collections::VecDeque;
use std::f32::consts::TAU;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use openmediatransport::{
    AudioIngress, Codec, ColorSpace, Discovery, FrameType, MediaFrame, NETWORK_ASYNC_COUNT,
    NETWORK_SEND_BUFFER, NETWORK_SEND_RECEIVE_BUFFER, OmtError, Quality, Sender, SenderConfig,
    SenderInfo,
};
use parking_lot::Mutex;

use crate::runtime;

const TICKS_PER_SECOND: i64 = 10_000_000;
/// How often [`SendStats`] are refreshed (connections, FPS window, etc.).
const STATS_INTERVAL: Duration = Duration::from_millis(100);
/// Default prefetched UYVY frames waiting for the paced sender.
pub const DEFAULT_VIDEO_FRAME_BUFFER_FRAMES: u32 = 3;
/// Minimum allowed send-side frame buffer depth.
pub const MIN_VIDEO_FRAME_BUFFER_FRAMES: u32 = 1;
/// Maximum allowed send-side frame buffer depth.
pub const MAX_VIDEO_FRAME_BUFFER_FRAMES: u32 = 16;
/// Max sleep slice while waiting for a frame deadline (keeps stop responsive).
const PACE_SLEEP_SLICE: Duration = Duration::from_millis(5);
/// Audio sleeps in short slices so `stop()` stays responsive.
const AUDIO_SLEEP_SLICE: Duration = Duration::from_millis(5);
/// If audio pacing falls this far behind, re-anchor the timeline to the wall clock.
///
/// Smaller scheduling jitter is absorbed by sending the next packet immediately
/// with a contiguous PTS, so receivers never see a gap in the tone.
const AUDIO_RESYNC_LATE: Duration = Duration::from_millis(200);

/// Clamp a configured frame-buffer depth into the supported range.
pub fn clamp_video_frame_buffer_frames(frames: u32) -> usize {
    frames.clamp(MIN_VIDEO_FRAME_BUFFER_FRAMES, MAX_VIDEO_FRAME_BUFFER_FRAMES) as usize
}

/// Audio tone configuration.
#[derive(Debug, Clone)]
pub struct AudioToneConfig {
    /// Sample rate (Hz).
    pub sample_rate: i32,
    /// Channel count.
    pub channels: i32,
    /// Tone frequency (Hz).
    pub tone_hz: f32,
    /// Peak level in dBFS (SMPTE alignment tone default: −20).
    pub level_dbfs: f32,
    /// Samples per packet (default 480 = 10 ms @ 48 kHz).
    pub samples: i32,
}

impl Default for AudioToneConfig {
    fn default() -> Self {
        Self {
            sample_rate: 48_000,
            channels: 2,
            tone_hz: 1000.0,
            level_dbfs: -20.0,
            samples: 480,
        }
    }
}

/// Configuration for an outbound test / capture session.
#[derive(Debug, Clone)]
pub struct SendSessionConfig {
    /// Discoverable source name.
    pub name: String,
    /// Frame width.
    pub width: i32,
    /// Frame height.
    pub height: i32,
    /// Frame rate numerator.
    pub fps_n: i32,
    /// Frame rate denominator.
    pub fps_d: i32,
    /// Encoding quality.
    pub quality: Quality,
    /// Whether UYVY content changes every frame.
    pub animate: bool,
    /// Prefetch depth for paced video sends (clamped to 1..=16).
    pub frame_buffer_frames: u32,
    /// Audio tone settings.
    pub audio: AudioToneConfig,
}

impl Default for SendSessionConfig {
    fn default() -> Self {
        Self {
            name: "Test Pattern".into(),
            width: 1920,
            height: 1080,
            fps_n: 30,
            fps_d: 1,
            quality: Quality::Medium,
            animate: true,
            frame_buffer_frames: DEFAULT_VIDEO_FRAME_BUFFER_FRAMES,
            audio: AudioToneConfig::default(),
        }
    }
}

/// Live send statistics snapshot.
#[derive(Debug, Clone, Default)]
pub struct SendStats {
    /// Approximate video FPS over the last window.
    pub video_fps: f32,
    /// Average encode time in milliseconds.
    pub encode_ms: f32,
    /// Frames submitted.
    pub frames: i64,
    /// Frames dropped by the sender queue.
    pub dropped: i64,
    /// True when encode cannot sustain target FPS.
    pub behind: bool,
    /// Listening TCP port.
    pub port: u16,
    /// Accepted peer TCP connections (≈ 2 per receiver client).
    pub connections: u32,
    /// Approximate receiver clients (`connections / 2`, rounded up).
    pub clients: u32,
    /// Peers subscribed to video.
    pub video_subscribers: u32,
    /// Peers subscribed to audio.
    pub audio_subscribers: u32,
    /// Cumulative bytes sent.
    pub bytes_sent: i64,
    /// Audio packets handed to the OMT sender.
    pub audio_frames: i64,
    /// Longest wall-clock gap between audio packets (milliseconds).
    pub audio_max_gap_ms: f32,
    /// Audio timeline resynchronisations caused by a long stall.
    pub audio_skips: i64,
}

type FrameProvider = Arc<dyn Fn(u64) -> Vec<u8> + Send + Sync>;

/// Shared, lock-free counters between the audio and video sender threads.
#[derive(Clone, Default)]
struct AudioTelemetry {
    active: Arc<AtomicBool>,
    frames: Arc<AtomicU64>,
    max_gap_ms: Arc<AtomicU32>,
    skips: Arc<AtomicU64>,
}

/// Background OMT sender owning OS video + audio pacing threads.
pub struct SendSession {
    running: Arc<AtomicBool>,
    stats: Arc<Mutex<SendStats>>,
    audio: Arc<Mutex<AudioToneConfig>>,
    animate: Arc<AtomicBool>,
    content_epoch: Arc<AtomicU64>,
    sender: Option<Arc<Mutex<Sender>>>,
    discovery: Option<Discovery>,
    source_name: String,
    fps_n: Arc<AtomicI32>,
    fps_d: Arc<AtomicI32>,
    frame_buffer_frames: Arc<AtomicU32>,
    audio_join: Option<thread::JoinHandle<()>>,
    video_join: Option<thread::JoinHandle<()>>,
}

impl SendSession {
    /// Start sending frames produced by `provider` (returns UYVY bytes per frame index).
    pub fn start(config: SendSessionConfig, provider: FrameProvider) -> Result<Self, OmtError> {
        let sender = Arc::new(Mutex::new(Sender::create_with_config(
            &config.name,
            FrameType::VIDEO | FrameType::AUDIO | FrameType::METADATA,
            SenderConfig {
                send_buffer: NETWORK_SEND_BUFFER,
                recv_buffer: NETWORK_SEND_RECEIVE_BUFFER,
                send_queue_depth: NETWORK_ASYNC_COUNT,
            },
        )?));
        {
            let mut s = sender.lock();
            s.set_sender_info(SenderInfo::new(
                "OMT Tools",
                "MikanseiLaboratory",
                env!("CARGO_PKG_VERSION"),
            ));
            s.set_quality(config.quality);
        }
        let port = sender.lock().port();
        let source_name = config.name.clone();
        let discovery = {
            let name = source_name.clone();
            runtime::handle()
                .block_on(runtime::spawn_blocking(move || {
                    let mut discovery = Discovery::new()?;
                    discovery.register(&name, port)?;
                    Ok::<_, OmtError>(discovery)
                }))
                .map_err(|e| OmtError::Discovery(e.to_string()))??
        };

        let running = Arc::new(AtomicBool::new(true));
        let stats = Arc::new(Mutex::new(SendStats {
            port,
            ..Default::default()
        }));
        let audio = Arc::new(Mutex::new(config.audio.clone()));
        let animate = Arc::new(AtomicBool::new(config.animate));
        let content_epoch = Arc::new(AtomicU64::new(0));
        let fps_n = Arc::new(AtomicI32::new(config.fps_n.max(1)));
        let fps_d = Arc::new(AtomicI32::new(config.fps_d.max(1)));
        let frame_buffer_frames = Arc::new(AtomicU32::new(clamp_video_frame_buffer_frames(
            config.frame_buffer_frames,
        ) as u32));
        let epoch = Instant::now();
        let telemetry = AudioTelemetry::default();
        // Audio owns its own ingress so it never waits for the video encode lock.
        let audio_ingress = sender.lock().audio_ingress();

        let audio_running = Arc::clone(&running);
        let audio_cfg = Arc::clone(&audio);
        let audio_telemetry = telemetry.clone();
        let audio_join = thread::Builder::new()
            .name("omt-send-audio".into())
            .spawn(move || {
                audio_loop(
                    audio_ingress,
                    audio_cfg,
                    audio_telemetry,
                    audio_running,
                    epoch,
                )
            })?;

        let video_running = Arc::clone(&running);
        let video_sender = Arc::clone(&sender);
        let video_stats = Arc::clone(&stats);
        let video_animate = Arc::clone(&animate);
        let video_epoch = Arc::clone(&content_epoch);
        let video_fps_n = Arc::clone(&fps_n);
        let video_fps_d = Arc::clone(&fps_d);
        let video_buffer = Arc::clone(&frame_buffer_frames);
        let video_telemetry = telemetry.clone();
        let video_cfg = config.clone();
        let video_join = thread::Builder::new()
            .name("omt-send-video".into())
            .spawn(move || {
                video_loop(
                    video_sender,
                    video_cfg,
                    provider,
                    video_animate,
                    video_epoch,
                    video_fps_n,
                    video_fps_d,
                    video_buffer,
                    video_running,
                    video_stats,
                    video_telemetry,
                    epoch,
                );
            })?;

        Ok(Self {
            running,
            stats,
            audio,
            animate,
            content_epoch,
            sender: Some(sender),
            discovery: Some(discovery),
            source_name,
            fps_n,
            fps_d,
            frame_buffer_frames,
            audio_join: Some(audio_join),
            video_join: Some(video_join),
        })
    }

    /// Snapshot of send statistics.
    pub fn stats(&self) -> SendStats {
        self.stats.lock().clone()
    }

    /// Hot-update tone parameters without tearing down the sender.
    ///
    /// Prefer this for `tone_hz` / `level_dbfs`. Changing `sample_rate`,
    /// `channels`, or `samples` mid-stream is allowed but may briefly
    /// confuse receivers — prefer a full restart for those.
    pub fn update_audio(&self, audio: AudioToneConfig) {
        *self.audio.lock() = audio;
    }

    /// Enable / disable per-frame content changes without restarting OMT.
    pub fn set_animate(&self, animate: bool) {
        self.animate.store(animate, Ordering::Relaxed);
        // Drop any still-frame cache so the next frame matches.
        self.content_epoch.fetch_add(1, Ordering::Relaxed);
    }

    /// Invalidate a cached still frame (pattern / image changed while not animating).
    pub fn invalidate_content(&self) {
        self.content_epoch.fetch_add(1, Ordering::Relaxed);
    }

    /// Hot-update encoding quality without tearing down the sender.
    pub fn set_quality(&self, quality: Quality) {
        if let Some(sender) = self.sender.as_ref() {
            sender.lock().set_quality(quality);
        }
    }

    /// Hot-update output frame rate. Pacing and per-frame metadata follow.
    pub fn set_frame_rate(&self, fps_n: i32, fps_d: i32) {
        self.fps_n.store(fps_n.max(1), Ordering::Relaxed);
        self.fps_d.store(fps_d.max(1), Ordering::Relaxed);
    }

    /// Hot-update paced video prefetch depth.
    pub fn set_frame_buffer_frames(&self, frames: u32) {
        self.frame_buffer_frames.store(
            clamp_video_frame_buffer_frames(frames) as u32,
            Ordering::Relaxed,
        );
    }

    /// Stop tasks / threads, close TCP peers, and deregister Discovery
    /// (`<Removed>True</Removed>` / DNS-SD unregister).
    pub fn stop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(join) = self.video_join.take() {
            let _ = join.join();
        }
        if let Some(join) = self.audio_join.take() {
            let _ = join.join();
        }
        // libomtnet OMTSend.Dispose: DeregisterAddress, then close listener/channels.
        if let Some(mut discovery) = self.discovery.take() {
            let _ = discovery.deregister(&self.source_name);
        }
        drop(self.sender.take());
    }
}

impl Drop for SendSession {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Paces a continuous audio timeline against the shared wall-clock epoch.
///
/// Kept separate from the thread loop so the timing rules (contiguous PTS on
/// small jitter, wall-clock re-anchor on a long stall) are unit-testable.
struct AudioPacer {
    samples: i64,
    rate: i64,
    next_samples: i64,
    started: bool,
}

impl AudioPacer {
    fn new() -> Self {
        Self {
            samples: 1,
            rate: 1,
            next_samples: 0,
            started: false,
        }
    }

    /// Update geometry. A sample-rate / packet-size change re-anchors the clock.
    fn configure(&mut self, samples: i64, rate: i64, elapsed: Duration) {
        let samples = samples.max(1);
        let rate = rate.max(1);
        if !self.started || self.samples != samples || self.rate != rate {
            self.samples = samples;
            self.rate = rate;
            self.started = true;
            self.resync(elapsed);
        }
    }

    /// Align the next packet to the current wall clock.
    fn resync(&mut self, elapsed: Duration) {
        let raw = (elapsed.as_secs_f64() * self.rate.max(1) as f64).round() as i64;
        let samples = self.samples.max(1);
        self.next_samples = raw - raw.rem_euclid(samples);
    }

    /// Wall-clock deadline for the next packet.
    fn due(&self) -> Duration {
        Duration::from_secs_f64(self.next_samples.max(0) as f64 / self.rate.max(1) as f64)
    }

    /// PTS for the next packet, without advancing.
    fn timestamp(&self) -> i64 {
        self.next_samples
            .saturating_mul(TICKS_PER_SECOND)
            .saturating_div(self.rate.max(1))
    }

    /// Return the next PTS and advance the timeline.
    fn advance(&mut self) -> i64 {
        let timestamp = self.timestamp();
        self.next_samples = self.next_samples.saturating_add(self.samples.max(1));
        timestamp
    }
}

fn audio_loop(
    ingress: AudioIngress,
    cfg: Arc<Mutex<AudioToneConfig>>,
    telemetry: AudioTelemetry,
    running: Arc<AtomicBool>,
    epoch: Instant,
) {
    let mut audio_buf = Vec::new();
    let mut phase = 0.0f64;
    let mut pacer = AudioPacer::new();
    let mut active = false;
    let mut last_sent: Option<Instant> = None;

    while running.load(Ordering::Relaxed) {
        if !telemetry.active.load(Ordering::Acquire) {
            active = false;
            last_sent = None;
            thread::sleep(Duration::from_millis(10));
            continue;
        }

        let snap = cfg.lock().clone();
        let samples = snap.samples.max(1) as i64;
        let rate = snap.sample_rate.max(1) as i64;
        let now = epoch.elapsed();

        if active {
            pacer.configure(samples, rate, now);
        } else {
            // A new receiver starts on the current wall clock, not on a stale
            // packet backlog from before the subscription.
            pacer.configure(samples, rate, now);
            pacer.resync(now);
            active = true;
            last_sent = None;
        }

        let due = pacer.due();
        if due > now {
            thread::sleep((due - now).min(AUDIO_SLEEP_SLICE));
            continue;
        }

        let late = now.saturating_sub(due);
        if late >= AUDIO_RESYNC_LATE {
            pacer.resync(now);
            telemetry.skips.fetch_add(1, Ordering::Relaxed);
            tracing::warn!(
                late_ms = late.as_millis(),
                "audio send fell behind; re-anchored to wall clock"
            );
        }

        audio_buf.clear();
        if snap.tone_hz > 0.0 {
            append_sine_planar(
                &mut audio_buf,
                snap.channels,
                samples as i32,
                snap.sample_rate,
                snap.tone_hz,
                snap.level_dbfs,
                &mut phase,
            );
        } else {
            // Mute: planar silence.
            let ch = snap.channels.max(1) as usize;
            let n = samples.max(0) as usize;
            audio_buf.resize(ch * n * 4, 0);
        }
        let timestamp = pacer.advance();
        let frame = MediaFrame {
            frame_type: FrameType::AUDIO,
            timestamp,
            codec: Codec::Fpa1 as i32,
            sample_rate: snap.sample_rate,
            channels: snap.channels,
            samples_per_channel: samples as i32,
            active_channels: 0,
            data: std::mem::take(&mut audio_buf),
            ..Default::default()
        };
        if ingress.send(frame).is_ok() {
            telemetry.frames.fetch_add(1, Ordering::Relaxed);
            let sent_at = Instant::now();
            if let Some(previous) = last_sent {
                let gap_ms = sent_at.duration_since(previous).as_secs_f32() * 1000.0;
                let gap_ms = gap_ms.round().clamp(0.0, u32::MAX as f32) as u32;
                telemetry.max_gap_ms.fetch_max(gap_ms, Ordering::Relaxed);
            }
            last_sent = Some(sent_at);
        }
    }
}

/// Wall-clock duration for an OMT timestamp (100 ns ticks).
fn duration_from_omt_ticks(ticks: i64) -> Duration {
    let ticks = ticks.max(0) as u64;
    Duration::from_nanos(ticks.saturating_mul(100))
}

/// Frame interval in OMT ticks for `fps_n / fps_d`.
fn video_interval_ticks(fps_n: i32, fps_d: i32) -> i64 {
    (fps_d as i64).saturating_mul(TICKS_PER_SECOND) / fps_n.max(1) as i64
}

/// How many whole frame intervals fit in `elapsed` on the OMT tick timeline.
fn frames_elapsed(elapsed: Duration, video_interval_ticks: i64) -> u64 {
    let interval = video_interval_ticks.max(1) as u128;
    let ticks = elapsed.as_nanos() / 100;
    (ticks / interval) as u64
}

/// Next paced index after a live frame-rate change.
///
/// Keeps timestamps on the shared wall clock and never steps backwards relative
/// to `last_timestamp` (receivers drop or stall on non-monotonic PTS).
fn next_frame_idx_after_rate_change(
    elapsed: Duration,
    new_interval_ticks: i64,
    last_timestamp: i64,
) -> u64 {
    let interval = new_interval_ticks.max(1);
    let wall_idx = frames_elapsed(elapsed, interval);
    if last_timestamp < 0 {
        return wall_idx;
    }
    let min_idx = (last_timestamp / interval) as u64 + 1;
    wall_idx.max(min_idx)
}

/// Absolute deadline for `frame_idx` on a shared wall-clock epoch.
fn frame_deadline(epoch: Instant, frame_idx: u64, video_interval_ticks: i64) -> Instant {
    let ticks = (frame_idx as i64).saturating_mul(video_interval_ticks.max(1));
    epoch + duration_from_omt_ticks(ticks)
}

/// Sleep until `deadline` (or until `running` clears), in short slices.
fn sleep_until_deadline(deadline: Instant, running: &AtomicBool) {
    while running.load(Ordering::Relaxed) {
        let now = Instant::now();
        if now >= deadline {
            return;
        }
        let slice = (deadline - now).min(PACE_SLEEP_SLICE);
        thread::sleep(slice);
    }
}

/// Relabel prefetched frames so the front matches `new_start`.
///
/// Used when skipping late slots: throw away timeline indices, keep pixels.
fn rebase_buffer_indices(buffer: &mut VecDeque<(u64, Vec<u8>)>, new_start: u64) {
    for (offset, (idx, _)) in buffer.iter_mut().enumerate() {
        *idx = new_start.saturating_add(offset as u64);
    }
}

#[allow(clippy::too_many_arguments)]
fn fill_video_buffer(
    buffer: &mut VecDeque<(u64, Vec<u8>)>,
    start_idx: u64,
    provider: &FrameProvider,
    animate: bool,
    content_epoch: u64,
    cached: &mut Option<(u64, Vec<u8>)>,
    frame_bytes: usize,
    depth: usize,
) -> bool {
    let depth = depth.max(1);
    while buffer.len() < depth {
        let idx = start_idx.saturating_add(buffer.len() as u64);
        let uyvy = if !animate {
            if let Some((cached_epoch, data)) = cached.as_ref()
                && *cached_epoch == content_epoch
            {
                data.clone()
            } else {
                let frame = provider(idx);
                if frame.len() != frame_bytes {
                    return false;
                }
                *cached = Some((content_epoch, frame.clone()));
                frame
            }
        } else {
            *cached = None;
            let frame = provider(idx);
            if frame.len() != frame_bytes {
                return false;
            }
            frame
        };
        buffer.push_back((idx, uyvy));
    }
    true
}

#[allow(clippy::too_many_arguments, unused_assignments)]
fn video_loop(
    sender: Arc<Mutex<Sender>>,
    cfg: SendSessionConfig,
    provider: FrameProvider,
    animate_flag: Arc<AtomicBool>,
    content_epoch: Arc<AtomicU64>,
    fps_n: Arc<AtomicI32>,
    fps_d: Arc<AtomicI32>,
    frame_buffer_frames: Arc<AtomicU32>,
    running: Arc<AtomicBool>,
    stats: Arc<Mutex<SendStats>>,
    audio_telemetry: AudioTelemetry,
    epoch: Instant,
) {
    let stride = (cfg.width as usize) * 2;
    let frame_bytes = stride * cfg.height as usize;
    let mut cached: Option<(u64, Vec<u8>)> = None;
    let mut buffer: VecDeque<(u64, Vec<u8>)> = VecDeque::new();
    let mut frame_idx = 0u64;
    let mut last_stats = Instant::now();
    let mut last_codec_time = 0i64;
    let mut last_frames = 0i64;
    let mut behind = false;
    let mut was_subscribed = false;
    let mut last_content_epoch = content_epoch.load(Ordering::Relaxed);
    let mut last_video_timestamp = -1i64;
    let mut fps_n_now = fps_n.load(Ordering::Relaxed).max(1);
    let mut fps_d_now = fps_d.load(Ordering::Relaxed).max(1);
    let mut video_interval = video_interval_ticks(fps_n_now, fps_d_now);
    let mut target_fps = fps_n_now as f64 / fps_d_now as f64;
    // Half a frame late still counts as the same slot; beyond that we skip ahead.
    let mut late_slack = duration_from_omt_ticks(video_interval.max(1) / 2);

    while running.load(Ordering::Relaxed) {
        let next_fps_n = fps_n.load(Ordering::Relaxed).max(1);
        let next_fps_d = fps_d.load(Ordering::Relaxed).max(1);
        if next_fps_n != fps_n_now || next_fps_d != fps_d_now {
            fps_n_now = next_fps_n;
            fps_d_now = next_fps_d;
            video_interval = video_interval_ticks(fps_n_now, fps_d_now);
            target_fps = fps_n_now as f64 / fps_d_now as f64;
            late_slack = duration_from_omt_ticks(video_interval.max(1) / 2);
            frame_idx = next_frame_idx_after_rate_change(
                epoch.elapsed(),
                video_interval,
                last_video_timestamp,
            );
            buffer.clear();
        }
        let buffer_depth =
            clamp_video_frame_buffer_frames(frame_buffer_frames.load(Ordering::Relaxed));
        let (video_ok, audio_ok) = {
            let mut s = sender.lock();
            let _ = s.poll_accept();
            let _ = s.poll_peer_metadata();
            (s.video_subscribed(), s.audio_subscribed())
        };
        // The audio thread reads this atomic and never takes the sender lock, so
        // a long VMX encode cannot stall the tone.
        audio_telemetry.active.store(audio_ok, Ordering::Release);
        if !video_ok {
            if was_subscribed {
                buffer.clear();
                was_subscribed = false;
            }
            thread::sleep(Duration::from_millis(5));
            continue;
        }
        if !was_subscribed {
            // Align to wall clock so we never dump a backlog after idle.
            frame_idx = frames_elapsed(epoch.elapsed(), video_interval);
            buffer.clear();
            was_subscribed = true;
            behind = false;
        }

        let epoch_n = content_epoch.load(Ordering::Relaxed);
        if epoch_n != last_content_epoch {
            cached = None;
            buffer.clear();
            last_content_epoch = epoch_n;
        }

        let animate = animate_flag.load(Ordering::Relaxed);
        // Generate at most one frame before the deadline check so a slow fill
        // cannot skip-loop forever (clearing the buffer each time it overruns
        // one slot). Remaining depth is prefetched after send.
        let warmup = if buffer.is_empty() { 1 } else { buffer.len() };
        if !fill_video_buffer(
            &mut buffer,
            frame_idx,
            &provider,
            animate,
            epoch_n,
            &mut cached,
            frame_bytes,
            warmup,
        ) {
            thread::sleep(Duration::from_millis(5));
            continue;
        }

        // Absolute deadline pacing: sleep when early; when late, skip whole
        // slots instead of bursting catch-up sends (Issue #27). Ready pixels
        // are kept and retimed so the receiver still gets a frame.
        let deadline = frame_deadline(epoch, frame_idx, video_interval);
        let now = Instant::now();
        if now < deadline {
            sleep_until_deadline(deadline, running.as_ref());
            if !running.load(Ordering::Relaxed) {
                break;
            }
            behind = false;
        } else {
            let late_by = now.saturating_duration_since(deadline);
            let ideal = frames_elapsed(epoch.elapsed(), video_interval);
            if ideal > frame_idx {
                rebase_buffer_indices(&mut buffer, ideal);
                frame_idx = ideal;
                behind = true;
            } else {
                behind = late_by > late_slack;
            }
        }

        let Some((_idx, uyvy)) = buffer.pop_front() else {
            behind = true;
            continue;
        };

        let timestamp = (frame_idx as i64).saturating_mul(video_interval);
        let frame = MediaFrame {
            frame_type: FrameType::VIDEO,
            timestamp,
            codec: Codec::Uyvy as i32,
            width: cfg.width,
            height: cfg.height,
            stride: stride as i32,
            frame_rate_n: fps_n_now,
            frame_rate_d: fps_d_now,
            aspect_ratio: cfg.width as f32 / cfg.height.max(1) as f32,
            color_space: ColorSpace::Undefined,
            data: uyvy,
            ..Default::default()
        };
        let _ = sender.lock().send_video(frame);
        last_video_timestamp = timestamp;
        frame_idx = frame_idx.saturating_add(1);
        let _ = fill_video_buffer(
            &mut buffer,
            frame_idx,
            &provider,
            animate,
            epoch_n,
            &mut cached,
            frame_bytes,
            buffer_depth,
        );

        if last_stats.elapsed() >= STATS_INTERVAL {
            let (st, port, connections, video_subs, audio_subs) = {
                let s = sender.lock();
                (
                    s.statistics(),
                    s.port(),
                    s.connection_count() as u32,
                    s.video_subscriber_count() as u32,
                    s.audio_subscriber_count() as u32,
                )
            };
            let window = last_stats.elapsed().as_secs_f64().max(0.001);
            let df = (st.frames - last_frames).max(0);
            let dt = (st.codec_time - last_codec_time).max(0);
            let avg_ms = if df > 0 {
                (dt as f64 / df as f64) / 1000.0
            } else {
                0.0
            };
            let mut snap = stats.lock();
            snap.video_fps = (df as f64 / window) as f32;
            snap.encode_ms = avg_ms as f32;
            snap.frames = st.frames;
            snap.dropped = st.frames_dropped;
            snap.behind = behind || snap.video_fps + 0.5 < target_fps as f32;
            snap.port = port;
            snap.connections = connections;
            snap.clients = connections.div_ceil(2);
            snap.video_subscribers = video_subs;
            snap.audio_subscribers = audio_subs;
            snap.bytes_sent = st.bytes_sent;
            snap.audio_frames = audio_telemetry.frames.load(Ordering::Relaxed) as i64;
            snap.audio_max_gap_ms = audio_telemetry.max_gap_ms.load(Ordering::Relaxed) as f32;
            snap.audio_skips = audio_telemetry.skips.load(Ordering::Relaxed) as i64;
            last_codec_time = st.codec_time;
            last_frames = st.frames;
            last_stats = Instant::now();
        }
    }
}

fn append_sine_planar(
    dst: &mut Vec<u8>,
    channels: i32,
    samples: i32,
    sample_rate: i32,
    tone_hz: f32,
    level_dbfs: f32,
    phase: &mut f64,
) {
    let ch = channels.max(1) as usize;
    let n = samples.max(0) as usize;
    let rate = sample_rate.max(1) as f64;
    let freq = tone_hz as f64;
    // Peak amplitude from dBFS (1.0 == 0 dBFS). Clamp to avoid NaN / runaway gain.
    let amplitude = 10f32.powf(level_dbfs.clamp(-120.0, 0.0) / 20.0);
    dst.reserve(ch * n * 4);
    let start = *phase;
    for _c in 0..ch {
        let mut p = start;
        for _s in 0..n {
            let sample = (TAU as f64 * freq * p / rate).sin() as f32 * amplitude;
            dst.extend_from_slice(&sample.to_le_bytes());
            p += 1.0;
            if p >= rate {
                p -= rate;
            }
        }
    }
    *phase = start + n as f64;
    if *phase >= rate {
        *phase %= rate;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interval(fps_n: i32, fps_d: i32) -> i64 {
        video_interval_ticks(fps_n, fps_d)
    }

    #[test]
    fn omt_tick_interval_matches_common_rates() {
        assert_eq!(interval(30, 1), 333_333);
        assert_eq!(interval(60, 1), 166_666);
        assert_eq!(interval(30_000, 1_001), 333_666);
        assert_eq!(interval(60_000, 1_001), 166_833);
    }

    #[test]
    fn rate_change_keeps_timestamps_monotonic() {
        let iv30 = interval(30, 1);
        let last_ts = 300 * iv30;
        let elapsed = duration_from_omt_ticks(last_ts);
        let iv60 = interval(60, 1);
        let idx = next_frame_idx_after_rate_change(elapsed, iv60, last_ts);
        let next_ts = (idx as i64).saturating_mul(iv60);
        assert!(next_ts > last_ts, "next={next_ts} last={last_ts}");

        let iv24 = interval(24, 1);
        let idx_down = next_frame_idx_after_rate_change(elapsed, iv24, last_ts);
        let next_down = (idx_down as i64).saturating_mul(iv24);
        assert!(next_down > last_ts, "next={next_down} last={last_ts}");
    }

    #[test]
    fn rate_change_without_prior_frame_follows_wall_clock() {
        let iv = interval(30, 1);
        let elapsed = duration_from_omt_ticks(iv.saturating_mul(10));
        assert_eq!(next_frame_idx_after_rate_change(elapsed, iv, -1), 10);
    }

    #[test]
    fn frames_elapsed_tracks_omt_tick_timeline() {
        let iv = interval(30, 1);
        let period = duration_from_omt_ticks(iv);
        assert_eq!(frames_elapsed(Duration::ZERO, iv), 0);
        assert_eq!(frames_elapsed(period, iv), 1);
        assert_eq!(frames_elapsed(period.saturating_mul(10), iv), 10);
        // Just shy of the next boundary stays on the previous index.
        assert_eq!(
            frames_elapsed(period.saturating_mul(10) - Duration::from_nanos(100), iv),
            9
        );
    }

    #[test]
    fn frame_deadline_is_monotonic_and_aligned() {
        let epoch = Instant::now();
        let iv = interval(30, 1);
        let d0 = frame_deadline(epoch, 0, iv);
        let d1 = frame_deadline(epoch, 1, iv);
        let d2 = frame_deadline(epoch, 2, iv);
        assert_eq!(d0, epoch);
        assert_eq!(d1.duration_since(d0), duration_from_omt_ticks(iv));
        assert_eq!(
            d2.duration_since(d0),
            duration_from_omt_ticks(iv.saturating_mul(2))
        );
        // Deadline and frames_elapsed agree at the boundary.
        assert_eq!(frames_elapsed(d1.duration_since(epoch), iv), 1);
    }

    #[test]
    fn fill_video_buffer_prefers_still_cache() {
        let calls = Arc::new(AtomicU64::new(0));
        let calls_c = Arc::clone(&calls);
        let provider: FrameProvider = Arc::new(move |_idx| {
            calls_c.fetch_add(1, Ordering::Relaxed);
            vec![0u8; 8]
        });
        let mut cached = None;
        let mut buffer = VecDeque::new();
        assert!(fill_video_buffer(
            &mut buffer,
            0,
            &provider,
            false,
            1,
            &mut cached,
            8,
            DEFAULT_VIDEO_FRAME_BUFFER_FRAMES as usize,
        ));
        assert_eq!(buffer.len(), DEFAULT_VIDEO_FRAME_BUFFER_FRAMES as usize);
        let first_calls = calls.load(Ordering::Relaxed);
        assert_eq!(first_calls, 1);
        buffer.clear();
        assert!(fill_video_buffer(
            &mut buffer,
            3,
            &provider,
            false,
            1,
            &mut cached,
            8,
            DEFAULT_VIDEO_FRAME_BUFFER_FRAMES as usize,
        ));
        assert_eq!(calls.load(Ordering::Relaxed), first_calls);
    }

    #[test]
    fn clamp_frame_buffer_frames_respects_bounds() {
        assert_eq!(clamp_video_frame_buffer_frames(0), 1);
        assert_eq!(
            clamp_video_frame_buffer_frames(DEFAULT_VIDEO_FRAME_BUFFER_FRAMES),
            DEFAULT_VIDEO_FRAME_BUFFER_FRAMES as usize
        );
        assert_eq!(
            clamp_video_frame_buffer_frames(MAX_VIDEO_FRAME_BUFFER_FRAMES + 10),
            MAX_VIDEO_FRAME_BUFFER_FRAMES as usize
        );
    }

    #[test]
    fn rebase_buffer_indices_keeps_pixels_and_retimes() {
        let mut buffer =
            VecDeque::from([(10u64, vec![1u8, 2]), (11, vec![3, 4]), (12, vec![5, 6])]);
        rebase_buffer_indices(&mut buffer, 40);
        assert_eq!(buffer[0].0, 40);
        assert_eq!(buffer[1].0, 41);
        assert_eq!(buffer[2].0, 42);
        assert_eq!(buffer[0].1, vec![1, 2]);
        let (idx, pixels) = buffer.pop_front().expect("ready frame");
        assert_eq!(idx, 40);
        assert_eq!(pixels, vec![1, 2]);
    }

    #[test]
    fn audio_pacer_keeps_contiguous_pts() {
        let mut pacer = AudioPacer::new();
        pacer.configure(480, 48_000, Duration::ZERO);
        assert_eq!(pacer.due(), Duration::ZERO);
        assert_eq!(pacer.advance(), 0);
        assert_eq!(pacer.due(), Duration::from_millis(10));
        assert_eq!(pacer.timestamp(), 100_000);
        assert_eq!(pacer.advance(), 100_000);
        assert_eq!(pacer.timestamp(), 200_000);

        // Slightly late packets catch up without changing the PTS cadence.
        pacer.configure(480, 48_000, Duration::from_millis(21));
        assert_eq!(pacer.due(), Duration::from_millis(20));
        assert_eq!(pacer.advance(), 200_000);
    }

    #[test]
    fn audio_pacer_resyncs_after_long_stall() {
        let mut pacer = AudioPacer::new();
        pacer.configure(480, 48_000, Duration::ZERO);
        pacer.resync(Duration::from_millis(250));
        assert_eq!(pacer.due(), Duration::from_millis(250));
        assert_eq!(pacer.timestamp(), 2_500_000);

        // 25 ms is rounded down to the previous 10 ms packet boundary.
        pacer.resync(Duration::from_millis(25));
        assert_eq!(pacer.due(), Duration::from_millis(20));
        assert_eq!(pacer.timestamp(), 200_000);
    }

    #[test]
    fn audio_pacer_reconfigures_on_sample_rate_change() {
        let mut pacer = AudioPacer::new();
        pacer.configure(480, 48_000, Duration::ZERO);
        pacer.advance();
        pacer.configure(441, 44_100, Duration::from_millis(10));
        assert_eq!(pacer.due(), Duration::from_millis(10));
        assert_eq!(pacer.timestamp(), 100_000);
        pacer.advance();
        assert_eq!(pacer.timestamp(), 200_000);
    }

    #[test]
    fn audio_pacer_timestamps_stay_monotonic() {
        let mut pacer = AudioPacer::new();
        pacer.configure(480, 48_000, Duration::ZERO);
        let mut previous = -1;
        for _ in 0..10_000 {
            let timestamp = pacer.advance();
            assert!(timestamp > previous, "timestamp went backwards");
            previous = timestamp;
        }
    }
}

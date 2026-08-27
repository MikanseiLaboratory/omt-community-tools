//! Background OMT A/V receive worker (Tokio).

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use openmediatransport::{
    FrameType, OmtError, Quality, ReceiverConfig, ReceiverSession, SessionState, SessionStatistics,
};
use parking_lot::{Condvar, Mutex};
use tokio::sync::mpsc;

use crate::audio_out::{AudioLevels, AudioOutput};
use crate::playout::{BufferSettings, Playout};
use crate::runtime;
use crate::stall::StallDetector;

const METADATA_LOG_CAP: usize = 256;

/// Connection / quality options for an inbound receive session.
#[derive(Debug, Clone)]
pub struct ConnectOptions {
    /// Source URL (`omt://…`).
    pub url: String,
    /// Discovery-time IP candidates tried before / with URL host resolution.
    pub addresses: Vec<String>,
    /// Suggested encode quality sent to the peer via subscribe metadata.
    pub quality: Quality,
    /// Request 1/8 progressive Preview (`VideoFlags::PREVIEW` / low bandwidth).
    pub preview: bool,
}

impl ConnectOptions {
    /// Connect with default quality settings.
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            addresses: Vec::new(),
            quality: Quality::Default,
            preview: false,
        }
    }
}

/// Decoded video frame ready for UI upload.
#[derive(Debug, Clone)]
pub struct VideoFrame {
    /// Pixel width.
    pub width: u32,
    /// Pixel height.
    pub height: u32,
    /// BGRA8 tightly packed pixels (shared ownership from the decoder).
    pub bgra: Arc<[u8]>,
    /// OMT timestamp (100 ns ticks).
    pub timestamp: i64,
    /// Declared frame rate numerator.
    pub fps_n: i32,
    /// Declared frame rate denominator.
    pub fps_d: i32,
}

/// One metadata / diagnostic log line from the receive path.
#[derive(Debug, Clone)]
pub struct MetadataLogEntry {
    /// Unix time in milliseconds.
    pub unix_ms: u64,
    /// Short kind label (`metadata`, `frame-meta`, `info`, …).
    pub kind: String,
    /// XML or text payload.
    pub text: String,
}

/// App-level receive counters (latest-wins semantics).
#[derive(Debug, Clone, Copy, Default)]
pub struct ReceiveCounters {
    /// Decoded video frames written into the latest slot.
    pub frames_decoded: u64,
    /// Frames overwritten before the UI took them (source / queue drops).
    pub frames_replaced: u64,
    /// Metadata / XML events received.
    pub metadata_events: u64,
    /// Decoded audio packets played / metered.
    pub audio_frames: u64,
}

/// Shared latest-frame slot + receive stats.
pub struct LatestVideo {
    /// Newest decoded frame (replaced, never queued).
    pub frame: Mutex<Option<VideoFrame>>,
    /// Wakes prep/UI waiters when a frame is published or cleared.
    frame_cv: Condvar,
    /// Last receiver statistics snapshot.
    pub stats: Mutex<SessionStatistics>,
    /// Latest [`ReceiverSession`] transport state.
    pub session_state: Mutex<SessionState>,
    /// App-level counters.
    pub counters: Mutex<ReceiveCounters>,
    /// Latest audio peak levels for VU.
    pub audio_levels: Mutex<AudioLevels>,
    /// Effective video buffer delay currently applied (milliseconds).
    pub video_buffer_delay_ms: Mutex<u32>,
    /// Effective audio buffer delay currently applied (milliseconds).
    pub audio_buffer_delay_ms: Mutex<u32>,
    /// Rolling metadata / XML log (oldest front).
    pub metadata_log: Mutex<VecDeque<MetadataLogEntry>>,
    /// Last error message, if any.
    pub error: Mutex<Option<String>>,
    /// Connected URL.
    pub url: Mutex<Option<String>>,
}

impl Default for LatestVideo {
    fn default() -> Self {
        Self {
            frame: Mutex::new(None),
            frame_cv: Condvar::new(),
            stats: Mutex::new(SessionStatistics::default()),
            session_state: Mutex::new(SessionState::Stopped),
            counters: Mutex::new(ReceiveCounters::default()),
            audio_levels: Mutex::new(AudioLevels::default()),
            video_buffer_delay_ms: Mutex::new(0),
            audio_buffer_delay_ms: Mutex::new(0),
            metadata_log: Mutex::new(VecDeque::new()),
            error: Mutex::new(None),
            url: Mutex::new(None),
        }
    }
}

impl std::fmt::Debug for LatestVideo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LatestVideo").finish_non_exhaustive()
    }
}

impl LatestVideo {
    /// Take the newest frame if present.
    pub fn take(&self) -> Option<VideoFrame> {
        self.frame.lock().take()
    }

    /// Wait up to `timeout` for a frame, then take it (latest-wins).
    pub fn wait_take(&self, timeout: Duration) -> Option<VideoFrame> {
        let mut slot = self.frame.lock();
        if slot.is_none() {
            let _ = self.frame_cv.wait_for(&mut slot, timeout);
        }
        slot.take()
    }

    /// Replace the latest video slot and wake waiters.
    pub fn publish_video(&self, video: VideoFrame, replaced_extra: u64) {
        let mut slot = self.frame.lock();
        let mut counters = self.counters.lock();
        if slot.is_some() {
            counters.frames_replaced = counters.frames_replaced.saturating_add(1);
        }
        counters.frames_replaced = counters.frames_replaced.saturating_add(replaced_extra);
        *slot = Some(video);
        counters.frames_decoded = counters.frames_decoded.saturating_add(1);
        drop(counters);
        drop(slot);
        self.frame_cv.notify_all();
    }

    /// Clear the frame slot and wake waiters (disconnect / teardown).
    pub fn clear_video(&self) {
        *self.frame.lock() = None;
        self.frame_cv.notify_all();
    }

    /// Peek without removing.
    pub fn peek(&self) -> Option<VideoFrame> {
        self.frame.lock().clone()
    }

    /// Drain new metadata log entries after `after_unix_ms` (exclusive).
    pub fn metadata_since(&self, after_unix_ms: u64) -> Vec<MetadataLogEntry> {
        self.metadata_log
            .lock()
            .iter()
            .filter(|e| e.unix_ms > after_unix_ms)
            .cloned()
            .collect()
    }
}

/// Commands for the receive worker.
#[derive(Debug, Clone)]
pub enum ReceiveCommand {
    /// Connect / switch to a source URL (with quality options).
    Connect(ConnectOptions),
    /// Disconnect and idle.
    Disconnect,
    /// Update A/V playout buffer depth.
    SetBuffer(BufferSettings),
    /// Stop the worker task.
    Shutdown,
}

/// Background Tokio task that owns an OMT [`ReceiverSession`].
pub struct ReceiveWorker {
    tx: mpsc::UnboundedSender<ReceiveCommand>,
    latest: Arc<LatestVideo>,
    stall: Arc<Mutex<StallDetector>>,
    audio: Arc<AudioOutput>,
    join: Option<tokio::task::JoinHandle<()>>,
}

impl ReceiveWorker {
    /// Spawn a worker on the shared media runtime.
    pub fn spawn() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let latest = Arc::new(LatestVideo::default());
        let stall = Arc::new(Mutex::new(StallDetector::default()));
        let audio = Arc::new(AudioOutput::new());
        let latest_c = Arc::clone(&latest);
        let stall_c = Arc::clone(&stall);
        let audio_c = Arc::clone(&audio);

        let join = runtime::spawn(async move {
            worker_loop(rx, latest_c, stall_c, audio_c).await;
        });

        Self {
            tx,
            latest,
            stall,
            audio,
            join: Some(join),
        }
    }

    /// Shared latest frame slot.
    pub fn latest(&self) -> Arc<LatestVideo> {
        Arc::clone(&self.latest)
    }

    /// Shared stall detector.
    pub fn stall(&self) -> Arc<Mutex<StallDetector>> {
        Arc::clone(&self.stall)
    }

    /// Shared audio output (levels / boost).
    pub fn audio(&self) -> Arc<AudioOutput> {
        Arc::clone(&self.audio)
    }

    /// Set playback boost in dB.
    pub fn set_audio_boost_db(&self, db: i32) {
        self.audio.set_boost_db(db);
    }

    /// Select system audio output (`None` = default device).
    pub fn set_audio_output_device(&self, name: Option<String>) {
        self.audio.set_output_device(name);
    }

    /// Currently selected audio output device name (`None` = default).
    pub fn audio_output_device(&self) -> Option<String> {
        self.audio.selected_device_name()
    }

    /// Set A/V playout buffer (milliseconds or frames).
    pub fn set_buffer(&self, settings: BufferSettings) {
        let _ = self.tx.send(ReceiveCommand::SetBuffer(settings));
    }

    /// Ask the worker to connect to `url` with default quality.
    pub fn connect(&self, url: impl Into<String>) {
        let _ = self
            .tx
            .send(ReceiveCommand::Connect(ConnectOptions::new(url)));
    }

    /// Connect with explicit quality options.
    pub fn connect_with(&self, options: ConnectOptions) {
        let _ = self.tx.send(ReceiveCommand::Connect(options));
    }

    /// Disconnect.
    pub fn disconnect(&self) {
        let _ = self.tx.send(ReceiveCommand::Disconnect);
    }

    /// Stop the worker.
    pub fn shutdown(&self) {
        let _ = self.tx.send(ReceiveCommand::Shutdown);
    }
}

impl Drop for ReceiveWorker {
    fn drop(&mut self) {
        let _ = self.tx.send(ReceiveCommand::Shutdown);
        if let Some(join) = self.join.take() {
            join.abort();
        }
    }
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn push_log(latest: &LatestVideo, kind: &str, text: impl Into<String>) {
    let mut q = latest.metadata_log.lock();
    if q.len() >= METADATA_LOG_CAP {
        q.pop_front();
    }
    q.push_back(MetadataLogEntry {
        unix_ms: now_unix_ms(),
        kind: kind.to_string(),
        text: text.into(),
    });
    drop(q);
    {
        let mut c = latest.counters.lock();
        c.metadata_events = c.metadata_events.saturating_add(1);
    }
}

async fn worker_loop(
    mut rx: mpsc::UnboundedReceiver<ReceiveCommand>,
    latest: Arc<LatestVideo>,
    stall: Arc<Mutex<StallDetector>>,
    audio: Arc<AudioOutput>,
) {
    let mut receiver: Option<ReceiverSession> = None;
    let mut playout = Playout::default();
    publish_buffer_delays(&latest, &playout);

    loop {
        // Drain + coalesce so a rapid A→B source switch never fully opens A.
        let mut pending_connect: Option<ConnectOptions> = None;
        let mut pending_disconnect = false;
        let mut pending_buffer: Option<BufferSettings> = None;
        let mut shutdown = false;

        while let Ok(cmd) = rx.try_recv() {
            match cmd {
                ReceiveCommand::Connect(opts) => {
                    pending_disconnect = false;
                    pending_connect = Some(opts);
                }
                ReceiveCommand::Disconnect => {
                    pending_connect = None;
                    pending_disconnect = true;
                }
                ReceiveCommand::SetBuffer(settings) => {
                    pending_buffer = Some(settings);
                }
                ReceiveCommand::Shutdown => {
                    shutdown = true;
                    break;
                }
            }
        }

        if shutdown {
            return;
        }
        if let Some(settings) = pending_buffer {
            playout.set_settings(settings);
            publish_buffer_delays(&latest, &playout);
        }
        if pending_disconnect {
            apply_disconnect(&mut receiver, &latest, &stall, &audio, &mut playout);
        }
        if let Some(opts) = pending_connect {
            apply_connect(opts, &mut receiver, &latest, &stall, &audio, &mut playout).await;
        }

        let Some(recv) = receiver.as_ref() else {
            match rx.recv().await {
                Some(first) => {
                    let mut pending_connect: Option<ConnectOptions> = None;
                    let mut pending_disconnect = false;
                    let mut pending_buffer: Option<BufferSettings> = None;
                    let mut cmds = vec![first];
                    while let Ok(cmd) = rx.try_recv() {
                        cmds.push(cmd);
                    }
                    for cmd in cmds {
                        match cmd {
                            ReceiveCommand::Connect(opts) => {
                                pending_disconnect = false;
                                pending_connect = Some(opts);
                            }
                            ReceiveCommand::Disconnect => {
                                pending_connect = None;
                                pending_disconnect = true;
                            }
                            ReceiveCommand::SetBuffer(settings) => {
                                pending_buffer = Some(settings);
                            }
                            ReceiveCommand::Shutdown => return,
                        }
                    }
                    if let Some(settings) = pending_buffer {
                        playout.set_settings(settings);
                        publish_buffer_delays(&latest, &playout);
                    }
                    if pending_disconnect {
                        apply_disconnect(&mut receiver, &latest, &stall, &audio, &mut playout);
                    }
                    if let Some(opts) = pending_connect {
                        apply_connect(opts, &mut receiver, &latest, &stall, &audio, &mut playout)
                            .await;
                    }
                }
                None => return,
            }
            continue;
        };

        // Wait briefly for video, then drain all ready A/V/metadata into playout.
        let mut got_any = false;
        if let Some(frame) =
            tokio::task::block_in_place(|| recv.recv_video_timeout(Duration::from_millis(5)))
        {
            ingest_video(&latest, &stall, &mut playout, frame);
            got_any = true;
        }
        while let Some(frame) = recv.try_recv_video() {
            ingest_video(&latest, &stall, &mut playout, frame);
            got_any = true;
        }
        while let Some(packet) = recv.try_recv_audio() {
            playout.push_audio(
                packet.timestamp,
                Arc::clone(&packet.pcm_planar_f32),
                packet.channels,
                packet.samples_per_channel,
                packet.sample_rate,
            );
            got_any = true;
        }
        while let Some(meta) = recv.try_recv_metadata() {
            push_log(&latest, "metadata", meta.xml.to_string());
            got_any = true;
        }

        if !got_any {
            stall.lock().tick();
        }
        *latest.stats.lock() = recv.statistics();
        let state = recv.state();
        let prev = *latest.session_state.lock();
        *latest.session_state.lock() = state;
        if state != prev {
            push_log(latest.as_ref(), "session", format!("{state:?}"));
        }
        if let Some(err) = recv.last_error() {
            *latest.error.lock() = Some(err);
        }
        playout.release(&latest, &audio);
        publish_buffer_delays(&latest, &playout);
    }
}

fn ingest_video(
    latest: &LatestVideo,
    stall: &Mutex<StallDetector>,
    playout: &mut Playout,
    frame: openmediatransport::DecodedVideoFrame,
) {
    if frame.width == 0 || frame.height == 0 {
        return;
    }
    if let Some(meta) = frame.frame_metadata.as_ref().filter(|s| !s.is_empty()) {
        push_log(latest, "frame-meta", meta.to_string());
    }
    let width = frame.width;
    let height = frame.height;
    let timestamp = frame.timestamp;
    let fps_n = frame.frame_rate_n;
    let fps_d = frame.frame_rate_d.max(1);
    let bgra = take_bgra(frame);
    let video = VideoFrame {
        width,
        height,
        bgra,
        timestamp,
        fps_n,
        fps_d,
    };
    stall.lock().on_frame(video.fps_n, video.fps_d);
    playout.push_video(video);
}

fn take_bgra(frame: openmediatransport::DecodedVideoFrame) -> Arc<[u8]> {
    let w = frame.width as usize;
    let h = frame.height as usize;
    let stride = frame.stride.max(1) as usize;
    let row = w.saturating_mul(4);
    if stride == row && frame.pixels.len() >= row.saturating_mul(h) {
        if frame.pixels.len() == row * h {
            return frame.pixels;
        }
        return Arc::from(&frame.pixels[..row * h]);
    }
    let mut out = Vec::with_capacity(row.saturating_mul(h));
    for y in 0..h {
        let start = y.saturating_mul(stride);
        let end = start.saturating_add(row);
        if end <= frame.pixels.len() {
            out.extend_from_slice(&frame.pixels[start..end]);
        } else {
            out.resize(row.saturating_mul(h), 0);
            break;
        }
    }
    Arc::from(out.into_boxed_slice())
}

fn publish_buffer_delays(latest: &LatestVideo, playout: &Playout) {
    *latest.video_buffer_delay_ms.lock() = playout.video_delay_ms();
    *latest.audio_buffer_delay_ms.lock() = playout.audio_delay_ms();
}

async fn apply_connect(
    opts: ConnectOptions,
    receiver: &mut Option<ReceiverSession>,
    latest: &LatestVideo,
    stall: &Mutex<StallDetector>,
    audio: &AudioOutput,
    playout: &mut Playout,
) {
    *latest.error.lock() = None;
    // Tear down the previous session before clearing UI state so decode threads
    // cannot publish leftover frames into the new selection.
    if let Some(old) = receiver.take() {
        old.disconnect();
    }
    // Close the prep identity gate and drop leftover pixels from the previous
    // source before opening the new session. Setting `url` only after connect
    // succeeds prevents republishing the old source under the new selection.
    *latest.url.lock() = None;
    latest.clear_video();
    *latest.stats.lock() = SessionStatistics::default();
    *latest.counters.lock() = ReceiveCounters::default();
    *latest.audio_levels.lock() = AudioLevels::default();
    latest.metadata_log.lock().clear();
    audio.clear();
    playout.reset();
    publish_buffer_delays(latest, playout);
    stall.lock().reset();
    *latest.session_state.lock() = SessionState::Connecting;
    let url = opts.url.clone();
    match open_receiver(opts).await {
        Ok(r) => {
            *latest.session_state.lock() = r.state();
            *latest.url.lock() = Some(url.clone());
            *receiver = Some(r);
            push_log(latest, "info", format!("connected {url}"));
        }
        Err(e) => {
            *receiver = None;
            *latest.session_state.lock() = SessionState::Stopped;
            *latest.error.lock() = Some(e.to_string());
            push_log(latest, "error", e.to_string());
        }
    }
}

fn apply_disconnect(
    receiver: &mut Option<ReceiverSession>,
    latest: &LatestVideo,
    stall: &Mutex<StallDetector>,
    audio: &AudioOutput,
    playout: &mut Playout,
) {
    if let Some(session) = receiver.take() {
        session.disconnect();
    }
    *latest.url.lock() = None;
    *latest.session_state.lock() = SessionState::Stopped;
    *latest.stats.lock() = SessionStatistics::default();
    latest.clear_video();
    *latest.audio_levels.lock() = AudioLevels::default();
    audio.clear();
    playout.reset();
    stall.lock().reset();
    push_log(latest, "info", "disconnected");
}

async fn open_receiver(opts: ConnectOptions) -> Result<ReceiverSession, OmtError> {
    let url = opts.url.clone();
    let addresses = opts.addresses.clone();
    let quality = opts.quality;
    let preview = opts.preview;
    tokio::task::spawn_blocking(move || {
        let config = ReceiverConfig {
            frame_types: FrameType::VIDEO | FrameType::AUDIO | FrameType::METADATA,
            quality,
            preview,
            connect_timeout: Duration::from_secs(5),
            auto_reconnect: true,
        };
        ReceiverSession::connect_with_addresses(url, &addresses, config)
    })
    .await
    .map_err(|e| OmtError::Network(e.to_string()))?
}

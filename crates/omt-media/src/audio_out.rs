//! System audio playback for received OMT PCM (cpal).

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, StreamConfig};
use parking_lot::Mutex;
use tracing::warn;

const RING_CAP_SAMPLES: usize = 48_000 * 2 * 2; // ~2s stereo @ 48 kHz
const TICKS_PER_SECOND: i64 = 10_000_000;
/// If a stream is open but the callback never consumes PCM, mute after this.
const DEAD_OUTPUT_GRACE: Duration = Duration::from_millis(500);

/// Peak levels for VU display (linear 0..1, per channel).
#[derive(Debug, Clone, Copy, Default)]
pub struct AudioLevels {
    /// Left (or mono) peak.
    pub peak_l: f32,
    /// Right peak (0 when mono).
    pub peak_r: f32,
    /// Audio packets pushed into the player.
    pub frames: u64,
    /// Declared sample rate of the last packet.
    pub sample_rate: i32,
    /// Declared channel count of the last packet.
    pub channels: i32,
}

/// Whether system audio playback is currently usable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioOutputStatus {
    /// Output device is being opened.
    Opening,
    /// Device stream is running.
    Ready,
    /// No usable device; PCM is discarded and video stays on wall clock.
    Unavailable,
}

/// A selectable system audio output device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioOutputDevice {
    /// Device name (stable enough for selection within a session).
    pub name: String,
    /// Whether this is the current host default output.
    pub is_default: bool,
}

/// List available cpal output devices (best-effort).
pub fn list_output_devices() -> Vec<AudioOutputDevice> {
    let host = cpal::default_host();
    let default_name = host.default_output_device().map(|d| d.to_string());
    let Ok(devices) = host.output_devices() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for device in devices {
        let name = device.to_string();
        let is_default = default_name.as_ref() == Some(&name);
        out.push(AudioOutputDevice { name, is_default });
    }
    out.sort_by(|a, b| match (b.is_default, a.is_default) {
        (true, false) => std::cmp::Ordering::Greater,
        (false, true) => std::cmp::Ordering::Less,
        _ => a.name.cmp(&b.name),
    });
    out
}

struct DeviceGeometry {
    channels: usize,
    rate: u32,
}

/// One queued PCM run with stream timeline metadata (device frames).
struct PtsRun {
    /// PTS of the first device frame in this run.
    pts_start: i64,
    /// Stream duration of this run in 100 ns ticks.
    pts_duration: i64,
    /// Device frames remaining in this run.
    frames_total: usize,
    /// Device frames already consumed from this run.
    frames_consumed: usize,
}

struct Shared {
    /// Interleaved f32 ring (device channel count).
    ring: Mutex<VecDeque<f32>>,
    /// Parallel PTS timeline for samples in `ring` (same order).
    pts_runs: Mutex<VecDeque<PtsRun>>,
    levels: Mutex<AudioLevels>,
    /// Playback boost in dB (applied when pushing).
    boost_db: AtomicI32,
    geometry: Mutex<DeviceGeometry>,
    /// `None` = system default output.
    device_name: Mutex<Option<String>>,
    /// PTS currently being output (valid when `playhead_valid`).
    playhead_pts: AtomicI64,
    playhead_valid: AtomicBool,
    /// Last time the device callback consumed queued PCM.
    last_playback_activity: Mutex<Option<Instant>>,
    status: Mutex<AudioOutputStatus>,
    /// Fast path for [`AudioOutput::push_planar_f32`].
    output_enabled: AtomicBool,
    /// At least one packet was queued after the current stream opened.
    queued_since_open: AtomicBool,
    /// When the first PCM packet was queued into the current Ready stream.
    /// Dead-output grace is measured from this instant, not stream open:
    /// the default device is opened at app start and then sits idle until
    /// an OMT source connects, often well past [`DEAD_OUTPUT_GRACE`].
    first_queued_at: Mutex<Option<Instant>>,
}

enum AudioCmd {
    SetDevice(Option<String>),
    Shutdown,
}

/// Plays decoded planar float PCM on a selectable output device.
///
/// The cpal stream is owned by a dedicated OS thread (`cpal::Stream` is not
/// `Send`), while PCM / levels are shared via [`Arc`].
pub struct AudioOutput {
    shared: Arc<Shared>,
    cmd_tx: Mutex<Option<Sender<AudioCmd>>>,
}

impl Default for AudioOutput {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioOutput {
    /// Open the default output device (falls back to a silent stub on failure).
    pub fn new() -> Self {
        let shared = Arc::new(Shared {
            ring: Mutex::new(VecDeque::with_capacity(RING_CAP_SAMPLES.min(8192))),
            pts_runs: Mutex::new(VecDeque::new()),
            levels: Mutex::new(AudioLevels::default()),
            boost_db: AtomicI32::new(0),
            geometry: Mutex::new(DeviceGeometry {
                channels: 2,
                rate: 48_000,
            }),
            device_name: Mutex::new(None),
            playhead_pts: AtomicI64::new(0),
            playhead_valid: AtomicBool::new(false),
            last_playback_activity: Mutex::new(None),
            status: Mutex::new(AudioOutputStatus::Opening),
            output_enabled: AtomicBool::new(false),
            queued_since_open: AtomicBool::new(false),
            first_queued_at: Mutex::new(None),
        });

        let (cmd_tx, cmd_rx) = mpsc::channel();
        let shared_thread = Arc::clone(&shared);
        if let Err(e) = thread::Builder::new()
            .name("omt-audio-out".into())
            .spawn(move || {
                if let Err(e) = run_output_thread(shared_thread, cmd_rx) {
                    warn!("audio output thread ended: {e}");
                }
            })
        {
            warn!("failed to spawn audio output thread: {e}");
            apply_status(&shared, AudioOutputStatus::Unavailable);
            return Self {
                shared,
                cmd_tx: Mutex::new(None),
            };
        }

        Self {
            shared,
            cmd_tx: Mutex::new(Some(cmd_tx)),
        }
    }

    /// Currently selected device name (`None` = system default).
    pub fn selected_device_name(&self) -> Option<String> {
        self.shared.device_name.lock().clone()
    }

    /// Snapshot of whether PCM is actually being sent to a device.
    pub fn status(&self) -> AudioOutputStatus {
        *self.shared.status.lock()
    }

    /// Switch output device. `None` selects the system default.
    pub fn set_output_device(&self, name: Option<String>) {
        *self.shared.device_name.lock() = name.clone();
        self.clear();
        if let Some(tx) = self.cmd_tx.lock().as_ref() {
            apply_status(&self.shared, AudioOutputStatus::Opening);
            let _ = tx.send(AudioCmd::SetDevice(name));
        } else {
            apply_status(&self.shared, AudioOutputStatus::Unavailable);
        }
    }

    /// Snapshot of recent true-peak levels (linear 0..1).
    pub fn levels(&self) -> AudioLevels {
        *self.shared.levels.lock()
    }

    /// Set playback boost in dB (typical 0 / 6 / 10 / 20).
    pub fn set_boost_db(&self, db: i32) {
        self.shared.boost_db.store(db, Ordering::Relaxed);
    }

    /// PTS of the sample currently being played, if the timeline is active.
    pub fn playhead_pts(&self) -> Option<i64> {
        if self.shared.playhead_valid.load(Ordering::Acquire) {
            Some(self.shared.playhead_pts.load(Ordering::Acquire))
        } else {
            None
        }
    }

    /// Whether the output callback has recently consumed queued PCM.
    pub(crate) fn playback_active(&self) -> bool {
        self.shared
            .last_playback_activity
            .lock()
            .is_some_and(|at| at.elapsed() <= Duration::from_millis(250))
    }

    /// Buffered audio duration currently sitting in the device ring (milliseconds).
    pub fn buffered_ms(&self) -> f64 {
        let (ch, rate) = {
            let geo = self.shared.geometry.lock();
            (geo.channels.max(1), geo.rate.max(1) as f64)
        };
        let samples = self.shared.ring.lock().len();
        let frames = samples / ch;
        (frames as f64) * 1000.0 / rate
    }

    /// Device output sample rate currently configured.
    pub fn device_sample_rate(&self) -> u32 {
        self.shared.geometry.lock().rate.max(1)
    }

    /// Clear the ring buffer (on disconnect / source change).
    pub fn clear(&self) {
        self.shared.ring.lock().clear();
        self.shared.pts_runs.lock().clear();
        *self.shared.levels.lock() = AudioLevels::default();
        *self.shared.last_playback_activity.lock() = None;
        self.invalidate_playhead();
    }

    /// Drop the media-clock playhead without touching buffered PCM.
    pub fn invalidate_playhead(&self) {
        self.shared.playhead_valid.store(false, Ordering::Release);
        self.shared.playhead_pts.store(0, Ordering::Release);
        *self.shared.last_playback_activity.lock() = None;
    }

    /// Push planar f32 PCM (`ch0[samples]…chN[samples]` tightly packed as LE bytes).
    ///
    /// `timestamp` is the OMT PTS (100 ns ticks) of the first sample in the packet.
    pub fn push_planar_f32(
        &self,
        data: &[u8],
        channels: i32,
        samples: i32,
        sample_rate: i32,
        timestamp: i64,
    ) {
        let ch = channels.max(1) as usize;
        let n = samples.max(0) as usize;
        let expected = ch * n * 4;
        if n == 0 || data.len() < expected {
            return;
        }

        if self.should_discard_pcm() {
            self.record_silent_packet(sample_rate, channels);
            return;
        }

        let gain = db_to_gain(self.shared.boost_db.load(Ordering::Relaxed));
        let mut peak_l = 0.0f32;
        let mut peak_r = 0.0f32;

        let mut planes: Vec<Vec<f32>> = Vec::with_capacity(ch);
        for c in 0..ch {
            let base = c * n * 4;
            let mut plane = Vec::with_capacity(n);
            for s in 0..n {
                let o = base + s * 4;
                let sample = f32::from_le_bytes(data[o..o + 4].try_into().unwrap()) * gain;
                let a = sample.abs();
                if c == 0 {
                    peak_l = peak_l.max(a);
                } else if c == 1 {
                    peak_r = peak_r.max(a);
                }
                plane.push(sample.clamp(-1.0, 1.0));
            }
            planes.push(plane);
        }
        if ch == 1 {
            peak_r = peak_l;
        }

        {
            let mut levels = self.shared.levels.lock();
            // Hold true packet peak for metering (matches external tools like vMix).
            levels.peak_l = peak_l;
            levels.peak_r = peak_r;
            levels.frames = levels.frames.saturating_add(1);
            levels.sample_rate = sample_rate;
            levels.channels = channels;
        }

        let (out_ch, dst_rate) = {
            let geo = self.shared.geometry.lock();
            (geo.channels.max(1), geo.rate.max(1))
        };
        let src_rate = sample_rate.max(1) as u32;
        let out_n = if src_rate == dst_rate {
            n
        } else {
            ((n as u64 * dst_rate as u64) / src_rate as u64).max(1) as usize
        };
        let pts_duration = (n as i64)
            .saturating_mul(TICKS_PER_SECOND)
            .saturating_div(i64::from(sample_rate.max(1)));

        let mut ring = self.shared.ring.lock();
        let mut pts_runs = self.shared.pts_runs.lock();

        // Drop oldest samples + matching timeline when the ring overflows.
        let mut dropped_frames = 0usize;
        for i in 0..out_n {
            let src_i = if out_n == n {
                i
            } else {
                ((i as u64 * n as u64) / out_n as u64) as usize
            };
            for c in 0..out_ch {
                let sample = if c < ch {
                    planes[c][src_i.min(n - 1)]
                } else if ch == 1 {
                    planes[0][src_i.min(n - 1)]
                } else {
                    0.0
                };
                if ring.len() >= RING_CAP_SAMPLES {
                    ring.pop_front();
                    if c == 0 {
                        dropped_frames += 1;
                    }
                }
                ring.push_back(sample);
            }
        }
        for _ in 0..dropped_frames {
            consume_pts_frames(&mut pts_runs, 1, &self.shared);
        }

        if out_n > 0 {
            pts_runs.push_back(PtsRun {
                pts_start: timestamp,
                pts_duration: pts_duration.max(1),
                frames_total: out_n,
                frames_consumed: 0,
            });
            if !self.shared.playhead_valid.load(Ordering::Acquire) {
                self.shared.playhead_pts.store(timestamp, Ordering::Release);
                self.shared.playhead_valid.store(true, Ordering::Release);
            }
        }
        drop(pts_runs);
        drop(ring);
        if out_n > 0 && !self.shared.queued_since_open.swap(true, Ordering::AcqRel) {
            *self.shared.first_queued_at.lock() = Some(Instant::now());
        }
    }

    fn should_discard_pcm(&self) -> bool {
        self.maybe_declare_dead_output();
        !self.shared.output_enabled.load(Ordering::Acquire)
    }

    fn maybe_declare_dead_output(&self) {
        if !self.shared.output_enabled.load(Ordering::Acquire) {
            return;
        }
        if self.playback_active() {
            return;
        }
        if !self.shared.queued_since_open.load(Ordering::Acquire) {
            return;
        }
        let Some(queued_at) = *self.shared.first_queued_at.lock() else {
            return;
        };
        if queued_at.elapsed() <= DEAD_OUTPUT_GRACE {
            return;
        }
        warn!("audio output opened but is not consuming PCM; muting");
        apply_status(&self.shared, AudioOutputStatus::Unavailable);
        self.shared.ring.lock().clear();
        self.shared.pts_runs.lock().clear();
        self.invalidate_playhead();
        let mut levels = self.shared.levels.lock();
        levels.peak_l = 0.0;
        levels.peak_r = 0.0;
    }

    fn record_silent_packet(&self, sample_rate: i32, channels: i32) {
        let mut levels = self.shared.levels.lock();
        levels.peak_l = 0.0;
        levels.peak_r = 0.0;
        levels.frames = levels.frames.saturating_add(1);
        levels.sample_rate = sample_rate;
        levels.channels = channels;
    }
}

impl Drop for AudioOutput {
    fn drop(&mut self) {
        if let Some(tx) = self.cmd_tx.lock().take() {
            let _ = tx.send(AudioCmd::Shutdown);
        }
    }
}

fn consume_pts_frames(runs: &mut VecDeque<PtsRun>, mut frames: usize, shared: &Shared) {
    while frames > 0 {
        let Some(run) = runs.front_mut() else {
            break;
        };
        let left = run.frames_total.saturating_sub(run.frames_consumed);
        if left == 0 {
            runs.pop_front();
            continue;
        }
        let take = frames.min(left);
        run.frames_consumed += take;
        let progressed = run.frames_consumed as i64;
        let total = run.frames_total.max(1) as i64;
        let pts = run.pts_start + (run.pts_duration.saturating_mul(progressed) / total);
        shared.playhead_pts.store(pts, Ordering::Release);
        shared.playhead_valid.store(true, Ordering::Release);
        if run.frames_consumed >= run.frames_total {
            runs.pop_front();
        }
        frames -= take;
    }
}

fn db_to_gain(db: i32) -> f32 {
    10f32.powf(db as f32 / 20.0)
}

fn apply_status(shared: &Shared, status: AudioOutputStatus) {
    *shared.status.lock() = status;
    let ready = matches!(status, AudioOutputStatus::Ready);
    shared.output_enabled.store(ready, Ordering::Release);
    if ready {
        *shared.first_queued_at.lock() = None;
        shared.queued_since_open.store(false, Ordering::Release);
    } else {
        *shared.first_queued_at.lock() = None;
        shared.queued_since_open.store(false, Ordering::Release);
        shared.output_enabled.store(false, Ordering::Release);
    }
}

fn wait_device_cmd(cmd_rx: &mpsc::Receiver<AudioCmd>) -> Option<Option<String>> {
    match cmd_rx.recv() {
        Ok(AudioCmd::SetDevice(name)) => Some(name),
        Ok(AudioCmd::Shutdown) | Err(_) => None,
    }
}

fn resolve_device(name: &Option<String>) -> Result<cpal::Device, String> {
    let host = cpal::default_host();
    if let Some(want) = name {
        let devices = host.output_devices().map_err(|e| e.to_string())?;
        for device in devices {
            if &device.to_string() == want {
                return Ok(device);
            }
        }
        warn!("audio device `{want}` not found; falling back to default");
    }
    host.default_output_device()
        .ok_or_else(|| "no default output device".to_string())
}

fn run_output_thread(shared: Arc<Shared>, cmd_rx: mpsc::Receiver<AudioCmd>) -> Result<(), String> {
    let mut selected = shared.device_name.lock().clone();
    loop {
        apply_status(&shared, AudioOutputStatus::Opening);
        let device = match resolve_device(&selected) {
            Ok(d) => d,
            Err(e) => {
                warn!("audio output unavailable: {e}");
                apply_status(&shared, AudioOutputStatus::Unavailable);
                match wait_device_cmd(&cmd_rx) {
                    Some(name) => {
                        selected = name;
                        *shared.device_name.lock() = selected.clone();
                        continue;
                    }
                    None => return Ok(()),
                }
            }
        };

        let supported = match device.default_output_config() {
            Ok(c) => c,
            Err(e) => {
                warn!("audio output config failed: {e}");
                apply_status(&shared, AudioOutputStatus::Unavailable);
                match wait_device_cmd(&cmd_rx) {
                    Some(name) => {
                        selected = name;
                        *shared.device_name.lock() = selected.clone();
                        continue;
                    }
                    None => return Ok(()),
                }
            }
        };
        let sample_format = supported.sample_format();
        let config: StreamConfig = supported.into();
        {
            let mut geo = shared.geometry.lock();
            geo.channels = config.channels as usize;
            geo.rate = config.sample_rate;
        }
        shared.ring.lock().clear();
        shared.pts_runs.lock().clear();
        shared.playhead_valid.store(false, Ordering::Release);
        *shared.last_playback_activity.lock() = None;

        let shared_cb = Arc::clone(&shared);
        let stream = match sample_format {
            SampleFormat::F32 => build_stream::<f32>(&device, &config, shared_cb),
            SampleFormat::I16 => build_stream::<i16>(&device, &config, shared_cb),
            SampleFormat::U16 => build_stream::<u16>(&device, &config, shared_cb),
            other => {
                warn!("unsupported sample format: {other:?}");
                apply_status(&shared, AudioOutputStatus::Unavailable);
                match wait_device_cmd(&cmd_rx) {
                    Some(name) => {
                        selected = name;
                        *shared.device_name.lock() = selected.clone();
                        continue;
                    }
                    None => return Ok(()),
                }
            }
        };
        let stream = match stream {
            Ok(s) => s,
            Err(e) => {
                warn!("audio stream build failed: {e}");
                apply_status(&shared, AudioOutputStatus::Unavailable);
                match wait_device_cmd(&cmd_rx) {
                    Some(name) => {
                        selected = name;
                        *shared.device_name.lock() = selected.clone();
                        continue;
                    }
                    None => return Ok(()),
                }
            }
        };
        if let Err(e) = stream.play() {
            warn!("audio stream play failed: {e}");
            drop(stream);
            apply_status(&shared, AudioOutputStatus::Unavailable);
            match wait_device_cmd(&cmd_rx) {
                Some(name) => {
                    selected = name;
                    *shared.device_name.lock() = selected.clone();
                    continue;
                }
                None => return Ok(()),
            }
        }
        apply_status(&shared, AudioOutputStatus::Ready);

        match wait_device_cmd(&cmd_rx) {
            Some(name) => {
                drop(stream);
                selected = name;
                *shared.device_name.lock() = selected.clone();
            }
            None => {
                drop(stream);
                apply_status(&shared, AudioOutputStatus::Unavailable);
                return Ok(());
            }
        }
    }
}

fn build_stream<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    shared: Arc<Shared>,
) -> Result<cpal::Stream, String>
where
    T: cpal::Sample + cpal::SizedSample + cpal::FromSample<f32>,
{
    let channels = config.channels.max(1) as usize;
    let err_shared = Arc::clone(&shared);
    device
        .build_output_stream(
            *config,
            move |data: &mut [T], _| {
                let mut ring = shared.ring.lock();
                let mut got = 0usize;
                for sample in data.iter_mut() {
                    match ring.pop_front() {
                        Some(v) => {
                            *sample = T::from_sample(v);
                            got += 1;
                        }
                        None => {
                            *sample = T::from_sample(0.0);
                        }
                    }
                }
                drop(ring);

                let frames_from_ring = got / channels;
                if frames_from_ring > 0 {
                    let mut runs = shared.pts_runs.lock();
                    consume_pts_frames(&mut runs, frames_from_ring, &shared);
                    *shared.last_playback_activity.lock() = Some(Instant::now());
                }
            },
            move |err| {
                warn!("audio stream error: {err}");
                apply_status(&err_shared, AudioOutputStatus::Unavailable);
            },
            None,
        )
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stub(status: AudioOutputStatus) -> AudioOutput {
        let ready = matches!(status, AudioOutputStatus::Ready);
        let shared = Arc::new(Shared {
            ring: Mutex::new(VecDeque::with_capacity(8192)),
            pts_runs: Mutex::new(VecDeque::new()),
            levels: Mutex::new(AudioLevels::default()),
            boost_db: AtomicI32::new(0),
            geometry: Mutex::new(DeviceGeometry {
                channels: 2,
                rate: 48_000,
            }),
            device_name: Mutex::new(None),
            playhead_pts: AtomicI64::new(0),
            playhead_valid: AtomicBool::new(false),
            last_playback_activity: Mutex::new(None),
            status: Mutex::new(status),
            output_enabled: AtomicBool::new(ready),
            queued_since_open: AtomicBool::new(false),
            first_queued_at: Mutex::new(None),
        });
        AudioOutput {
            shared,
            cmd_tx: Mutex::new(None),
        }
    }

    fn loud_packet() -> Vec<u8> {
        let sample = 0.75f32.to_le_bytes();
        let mut data = Vec::with_capacity(8);
        data.extend_from_slice(&sample);
        data.extend_from_slice(&sample);
        data
    }

    #[test]
    fn unavailable_output_discards_pcm_and_vu() {
        let audio = stub(AudioOutputStatus::Unavailable);
        audio.push_planar_f32(&loud_packet(), 1, 1, 48_000, 10_000);
        let levels = audio.levels();
        assert_eq!(levels.frames, 1);
        assert_eq!(levels.peak_l, 0.0);
        assert_eq!(levels.peak_r, 0.0);
        assert!(audio.playhead_pts().is_none());
        assert_eq!(audio.buffered_ms(), 0.0);
        assert_eq!(audio.status(), AudioOutputStatus::Unavailable);
    }

    #[test]
    fn idle_ready_stream_mutes_after_grace() {
        let audio = stub(AudioOutputStatus::Ready);
        audio.push_planar_f32(&loud_packet(), 1, 1, 48_000, 10_000);
        assert!(audio.buffered_ms() > 0.0);
        assert_eq!(audio.status(), AudioOutputStatus::Ready);

        *audio.shared.first_queued_at.lock() =
            Some(Instant::now() - DEAD_OUTPUT_GRACE - Duration::from_millis(50));
        audio.push_planar_f32(&loud_packet(), 1, 1, 48_000, 20_000);
        assert_eq!(audio.status(), AudioOutputStatus::Unavailable);
        assert_eq!(audio.buffered_ms(), 0.0);
        assert!(audio.playhead_pts().is_none());
        let levels = audio.levels();
        assert_eq!(levels.peak_l, 0.0);
        assert_eq!(levels.frames, 2);
    }

    #[test]
    fn preroll_burst_on_long_idle_default_device_stays_ready() {
        // System default is opened at app start. After the user later connects,
        // playout releases the preroll as a burst before the callback can run.
        let audio = stub(AudioOutputStatus::Ready);
        audio.push_planar_f32(&loud_packet(), 1, 1, 48_000, 10_000);
        audio.push_planar_f32(&loud_packet(), 1, 1, 48_000, 20_000);
        audio.push_planar_f32(&loud_packet(), 1, 1, 48_000, 30_000);
        assert_eq!(audio.status(), AudioOutputStatus::Ready);
        assert!(audio.buffered_ms() > 0.0);
        let levels = audio.levels();
        assert_eq!(levels.frames, 3);
        assert!(levels.peak_l > 0.0);
    }
}

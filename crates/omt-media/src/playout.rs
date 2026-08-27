//! PTS-based A/V playout with linked or independent delay buffers.
//!
//! Once audio is playing, the device playhead is the media clock master so
//! video stays locked to PCM even when wall time and the DAC clock drift.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;

use crate::audio_out::AudioOutput;
use crate::receive::{LatestVideo, VideoFrame};
use openmediatransport::{DecodedVideoGpuFrame, GpuVideoContext};

const TICKS_PER_SECOND: f64 = 10_000_000.0;
const TICKS_PER_MS: i64 = 10_000;
const VIDEO_Q_CAP: usize = 8;
const AUDIO_Q_CAP: usize = 48;
/// Owned GPU copies: playout depth plus one in-flight ingest and one held by the UI.
const GPU_TEX_POOL: usize = VIDEO_Q_CAP + 2;
/// Soft resync when oldest queued PTS is this late vs the media clock.
const RESNAP_MIN_LATE_MS: i64 = 1_500;
/// Never hard-snap unless at least this late (prefer soft catch-up).
const RESNAP_HARD_LATE_MS: i64 = 3_000;

/// Whether buffer depth is measured in milliseconds or video frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BufferUnit {
    /// Wall-clock milliseconds of pre-roll.
    #[default]
    Milliseconds,
    /// Source frame intervals (uses last known FPS, default 30).
    Frames,
}

/// One stream's delay setting (amount + unit).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DelaySetting {
    /// Magnitude in [`BufferUnit`] units.
    pub amount: u32,
    /// Unit for [`amount`](Self::amount).
    pub unit: BufferUnit,
}

impl DelaySetting {
    /// Convert to a clamped delay in milliseconds.
    pub fn delay_ms(self, fps_n: i32, fps_d: i32) -> u32 {
        let ms = match self.unit {
            BufferUnit::Milliseconds => self.amount as f64,
            BufferUnit::Frames => {
                let fps = fps_n.max(1) as f64 / fps_d.max(1) as f64;
                self.amount as f64 * 1000.0 / fps.max(1.0)
            }
        };
        ms.round().clamp(0.0, 2_000.0) as u32
    }

    /// Build a setting in `unit` that matches approximately `ms` at the given FPS.
    pub fn from_ms(ms: u32, unit: BufferUnit, fps_n: i32, fps_d: i32) -> Self {
        let ms = (ms as f64).clamp(0.0, 2_000.0);
        match unit {
            BufferUnit::Milliseconds => Self {
                amount: ms.round() as u32,
                unit,
            },
            BufferUnit::Frames => {
                let fps = fps_n.max(1) as f64 / fps_d.max(1) as f64;
                let frames = (ms * fps.max(1.0) / 1000.0).round().clamp(0.0, 120.0) as u32;
                Self {
                    amount: frames,
                    unit,
                }
            }
        }
    }
}

/// User-facing A/V buffer depth (linked or independent).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BufferSettings {
    /// When true, video/audio delays stay matched via the current frame rate.
    pub linked: bool,
    /// Video playout delay.
    pub video: DelaySetting,
    /// Audio playout delay.
    pub audio: DelaySetting,
}

impl Default for BufferSettings {
    fn default() -> Self {
        // 6 frames @ 30 fps ≈ 200 ms — linked by default (more jitter headroom).
        Self {
            linked: true,
            video: DelaySetting {
                amount: 6,
                unit: BufferUnit::Frames,
            },
            audio: DelaySetting {
                amount: 200,
                unit: BufferUnit::Milliseconds,
            },
        }
    }
}

impl BufferSettings {
    /// Effective video delay in milliseconds.
    pub fn video_delay_ms(self, fps_n: i32, fps_d: i32) -> u32 {
        self.video.delay_ms(fps_n, fps_d)
    }

    /// Effective audio delay in milliseconds.
    pub fn audio_delay_ms(self, fps_n: i32, fps_d: i32) -> u32 {
        if self.linked {
            self.video.delay_ms(fps_n, fps_d)
        } else {
            self.audio.delay_ms(fps_n, fps_d)
        }
    }

    /// Update video delay; when linked, refresh audio to the rounded equivalent.
    pub fn set_video(&mut self, video: DelaySetting, fps_n: i32, fps_d: i32) {
        self.video = video;
        if self.linked {
            let ms = video.delay_ms(fps_n, fps_d);
            self.audio = DelaySetting::from_ms(ms, self.audio.unit, fps_n, fps_d);
        }
    }

    /// Update audio delay; when linked, refresh video to the rounded equivalent.
    pub fn set_audio(&mut self, audio: DelaySetting, fps_n: i32, fps_d: i32) {
        self.audio = audio;
        if self.linked {
            let ms = audio.delay_ms(fps_n, fps_d);
            self.video = DelaySetting::from_ms(ms, self.video.unit, fps_n, fps_d);
        }
    }

    /// Enable/disable link. Turning link on snaps audio to video.
    pub fn set_linked(&mut self, linked: bool, fps_n: i32, fps_d: i32) {
        self.linked = linked;
        if linked {
            let ms = self.video.delay_ms(fps_n, fps_d);
            self.audio = DelaySetting::from_ms(ms, self.audio.unit, fps_n, fps_d);
        }
    }

    /// Keep the linked pair consistent after FPS changes (video is master).
    pub fn resync_linked(&mut self, fps_n: i32, fps_d: i32) {
        if self.linked {
            let ms = self.video.delay_ms(fps_n, fps_d);
            self.audio = DelaySetting::from_ms(ms, self.audio.unit, fps_n, fps_d);
        }
    }
}

struct PendingAudio {
    timestamp: i64,
    data: Arc<[u8]>,
    channels: i32,
    samples: i32,
    sample_rate: i32,
}

enum QueuedVideo {
    Cpu(VideoFrame),
    Gpu(DecodedVideoGpuFrame),
}

impl QueuedVideo {
    fn timestamp(&self) -> i64 {
        match self {
            Self::Cpu(v) => v.timestamp,
            Self::Gpu(v) => v.timestamp,
        }
    }

    fn fps(&self) -> (i32, i32) {
        match self {
            Self::Cpu(v) => (v.fps_n, v.fps_d.max(1)),
            Self::Gpu(v) => (v.frame_rate_n, v.frame_rate_d.max(1)),
        }
    }
}

/// Shared media-clock gate for video + audio packets.
pub struct Playout {
    settings: BufferSettings,
    pts_origin: Option<i64>,
    wall_origin: Option<Instant>,
    /// Soft correction applied on top of the wall clock (100 ns ticks).
    clock_skew_ticks: i64,
    fps_n: i32,
    fps_d: i32,
    video_q: VecDeque<QueuedVideo>,
    audio_q: VecDeque<PendingAudio>,
    gpu_pool: GpuCopyPool,
}

impl Default for Playout {
    fn default() -> Self {
        Self {
            settings: BufferSettings::default(),
            pts_origin: None,
            wall_origin: None,
            clock_skew_ticks: 0,
            fps_n: 30,
            fps_d: 1,
            video_q: VecDeque::new(),
            audio_q: VecDeque::new(),
            gpu_pool: GpuCopyPool::default(),
        }
    }
}

/// Reusable `Bgra8Unorm` textures so playout does not hold vmx's RING=3 output.
struct GpuCopyPool {
    width: u32,
    height: u32,
    free: Vec<wgpu::Texture>,
}

impl Default for GpuCopyPool {
    fn default() -> Self {
        Self {
            width: 0,
            height: 0,
            free: Vec::new(),
        }
    }
}

impl GpuCopyPool {
    fn acquire(&mut self, device: &wgpu::Device, width: u32, height: u32) -> wgpu::Texture {
        if self.width != width || self.height != height {
            self.free.clear();
            self.width = width;
            self.height = height;
        }
        self.free.pop().unwrap_or_else(|| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("omt-playout-gpu"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Bgra8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        })
    }

    fn release(&mut self, tex: wgpu::Texture) {
        let size = tex.size();
        if size.width != self.width || size.height != self.height {
            return;
        }
        if self.free.len() < GPU_TEX_POOL {
            self.free.push(tex);
        }
    }

    fn clear(&mut self) {
        self.free.clear();
        self.width = 0;
        self.height = 0;
    }
}

fn copy_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    src: &wgpu::Texture,
    dst: &wgpu::Texture,
    width: u32,
    height: u32,
) {
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("omt-gpu-ingest-copy"),
    });
    enc.copy_texture_to_texture(
        wgpu::TexelCopyTextureInfo {
            texture: src,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyTextureInfo {
            texture: dst,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(enc.finish()));
}

impl Playout {
    /// Replace buffer settings (clock continues; depth changes immediately).
    pub fn set_settings(&mut self, settings: BufferSettings) {
        self.settings = settings;
    }

    /// Effective video delay in milliseconds at the current FPS.
    pub fn video_delay_ms(&self) -> u32 {
        self.settings.video_delay_ms(self.fps_n, self.fps_d)
    }

    /// Effective audio delay in milliseconds at the current FPS.
    pub fn audio_delay_ms(&self) -> u32 {
        self.settings.audio_delay_ms(self.fps_n, self.fps_d)
    }

    /// Clear queues and clock (disconnect / reconnect).
    pub fn reset(&mut self) {
        self.pts_origin = None;
        self.wall_origin = None;
        self.clock_skew_ticks = 0;
        while let Some(frame) = self.video_q.pop_front() {
            self.recycle_queued(frame);
        }
        self.audio_q.clear();
        self.gpu_pool.clear();
    }

    /// GPU-copy a vmx ring texture into a playout-owned slot (no CPU wait).
    pub(crate) fn copy_gpu_frame(
        &mut self,
        ctx: &GpuVideoContext,
        mut frame: DecodedVideoGpuFrame,
    ) -> DecodedVideoGpuFrame {
        let dst = self
            .gpu_pool
            .acquire(&ctx.device, frame.width, frame.height);
        copy_texture(
            &ctx.device,
            &ctx.queue,
            &frame.texture,
            &dst,
            frame.width,
            frame.height,
        );
        frame.texture = dst;
        frame
    }

    fn recycle_queued(&mut self, frame: QueuedVideo) {
        if let QueuedVideo::Gpu(gpu) = frame {
            self.gpu_pool.release(gpu.texture);
        }
    }

    /// Enqueue a decoded video frame.
    pub fn push_video(&mut self, frame: VideoFrame) {
        self.push_queued(QueuedVideo::Cpu(frame));
    }

    /// Enqueue a GPU-decoded texture frame (same PTS gate as CPU).
    pub fn push_gpu_video(&mut self, frame: DecodedVideoGpuFrame) {
        self.push_queued(QueuedVideo::Gpu(frame));
    }

    fn push_queued(&mut self, frame: QueuedVideo) {
        let (fps_n, fps_d) = frame.fps();
        if fps_n > 0 {
            let changed = self.fps_n != fps_n || self.fps_d != fps_d;
            self.fps_n = fps_n;
            self.fps_d = fps_d;
            if changed {
                self.settings.resync_linked(self.fps_n, self.fps_d);
            }
        }
        self.note_clock(frame.timestamp());
        self.video_q.push_back(frame);
        while self.video_q.len() > VIDEO_Q_CAP {
            if let Some(old) = self.video_q.pop_front() {
                self.recycle_queued(old);
            }
        }
    }

    /// Enqueue a decoded audio packet (planar f32 bytes).
    pub fn push_audio(
        &mut self,
        timestamp: i64,
        data: Arc<[u8]>,
        channels: i32,
        samples: i32,
        sample_rate: i32,
    ) {
        self.note_clock(timestamp);
        self.audio_q.push_back(PendingAudio {
            timestamp,
            data,
            channels,
            samples,
            sample_rate,
        });
        while self.audio_q.len() > AUDIO_Q_CAP {
            self.audio_q.pop_front();
        }
    }

    /// Release packets whose PTS is due on the (possibly split) media clock.
    pub fn release(&mut self, latest: &LatestVideo, audio: &AudioOutput) {
        self.steer_clock_to_audio_ring(audio);

        let audio_delay = self.audio_delay_ms();
        let video_delay = self.video_delay_ms();
        let Some((audio_mt, video_mt)) = self.gate_times(audio, audio_delay, video_delay) else {
            return;
        };

        while self
            .audio_q
            .front()
            .is_some_and(|p| p.timestamp <= audio_mt)
        {
            let Some(packet) = self.audio_q.pop_front() else {
                break;
            };
            audio.push_planar_f32(
                packet.data.as_ref(),
                packet.channels,
                packet.samples,
                packet.sample_rate,
                packet.timestamp,
            );
            let levels = audio.levels();
            *latest.audio_levels.lock() = levels;
            let mut counters = latest.counters.lock();
            counters.audio_frames = levels.frames;
        }

        self.release_video(latest, video_mt);

        // If we are hopelessly behind, soft-catch or hard-snap.
        self.maybe_resnap(audio);
    }

    fn release_video(&mut self, latest: &LatestVideo, video_mt: i64) {
        let frame_ticks = self.frame_duration_ticks();
        let late_thresh = video_mt.saturating_sub(frame_ticks.saturating_mul(2));

        let Some(front) = self.video_q.front() else {
            return;
        };
        if front.timestamp() > video_mt {
            return;
        }

        let mut due: Option<QueuedVideo> = None;
        let mut replaced = 0u64;

        if front.timestamp() < late_thresh {
            // Badly behind: keep the newest due frame only.
            while self
                .video_q
                .front()
                .is_some_and(|f| f.timestamp() <= video_mt)
            {
                if let Some(old) = due.take() {
                    replaced += 1;
                    self.recycle_queued(old);
                }
                due = self.video_q.pop_front();
            }
        } else {
            // On time / slightly late: release exactly one frame per tick so
            // the display path is not force-coalesced into a stutter jump.
            due = self.video_q.pop_front();
        }

        match due {
            Some(QueuedVideo::Cpu(video)) => latest.publish_video(video, replaced),
            Some(QueuedVideo::Gpu(video)) => latest.publish_gpu_video(video, replaced),
            None => {}
        }
    }

    fn frame_duration_ticks(&self) -> i64 {
        let fps = self.fps_n.max(1) as f64 / self.fps_d.max(1) as f64;
        (TICKS_PER_SECOND / fps.max(1.0)).round() as i64
    }

    fn note_clock(&mut self, pts: i64) {
        if self.pts_origin.is_none() {
            self.pts_origin = Some(pts);
            self.wall_origin = Some(Instant::now());
            self.clock_skew_ticks = 0;
        }
    }

    fn media_time_wall(&self, delay_ms: u32) -> Option<i64> {
        let pts0 = self.pts_origin?;
        let wall0 = self.wall_origin?;
        let elapsed_ticks = (wall0.elapsed().as_secs_f64() * TICKS_PER_SECOND).round() as i64;
        let buffer_ticks = i64::from(delay_ms) * TICKS_PER_MS;
        Some(pts0 + elapsed_ticks - buffer_ticks + self.clock_skew_ticks)
    }

    /// Wall-clock gates for releasing packets into the device ring / video slot.
    ///
    /// The DAC playhead is only used as a soft skew hint via
    /// [`steer_clock_to_audio_ring`]; tying release deadlines directly to it
    /// caused silent output when the playhead stalled or raced.
    fn gate_times(
        &self,
        _audio: &AudioOutput,
        audio_delay: u32,
        video_delay: u32,
    ) -> Option<(i64, i64)> {
        Some((
            self.media_time_wall(audio_delay)?,
            self.media_time_wall(video_delay)?,
        ))
    }

    /// Nudge wall skew so the audio ring stays near the configured delay depth.
    fn steer_clock_to_audio_ring(&mut self, audio: &AudioOutput) {
        // Merely queueing PCM creates a playhead PTS. Only steer while the
        // device callback is actually consuming samples; otherwise a missing
        // or stalled output device winds the shared A/V clock backwards and
        // freezes video after the initial buffered frames.
        if !audio.playback_active() {
            self.clock_skew_ticks = 0;
            return;
        }
        let target_ms = (self.audio_delay_ms() as f64).max(40.0);
        let actual_ms = audio.buffered_ms();
        let err_ms = target_ms - actual_ms;
        // Positive err → ring low → advance media time (release sooner).
        // Cap per tick (~every few ms) so we slew instead of jumping.
        let slew_ms = (err_ms * 0.12).clamp(-0.75, 0.75);
        let delta = (slew_ms * TICKS_PER_MS as f64).round() as i64;
        self.clock_skew_ticks = (self.clock_skew_ticks + delta).clamp(-50_000_000, 50_000_000); // ±5 s
    }

    fn maybe_resnap(&mut self, audio: &AudioOutput) {
        let delay = self.video_delay_ms().max(self.audio_delay_ms());
        let Some(media_time) = self
            .gate_times(audio, self.audio_delay_ms(), self.video_delay_ms())
            .map(|(a, v)| a.max(v))
        else {
            return;
        };
        let oldest = match (self.video_q.front(), self.audio_q.front()) {
            (Some(v), Some(a)) => Some(v.timestamp().min(a.timestamp)),
            (Some(v), None) => Some(v.timestamp()),
            (None, Some(a)) => Some(a.timestamp),
            (None, None) => None,
        };
        let Some(oldest) = oldest else {
            return;
        };

        let late_ticks = media_time.saturating_sub(oldest);
        let soft_thresh =
            (i64::from(delay).saturating_mul(3).max(RESNAP_MIN_LATE_MS)) * TICKS_PER_MS;
        let hard_thresh =
            (i64::from(delay).saturating_mul(6).max(RESNAP_HARD_LATE_MS)) * TICKS_PER_MS;

        if late_ticks < soft_thresh {
            return;
        }

        if late_ticks >= hard_thresh {
            // Hard snap only when hopelessly behind. Never clear the PCM ring here:
            // VU meters from push peaks while clear() would wipe audible output and
            // then underrun→resnap can death-spiral into permanent silence.
            self.pts_origin = Some(oldest);
            self.wall_origin = Some(Instant::now());
            self.clock_skew_ticks = 0;
            audio.invalidate_playhead();
            return;
        }

        // Soft catch-up: advance skew toward the backlog without resetting.
        let catch_ms = ((late_ticks / TICKS_PER_MS) as f64 * 0.08).clamp(1.0, 8.0);
        self.clock_skew_ticks += (catch_ms * TICKS_PER_MS as f64).round() as i64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_convert_to_ms() {
        let s = DelaySetting {
            amount: 3,
            unit: BufferUnit::Frames,
        };
        assert_eq!(s.delay_ms(30, 1), 100);
        assert_eq!(s.delay_ms(60, 1), 50);
    }

    #[test]
    fn linked_set_video_updates_audio() {
        let mut s = BufferSettings::default();
        s.set_video(
            DelaySetting {
                amount: 6,
                unit: BufferUnit::Frames,
            },
            30,
            1,
        );
        assert_eq!(s.audio.amount, 200);
        assert_eq!(s.audio.unit, BufferUnit::Milliseconds);
    }

    #[test]
    fn unlinked_keeps_independent_delays() {
        let mut s = BufferSettings::default();
        s.set_linked(false, 30, 1);
        s.set_video(
            DelaySetting {
                amount: 5,
                unit: BufferUnit::Frames,
            },
            30,
            1,
        );
        s.set_audio(
            DelaySetting {
                amount: 0,
                unit: BufferUnit::Milliseconds,
            },
            30,
            1,
        );
        assert_eq!(s.video_delay_ms(30, 1), 167);
        assert_eq!(s.audio_delay_ms(30, 1), 0);
    }

    #[test]
    fn default_buffer_is_linked_200ms() {
        let s = BufferSettings::default();
        assert!(s.linked);
        assert_eq!(s.video_delay_ms(30, 1), 200);
        assert_eq!(s.audio_delay_ms(30, 1), 200);
    }
}

//! Shared OMT media helpers for the tools suite.

#![deny(missing_docs)]

mod audio_out;
mod color;
mod discovery;
mod playout;
mod receive;
mod runtime;
mod send;
mod stall;
mod stats;

pub use audio_out::{
    AudioLevels, AudioOutput, AudioOutputDevice, AudioOutputStatus, list_output_devices,
};
pub use color::{rgb_to_uyvy_pixel, uyvy_from_rgb_frame};
pub use discovery::{DiscoveredSource, SourceBrowser, discover_sources, spawn_discover};
pub use playout::{BufferSettings, BufferUnit, DelaySetting};
pub use receive::{
    ConnectOptions, LatestVideo, MetadataLogEntry, ReceiveCounters, ReceiveWorker, VideoFrame,
};
pub use send::{
    AudioToneConfig, DEFAULT_VIDEO_FRAME_BUFFER_FRAMES, MAX_VIDEO_FRAME_BUFFER_FRAMES,
    MIN_VIDEO_FRAME_BUFFER_FRAMES, SendSession, SendSessionConfig, SendStats,
    clamp_video_frame_buffer_frames,
};
pub use stall::{StallDetector, StallState};
pub use stats::FpsCounter;

pub use openmediatransport::{
    Codec, ColorSpace, DecodedAudioFrame, DecodedVideoFrame, Discovery, FrameType, MediaFrame,
    MetadataFrame, Quality, ReceiverConfig, ReceiverSession, Sender, SenderConfig, SenderInfo,
    SessionState, SessionStatistics, Statistics, bgra_alpha_mask, bgra_to_rgba, bgra_to_rgba_into,
};
pub use vmx::{
    Codec as VmxCodec, Config as VmxConfig, Profile as VmxProfile,
    SimdCapabilities as VmxSimdCapabilities, SimdPath as VmxSimdPath,
};

//! Viewer settings shared across UI modules.

use omt_media::{BufferSettings, Quality};

/// Receive quality preset (suggested encode quality + optional Preview).
///
/// `LowBandwidth` maps to [`Quality::Low`] plus 1/8 progressive Preview
/// (`ReceiverConfig.preview`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoQualityPreset {
    Default,
    Low,
    Medium,
    High,
    LowBandwidth,
}

impl VideoQualityPreset {
    pub fn to_connect_parts(self) -> (Quality, bool) {
        match self {
            Self::Default => (Quality::Default, false),
            Self::Low => (Quality::Low, false),
            Self::Medium => (Quality::Medium, false),
            Self::High => (Quality::High, false),
            Self::LowBandwidth => (Quality::Low, true),
        }
    }
}

/// Viewer settings toggled from the context menu / preferences.
#[derive(Debug, Clone)]
pub struct MonitorSettings {
    pub show_alpha: bool,
    /// SMPTE ST 2046-1 action/title safe guides over the picture.
    pub safe_area: bool,
    pub vu_meter: bool,
    pub quality: VideoQualityPreset,
    pub audio_boost_db: i32,
    /// Linked or independent A/V playout buffers (PTS gate).
    pub buffer: BufferSettings,
}

impl Default for MonitorSettings {
    fn default() -> Self {
        Self {
            show_alpha: false,
            safe_area: false,
            vu_meter: true,
            quality: VideoQualityPreset::Default,
            audio_boost_db: 0,
            buffer: BufferSettings::default(),
        }
    }
}

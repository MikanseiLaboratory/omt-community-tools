//! Persistent suite and per-app configuration.

use std::fs;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::i18n::Language;
use crate::theme::ThemePreference;

/// Errors related to config IO.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// Config directory could not be resolved.
    #[error("could not resolve config directory")]
    NoConfigDir,
    /// Underlying IO failure.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// JSON serialize/deserialize failure.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

/// Shared suite preferences (language / theme).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SuiteConfig {
    /// UI language.
    pub language: Language,
    /// Theme preference.
    pub theme: ThemePreference,
    /// Schema version.
    pub schema_version: u32,
}

impl Default for SuiteConfig {
    fn default() -> Self {
        Self {
            language: Language::default(),
            theme: ThemePreference::default(),
            schema_version: 1,
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_side_panel_w() -> u32 {
    360
}

fn default_sidebar_w() -> u32 {
    300
}

fn default_stats_w() -> u32 {
    280
}

fn default_log_h() -> u32 {
    180
}

/// Encoding quality stored in Test Patterns preferences.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TestPatternsQuality {
    /// Low quality.
    Low,
    /// Medium / standard quality.
    #[default]
    Medium,
    /// High quality.
    High,
}

/// Test Patterns tool preferences (`test-patterns.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TestPatternsConfig {
    /// Schema version.
    pub schema_version: u32,
    /// User-registered still-image paths (loaded at startup).
    pub custom_images: Vec<PathBuf>,
    /// Prefetched video frames for paced OMT send (1..=16, default 3).
    pub frame_buffer_frames: u32,
    /// Discoverable OMT source name.
    pub name: String,
    /// Output width in pixels.
    pub width: i32,
    /// Output height in pixels.
    pub height: i32,
    /// Frame rate numerator.
    pub fps_n: i32,
    /// Frame rate denominator.
    pub fps_d: i32,
    /// Encoding quality.
    pub quality: TestPatternsQuality,
    /// Whether pattern / image scroll animation is enabled.
    pub animate: bool,
    /// Horizontal scroll speed percent (−200..=200).
    pub anim_speed_h_pct: i32,
    /// Vertical scroll speed percent (−200..=200).
    pub anim_speed_v_pct: i32,
    /// Tone frequency in Hz; `0` means mute.
    pub tone_hz: f32,
    /// Tone peak level in dBFS.
    pub level_dbfs: f32,
    /// Right side-panel width in logical pixels.
    #[serde(default = "default_side_panel_w")]
    pub side_panel_w: u32,
    /// Whether the statistics group is expanded.
    #[serde(default = "default_true")]
    pub stats_open: bool,
    /// Whether the settings group is expanded.
    #[serde(default = "default_true")]
    pub settings_open: bool,
}

impl Default for TestPatternsConfig {
    fn default() -> Self {
        Self {
            schema_version: 1,
            custom_images: Vec::new(),
            frame_buffer_frames: 3,
            name: "Test Pattern".into(),
            width: 1920,
            height: 1080,
            fps_n: 30_000,
            fps_d: 1_001,
            quality: TestPatternsQuality::Medium,
            animate: true,
            anim_speed_h_pct: 100,
            anim_speed_v_pct: 100,
            tone_hz: 1000.0,
            level_dbfs: -20.0,
            side_panel_w: default_side_panel_w(),
            stats_open: true,
            settings_open: true,
        }
    }
}

impl TestPatternsConfig {
    /// Clamp user-facing values into ranges the Test Patterns UI accepts.
    pub fn sanitized(mut self) -> Self {
        let trimmed = self.name.trim();
        self.name = if trimmed.is_empty() {
            "Test Pattern".into()
        } else {
            let mut name = trimmed.to_string();
            while name.len() > 64 {
                name.pop();
            }
            name
        };
        let width = self.width.clamp(64, 7680);
        self.width = width - (width % 2);
        self.height = self.height.clamp(64, 4320);
        self.fps_n = self.fps_n.max(1);
        self.fps_d = self.fps_d.max(1);
        self.frame_buffer_frames = self.frame_buffer_frames.clamp(1, 16);
        self.anim_speed_h_pct = self.anim_speed_h_pct.clamp(-200, 200);
        self.anim_speed_v_pct = self.anim_speed_v_pct.clamp(-200, 200);
        if self.tone_hz < 0.0 {
            self.tone_hz = 0.0;
        }
        self.level_dbfs = self.level_dbfs.clamp(-120.0, 0.0);
        if self.side_panel_w == 0 {
            self.side_panel_w = default_side_panel_w();
        } else {
            self.side_panel_w = self.side_panel_w.clamp(240, 520);
        }
        self
    }
}

/// Studio Monitor tool preferences (`studio-monitor.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StudioMonitorConfig {
    /// Schema version.
    pub schema_version: u32,
    /// Left sources sidebar width in logical pixels.
    #[serde(default = "default_sidebar_w")]
    pub sidebar_w: u32,
    /// Right statistics panel width in logical pixels.
    #[serde(default = "default_stats_w")]
    pub stats_w: u32,
    /// Bottom log panel height in logical pixels.
    #[serde(default = "default_log_h")]
    pub log_h: u32,
    /// Whether the video statistics group is expanded.
    #[serde(default = "default_true")]
    pub stats_video_open: bool,
    /// Whether the audio statistics group is expanded.
    #[serde(default = "default_true")]
    pub stats_audio_open: bool,
    /// Whether the source-info group is expanded.
    #[serde(default = "default_true")]
    pub stats_source_open: bool,
}

impl Default for StudioMonitorConfig {
    fn default() -> Self {
        Self {
            schema_version: 1,
            sidebar_w: default_sidebar_w(),
            stats_w: default_stats_w(),
            log_h: default_log_h(),
            stats_video_open: true,
            stats_audio_open: true,
            stats_source_open: true,
        }
    }
}

/// Launcher preferences (`launcher.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LauncherConfig {
    /// Schema version.
    pub schema_version: u32,
}

impl Default for LauncherConfig {
    fn default() -> Self {
        Self { schema_version: 1 }
    }
}

/// Discovery Server GUI preferences (`discovery-server.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DiscoveryServerConfig {
    /// Schema version.
    pub schema_version: u32,
    /// Listen address (`::` = dual-stack any, matching the official app).
    pub bind: String,
    /// Listen port (default 6399).
    pub port: u16,
}

impl Default for DiscoveryServerConfig {
    fn default() -> Self {
        Self {
            schema_version: 1,
            bind: "::".into(),
            port: 6399,
        }
    }
}

impl DiscoveryServerConfig {
    /// Clamp bind/port into values the GUI and CLI accept.
    pub fn sanitized(mut self) -> Self {
        let trimmed = self.bind.trim();
        self.bind = if trimmed.is_empty() {
            "::".into()
        } else {
            trimmed.to_string()
        };
        if self.port == 0 {
            self.port = 6399;
        }
        self
    }
}

/// Resolve the suite config directory.
pub fn config_dir() -> Result<PathBuf, ConfigError> {
    let dirs = ProjectDirs::from("lab", "Mikansei", "OMT Tools").ok_or(ConfigError::NoConfigDir)?;
    Ok(dirs.config_dir().to_path_buf())
}

/// Path to the shared suite config (`suite.json`).
pub fn config_path() -> Result<PathBuf, ConfigError> {
    Ok(config_dir()?.join("suite.json"))
}

/// Path to a per-app config file (`{app_id}.json`).
pub fn app_config_path(app_id: &str) -> Result<PathBuf, ConfigError> {
    Ok(config_dir()?.join(format!("{app_id}.json")))
}

/// Load shared suite config.
pub fn load_config() -> Result<SuiteConfig, ConfigError> {
    load_json_file(&config_path()?)
}

/// Persist shared suite config.
pub fn save_config(cfg: &SuiteConfig) -> Result<(), ConfigError> {
    save_json_file(&config_path()?, cfg)
}

/// Load Test Patterns preferences.
pub fn load_test_patterns_config() -> Result<TestPatternsConfig, ConfigError> {
    load_json_file(&app_config_path("test-patterns")?)
}

/// Persist Test Patterns preferences.
pub fn save_test_patterns_config(cfg: &TestPatternsConfig) -> Result<(), ConfigError> {
    save_json_file(&app_config_path("test-patterns")?, cfg)
}

/// Load Studio Monitor preferences.
pub fn load_studio_monitor_config() -> Result<StudioMonitorConfig, ConfigError> {
    load_json_file(&app_config_path("studio-monitor")?)
}

/// Persist Studio Monitor preferences.
pub fn save_studio_monitor_config(cfg: &StudioMonitorConfig) -> Result<(), ConfigError> {
    save_json_file(&app_config_path("studio-monitor")?, cfg)
}

/// Load Launcher preferences.
pub fn load_launcher_config() -> Result<LauncherConfig, ConfigError> {
    load_json_file(&app_config_path("launcher")?)
}

/// Persist Launcher preferences.
pub fn save_launcher_config(cfg: &LauncherConfig) -> Result<(), ConfigError> {
    save_json_file(&app_config_path("launcher")?, cfg)
}

/// Load Discovery Server GUI preferences.
pub fn load_discovery_server_config() -> Result<DiscoveryServerConfig, ConfigError> {
    load_json_file(&app_config_path("discovery-server")?)
}

/// Persist Discovery Server GUI preferences.
pub fn save_discovery_server_config(cfg: &DiscoveryServerConfig) -> Result<(), ConfigError> {
    save_json_file(&app_config_path("discovery-server")?, cfg)
}

fn load_json_file<T: DeserializeOwned + Default>(path: &Path) -> Result<T, ConfigError> {
    if !path.exists() {
        return Ok(T::default());
    }
    let raw = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&raw)?)
}

fn save_json_file<T: Serialize>(path: &Path, value: &T) -> Result<(), ConfigError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(value)?;
    fs::write(path, json)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_suite_defaults() {
        let cfg = SuiteConfig::default();
        let json = serde_json::to_string(&cfg).unwrap();
        let parsed: SuiteConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, parsed);
        assert_eq!(parsed.schema_version, 1);
    }

    #[test]
    fn roundtrip_custom_images() {
        let cfg = TestPatternsConfig {
            custom_images: vec![PathBuf::from("C:/images/bars.png")],
            frame_buffer_frames: 5,
            name: "Cam A".into(),
            width: 1280,
            height: 720,
            fps_n: 60,
            fps_d: 1,
            quality: TestPatternsQuality::High,
            animate: false,
            anim_speed_h_pct: 50,
            anim_speed_v_pct: -20,
            tone_hz: 440.0,
            level_dbfs: -6.0,
            side_panel_w: 400,
            stats_open: false,
            settings_open: false,
            ..Default::default()
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let parsed: TestPatternsConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.custom_images.len(), 1);
        assert_eq!(parsed.custom_images[0], PathBuf::from("C:/images/bars.png"));
        assert_eq!(parsed.frame_buffer_frames, 5);
        assert_eq!(parsed.name, "Cam A");
        assert_eq!(parsed.width, 1280);
        assert_eq!(parsed.height, 720);
        assert_eq!(parsed.fps_n, 60);
        assert_eq!(parsed.fps_d, 1);
        assert_eq!(parsed.quality, TestPatternsQuality::High);
        assert!(!parsed.animate);
        assert_eq!(parsed.anim_speed_h_pct, 50);
        assert_eq!(parsed.anim_speed_v_pct, -20);
        assert!((parsed.tone_hz - 440.0).abs() < f32::EPSILON);
        assert!((parsed.level_dbfs + 6.0).abs() < f32::EPSILON);
        assert_eq!(parsed.side_panel_w, 400);
        assert!(!parsed.stats_open);
        assert!(!parsed.settings_open);
    }

    #[test]
    fn legacy_test_patterns_json_defaults_new_fields() {
        let parsed: TestPatternsConfig =
            serde_json::from_str(r#"{"schema_version":1,"custom_images":[]}"#).unwrap();
        assert_eq!(parsed.frame_buffer_frames, 3);
        assert_eq!(parsed.name, "Test Pattern");
        assert_eq!(parsed.width, 1920);
        assert_eq!(parsed.height, 1080);
        assert_eq!(parsed.fps_n, 30_000);
        assert_eq!(parsed.fps_d, 1_001);
        assert_eq!(parsed.quality, TestPatternsQuality::Medium);
        assert!(parsed.animate);
        assert_eq!(parsed.anim_speed_h_pct, 100);
        assert_eq!(parsed.anim_speed_v_pct, 100);
        assert!((parsed.tone_hz - 1000.0).abs() < f32::EPSILON);
        assert!((parsed.level_dbfs + 20.0).abs() < f32::EPSILON);
        assert_eq!(parsed.side_panel_w, 360);
        assert!(parsed.stats_open);
        assert!(parsed.settings_open);
    }

    #[test]
    fn sanitizes_out_of_range_test_patterns_values() {
        let parsed = TestPatternsConfig {
            name: "  ".into(),
            width: 1921,
            height: 10,
            fps_n: 0,
            fps_d: 0,
            anim_speed_h_pct: 999,
            anim_speed_v_pct: -999,
            tone_hz: -1.0,
            level_dbfs: 12.0,
            frame_buffer_frames: 99,
            ..Default::default()
        }
        .sanitized();
        assert_eq!(parsed.name, "Test Pattern");
        assert_eq!(parsed.width, 1920);
        assert_eq!(parsed.height, 64);
        assert_eq!(parsed.fps_n, 1);
        assert_eq!(parsed.fps_d, 1);
        assert_eq!(parsed.anim_speed_h_pct, 200);
        assert_eq!(parsed.anim_speed_v_pct, -200);
        assert!((parsed.tone_hz - 0.0).abs() < f32::EPSILON);
        assert!((parsed.level_dbfs - 0.0).abs() < f32::EPSILON);
        assert_eq!(parsed.frame_buffer_frames, 16);
    }

    #[test]
    fn legacy_studio_monitor_json_defaults_layout() {
        let parsed: StudioMonitorConfig = serde_json::from_str(r#"{"schema_version":1}"#).unwrap();
        assert_eq!(parsed.sidebar_w, 300);
        assert_eq!(parsed.stats_w, 280);
        assert_eq!(parsed.log_h, 180);
        assert!(parsed.stats_video_open);
        assert!(parsed.stats_audio_open);
        assert!(parsed.stats_source_open);
    }

    #[test]
    fn app_config_paths() {
        let dir = config_dir().unwrap();
        assert_eq!(config_path().unwrap(), dir.join("suite.json"));
        assert_eq!(
            app_config_path("test-patterns").unwrap(),
            dir.join("test-patterns.json")
        );
        assert_eq!(
            app_config_path("studio-monitor").unwrap(),
            dir.join("studio-monitor.json")
        );
        assert_eq!(
            app_config_path("launcher").unwrap(),
            dir.join("launcher.json")
        );
        assert_eq!(
            app_config_path("discovery-server").unwrap(),
            dir.join("discovery-server.json")
        );
    }

    #[test]
    fn sanitizes_discovery_server_bind_and_port() {
        let parsed = DiscoveryServerConfig {
            bind: "  ".into(),
            port: 0,
            ..Default::default()
        }
        .sanitized();
        assert_eq!(parsed.bind, "::");
        assert_eq!(parsed.port, 6399);
    }
}

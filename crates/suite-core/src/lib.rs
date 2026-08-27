//! Shared settings, localization, SIMD diagnostics, and suite version metadata for OMT Tools.

#![deny(missing_docs)]

mod config;
mod fonts;
mod i18n;
mod logging;
mod reveal;
mod simd;
mod theme;
mod version;

pub use config::{
    DiscoveryServerConfig, LauncherConfig, StudioMonitorConfig, SuiteConfig, TestPatternsConfig,
    TestPatternsQuality, app_config_path, config_dir, config_path, load_config,
    load_discovery_server_config, load_launcher_config, load_studio_monitor_config,
    load_test_patterns_config, save_config, save_discovery_server_config, save_launcher_config,
    save_studio_monitor_config, save_test_patterns_config,
};
#[cfg(feature = "egui-fonts")]
pub use fonts::install_egui_cjk_fonts;
pub use fonts::load_cjk_font_bytes;
pub use i18n::{Language, t};
pub use logging::init_tracing;
pub use reveal::{RevealError, reveal_in_file_manager};
pub use simd::SimdCapabilities;
pub use theme::ThemePreference;
pub use version::{SUITE_VERSION, SuiteManifest, ToolId, ToolInfo, suite_manifest};

/// Environment variables used when the launcher starts a tool.
pub mod env {
    /// Language override (`ja` / `en`).
    pub const LANGUAGE: &str = "OMT_TOOLS_LANGUAGE";
    /// Theme override (`light` / `dark` / `system`).
    pub const THEME: &str = "OMT_TOOLS_THEME";
    /// Suite version string passed to child tools.
    pub const SUITE_VERSION: &str = "OMT_TOOLS_SUITE_VERSION";
}

/// CLI / process launch overrides resolved from args then environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchOverrides {
    /// Selected UI language.
    pub language: Language,
    /// Selected theme preference.
    pub theme: ThemePreference,
    /// Suite version string.
    pub suite_version: String,
}

impl LaunchOverrides {
    /// Resolve overrides from process environment and optional CLI values.
    pub fn resolve(
        language: Option<Language>,
        theme: Option<ThemePreference>,
        suite_version: Option<String>,
    ) -> Self {
        let cfg = load_config().unwrap_or_default();
        let language = language
            .or_else(|| {
                std::env::var(env::LANGUAGE)
                    .ok()
                    .and_then(|s| s.parse().ok())
            })
            .unwrap_or(cfg.language);
        let theme = theme
            .or_else(|| std::env::var(env::THEME).ok().and_then(|s| s.parse().ok()))
            .unwrap_or(cfg.theme);
        let suite_version = suite_version
            .or_else(|| std::env::var(env::SUITE_VERSION).ok())
            .unwrap_or_else(|| SUITE_VERSION.to_string());
        Self {
            language,
            theme,
            suite_version,
        }
    }

    /// Apply overrides as environment variables for a child process.
    pub fn apply_to_command(&self, cmd: &mut std::process::Command) {
        cmd.env(env::LANGUAGE, self.language.as_str());
        cmd.env(env::THEME, self.theme.as_str());
        cmd.env(env::SUITE_VERSION, &self.suite_version);
    }
}

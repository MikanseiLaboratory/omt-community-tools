//! OMT Studio Monitor (egui/eframe).

// Hide the console window for release GUI launches on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod chrome;
mod frame_prep;
mod output;
mod overlay;
mod preferences;
mod settings;

use clap::Parser;
use suite_core::{LaunchOverrides, init_tracing, t};

#[derive(Debug, Parser)]
#[command(name = "omt-studio-monitor", about = "OMT Studio Monitor")]
struct Args {
    /// UI language (`en` / `ja`).
    #[arg(long)]
    language: Option<String>,
    /// Theme (`light` / `dark` / `system`).
    #[arg(long)]
    theme: Option<String>,
    /// Optional initial `omt://` URL.
    #[arg(long)]
    url: Option<String>,
}

fn main() -> anyhow::Result<()> {
    init_tracing();

    let args = Args::parse();
    let overrides = LaunchOverrides::resolve(
        args.language.as_deref().and_then(|s| s.parse().ok()),
        args.theme.as_deref().and_then(|s| s.parse().ok()),
        None,
    );

    let title = t(overrides.language, "tool.studio_monitor").to_string();
    app::run_eframe(title, overrides.language, overrides.theme, args.url)
}

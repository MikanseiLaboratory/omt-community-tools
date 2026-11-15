//! OMT Test Patterns sender (GPUI).

// Hide the console window for release GUI launches on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ui;

use clap::Parser;
use suite_core::{LaunchOverrides, init_tracing, t};

#[derive(Debug, Parser)]
#[command(name = "omt-test-patterns", about = "OMT Test Patterns")]
struct Args {
    /// UI language (`en` / `ja`).
    #[arg(long)]
    language: Option<String>,
    /// Theme (`light` / `dark` / `system`).
    #[arg(long)]
    theme: Option<String>,
    /// Instance-local OMT source name.
    ///
    /// Useful when running more than one Test Patterns on the same machine:
    /// the name is used for this process only and is not written back to the
    /// shared `test-patterns.json`.
    #[arg(long = "name", value_name = "SOURCE_NAME")]
    name: Option<String>,
}

fn main() -> anyhow::Result<()> {
    init_tracing();

    let args = Args::parse();
    let overrides = LaunchOverrides::resolve(
        args.language.as_deref().and_then(|s| s.parse().ok()),
        args.theme.as_deref().and_then(|s| s.parse().ok()),
        None,
    );

    let title = t(overrides.language, "tool.test_patterns").to_string();
    ui::run_gpui(title, overrides.language, args.name)
}

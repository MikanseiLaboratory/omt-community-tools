//! OMT Tools Tauri launcher backend.

mod os_shortcuts;
mod tools;

use serde::{Deserialize, Serialize};
use suite_core::{
    Language, SimdCapabilities, SuiteManifest, ThemePreference, load_config, save_config,
    suite_manifest, t,
};
use tauri::Manager;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ToolCard {
    id: String,
    title: String,
    description: String,
    binary: String,
    enabled: bool,
    available: bool,
    version: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LauncherState {
    language: String,
    theme: String,
    suite_version: String,
    /// Compact SIMD capability summary for the settings panel.
    simd: String,
    labels: Labels,
    tools: Vec<ToolCard>,
    manifest: SuiteManifest,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Labels {
    title: String,
    subtitle: String,
    settings: String,
    docs: String,
    save: String,
    language: String,
    theme: String,
    version: String,
    launch: String,
    launching: String,
    back: String,
    theme_light: String,
    theme_dark: String,
    theme_system: String,
    unavailable: String,
    simd: String,
    update_check: String,
    update_checking: String,
    update_available_title: String,
    update_available_body: String,
    update_install: String,
    update_later: String,
    update_none: String,
    update_installing: String,
    update_failed: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveSettingsArgs {
    language: String,
    theme: String,
}

fn labels(lang: Language) -> Labels {
    Labels {
        title: t(lang, "app.title").to_string(),
        subtitle: t(lang, "launcher.subtitle").to_string(),
        settings: t(lang, "settings").to_string(),
        docs: t(lang, "docs").to_string(),
        save: t(lang, "save").to_string(),
        language: t(lang, "language").to_string(),
        theme: t(lang, "theme").to_string(),
        version: t(lang, "version").to_string(),
        launch: t(lang, "launch").to_string(),
        launching: t(lang, "launching").to_string(),
        back: t(lang, "back").to_string(),
        theme_light: t(lang, "theme.light").to_string(),
        theme_dark: t(lang, "theme.dark").to_string(),
        theme_system: t(lang, "theme.system").to_string(),
        unavailable: t(lang, "tool.unavailable").to_string(),
        simd: t(lang, "simd").to_string(),
        update_check: t(lang, "update.check").to_string(),
        update_checking: t(lang, "update.checking").to_string(),
        update_available_title: t(lang, "update.available.title").to_string(),
        update_available_body: t(lang, "update.available.body").to_string(),
        update_install: t(lang, "update.install").to_string(),
        update_later: t(lang, "update.later").to_string(),
        update_none: t(lang, "update.none").to_string(),
        update_installing: t(lang, "update.installing").to_string(),
        update_failed: t(lang, "update.failed").to_string(),
    }
}

fn build_state(app: &tauri::AppHandle) -> Result<LauncherState, String> {
    let cfg = load_config().unwrap_or_default();
    let manifest = suite_manifest();
    let mut tools = Vec::new();
    for info in &manifest.tools {
        let available = tools::tool_available(app, info.id);
        tools.push(ToolCard {
            id: info.id.os_entry_id().to_string(),
            title: t(cfg.language, info.id.title_key()).to_string(),
            description: t(cfg.language, info.id.description_key()).to_string(),
            binary: info.binary.clone(),
            enabled: info.enabled,
            available: available && info.enabled,
            version: info.version.clone(),
        });
    }
    Ok(LauncherState {
        language: cfg.language.as_str().to_string(),
        theme: cfg.theme.as_str().to_string(),
        suite_version: manifest.suite_version.clone(),
        simd: SimdCapabilities::detect().summary(),
        labels: labels(cfg.language),
        tools,
        manifest,
    })
}

#[tauri::command]
fn get_launcher_state(app: tauri::AppHandle) -> Result<LauncherState, String> {
    build_state(&app)
}

#[tauri::command]
fn save_settings(app: tauri::AppHandle, args: SaveSettingsArgs) -> Result<LauncherState, String> {
    let language = args
        .language
        .parse::<Language>()
        .map_err(|_| "invalid language".to_string())?;
    let theme = args
        .theme
        .parse::<ThemePreference>()
        .map_err(|_| "invalid theme".to_string())?;
    let mut cfg = load_config().unwrap_or_default();
    cfg.language = language;
    cfg.theme = theme;
    save_config(&cfg).map_err(|e| e.to_string())?;
    build_state(&app)
}

#[tauri::command]
fn launch_tool(app: tauri::AppHandle, tool_id: String) -> Result<(), String> {
    let id = tools::parse_tool_id(&tool_id)?;
    tools::launch_tool(&app, id)
}

#[tauri::command]
fn list_running_tools() -> Result<Vec<String>, String> {
    Ok(tools::running_tool_names())
}

/// macOS menu bar takes its app title from the first submenu. An empty menu
/// (`Menu::new`) makes that title blank. Windows / Linux stay menuless.
fn native_menu<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> tauri::Result<tauri::menu::Menu<R>> {
    #[cfg(target_os = "macos")]
    {
        use tauri::menu::{AboutMetadata, Menu, PredefinedMenuItem, Submenu};
        let name = app
            .config()
            .product_name
            .clone()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "OMT Tools".to_string());
        let about = AboutMetadata {
            name: Some(name.clone()),
            version: Some(app.package_info().version.to_string()),
            copyright: app.config().bundle.copyright.clone(),
            authors: app.config().bundle.publisher.clone().map(|p| vec![p]),
            ..Default::default()
        };
        let app_menu = Submenu::with_items(
            app,
            &name,
            true,
            &[
                &PredefinedMenuItem::about(app, None, Some(about))?,
                &PredefinedMenuItem::separator(app)?,
                &PredefinedMenuItem::services(app, None)?,
                &PredefinedMenuItem::separator(app)?,
                &PredefinedMenuItem::hide(app, None)?,
                &PredefinedMenuItem::hide_others(app, None)?,
                &PredefinedMenuItem::show_all(app, None)?,
                &PredefinedMenuItem::separator(app)?,
                &PredefinedMenuItem::quit(app, None)?,
            ],
        )?;
        Menu::with_items(app, &[&app_menu])
    }
    #[cfg(not(target_os = "macos"))]
    {
        tauri::menu::Menu::new(app)
    }
}

/// WebKitGTK DMA-BUF compositing can scramble the window on Raspberry Pi OS
/// (and some other Mesa stacks). Safe to set on other Linux GPUs when unset.
fn apply_linux_webview_env() {
    #[cfg(target_os = "linux")]
    {
        const KEYS: &[(&str, &str)] = &[
            ("WEBKIT_DISABLE_DMABUF_RENDERER", "1"),
            ("WEBKIT_DISABLE_COMPOSITING_MODE", "1"),
        ];
        for (key, value) in KEYS {
            if std::env::var_os(key).is_none() {
                // SAFETY: called from `run()` before the webview or worker threads start.
                unsafe { std::env::set_var(key, value) };
            }
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    apply_linux_webview_env();
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .menu(native_menu)
        .setup(|app| {
            if let Some(tool_id) = os_shortcuts::parse_launch_tool_id() {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
                let id = tools::parse_tool_id(&tool_id)?;
                tools::launch_tool(app.handle(), id)?;
                std::process::exit(0);
            }
            os_shortcuts::ensure_registered(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_launcher_state,
            save_settings,
            launch_tool,
            list_running_tools
        ])
        .run(tauri::generate_context!())
        .expect("error while running OMT Tools launcher");
}

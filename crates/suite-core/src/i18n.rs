//! Localization helpers (Japanese / English).

use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// Supported UI languages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum Language {
    /// English.
    #[default]
    #[serde(rename = "en", alias = "english")]
    English,
    /// Japanese.
    #[serde(rename = "ja", alias = "japanese", alias = "jp")]
    Japanese,
}

impl Language {
    /// Wire / env / CLI token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::Japanese => "ja",
        }
    }

    /// Human-readable label in the language itself.
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::Japanese => "日本語",
        }
    }
}

impl FromStr for Language {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "en" | "english" => Ok(Self::English),
            "ja" | "jp" | "japanese" => Ok(Self::Japanese),
            _ => Err(()),
        }
    }
}

impl std::fmt::Display for Language {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Translate a message key for the given language.
pub fn t(lang: Language, key: &str) -> &'static str {
    match (lang, key) {
        // Launcher
        (Language::English, "app.title") => "OMT Tools",
        (Language::Japanese, "app.title") => "OMT Tools",
        (Language::English, "launcher.subtitle") => "Open Media Transport production utilities",
        (Language::Japanese, "launcher.subtitle") => "Open Media Transport 配信用ユーティリティ",
        (Language::English, "docs") => "Docs & Guides",
        (Language::Japanese, "docs") => "ドキュメント",
        (Language::English, "save") => "Save",
        (Language::Japanese, "save") => "保存",
        (Language::English, "settings") => "Settings",
        (Language::Japanese, "settings") => "設定",
        (Language::English, "language") => "Language",
        (Language::Japanese, "language") => "言語",
        (Language::English, "theme") => "Theme",
        (Language::Japanese, "theme") => "テーマ",
        (Language::English, "version") => "Version",
        (Language::Japanese, "version") => "バージョン",
        (Language::English, "simd") => "SIMD",
        (Language::Japanese, "simd") => "SIMD",
        (Language::English, "license") => "License",
        (Language::Japanese, "license") => "ライセンス",
        (Language::English, "launch") => "Launch",
        (Language::Japanese, "launch") => "起動",
        (Language::English, "launching") => "Launching…",
        (Language::Japanese, "launching") => "起動中…",
        (Language::English, "back") => "Back",
        (Language::Japanese, "back") => "戻る",
        (Language::English, "theme.light") => "Light",
        (Language::Japanese, "theme.light") => "ライト",
        (Language::English, "theme.dark") => "Dark",
        (Language::Japanese, "theme.dark") => "ダーク",
        (Language::English, "theme.system") => "System",
        (Language::Japanese, "theme.system") => "システム",

        // Tools
        (Language::English, "tool.studio_monitor") => "Studio Monitor",
        (Language::Japanese, "tool.studio_monitor") => "Studio Monitor",
        (Language::English, "tool.studio_monitor.desc") => "Browse and view OMT sources on the LAN",
        (Language::Japanese, "tool.studio_monitor.desc") => {
            "LAN上のOMTソースを一覧表示・プレビュー"
        }
        (Language::English, "tool.test_patterns") => "Test Patterns",
        (Language::Japanese, "tool.test_patterns") => "Test Patterns",
        (Language::English, "tool.test_patterns.desc") => {
            "Pick a pattern and send video + tone over OMT"
        }
        (Language::Japanese, "tool.test_patterns.desc") => {
            "パターンを選んで映像とトーンをOMTで送出"
        }
        (Language::English, "tool.config_manager") => "Config Manager",
        (Language::Japanese, "tool.config_manager") => "Config Manager",
        (Language::English, "tool.config_manager.desc") => {
            "View and edit the global OMT settings.xml"
        }
        (Language::Japanese, "tool.config_manager.desc") => "OMT全体の settings.xml を表示・編集",
        (Language::English, "tool.discovery_server") => "Discovery Server",
        (Language::Japanese, "tool.discovery_server") => "Discovery Server",
        (Language::English, "tool.discovery_server.desc") => {
            "Run an OMT discovery server for networks that block multicast"
        }
        (Language::Japanese, "tool.discovery_server.desc") => {
            "マルチキャストが使えないネットワーク向けのOMT Discovery Server"
        }
        (Language::English, "tool.unavailable") => "Not installed",
        (Language::Japanese, "tool.unavailable") => "未インストール",

        // Updater
        (Language::English, "update.check") => "Check for updates",
        (Language::Japanese, "update.check") => "更新を確認",
        (Language::English, "update.checking") => "Checking for updates…",
        (Language::Japanese, "update.checking") => "更新を確認しています…",
        (Language::English, "update.available.title") => "Update available",
        (Language::Japanese, "update.available.title") => "更新があります",
        (Language::English, "update.available.body") => {
            "Version {version} is available. The app will restart after installation."
        }
        (Language::Japanese, "update.available.body") => {
            "バージョン {version} が利用できます。インストール後にアプリが再起動します。"
        }
        (Language::English, "update.install") => "Update",
        (Language::Japanese, "update.install") => "更新する",
        (Language::English, "update.later") => "Later",
        (Language::Japanese, "update.later") => "後で",
        (Language::English, "update.none") => "You're up to date.",
        (Language::Japanese, "update.none") => "最新です。",
        (Language::English, "update.installing") => "Downloading and installing update…",
        (Language::Japanese, "update.installing") => {
            "更新をダウンロードしてインストールしています…"
        }
        (Language::English, "update.failed") => "Could not check for updates.",
        (Language::Japanese, "update.failed") => "更新の確認に失敗しました。",

        // Monitor
        (Language::English, "monitor.sources") => "Sources",
        (Language::Japanese, "monitor.sources") => "ソース",
        (Language::English, "monitor.refresh") => "Refresh",
        (Language::Japanese, "monitor.refresh") => "更新",
        (Language::English, "monitor.no_sources") => "No OMT sources discovered",
        (Language::Japanese, "monitor.no_sources") => "OMTソースが見つかりません",
        (Language::English, "monitor.waiting") => "Waiting for video…",
        (Language::Japanese, "monitor.waiting") => "映像待機中…",
        (Language::English, "monitor.stalled") => "SIGNAL LOST",
        (Language::Japanese, "monitor.stalled") => "信号途絶",
        (Language::English, "monitor.alpha_mask") => "Alpha mask",
        (Language::Japanese, "monitor.alpha_mask") => "アルファマスク",
        (Language::English, "monitor.fit") => "Fit",
        (Language::Japanese, "monitor.fit") => "フィット",
        (Language::English, "monitor.fill") => "Fill",
        (Language::Japanese, "monitor.fill") => "フィル",
        (Language::English, "monitor.stats") => "Statistics",
        (Language::Japanese, "monitor.stats") => "統計",
        (Language::English, "monitor.source_info") => "Source",
        (Language::Japanese, "monitor.source_info") => "ソース情報",
        (Language::English, "monitor.log") => "Log",
        (Language::Japanese, "monitor.log") => "ログ",
        (Language::English, "monitor.log_realtime") => "Realtime",
        (Language::Japanese, "monitor.log_realtime") => "リアルタイム",
        (Language::English, "monitor.log_append") => "Append",
        (Language::Japanese, "monitor.log_append") => "追記",
        (Language::English, "monitor.zoom_reset") => "100%",
        (Language::Japanese, "monitor.zoom_reset") => "100%",
        (Language::English, "monitor.clear_log") => "Clear",
        (Language::Japanese, "monitor.clear_log") => "クリア",
        (Language::English, "monitor.none") => "None",
        (Language::Japanese, "monitor.none") => "なし",
        (Language::English, "monitor.fullscreen") => "Fullscreen",
        (Language::Japanese, "monitor.fullscreen") => "全画面表示",
        (Language::English, "monitor.settings") => "Settings",
        (Language::Japanese, "monitor.settings") => "設定",
        (Language::English, "monitor.audio") => "Audio",
        (Language::Japanese, "monitor.audio") => "音声",
        (Language::English, "monitor.video") => "Video",
        (Language::Japanese, "monitor.video") => "映像",
        (Language::English, "monitor.overlay") => "Overlay",
        (Language::Japanese, "monitor.overlay") => "オーバーレイ",
        (Language::English, "monitor.help") => "Help",
        (Language::Japanese, "monitor.help") => "ヘルプ",
        (Language::English, "monitor.exit") => "Exit",
        (Language::Japanese, "monitor.exit") => "終了",
        (Language::English, "monitor.audio_boost") => "Boost",
        (Language::Japanese, "monitor.audio_boost") => "ブースト",
        (Language::English, "monitor.audio_output") => "Audio output",
        (Language::Japanese, "monitor.audio_output") => "音声出力先",
        (Language::English, "monitor.audio_default") => "System default",
        (Language::Japanese, "monitor.audio_default") => "システム既定",
        (Language::English, "monitor.audio_system_default") => "default",
        (Language::Japanese, "monitor.audio_system_default") => "既定",
        (Language::English, "monitor.audio_none") => "No output devices found",
        (Language::Japanese, "monitor.audio_none") => "出力デバイスが見つかりません",
        (Language::English, "monitor.audio_unavailable") => "No audio output",
        (Language::Japanese, "monitor.audio_unavailable") => "音声出力なし",
        (Language::English, "monitor.audio_unavailable_hint") => {
            "Video continues. Choose another output device in Settings."
        }
        (Language::Japanese, "monitor.audio_unavailable_hint") => {
            "映像は再生を続けます。設定から別の出力デバイスを選んでください。"
        }
        (Language::English, "monitor.av_buffer") => "A/V buffer",
        (Language::Japanese, "monitor.av_buffer") => "A/Vバッファ",
        (Language::English, "monitor.buffer_video") => "Video delay",
        (Language::Japanese, "monitor.buffer_video") => "映像遅延",
        (Language::English, "monitor.buffer_audio") => "Audio delay",
        (Language::Japanese, "monitor.buffer_audio") => "音声遅延",
        (Language::English, "monitor.buffer_link") => "Link video & audio",
        (Language::Japanese, "monitor.buffer_link") => "映像と音声をリンク",
        (Language::English, "monitor.buffer_unlink_info") => {
            "When unchecked, buffers run independently and A/V will not stay in sync."
        }
        (Language::Japanese, "monitor.buffer_unlink_info") => {
            "チェックを外すとバッファが独立動作になり、映像と音声が同期されなくなります。"
        }
        (Language::English, "monitor.buffer_ms") => "Milliseconds",
        (Language::Japanese, "monitor.buffer_ms") => "ミリ秒",
        (Language::English, "monitor.buffer_frame") => "frame",
        (Language::Japanese, "monitor.buffer_frame") => "フレーム",
        (Language::English, "monitor.buffer_frames") => "frames",
        (Language::Japanese, "monitor.buffer_frames") => "フレーム",
        (Language::English, "monitor.buffer_equiv") => "≈",
        (Language::Japanese, "monitor.buffer_equiv") => "≈",
        (Language::English, "monitor.quality") => "Quality",
        (Language::Japanese, "monitor.quality") => "品質",
        (Language::English, "monitor.quality_default") => "Default",
        (Language::Japanese, "monitor.quality_default") => "既定",
        (Language::English, "monitor.quality_low") => "Low",
        (Language::Japanese, "monitor.quality_low") => "低",
        (Language::English, "monitor.quality_medium") => "Medium",
        (Language::Japanese, "monitor.quality_medium") => "中",
        (Language::English, "monitor.quality_high") => "High",
        (Language::Japanese, "monitor.quality_high") => "高",
        (Language::English, "monitor.quality_low_bw") => "Low Bandwidth",
        (Language::Japanese, "monitor.quality_low_bw") => "低帯域",
        (Language::English, "monitor.safe_area") => "Safe areas",
        (Language::Japanese, "monitor.safe_area") => "セーフエリア",
        (Language::English, "monitor.vu_meter") => "VU meter",
        (Language::Japanese, "monitor.vu_meter") => "VUメーター",
        (Language::English, "monitor.preferences") => "Preferences",
        (Language::Japanese, "monitor.preferences") => "環境設定",
        (Language::English, "monitor.license_spdx") => "MIT License",
        (Language::Japanese, "monitor.license_spdx") => "MIT ライセンス",
        (Language::English, "monitor.license_body") => {
            "Copyright (c) MikanseiLaboratory. Permission is hereby granted, free of charge, to use, copy, modify, and distribute this software under the MIT License terms."
        }
        (Language::Japanese, "monitor.license_body") => {
            "Copyright (c) MikanseiLaboratory. MIT ライセンスの条件下で、本ソフトウェアの使用・複製・改変・再配布が許可されています。"
        }
        (Language::English, "monitor.license_link") => "View full license on GitHub",
        (Language::Japanese, "monitor.license_link") => "GitHubで全文を表示",
        (Language::English, "monitor.host") => "Host",
        (Language::Japanese, "monitor.host") => "ホスト",
        (Language::English, "monitor.disconnect") => "Disconnect",
        (Language::Japanese, "monitor.disconnect") => "切断",

        // Patterns
        (Language::English, "patterns.start") => "Start",
        (Language::Japanese, "patterns.start") => "開始",
        (Language::English, "patterns.stop") => "Stop",
        (Language::Japanese, "patterns.stop") => "停止",
        (Language::English, "patterns.name") => "Source name",
        (Language::Japanese, "patterns.name") => "ソース名",
        (Language::English, "patterns.restart_required") => {
            "Stop sending to change the source name, resolution, or framerate."
        }
        (Language::Japanese, "patterns.restart_required") => {
            "ソース名・解像度・フレームレートを変更するには送出を停止してください"
        }
        (Language::English, "patterns.pattern") => "Pattern",
        (Language::Japanese, "patterns.pattern") => "パターン",
        (Language::English, "patterns.animate") => "Animate",
        (Language::Japanese, "patterns.animate") => "アニメーション",
        (Language::English, "patterns.anim_speed_h") => "H speed",
        (Language::Japanese, "patterns.anim_speed_h") => "横スピード",
        (Language::English, "patterns.anim_speed_v") => "V speed",
        (Language::Japanese, "patterns.anim_speed_v") => "縦スピード",
        (Language::English, "patterns.frame_buffer") => "Frame buffer",
        (Language::Japanese, "patterns.frame_buffer") => "フレームバッファ",
        (Language::English, "patterns.tone") => "Tone",
        (Language::Japanese, "patterns.tone") => "トーン",
        (Language::English, "patterns.tone_mute") => "Mute (−∞)",
        (Language::Japanese, "patterns.tone_mute") => "ミュート (−∞)",
        (Language::English, "patterns.tone_hz") => "Frequency",
        (Language::Japanese, "patterns.tone_hz") => "周波数",
        (Language::English, "patterns.tone_level") => "Level",
        (Language::Japanese, "patterns.tone_level") => "レベル",
        (Language::English, "patterns.sample_rate") => "Sample rate",
        (Language::Japanese, "patterns.sample_rate") => "サンプルレート",
        (Language::English, "patterns.channels") => "Channels",
        (Language::Japanese, "patterns.channels") => "チャンネル",
        (Language::English, "patterns.samples") => "Samples / packet",
        (Language::Japanese, "patterns.samples") => "サンプル／パケット",
        (Language::English, "patterns.resolution") => "Resolution",
        (Language::Japanese, "patterns.resolution") => "解像度",
        (Language::English, "patterns.fps") => "Framerate",
        (Language::Japanese, "patterns.fps") => "フレームレート",
        (Language::English, "patterns.profile") => "Quality",
        (Language::Japanese, "patterns.profile") => "品質",
        (Language::English, "patterns.settings") => "Settings",
        (Language::Japanese, "patterns.settings") => "設定",
        (Language::English, "patterns.image") => "Custom images",
        (Language::Japanese, "patterns.image") => "カスタム画像",
        (Language::English, "patterns.image_add") => "Add image",
        (Language::Japanese, "patterns.image_add") => "画像を追加",
        (Language::English, "patterns.image_remove") => "Remove",
        (Language::Japanese, "patterns.image_remove") => "削除",
        (Language::English, "patterns.image_reveal") => {
            #[cfg(target_os = "windows")]
            {
                "Reveal in Explorer"
            }
            #[cfg(target_os = "macos")]
            {
                "Reveal in Finder"
            }
            #[cfg(not(any(target_os = "windows", target_os = "macos")))]
            {
                "Show in Folder"
            }
        }
        (Language::Japanese, "patterns.image_reveal") => {
            #[cfg(target_os = "windows")]
            {
                "エクスプローラーで表示"
            }
            #[cfg(target_os = "macos")]
            {
                "Finder で表示"
            }
            #[cfg(not(any(target_os = "windows", target_os = "macos")))]
            {
                "フォルダーで表示"
            }
        }
        (Language::English, "patterns.sending") => "Sending",
        (Language::Japanese, "patterns.sending") => "送出中",
        (Language::English, "patterns.idle") => "Idle",
        (Language::Japanese, "patterns.idle") => "停止中",
        (Language::English, "patterns.output") => "Output",
        (Language::Japanese, "patterns.output") => "出力中",
        (Language::English, "patterns.stats") => "Statistics",
        (Language::Japanese, "patterns.stats") => "統計",
        (Language::English, "patterns.clients") => "Clients",
        (Language::Japanese, "patterns.clients") => "クライアント",
        (Language::English, "patterns.connections") => "Connections",
        (Language::Japanese, "patterns.connections") => "接続数",
        (Language::English, "patterns.video_subs") => "Video subs",
        (Language::Japanese, "patterns.video_subs") => "映像接続数",
        (Language::English, "patterns.audio_subs") => "Audio subs",
        (Language::Japanese, "patterns.audio_subs") => "音声接続数",
        (Language::English, "patterns.perf_warn") => {
            "Encode cannot sustain target FPS — lower quality"
        }
        (Language::Japanese, "patterns.perf_warn") => {
            "目標FPSを維持できません — 品質を下げてください"
        }

        // Config Manager
        (Language::English, "config.path") => "File",
        (Language::Japanese, "config.path") => "ファイル",
        (Language::English, "config.reload") => "Reload",
        (Language::Japanese, "config.reload") => "再読込",
        (Language::English, "config.reveal") => "Show in folder",
        (Language::Japanese, "config.reveal") => "フォルダーで表示",
        (Language::English, "config.discovery") => "Discovery Server",
        (Language::Japanese, "config.discovery") => "Discovery Server",
        (Language::English, "config.discovery_hint") => {
            "Example: omt://192.168.0.10:6399 — empty means DNS-SD (LAN multicast)"
        }
        (Language::Japanese, "config.discovery_hint") => {
            "例: omt://192.168.0.10:6399 — 空欄は DNS-SD（LAN のマルチキャスト検索）"
        }
        (Language::English, "config.clear_discovery") => "Clear field (use DNS-SD)",
        (Language::Japanese, "config.clear_discovery") => "空欄にする（DNS-SD）",
        (Language::English, "config.dns_sd_cleared") => {
            "Cleared. Click Save to write an empty DiscoveryServer (DNS-SD)."
        }
        (Language::Japanese, "config.dns_sd_cleared") => {
            "空欄にしました。保存すると DNS-SD になります。"
        }
        (Language::English, "config.dns_sd_already") => {
            "Already empty — DNS-SD is already selected. Click Save if the file still has a URL."
        }
        (Language::Japanese, "config.dns_sd_already") => {
            "すでに空欄です（DNS-SD）。ファイル側に URL が残っている場合は保存してください。"
        }
        (Language::English, "config.mode_dns_sd") => "Mode: DNS-SD",
        (Language::Japanese, "config.mode_dns_sd") => "現在: DNS-SD",
        (Language::English, "config.mode_unicast") => "Mode: unicast Discovery Server",
        (Language::Japanese, "config.mode_unicast") => "現在: 指定サーバー",
        (Language::English, "config.ph_discovery") => "omt://192.168.0.10:6399",
        (Language::Japanese, "config.ph_discovery") => "omt://192.168.0.10:6399",
        (Language::English, "config.port_range") => "Port range",
        (Language::Japanese, "config.port_range") => "使用ポート範囲",
        (Language::English, "config.port_range_sep") => "~",
        (Language::Japanese, "config.port_range_sep") => "～",
        (Language::English, "config.port_range_hint") => {
            "TCP ports senders may bind (default 6400 ~ 6600)"
        }
        (Language::Japanese, "config.port_range_hint") => {
            "送信側が使う TCP ポート範囲（既定 6400 ～ 6600）"
        }
        (Language::English, "config.port_start") => "Start",
        (Language::Japanese, "config.port_start") => "開始",
        (Language::English, "config.port_end") => "End",
        (Language::Japanese, "config.port_end") => "終了",
        (Language::English, "config.ph_port_start") => "6400",
        (Language::Japanese, "config.ph_port_start") => "6400",
        (Language::English, "config.ph_port_end") => "6600",
        (Language::Japanese, "config.ph_port_end") => "6600",
        (Language::English, "config.extra") => "All keys",
        (Language::Japanese, "config.extra") => "すべてのキー",
        (Language::English, "config.extra_hint") => {
            "Extra XML tags. Key is the element name, value is the text inside."
        }
        (Language::Japanese, "config.extra_hint") => {
            "追加の XML タグです。キーが要素名、値が中身です。"
        }
        (Language::English, "config.add") => "Add",
        (Language::Japanese, "config.add") => "追加",
        (Language::English, "config.delete") => "Delete",
        (Language::Japanese, "config.delete") => "削除",
        (Language::English, "config.key") => "Key",
        (Language::Japanese, "config.key") => "キー",
        (Language::English, "config.value") => "Value",
        (Language::Japanese, "config.value") => "値",
        (Language::English, "config.ph_key") => "VendorKey",
        (Language::Japanese, "config.ph_key") => "VendorKey",
        (Language::English, "config.ph_value") => "example",
        (Language::Japanese, "config.ph_value") => "example",
        (Language::English, "config.xml_preview") => "settings.xml (preview)",
        (Language::Japanese, "config.xml_preview") => "settings.xml（プレビュー）",
        (Language::English, "config.xml_preview_hint") => {
            "Read-only live view of what Save will write"
        }
        (Language::Japanese, "config.xml_preview_hint") => {
            "保存すると書き込まれる内容のリアルタイム表示（読み取り専用）"
        }
        (Language::English, "config.saved") => "Saved",
        (Language::Japanese, "config.saved") => "保存しました",
        (Language::English, "config.reloaded") => "Reloaded",
        (Language::Japanese, "config.reloaded") => "再読込しました",

        // Discovery Server GUI
        (Language::English, "discovery.bind") => "Bind address",
        (Language::Japanese, "discovery.bind") => "バインドアドレス",
        (Language::English, "discovery.port") => "Port",
        (Language::Japanese, "discovery.port") => "ポート",
        (Language::English, "discovery.start") => "Start",
        (Language::Japanese, "discovery.start") => "開始",
        (Language::English, "discovery.stop") => "Stop",
        (Language::Japanese, "discovery.stop") => "停止",
        (Language::English, "discovery.running") => "Running",
        (Language::Japanese, "discovery.running") => "稼働中",
        (Language::English, "discovery.stopped") => "Stopped",
        (Language::Japanese, "discovery.stopped") => "停止中",
        (Language::English, "discovery.peers") => "Clients",
        (Language::Japanese, "discovery.peers") => "クライアント",
        (Language::English, "discovery.sources") => "Registered sources",
        (Language::Japanese, "discovery.sources") => "登録ソース",
        (Language::English, "discovery.none") => "None",
        (Language::Japanese, "discovery.none") => "なし",
        (Language::English, "discovery.log") => "Event log",
        (Language::Japanese, "discovery.log") => "イベントログ",
        (Language::English, "discovery.clear_log") => "Clear",
        (Language::Japanese, "discovery.clear_log") => "クリア",
        (Language::English, "discovery.bind_hint") => {
            "Click a NIC below, or type :: / 0.0.0.0 / an address"
        }
        (Language::Japanese, "discovery.bind_hint") => {
            "下の NIC をクリックするか、:: / 0.0.0.0 / アドレスを入力"
        }
        (Language::English, "discovery.nic") => "Network interface",
        (Language::Japanese, "discovery.nic") => "ネットワークインターフェイス",
        (Language::English, "discovery.nic_all") => "All (::)",
        (Language::Japanese, "discovery.nic_all") => "すべて (::)",
        (Language::English, "discovery.nic_all_v4") => "All IPv4 (0.0.0.0)",
        (Language::Japanese, "discovery.nic_all_v4") => "すべて IPv4 (0.0.0.0)",

        _ => "???",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_language() {
        assert_eq!("ja".parse::<Language>().unwrap(), Language::Japanese);
        assert_eq!("en".parse::<Language>().unwrap(), Language::English);
    }

    #[test]
    fn required_keys_present() {
        let keys = [
            "app.title",
            "settings",
            "docs",
            "save",
            "language",
            "theme",
            "version",
            "simd",
            "update.check",
            "update.available.body",
            "tool.studio_monitor",
            "tool.test_patterns",
            "tool.config_manager",
            "tool.discovery_server",
            "monitor.stalled",
            "patterns.start",
            "patterns.restart_required",
            "config.path",
            "config.port_range",
            "config.xml_preview",
            "config.ph_discovery",
            "discovery.start",
            "discovery.nic",
        ];
        for key in keys {
            assert_ne!(t(Language::English, key), key, "missing en key {key}");
            assert_ne!(t(Language::Japanese, key), key, "missing ja key {key}");
        }
    }
}

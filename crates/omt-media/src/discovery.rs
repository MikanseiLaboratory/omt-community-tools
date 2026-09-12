//! Discovery browser wrapper.

use std::collections::HashSet;
use std::time::Duration;

use openmediatransport::{Discovery, OmtError};

use crate::runtime;

/// Brief browse used to avoid colliding with another local sender.
const UNIQUIFY_BROWSE: Duration = Duration::from_millis(500);

/// A discovered OMT source suitable for UI lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredSource {
    /// Display name (`HOSTNAME (Source)`).
    pub name: String,
    /// Host / machine label (`HOSTNAME`).
    pub host: String,
    /// Source name within the host.
    pub source: String,
    /// Connectable `omt://` URL (machine-name based).
    pub url: String,
    /// Advertised TCP port.
    pub port: u16,
    /// Discovery-time IP candidates for TCP connect (ordered by preference).
    pub addresses: Vec<String>,
}

impl From<&openmediatransport::OmtAddress> for DiscoveredSource {
    fn from(addr: &openmediatransport::OmtAddress) -> Self {
        let (host, source) = split_host_source(&addr.machine_name, &addr.name);
        Self {
            name: addr.instance_name(),
            host,
            source,
            url: addr.to_url(),
            port: addr.port,
            addresses: addr.addresses.clone(),
        }
    }
}

fn split_host_source(machine: &str, name: &str) -> (String, String) {
    if !machine.is_empty() {
        return (machine.to_string(), name.to_string());
    }
    // Fallback: parse `HOST (Source)` from a combined display name.
    if let Some(open) = name.find('(')
        && let Some(close) = name.rfind(')')
        && close > open
    {
        let host = name[..open].trim().to_string();
        let source = name[open + 1..close].trim().to_string();
        if !host.is_empty() && !source.is_empty() {
            return (host, source);
        }
    }
    ("Unknown".into(), name.to_string())
}

/// Thin wrapper around [`Discovery`] for periodic LAN browsing.
#[derive(Debug, Default)]
pub struct SourceBrowser {
    discovery: Option<Discovery>,
    sources: Vec<DiscoveredSource>,
}

impl SourceBrowser {
    /// Create an empty browser.
    pub fn new() -> Self {
        Self::default()
    }

    /// Refresh the source list, waiting up to `wait` for mDNS answers.
    pub fn refresh(&mut self, wait: Duration) -> Result<&[DiscoveredSource], OmtError> {
        if self.discovery.is_none() {
            self.discovery = Some(Discovery::new()?);
        }
        let discovery = self.discovery.as_mut().expect("discovery initialized");
        discovery.refresh_for(wait)?;
        self.sources = discovery
            .sources()
            .iter()
            .map(DiscoveredSource::from)
            .collect();
        Ok(&self.sources)
    }

    /// Last known sources.
    pub fn sources(&self) -> &[DiscoveredSource] {
        &self.sources
    }
}

/// One-shot LAN discovery on the shared Tokio blocking pool.
pub fn discover_sources(
    wait: Duration,
) -> tokio::task::JoinHandle<Result<Vec<DiscoveredSource>, OmtError>> {
    runtime::spawn_blocking(move || {
        let mut browser = SourceBrowser::new();
        browser.refresh(wait)?;
        Ok(browser.sources().to_vec())
    })
}

/// Browse the LAN and return `desired` or `desired (2)` / `(3)` when this
/// machine already advertises that source name.
pub fn uniquify_local_source_name(desired: &str) -> String {
    let sources = runtime::handle()
        .block_on(discover_sources(UNIQUIFY_BROWSE))
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default();
    uniquify_source_name(desired, taken_source_names_on_this_host(&sources))
}

/// Next free source name on this host (`Name`, then `Name (2)`, `Name (3)`, …).
pub fn uniquify_source_name(
    desired: &str,
    taken: impl IntoIterator<Item = impl AsRef<str>>,
) -> String {
    let desired = desired.trim();
    let desired = if desired.is_empty() {
        "Test Pattern"
    } else {
        desired
    };
    let taken: HashSet<String> = taken.into_iter().map(|s| s.as_ref().to_string()).collect();
    if !taken.contains(desired) {
        return clip_source_name(desired);
    }
    let stem = name_collision_stem(desired);
    for n in 2..=99 {
        let candidate = format!("{stem} ({n})");
        if !taken.contains(&candidate) {
            return clip_source_name(&candidate);
        }
    }
    clip_source_name(&format!("{stem} ({})", std::process::id()))
}

fn taken_source_names_on_this_host(sources: &[DiscoveredSource]) -> Vec<String> {
    sources
        .iter()
        .filter(|src| is_this_machine(&src.host))
        .map(|src| src.source.clone())
        .collect()
}

fn is_this_machine(host: &str) -> bool {
    let host = host.trim();
    if host.is_empty() {
        return false;
    }
    let upper = host.to_ascii_uppercase();
    this_machine_labels().iter().any(|label| label == &upper)
}

fn this_machine_labels() -> Vec<String> {
    let mut labels = Vec::new();
    for key in ["COMPUTERNAME", "HOSTNAME"] {
        if let Ok(value) = std::env::var(key) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                labels.push(trimmed.to_ascii_uppercase());
            }
        }
    }
    labels.push("LOCALHOST".into());
    labels
}

fn name_collision_stem(name: &str) -> &str {
    if let Some(rest) = name.strip_suffix(')')
        && let Some(open) = rest.rfind(" (")
    {
        let inside = &rest[open + 2..];
        if !inside.is_empty() && inside.chars().all(|c| c.is_ascii_digit()) {
            return rest[..open].trim_end();
        }
    }
    if let Some(dash) = name.rfind('-') {
        let tail = &name[dash + 1..];
        if !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit()) {
            return name[..dash].trim_end();
        }
    }
    name
}

fn clip_source_name(name: &str) -> String {
    let mut out = name.trim().to_string();
    while out.len() > 64 {
        out.pop();
    }
    out
}

/// Spawn discovery and deliver the result on a std mpsc channel (UI-friendly).
pub fn spawn_discover(
    wait: Duration,
) -> std::sync::mpsc::Receiver<Result<Vec<DiscoveredSource>, String>> {
    let (tx, rx) = std::sync::mpsc::channel();
    runtime::spawn(async move {
        let result = match discover_sources(wait).await {
            Ok(Ok(list)) => Ok(list),
            Ok(Err(e)) => Err(e.to_string()),
            Err(e) => Err(e.to_string()),
        };
        let _ = tx.send(result);
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::{name_collision_stem, split_host_source, uniquify_source_name};

    #[test]
    fn splits_combined_name() {
        let (h, s) = split_host_source("", "DESKTOP (Cam1)");
        assert_eq!(h, "DESKTOP");
        assert_eq!(s, "Cam1");
    }

    #[test]
    fn stem_strips_paren_or_dash_index() {
        assert_eq!(name_collision_stem("Test Pattern"), "Test Pattern");
        assert_eq!(name_collision_stem("Test Pattern (2)"), "Test Pattern");
        assert_eq!(name_collision_stem("Test Pattern-3"), "Test Pattern");
        assert_eq!(name_collision_stem("Cam (main)"), "Cam (main)");
    }

    #[test]
    fn uniquify_appends_paren_index() {
        assert_eq!(
            uniquify_source_name("Test Pattern", None::<&str>),
            "Test Pattern"
        );
        assert_eq!(
            uniquify_source_name("Test Pattern", ["Test Pattern"]),
            "Test Pattern (2)"
        );
        assert_eq!(
            uniquify_source_name("Test Pattern", ["Test Pattern", "Test Pattern (2)"]),
            "Test Pattern (3)"
        );
        assert_eq!(
            uniquify_source_name("Test Pattern (2)", ["Test Pattern", "Test Pattern (2)"]),
            "Test Pattern (3)"
        );
        assert_eq!(
            uniquify_source_name("Test Pattern-2", ["Test Pattern-2"]),
            "Test Pattern (2)"
        );
    }
}

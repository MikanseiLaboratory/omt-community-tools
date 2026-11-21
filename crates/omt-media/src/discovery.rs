//! Discovery browser wrapper.

use std::net::{IpAddr, SocketAddr, TcpStream};
use std::time::Duration;

use openmediatransport::{Discovery, OmtAddress, OmtError};

use crate::runtime;

/// Brief browse used to detect a colliding local sender.
const COLLISION_BROWSE: Duration = Duration::from_millis(500);

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

/// Browse the LAN and return a live source on this machine that still owns
/// `desired`, if any.
///
/// Test Patterns refuses to start when this returns a source. Stale DNS-SD
/// entries are ignored by probing the advertised TCP port.
pub fn find_local_name_collision(desired: &str) -> Option<DiscoveredSource> {
    let desired = desired.trim();
    if desired.is_empty() {
        return None;
    }
    let sources = runtime::handle()
        .block_on(discover_sources(COLLISION_BROWSE))
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default();
    local_name_collision(desired, &sources, source_is_live)
}

/// Pure collision check used by [`find_local_name_collision`] and tests.
pub fn local_name_collision(
    desired: &str,
    sources: &[DiscoveredSource],
    is_live: impl Fn(&DiscoveredSource) -> bool,
) -> Option<DiscoveredSource> {
    let local = local_identity(desired);
    collision_with(sources, local.as_ref(), is_live)
}

/// Collision check against an explicit local identity (test seam).
fn collision_with(
    sources: &[DiscoveredSource],
    local: Option<&OmtAddress>,
    is_live: impl Fn(&DiscoveredSource) -> bool,
) -> Option<DiscoveredSource> {
    let local = local?;
    sources
        .iter()
        // Compare the advertised name, not the raw setting: a long source
        // name is truncated to fit the 63-byte DNS-SD instance label.
        .find(|src| src.source == local.name && source_is_local(src, local) && is_live(src))
        .cloned()
}

/// This machine's identity as a sender would advertise it for `source_name`.
///
/// Built with the same [`OmtAddress::local`] the sender uses, so the machine
/// label — including its DNS-SD length truncation — matches what peers
/// discover.
fn local_identity(source_name: &str) -> Option<OmtAddress> {
    OmtAddress::local(source_name, 0).ok()
}

/// Whether `source` is advertised by this machine.
///
/// Two independent signals, because neither is reliable on its own:
///
/// - Addresses: a local sender advertises one of this machine's non-loopback
///   interface addresses. Loopback is never a locality signal: it identifies
///   whichever machine is listening, not the sender, so an entry that lists
///   `127.0.0.1` must not be classified as local by that address.
/// - Machine name: compared with the same `gethostname` value the sender
///   advertises. `LOCALHOST` is never treated as an identity — it is a
///   loopback alias every machine can report, so a remote sender may use it
///   too. A sender named `LOCALHOST` is still recognized through its
///   addresses.
fn source_is_local(source: &DiscoveredSource, local: &OmtAddress) -> bool {
    let by_address = source.addresses.iter().any(|addr| {
        !is_loopback(addr)
            && local
                .addresses
                .iter()
                .any(|own| own.eq_ignore_ascii_case(addr))
    });
    if by_address {
        return true;
    }
    let machine = local.machine_name.trim();
    !machine.is_empty()
        && !machine.eq_ignore_ascii_case("LOCALHOST")
        && source.host.trim().eq_ignore_ascii_case(machine)
}

fn is_loopback(address: &str) -> bool {
    address.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

/// Whether the advertised sender still accepts TCP connections.
fn source_is_live(source: &DiscoveredSource) -> bool {
    if source.port == 0 {
        return false;
    }
    let mut hosts: Vec<&str> = vec!["127.0.0.1"];
    for address in &source.addresses {
        if !hosts.contains(&address.as_str()) {
            hosts.push(address);
        }
    }
    hosts.iter().any(|host| {
        host.parse::<IpAddr>().is_ok_and(|ip| {
            TcpStream::connect_timeout(
                &SocketAddr::new(ip, source.port),
                Duration::from_millis(120),
            )
            .is_ok()
        })
    })
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
    use super::{DiscoveredSource, collision_with, source_is_local, split_host_source};
    use openmediatransport::OmtAddress;

    fn source(host: &str, name: &str, port: u16, addresses: &[&str]) -> DiscoveredSource {
        DiscoveredSource {
            name: format!("{host} ({name})"),
            host: host.into(),
            source: name.into(),
            url: format!("omt://{host}:{port}/{name}"),
            port,
            addresses: addresses.iter().map(|a| (*a).into()).collect(),
        }
    }

    fn local(machine: &str, addresses: &[&str]) -> OmtAddress {
        let mut address = OmtAddress::new("Test Pattern", 0);
        address.machine_name = machine.into();
        address.addresses = addresses.iter().map(|a| (*a).into()).collect();
        address
    }

    #[test]
    fn splits_combined_name() {
        let (h, s) = split_host_source("", "DESKTOP (Cam1)");
        assert_eq!(h, "DESKTOP");
        assert_eq!(s, "Cam1");
    }

    #[test]
    fn remote_localhost_is_not_this_machine() {
        // A remote sender advertising the `LOCALHOST` fallback must not be
        // mistaken for this machine: that made Test Patterns refuse to start
        // when a same-named source existed anywhere on the LAN.
        let remote = source("LOCALHOST", "Test Pattern", 6401, &["192.0.2.10"]);
        let local = local("MYMAC.LOCAL", &["192.0.2.5", "127.0.0.1"]);
        assert!(!source_is_local(&remote, &local));
    }

    #[test]
    fn local_sender_is_recognized_by_address() {
        // A local sender whose machine name is the `LOCALHOST` fallback is
        // still local: it advertises this machine's interface address.
        let sender = source(
            "LOCALHOST",
            "Test Pattern",
            6401,
            &["192.0.2.5", "127.0.0.1"],
        );
        let local = local("LOCALHOST", &["192.0.2.5", "127.0.0.1"]);
        assert!(source_is_local(&sender, &local));
    }

    #[test]
    fn local_sender_is_recognized_by_machine_name_without_addresses() {
        // macOS Finder launches have no `HOSTNAME`; the advertised machine
        // name still identifies this machine.
        let sender = source("MYMAC.LOCAL", "Test Pattern", 6401, &[]);
        let local = local("MYMAC.LOCAL", &["192.0.2.5"]);
        assert!(source_is_local(&sender, &local));
    }

    #[test]
    fn localhost_fallback_name_alone_is_not_identity() {
        let sender = source("LOCALHOST", "Test Pattern", 6401, &[]);
        let local = local("LOCALHOST", &["192.0.2.5", "127.0.0.1"]);
        assert!(!source_is_local(&sender, &local));
    }

    #[test]
    fn collision_only_matches_live_local_sources() {
        let local = local("MYMAC.LOCAL", &["192.0.2.5", "127.0.0.1"]);
        let sources = vec![
            source("LOCALHOST", "Test Pattern", 6401, &["192.0.2.5"]),
            source("REMOTE", "Test Pattern", 6402, &["192.0.2.10"]),
        ];
        let live_local = |src: &DiscoveredSource| src.host == "LOCALHOST";
        assert_eq!(
            collision_with(&sources, Some(&local), live_local)
                .expect("local collision")
                .port,
            6401
        );
        assert!(collision_with(&sources, Some(&local), |_| false).is_none());

        let mut other_name = local.clone();
        other_name.name = "Test Pattern 2".into();
        assert!(collision_with(&sources, Some(&other_name), live_local).is_none());
    }
}

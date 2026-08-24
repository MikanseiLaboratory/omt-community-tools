//! In-memory editor for libomtnet-compatible `settings.xml`.

use std::fs;
use std::path::{Path, PathBuf};

use openmediatransport::{
    KEY_DISCOVERY_SERVER, KEY_NETWORK_PORT_END, KEY_NETWORK_PORT_START, NETWORK_PORT_END,
    NETWORK_PORT_START, Settings, settings_file_path,
};

const KNOWN_KEYS: [&str; 3] = [
    KEY_DISCOVERY_SERVER,
    KEY_NETWORK_PORT_START,
    KEY_NETWORK_PORT_END,
];

/// Editable view of a `settings.xml` file.
#[derive(Debug, Clone)]
pub struct SettingsEditor {
    /// Absolute path of the XML file.
    pub path: PathBuf,
    /// `DiscoveryServer` value (`omt://host:port` or empty for DNS-SD).
    pub discovery_server: String,
    /// `NetworkPortStart` as edited text.
    pub port_start: String,
    /// `NetworkPortEnd` as edited text.
    pub port_end: String,
    /// Unknown / extra keys, in sorted order.
    pub extras: Vec<(String, String)>,
    /// Draft key for the add-row form.
    pub new_key: String,
    /// Draft value for the add-row form.
    pub new_value: String,
    /// Last success message.
    pub status: Option<String>,
    /// Last error / validation message. Save is blocked while unreadable.
    pub error: Option<String>,
    /// True when the on-disk file could not be read.
    pub unreadable: bool,
}

impl SettingsEditor {
    /// Load the process-wide settings file.
    pub fn load() -> Self {
        Self::load_from_path(settings_file_path())
    }

    /// Load `path` without writing it.
    pub fn load_from_path(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let mut editor = Self {
            path: path.clone(),
            discovery_server: String::new(),
            port_start: NETWORK_PORT_START.to_string(),
            port_end: NETWORK_PORT_END.to_string(),
            extras: Vec::new(),
            new_key: String::new(),
            new_value: String::new(),
            status: None,
            error: None,
            unreadable: false,
        };
        editor.reload_from(&path);
        editor
    }

    /// Re-read the current path from disk.
    pub fn reload(&mut self) {
        let path = self.path.clone();
        self.reload_from(&path);
        if self.error.is_none() {
            self.status = Some("Reloaded".into());
        }
    }

    fn reload_from(&mut self, path: &Path) {
        self.status = None;
        self.error = None;
        self.unreadable = false;
        if path.exists() {
            match fs::read_to_string(path) {
                Ok(_) => {}
                Err(e) => {
                    self.unreadable = true;
                    self.error = Some(format!("could not read {}: {e}", path.display()));
                    return;
                }
            }
        }
        let settings = Settings::from_path(path);
        self.discovery_server = settings.get_string(KEY_DISCOVERY_SERVER, "");
        let (start, end) = settings.network_port_range();
        self.port_start = start.to_string();
        self.port_end = end.to_string();
        self.extras = settings
            .to_vec()
            .into_iter()
            .filter(|(k, _)| !KNOWN_KEYS.contains(&k.as_str()))
            .collect();
    }

    /// Validate fields without writing.
    pub fn validate(&self) -> Result<(), String> {
        if self.unreadable {
            return Err("file could not be read; fix permissions and reload before saving".into());
        }
        validate_discovery_url(&self.discovery_server)?;
        let start = parse_port(&self.port_start, "Sender port start")?;
        let end = parse_port(&self.port_end, "Sender port end")?;
        if start > end {
            return Err("Sender port start must be less than or equal to port end".into());
        }
        for (key, _) in &self.extras {
            if !is_xml_name(key) {
                return Err(format!("invalid extra key {key:?}"));
            }
            if KNOWN_KEYS.contains(&key.as_str()) {
                return Err(format!(
                    "{key} is a dedicated field; remove it from extra keys"
                ));
            }
        }
        Ok(())
    }

    /// Persist the editor contents. Does not write when validation fails.
    pub fn save(&mut self) -> Result<(), String> {
        self.validate()?;
        let mut settings = Settings::from_path(&self.path);
        let desired: Vec<String> = std::iter::once(KEY_DISCOVERY_SERVER.to_string())
            .chain([
                KEY_NETWORK_PORT_START.to_string(),
                KEY_NETWORK_PORT_END.to_string(),
            ])
            .chain(self.extras.iter().map(|(k, _)| k.clone()))
            .collect();
        for (key, _) in settings.to_vec() {
            if !desired.iter().any(|k| k == &key) {
                settings.remove(&key);
            }
        }
        settings.set_string(KEY_DISCOVERY_SERVER, self.discovery_server.trim());
        settings.set_integer(
            KEY_NETWORK_PORT_START,
            i32::from(parse_port(&self.port_start, "Sender port start")?),
        );
        settings.set_integer(
            KEY_NETWORK_PORT_END,
            i32::from(parse_port(&self.port_end, "Sender port end")?),
        );
        for (key, value) in &self.extras {
            settings.set_string(key, value);
        }
        settings.save().map_err(|e| e.to_string())?;
        self.error = None;
        self.status = Some("Saved".into());
        Ok(())
    }

    /// Clear DiscoveryServer so clients fall back to DNS-SD.
    pub fn clear_discovery(&mut self) {
        self.discovery_server.clear();
        self.status = None;
        self.error = None;
    }

    /// Append the draft extra key if it is a valid XML name.
    pub fn add_extra(&mut self) -> Result<(), String> {
        let key = self.new_key.trim().to_string();
        if !is_xml_name(&key) {
            return Err("key must be a valid XML name (letter or _ first)".into());
        }
        if KNOWN_KEYS.contains(&key.as_str()) || self.extras.iter().any(|(k, _)| k == &key) {
            return Err(format!("{key} already exists"));
        }
        self.extras.push((key, self.new_value.clone()));
        self.extras.sort_by(|a, b| a.0.cmp(&b.0));
        self.new_key.clear();
        self.new_value.clear();
        self.error = None;
        Ok(())
    }

    /// Remove an extra key by index.
    pub fn remove_extra(&mut self, index: usize) {
        if index < self.extras.len() {
            self.extras.remove(index);
        }
    }

    /// XML that [`Self::save`] would write, from the current editor fields.
    pub fn preview_xml(&self) -> String {
        let mut values = std::collections::BTreeMap::new();
        values.insert(
            KEY_DISCOVERY_SERVER.to_string(),
            self.discovery_server.trim().to_string(),
        );
        values.insert(
            KEY_NETWORK_PORT_START.to_string(),
            self.port_start.trim().to_string(),
        );
        values.insert(
            KEY_NETWORK_PORT_END.to_string(),
            self.port_end.trim().to_string(),
        );
        for (key, value) in &self.extras {
            let key = key.trim();
            if !key.is_empty() {
                values.insert(key.to_string(), value.clone());
            }
        }
        let mut out = String::from("<Settings>\n");
        for (k, v) in values {
            out.push_str("  <");
            out.push_str(&k);
            out.push('>');
            out.push_str(&escape_xml(&v));
            out.push_str("</");
            out.push_str(&k);
            out.push_str(">\n");
        }
        out.push_str("</Settings>\n");
        out
    }
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn parse_port(raw: &str, label: &str) -> Result<u16, String> {
    let value: u32 = raw
        .trim()
        .parse()
        .map_err(|_| format!("{label} must be an integer from 1 to 65535"))?;
    if (1..=65535).contains(&value) {
        Ok(value as u16)
    } else {
        Err(format!("{label} must be an integer from 1 to 65535"))
    }
}

fn validate_discovery_url(raw: &str) -> Result<(), String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(());
    }
    let rest = raw
        .strip_prefix("omt://")
        .ok_or_else(|| "Discovery Server must be empty (DNS-SD) or omt://host:port".to_string())?;
    let rest = rest.split('/').next().unwrap_or(rest);
    if rest.is_empty() {
        return Err("Discovery Server host is missing".into());
    }
    if let Some((host, port)) = rest.rsplit_once(':') {
        if host.is_empty() || host == "[" {
            return Err("Discovery Server host is missing".into());
        }
        parse_port(port, "Discovery Server port")?;
    }
    Ok(())
}

fn is_xml_name(key: &str) -> bool {
    let mut chars = key.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    /// Unique per call so parallel tests never share a directory.
    /// Wall-clock nanos alone collide on macOS (coarse clock + cargo test threads).
    fn temp_path() -> PathBuf {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("omt-cfg-{}-{n}-{seq}", std::process::id()));
        fs::create_dir_all(&dir).expect("temp settings dir");
        dir.join("settings.xml")
    }

    fn cleanup(path: &Path) {
        if let Some(dir) = path.parent() {
            let _ = fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn roundtrip_known_and_unknown_keys() {
        let path = temp_path();
        fs::write(
            &path,
            "<Settings>\n  <DiscoveryServer>omt://10.0.0.1:6399</DiscoveryServer>\n  <VendorKey>keep</VendorKey>\n</Settings>\n",
        )
        .unwrap();
        let mut editor = SettingsEditor::load_from_path(&path);
        assert_eq!(editor.discovery_server, "omt://10.0.0.1:6399");
        assert_eq!(editor.extras, vec![("VendorKey".into(), "keep".into())]);
        editor.discovery_server = "omt://127.0.0.1:6399".into();
        editor.port_start = "6500".into();
        editor.port_end = "6510".into();
        editor.extras[0].1 = "updated".into();
        editor.save().unwrap();

        let loaded = Settings::from_path(&path);
        assert_eq!(
            loaded.get_string(KEY_DISCOVERY_SERVER, ""),
            "omt://127.0.0.1:6399"
        );
        assert_eq!(loaded.get_integer(KEY_NETWORK_PORT_START, 0), 6500);
        assert_eq!(loaded.get_integer(KEY_NETWORK_PORT_END, 0), 6510);
        assert_eq!(loaded.get_string("VendorKey", ""), "updated");
        cleanup(&path);
    }

    #[test]
    fn rejects_invalid_url_and_inverted_ports_without_writing() {
        let path = temp_path();
        let mut editor = SettingsEditor::load_from_path(&path);
        editor.discovery_server = "http://nope".into();
        assert!(editor.save().is_err());
        assert!(!path.exists());

        editor.discovery_server = "omt://127.0.0.1:6399".into();
        editor.port_start = "6600".into();
        editor.port_end = "6400".into();
        assert!(
            editor
                .validate()
                .unwrap_err()
                .contains("less than or equal")
        );
        assert!(!path.exists());
        cleanup(&path);
    }

    #[test]
    fn remove_extra_key_is_persisted() {
        let path = temp_path();
        let mut editor = SettingsEditor::load_from_path(&path);
        editor.new_key = "CustomFlag".into();
        editor.new_value = "yes".into();
        editor.add_extra().unwrap();
        editor.save().unwrap();
        assert!(fs::read_to_string(&path).unwrap().contains("CustomFlag"));

        editor.remove_extra(0);
        editor.save().unwrap();
        let xml = fs::read_to_string(&path).unwrap();
        assert!(!xml.contains("CustomFlag"));
        cleanup(&path);
    }

    #[test]
    fn empty_discovery_means_dns_sd() {
        let path = temp_path();
        let mut editor = SettingsEditor::load_from_path(&path);
        editor.discovery_server = "omt://10.0.0.2:6399".into();
        editor.save().unwrap();
        editor.clear_discovery();
        editor.save().unwrap();
        let loaded = Settings::from_path(&path);
        assert!(loaded.discovery_server().is_none());
        cleanup(&path);
    }

    #[test]
    fn preview_xml_follows_editor_fields() {
        let path = temp_path();
        let mut editor = SettingsEditor::load_from_path(&path);
        editor.discovery_server.clear();
        editor.port_start = "6400".into();
        editor.port_end = "6600".into();
        editor.new_key = "VendorKey".into();
        editor.new_value = "keep".into();
        editor.add_extra().unwrap();
        let xml = editor.preview_xml();
        assert!(xml.contains("<DiscoveryServer></DiscoveryServer>"));
        assert!(xml.contains("<NetworkPortStart>6400</NetworkPortStart>"));
        assert!(xml.contains("<NetworkPortEnd>6600</NetworkPortEnd>"));
        assert!(xml.contains("<VendorKey>keep</VendorKey>"));
        cleanup(&path);
    }

    #[test]
    fn unreadable_file_does_not_save() {
        let path = temp_path();
        let mut editor = SettingsEditor::load_from_path(&path);
        editor.unreadable = true;
        editor.error = Some("denied".into());
        assert!(editor.save().unwrap_err().contains("could not be read"));
        cleanup(&path);
    }
}

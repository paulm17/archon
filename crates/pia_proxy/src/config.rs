use crate::error::{ProxyError, Result};
use serde::Deserialize;
use std::path::Path;

/// Top level configuration, deserialised from `config.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub listen: Listen,
    /// Credentials; normally absent from the file and supplied through
    /// `PIA_USER` / `PIA_PASS`.
    #[serde(default)]
    pub pia: Pia,
    #[serde(default)]
    pub rotation: Rotation,
    #[serde(default)]
    pub regions: Vec<Region>,
    /// Optional extra upstream: PIA's Netherlands SOCKS5 proxy.
    #[serde(default)]
    pub socks5: Option<Socks5Proxy>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Listen {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
}

impl Default for Listen {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Pia {
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Rotation {
    /// How many upstreams to try for a single client connection.
    #[serde(default = "default_max_attempts")]
    pub max_attempts: usize,
    /// How long a retired upstream waits before it is probed again.
    #[serde(default = "default_cooldown")]
    pub cooldown_secs: u64,
    /// Total time one client connection may spend trying upstreams (including
    /// DNS) before it fails. Shared between attempts, so this is also the hard
    /// upper bound on what a blackholed tunnel can cost the browser navigation
    /// waiting on it.
    #[serde(default = "default_connect_timeout")]
    pub connect_timeout_secs: u64,
    /// Consecutive failures before an upstream leaves rotation. A single blip
    /// never retires anything.
    #[serde(default = "default_failures_before_retire")]
    pub failures_before_retire: u32,
}

impl Default for Rotation {
    fn default() -> Self {
        Self {
            max_attempts: default_max_attempts(),
            cooldown_secs: default_cooldown(),
            connect_timeout_secs: default_connect_timeout(),
            failures_before_retire: default_failures_before_retire(),
        }
    }
}

/// One PIA region to keep a WireGuard tunnel open to.
#[derive(Debug, Clone, Deserialize)]
pub struct Region {
    /// PIA region id, e.g. `us_chicago`.
    pub id: String,
    /// Human label, defaults to the id.
    #[serde(default)]
    pub name: Option<String>,
    /// Optional fixed local port serving only this region, gluetun style.
    #[serde(default)]
    pub port: Option<u16>,
}

/// PIA's SOCKS5 proxy (Netherlands only), used as an extra upstream.
#[derive(Debug, Clone, Deserialize)]
pub struct Socks5Proxy {
    #[serde(default = "default_socks5_endpoint")]
    pub endpoint: String,
    pub username: String,
    pub password: String,
}

fn default_host() -> String {
    "127.0.0.1".to_string()
}
fn default_port() -> u16 {
    8889
}
fn default_max_attempts() -> usize {
    3
}
fn default_cooldown() -> u64 {
    60
}
fn default_connect_timeout() -> u64 {
    12
}
fn default_failures_before_retire() -> u32 {
    3
}
fn default_socks5_endpoint() -> String {
    "proxy-nl.privateinternetaccess.com:1080".to_string()
}

impl Config {
    /// Read and validate `config.toml` from disk.
    pub async fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let raw = tokio::fs::read_to_string(path).await.map_err(|e| {
            ProxyError::Config(format!("cannot read {}: {e}", path.display()))
        })?;
        Self::from_str(&raw)
    }

    /// Parse and validate configuration text.
    pub fn from_str(raw: &str) -> Result<Self> {
        let mut config: Config = toml::from_str(raw)
            .map_err(|e| ProxyError::Config(format!("cannot parse config: {e}")))?;

        // Environment wins over the file so credentials can stay out of it.
        if let Ok(user) = std::env::var("PIA_USER") {
            if !user.trim().is_empty() {
                config.pia.username = user.trim().to_string();
            }
        }
        if let Ok(pass) = std::env::var("PIA_PASS") {
            if !pass.is_empty() {
                config.pia.password = pass;
            }
        }

        config.validate()?;
        Ok(config)
    }

    /// Reject configuration that cannot work, before any network access.
    pub fn validate(&self) -> Result<()> {
        if self.pia.username.trim().is_empty() || self.pia.password.is_empty() {
            return Err(ProxyError::Config(
                "PIA_USER and PIA_PASS are required — export them in the environment".into(),
            ));
        }
        if self.regions.is_empty() && self.socks5.is_none() {
            return Err(ProxyError::Config(
                "at least one [[regions]] entry or a [socks5] upstream is required".into(),
            ));
        }
        if self.rotation.max_attempts == 0 {
            return Err(ProxyError::Config("rotation.max_attempts must be at least 1".into()));
        }
        if self.rotation.failures_before_retire == 0 {
            return Err(ProxyError::Config(
                "rotation.failures_before_retire must be at least 1".into(),
            ));
        }
        if self.rotation.connect_timeout_secs == 0 {
            return Err(ProxyError::Config(
                "rotation.connect_timeout_secs must be at least 1".into(),
            ));
        }
        let mut seen = std::collections::HashSet::new();
        for region in &self.regions {
            if region.id.trim().is_empty() {
                return Err(ProxyError::Config("a [[regions]] entry has an empty id".into()));
            }
            if !seen.insert(region.id.clone()) {
                return Err(ProxyError::Config(format!(
                    "region '{}' is listed more than once",
                    region.id
                )));
            }
        }
        Ok(())
    }

    /// Warn when credentials are readable by other users.
    pub fn warn_if_world_readable(path: impl AsRef<Path>) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(meta) = std::fs::metadata(path.as_ref()) {
                let mode = meta.permissions().mode() & 0o777;
                if mode & 0o077 != 0 {
                    tracing::warn!(
                        path = %path.as_ref().display(),
                        mode = format!("{mode:o}"),
                        "config holds credentials but is readable by other users; consider chmod 600"
                    );
                }
            }
        }
    }

    /// Optional pinned per-region ports, for gluetun-style fixed endpoints.
    pub fn pinned_ports(&self) -> Vec<(u16, String)> {
        self.regions
            .iter()
            .filter_map(|r| r.port.map(|p| (p, r.id.clone())))
            .collect()
    }
}

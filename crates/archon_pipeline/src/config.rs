//! Run configuration (config.toml). Domain data lives in the catalog.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub catalog: CatalogSection,
    #[serde(default)]
    pub output: Output,
    #[serde(default)]
    pub browser: Browser,
    #[serde(default)]
    pub proxy: Proxy,
    #[serde(default)]
    pub proxy_pool: ProxyPool,
    #[serde(default)]
    pub retry: Retry,
    #[serde(default)]
    pub timing: Timing,
    #[serde(default)]
    pub targets: Vec<Target>,
    #[serde(default)]
    pub scrape: Scrape,
    #[serde(default)]
    pub json: Json,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct CatalogSection {
    /// Omit to use the catalog compiled into the binary.
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Output {
    pub gospels: String,
    pub json: String,
    pub books: String,
    pub report: String,
}

impl Default for Output {
    fn default() -> Self {
        Self {
            gospels: "gospels".into(),
            json: "json".into(),
            books: "books".into(),
            report: "reports".into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Browser {
    pub headless: bool,
    pub window_width: u32,
    pub window_height: u32,
    pub user_data_dir: String,
    pub extensions_dir: String,
    pub user_agents: Vec<String>,
}

impl Default for Browser {
    fn default() -> Self {
        Self {
            headless: false,
            window_width: 1920,
            window_height: 1080,
            user_data_dir: "chrome/data".into(),
            extensions_dir: String::new(),
            user_agents: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Proxy {
    pub enabled: bool,
    #[serde(rename = "type", default = "default_proxy_type")]
    pub proxy_type: String,
    #[serde(default)]
    pub endpoints: Vec<ProxyEndpoint>,
}

fn default_proxy_type() -> String {
    "http".into()
}

impl Default for Proxy {
    fn default() -> Self {
        Self {
            enabled: false,
            proxy_type: default_proxy_type(),
            endpoints: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProxyEndpoint {
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub region: String,
}

impl Default for ProxyEndpoint {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: 8889,
            region: String::new(),
        }
    }
}

/// The PIA proxy: the `pia_proxy` library in `crates/pia_proxy`.
#[derive(Debug, Clone, Deserialize)]
pub struct ProxyPool {
    /// Start the proxy in-process during `scrape`.
    #[serde(default)]
    pub enabled: bool,
    /// Path to the proxy crate's config.toml, which holds the PIA credentials
    /// and the region list. This app never reads or prints that file.
    #[serde(default = "default_pia_config")]
    pub config: String,
    /// Provision at most this many regions from that list. PIA rate-limits its
    /// token endpoint, so opening every region on each start is what trips a
    /// 429; a handful is plenty for rotation.
    #[serde(default)]
    pub regions: Option<usize>,
}

fn default_pia_config() -> String {
    "../pia_proxy/config.toml".to_string()
}

impl Default for ProxyPool {
    fn default() -> Self {
        Self {
            enabled: false,
            config: default_pia_config(),
            regions: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Retry {
    pub max_attempts: u32,
    pub initial_delay_ms: u64,
    pub max_delay_ms: u64,
    pub backoff_multiplier: f64,
    pub jitter_factor: f64,
}

impl Default for Retry {
    fn default() -> Self {
        Self {
            max_attempts: 10,
            initial_delay_ms: 1000,
            max_delay_ms: 32000,
            backoff_multiplier: 2.0,
            jitter_factor: 0.1,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Timing {
    pub page_load: u64,
    pub navigation: u64,
    pub element_click: u64,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            page_load: 2000,
            navigation: 30000,
            element_click: 500,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Target {
    #[serde(default)]
    pub name: String,
    pub site: String,
    #[serde(default)]
    pub languages: Vec<String>,
    #[serde(default)]
    pub translations: Vec<String>,
    #[serde(default)]
    pub books: Option<Vec<String>>,
    #[serde(default)]
    pub chapters: Option<Vec<u32>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Scrape {
    pub skip_existing: bool,
    pub verify_after_save: bool,
    pub write_manifest: bool,
}

impl Default for Scrape {
    fn default() -> Self {
        Self {
            skip_existing: true,
            verify_after_save: true,
            write_manifest: true,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Json {
    pub skip_existing: bool,
    pub pretty: bool,
}

impl Default for Json {
    fn default() -> Self {
        Self {
            skip_existing: false,
            pretty: true,
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        let config: Config =
            toml::from_str(&text).with_context(|| format!("Invalid {}", path.display()))?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(self.retry.max_attempts > 0, "retry.max_attempts must be greater than zero");
        anyhow::ensure!(
            self.retry.backoff_multiplier >= 1.0,
            "retry.backoff_multiplier must be at least 1.0"
        );
        anyhow::ensure!(
            (0.0..=1.0).contains(&self.retry.jitter_factor),
            "retry.jitter_factor must be between 0.0 and 1.0"
        );
        anyhow::ensure!(
            self.timing.navigation > 0 && self.timing.element_click > 0,
            "timing.navigation and timing.element_click must be greater than zero"
        );
        for (name, dir) in [
            ("output.gospels", &self.output.gospels),
            ("output.json", &self.output.json),
            ("output.books", &self.output.books),
            ("output.report", &self.output.report),
        ] {
            anyhow::ensure!(!dir.trim().is_empty(), "{name} must not be empty");
        }
        if self.proxy.enabled {
            anyhow::ensure!(
                !self.proxy.endpoints.is_empty(),
                "proxy.enabled requires at least one endpoint"
            );
        }
        if self.proxy_pool.enabled {
            anyhow::ensure!(
                !self.proxy_pool.config.trim().is_empty(),
                "proxy_pool.enabled requires proxy_pool.config to name the PIA proxy config.toml"
            );
            anyhow::ensure!(
                self.proxy_pool.regions.is_none_or(|regions| regions > 0),
                "proxy_pool.regions must be at least 1 when set"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_config_is_valid() {
        let config: Config = toml::from_str(include_str!("../config.toml")).expect("config.toml");
        config.validate().unwrap();
        assert_eq!(config.output.gospels, "gospels");
        assert_eq!(config.targets.len(), 15, "one target per site, plus the narrow test targets");
        assert!(config.proxy.enabled);
        assert_eq!(config.proxy.endpoints[0].port, 8889);
        assert_eq!(config.retry.max_attempts, 10);
        assert_eq!(config.proxy_pool.config, "../pia_proxy/config.toml");
    }

    #[test]
    fn rejects_invalid_retry_settings() {
        let mut config = Config::default();
        config.retry.max_attempts = 0;
        assert!(config.validate().is_err());

        let mut config = Config::default();
        config.retry.backoff_multiplier = 0.5;
        assert!(config.validate().is_err());

        let mut config = Config::default();
        config.retry.jitter_factor = 1.5;
        assert!(config.validate().is_err());
    }

    #[test]
    fn enabled_proxy_requires_an_endpoint() {
        let mut config = Config::default();
        config.proxy.enabled = true;
        assert!(config.validate().is_err());
    }

    #[test]
    fn enabled_pool_requires_a_config_path() {
        let mut config = Config::default();
        config.proxy_pool.enabled = true;
        config.validate().unwrap();

        config.proxy_pool.config.clear();
        assert!(config.validate().is_err());
    }
}

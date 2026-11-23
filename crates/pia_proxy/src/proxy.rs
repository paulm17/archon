//! The library facade: provision regions, then hand out connections.

use crate::config::Config;
use crate::error::{ProxyError, Result};
use crate::pia::PiaClient;
use crate::pool::Pool;
use crate::tunnel::Tunnel;
use crate::upstream::{Stream, Upstream};
use crate::Target;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

/// A running set of regional tunnels plus the round-robin pool over them.
pub struct Proxy {
    pool: Arc<Pool>,
    /// One pool per configured region, for pinned ports.
    pinned: HashMap<String, Arc<Pool>>,
    regions: Vec<String>,
}

impl Proxy {
    /// Provision every configured region and start its tunnel.
    ///
    /// An unknown region id is a hard error (it is a typo in the config);
    /// a region that fails to provision at runtime is logged and skipped so one
    /// dead server does not take the whole proxy down.
    pub async fn new(config: &Config) -> Result<Self> {
        let client = PiaClient::new(&config.pia.username, &config.pia.password)?;
        let connect_budget = Duration::from_secs(config.rotation.connect_timeout_secs);

        // Fail fast on bad region ids before touching the network for keys.
        let mut upstreams: Vec<Arc<Upstream>> = Vec::new();
        let mut pinned_slots: Vec<(String, usize)> = Vec::new();
        let mut regions = Vec::new();

        if !config.regions.is_empty() {
            let known = client.regions().await?;
            for region in &config.regions {
                // Typo in the config: report it with suggestions.
                PiaClient::find_region(&known, &region.id)?;
            }
            // Fail fast, with a clear message, when the account is rate limited
            // or the credentials are wrong, instead of region by region.
            client.token().await?;
        }

        for region in &config.regions {
            let tunnel_config = match client.provision(&region.id).await {
                Ok(config) => config,
                Err(error) => {
                    warn!(region = %region.id, error = %error, "cannot provision region, skipping");
                    continue;
                }
            };

            let label = region
                .name
                .clone()
                .unwrap_or_else(|| tunnel_config.region_name.clone());

            info!(
                region = %label,
                id = %region.id,
                endpoint = %tunnel_config.endpoint,
                exit_ip = %tunnel_config.peer_ip,
                "tunnel up"
            );

            let upstream = Arc::new(Upstream::wireguard(label.clone(), Tunnel::spawn(&region.id, tunnel_config)));
            if region.port.is_some() {
                // Remember where this region sits in the shared pool so its
                // pinned port can share the same health state.
                pinned_slots.push((region.id.clone(), upstreams.len()));
            }
            regions.push(label);
            upstreams.push(upstream);
        }

        if let Some(proxy) = &config.socks5 {
            info!(endpoint = %proxy.endpoint, "PIA SOCKS5 upstream configured");
            upstreams.push(Arc::new(Upstream::socks5(
                "PIA NL SOCKS5",
                proxy.endpoint.clone(),
                proxy.username.clone(),
                proxy.password.clone(),
            )));
            regions.push("PIA NL SOCKS5".to_string());
        }

        if upstreams.is_empty() {
            return Err(ProxyError::NoUpstream(
                "no region could be provisioned and no SOCKS5 upstream is configured".into(),
            ));
        }

        let pool = Arc::new(
            Pool::new(upstreams, config.rotation.max_attempts)?
                .with_retirement(
                    config.rotation.failures_before_retire,
                    Duration::from_secs(config.rotation.cooldown_secs),
                )
                .with_connect_budget(connect_budget),
        );

        // Pinned ports are one-region views onto the same pool, so a region's
        // health is identical through 8889 and through its own port.
        let mut pinned: HashMap<String, Arc<Pool>> = HashMap::new();
        for (id, index) in pinned_slots {
            pinned.insert(id, pool.pinned(index)?);
        }

        Ok(Self {
            pool,
            pinned,
            regions,
        })
    }

    /// Connect to `target` through the next upstream in rotation.
    pub async fn connect(&self, target: &Target) -> Result<Stream> {
        let (_name, stream) = self.pool.connect(target).await?;
        Ok(stream)
    }

    /// Connect and report which upstream carried it.
    pub async fn connect_labelled(&self, target: &Target) -> Result<(String, Stream)> {
        self.pool.connect(target).await
    }

    pub fn names(&self) -> Vec<String> {
        self.regions.clone()
    }

    pub fn pool(&self) -> Arc<Pool> {
        self.pool.clone()
    }

    /// Pool pinned to a single region id, for its dedicated port.
    pub fn pinned_pool(&self, region_id: &str) -> Option<Arc<Pool>> {
        self.pinned.get(region_id).cloned()
    }
}

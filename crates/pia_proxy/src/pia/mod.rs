//! Private Internet Access: account API plus the data needed to open a tunnel.

pub mod api;
pub mod certs;
pub mod keys;

use crate::error::{ProxyError, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use boringtun::x25519::PublicKey;
use std::net::{Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

pub use api::{AddKey, Endpoint, PiaRegion, Servers};
pub use keys::Keypair;

/// PIA's DNS resolvers inside the tunnel, used when `/addKey` omits them.
const FALLBACK_DNS: [Ipv4Addr; 2] = [Ipv4Addr::new(10, 0, 0, 242), Ipv4Addr::new(10, 0, 0, 243)];

/// A fully provisioned WireGuard session to one PIA region.
#[derive(Clone)]
pub struct TunnelConfig {
    pub region_id: String,
    pub region_name: String,
    pub keypair: Keypair,
    pub peer_public: PublicKey,
    /// UDP endpoint of the region's WireGuard server.
    pub endpoint: SocketAddr,
    /// Address PIA assigned to us inside the tunnel.
    pub peer_ip: Ipv4Addr,
    pub dns: Vec<Ipv4Addr>,
}

impl std::fmt::Debug for TunnelConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TunnelConfig")
            .field("region_id", &self.region_id)
            .field("endpoint", &self.endpoint)
            .field("peer_ip", &self.peer_ip)
            .field("dns", &self.dns)
            .finish_non_exhaustive()
    }
}

/// How long a fetched token and server list are reused for.
const CACHE_TTL: Duration = Duration::from_secs(20 * 60);

struct Cached<T> {
    value: T,
    fetched: Instant,
}

impl<T> Cached<T> {
    fn fresh(&self) -> bool {
        self.fetched.elapsed() < CACHE_TTL
    }
}

/// Thin wrapper over PIA's account API.
///
/// The token and the server list are fetched once and cached: provisioning many
/// regions used to ask for a token per region, which trips PIA's rate limiter.
pub struct PiaClient {
    http: reqwest::Client,
    username: String,
    password: String,
    token: Mutex<Option<Cached<String>>>,
    regions: Mutex<Option<Cached<Vec<PiaRegion>>>>,
}

impl PiaClient {
    pub fn new(username: impl Into<String>, password: impl Into<String>) -> Result<Self> {
        Ok(Self {
            http: api::http_client()?,
            username: username.into(),
            password: password.into(),
            token: Mutex::new(None),
            regions: Mutex::new(None),
        })
    }

    /// The account token, fetched at most once per `CACHE_TTL`.
    pub async fn token(&self) -> Result<String> {
        let mut cached = self.token.lock().await;
        if let Some(entry) = cached.as_ref() {
            if entry.fresh() {
                return Ok(entry.value.clone());
            }
        }
        let token = api::fetch_token(&self.http, &self.username, &self.password).await?;
        *cached = Some(Cached {
            value: token.clone(),
            fetched: Instant::now(),
        });
        Ok(token)
    }

    /// Forget the cached token, so the next call fetches a fresh one.
    pub async fn invalidate_token(&self) {
        *self.token.lock().await = None;
    }

    /// The region and server list, fetched at most once per `CACHE_TTL`.
    pub async fn regions(&self) -> Result<Vec<PiaRegion>> {
        let mut cached = self.regions.lock().await;
        if let Some(entry) = cached.as_ref() {
            if entry.fresh() {
                return Ok(entry.value.clone());
            }
        }
        let regions = api::fetch_regions(&self.http).await?;
        *cached = Some(Cached {
            value: regions.clone(),
            fetched: Instant::now(),
        });
        Ok(regions)
    }

    /// Look up a region id, listing near matches when the id is wrong.
    pub fn find_region<'a>(regions: &'a [PiaRegion], id: &str) -> Result<&'a PiaRegion> {
        if let Some(found) = regions.iter().find(|r| r.id == id) {
            return Ok(found);
        }

        let needle = id.to_lowercase().replace('-', "_");
        let mut suggestions: Vec<&str> = regions
            .iter()
            .filter(|r| {
                let candidate = r.id.to_lowercase().replace('-', "_");
                candidate.contains(&needle) || needle.contains(&candidate)
            })
            .map(|r| r.id.as_str())
            .take(6)
            .collect();

        if suggestions.is_empty() {
            let prefix = needle.split('_').next().unwrap_or("");
            suggestions = regions
                .iter()
                .filter(|r| r.id.to_lowercase().starts_with(prefix))
                .map(|r| r.id.as_str())
                .take(6)
                .collect();
        }

        let hint = if suggestions.is_empty() {
            "no similar ids found".to_string()
        } else {
            format!("did you mean: {}", suggestions.join(", "))
        };
        Err(ProxyError::Pia(format!("unknown PIA region '{id}' ({hint})")))
    }

    /// Fetch a token, register a fresh keypair and return a ready tunnel config.
    pub async fn provision(&self, region_id: &str) -> Result<TunnelConfig> {
        let regions = self.regions().await?;
        let region = Self::find_region(&regions, region_id)?;

        if region.offline {
            return Err(ProxyError::Pia(format!("region '{region_id}' is offline")));
        }

        let server = region
            .servers
            .wg
            .first()
            .ok_or_else(|| {
                ProxyError::Pia(format!(
                    "region '{region_id}' has no WireGuard servers ({} OpenVPN servers available)",
                    region.servers.ovpnudp.len()
                ))
            })?
            .clone();

        let token = self.token().await?;
        let keypair = Keypair::generate();
        let public = keypair.public_base64();
        let registration = match api::add_key(&server.cn, &server.ip, &token, &public).await {
            Ok(registration) => registration,
            // A cached token can expire between regions; refresh once and retry.
            Err(error) if error.to_string().contains("HTTP 401") => {
                self.invalidate_token().await;
                let token = self.token().await?;
                api::add_key(&server.cn, &server.ip, &token, &public).await?
            }
            Err(error) => return Err(error),
        };

        let peer_bytes = STANDARD
            .decode(registration.server_key.trim())
            .map_err(|e| ProxyError::Pia(format!("server_key is not base64: {e}")))?;
        let peer_bytes: [u8; 32] = peer_bytes
            .try_into()
            .map_err(|_| ProxyError::Pia("server_key is not 32 bytes".into()))?;

        let peer_ip: Ipv4Addr = registration
            .peer_ip
            .parse()
            .map_err(|_| ProxyError::Pia(format!("bad peer_ip: {}", registration.peer_ip)))?;

        let server_ip: Ipv4Addr = registration
            .server_ip
            .parse()
            .map_err(|_| ProxyError::Pia(format!("bad server_ip: {}", registration.server_ip)))?;

        let mut dns: Vec<Ipv4Addr> = registration
            .dns_servers
            .iter()
            .filter_map(|s| s.parse::<Ipv4Addr>().ok())
            .collect();
        if dns.is_empty() {
            dns = FALLBACK_DNS.to_vec();
        }

        Ok(TunnelConfig {
            region_id: region.id.clone(),
            region_name: region.name.clone(),
            keypair,
            peer_public: PublicKey::from(peer_bytes),
            endpoint: SocketAddr::from((server_ip, registration.server_port)),
            peer_ip,
            dns,
        })
    }
}

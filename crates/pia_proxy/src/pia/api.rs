use crate::error::{ProxyError, Result};
use crate::pia::certs;
use serde::Deserialize;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

const TOKEN_URL: &str = "https://privateinternetaccess.com/api/client/v2/token";
const SERVER_LIST_URL: &str = "https://serverlist.piaservers.net/vpninfo/servers/v6";
const ADD_KEY_PORT: u16 = 1337;

/// How many times a 429 is retried before giving up.
const RETRY_LIMIT: u32 = 4;
const RETRY_BASE: Duration = Duration::from_millis(300);

/// Send a request, retrying with exponential backoff while PIA answers 429.
async fn send_retrying<F>(mut build: F) -> Result<reqwest::Response>
where
    F: FnMut() -> reqwest::RequestBuilder,
{
    let mut attempt = 0u32;
    loop {
        let response = build().send().await?;
        if response.status().as_u16() != 429 || attempt >= RETRY_LIMIT {
            return Ok(response);
        }
        let wait = retry_after(&response).unwrap_or_else(|| RETRY_BASE * 2u32.pow(attempt));
        tracing::warn!(
            attempt = attempt + 1,
            wait_ms = wait.as_millis() as u64,
            "PIA rate limited us (HTTP 429); backing off"
        );
        tokio::time::sleep(wait).await;
        attempt += 1;
    }
}

/// Honour a `Retry-After: <seconds>` header when PIA sends one.
fn retry_after(response: &reqwest::Response) -> Option<Duration> {
    response
        .headers()
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    token: String,
}

/// One PIA server: its IP and the hostname we must present for TLS.
#[derive(Debug, Clone, Deserialize)]
pub struct Endpoint {
    pub ip: String,
    pub cn: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Servers {
    #[serde(default)]
    pub wg: Vec<Endpoint>,
    #[serde(default)]
    pub ovpnudp: Vec<Endpoint>,
    #[serde(default)]
    pub ovpntcp: Vec<Endpoint>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PiaRegion {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub country: String,
    #[serde(default)]
    pub offline: bool,
    #[serde(default)]
    pub port_forward: bool,
    #[serde(default)]
    pub servers: Servers,
}

#[derive(Debug, Deserialize)]
struct ServerListEnvelope {
    regions: Vec<PiaRegion>,
}

/// Response from a regional server's `/addKey` registration call.
#[derive(Debug, Clone, Deserialize)]
pub struct AddKey {
    pub status: String,
    pub server_key: String,
    pub server_port: u16,
    pub server_ip: String,
    pub peer_ip: String,
    #[serde(default)]
    pub dns_servers: Vec<String>,
}

/// Shared HTTP client for PIA's public API endpoints.
pub fn http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .use_rustls_tls()
        .timeout(std::time::Duration::from_secs(25))
        .user_agent("proxy-multiplexer/0.2")
        .build()
        .map_err(ProxyError::Http)
}

/// Exchange account credentials for a short lived PIA token.
pub async fn fetch_token(client: &reqwest::Client, username: &str, password: &str) -> Result<String> {
    let response = send_retrying(|| {
        client
            .post(TOKEN_URL)
            .form(&[("username", username), ("password", password)])
    })
    .await?;

    let status = response.status();
    if !status.is_success() {
        let hint = match status.as_u16() {
            401 | 403 => "check PIA_USER / PIA_PASS",
            429 => "this account is rate limited, wait a minute and retry",
            _ => "try again later",
        };
        return Err(ProxyError::Pia(format!(
            "token endpoint returned HTTP {} ({hint})",
            status.as_u16()
        )));
    }

    let body: TokenResponse = response.json().await?;
    if body.token.is_empty() {
        return Err(ProxyError::Pia("token endpoint returned an empty token".into()));
    }
    Ok(body.token)
}

/// Fetch and parse the region/server list.
///
/// The response is two JSON documents separated by a newline: the payload and a
/// detached signature over it. Only the first line is the payload.
pub async fn fetch_regions(client: &reqwest::Client) -> Result<Vec<PiaRegion>> {
    let body = send_retrying(|| client.get(SERVER_LIST_URL))
        .await?
        .text()
        .await?;
    let payload = body
        .lines()
        .next()
        .unwrap_or("")
        .trim_end_matches('\r')
        .trim();

    if payload.is_empty() {
        return Err(ProxyError::Pia("server list response was empty".into()));
    }

    let envelope: ServerListEnvelope = serde_json::from_str(payload)
        .map_err(|e| ProxyError::Pia(format!("cannot parse server list: {e}")))?;
    if envelope.regions.is_empty() {
        return Err(ProxyError::Pia("server list contained no regions".into()));
    }
    Ok(envelope.regions)
}

/// Register our WireGuard public key with one PIA server.
///
/// The request goes to `https://<cn>:1337/addKey` but connects to the server IP
/// chosen from the list, and validates TLS against PIA's own CA.
pub async fn add_key(cn: &str, ip: &str, token: &str, public_key: &str) -> Result<AddKey> {
    let ip: IpAddr = ip
        .parse()
        .map_err(|_| ProxyError::Pia(format!("server '{cn}' has an invalid IP: {ip}")))?;

    let ca = reqwest::Certificate::from_pem(certs::PIA_CA_PEM.as_bytes())
        .map_err(|e| ProxyError::Pia(format!("cannot load embedded PIA CA: {e}")))?;

    let client = reqwest::Client::builder()
        .use_rustls_tls()
        .timeout(std::time::Duration::from_secs(25))
        .user_agent("proxy-multiplexer/0.2")
        .add_root_certificate(ca)
        .resolve(cn, SocketAddr::new(ip, ADD_KEY_PORT))
        .build()
        .map_err(ProxyError::Http)?;

    let url = format!("https://{cn}:{ADD_KEY_PORT}/addKey");
    let response = send_retrying(|| {
        client
            .get(&url)
            .query(&[("pt", token), ("pubkey", public_key)])
    })
    .await?;

    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        return Err(ProxyError::Pia(format!(
            "addKey on {cn} returned HTTP {}: {}",
            status.as_u16(),
            body.trim().chars().take(200).collect::<String>()
        )));
    }

    let parsed: AddKey = serde_json::from_str(body.trim()).map_err(|e| {
        ProxyError::Pia(format!(
            "cannot parse addKey response from {cn}: {e} (body: {})",
            body.trim().chars().take(200).collect::<String>()
        ))
    })?;

    if !parsed.status.eq_ignore_ascii_case("ok") {
        return Err(ProxyError::Pia(format!(
            "addKey on {cn} rejected the key: status={}",
            parsed.status
        )));
    }
    Ok(parsed)
}

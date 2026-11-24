//! PIA proxy integration.
//!
//! `crates/pia_proxy` is a library (`pia_proxy`): it opens a userspace
//! WireGuard tunnel per configured PIA region and serves one round-robin SOCKS5
//! + HTTP CONNECT listener on `127.0.0.1:8889`. There are no upstream ports to
//! babysit — the tunnels are created in-process when the listener starts, which
//! is why 8889 is the only endpoint this app ever knows about.
//!
//! Regions are configured in the proxy crate. Credentials come from PIA_USER and PIA_PASS.

use anyhow::{Context, Result, bail};
use std::path::Path;
use std::time::Duration;

/// Provisioning costs roughly two seconds per region, so a many-region config
/// needs a generous window before the listener is up.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(180);
const POLL: Duration = Duration::from_millis(250);

/// Drive the owned listener and the scrape in the same task so listener errors
/// reach the caller and the listener is dropped when the scrape finishes.
pub async fn with_proxy<T>(
    pool: &crate::config::ProxyPool,
    scrape: impl std::future::Future<Output = Result<T>>,
) -> Result<T> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let config = load_config(root, pool).await?;
    let host = config.listen.host.clone();
    let port = config.listen.port;
    if is_listening(&host, port).await {
        return scrape.await;
    }
    let work = async {
        tokio::time::timeout(STARTUP_TIMEOUT, async {
            while !is_listening(&host, port).await {
                tokio::time::sleep(POLL).await;
            }
        })
        .await
        .context("timed out starting PIA proxy")?;
        scrape.await
    };
    tokio::select! {
        result = pia_proxy::serve(config) => {
            result.context("PIA proxy stopped")?;
            bail!("PIA proxy stopped before the scrape completed")
        }
        result = work => result,
    }
}

/// True when something already accepts TCP connections on `host:port`.
pub async fn is_listening(host: &str, port: u16) -> bool {
    tokio::net::TcpStream::connect((host, port)).await.is_ok()
}

async fn load_config(root: &Path, pool: &crate::config::ProxyPool) -> Result<pia_proxy::Config> {
    let path = crate::resolve_path(root, &pool.config);
    let mut config = pia_proxy::Config::load(&path).await.with_context(|| {
        format!(
            "loading the PIA proxy config at {}",
            path.display()
        )
    })?;
    if let Some(limit) = pool.regions {
        config.regions.truncate(limit.max(1));
    }
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_closed_port_is_not_listening() {
        // Bind and drop to obtain a port nothing is serving.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        assert!(!is_listening("127.0.0.1", port).await);
    }

    #[tokio::test]
    async fn an_open_port_is_listening() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(is_listening("127.0.0.1", port).await);
    }

    #[test]
    fn a_missing_config_reports_its_path() {
        let root = Path::new("/nonexistent-project");
        let pool = crate::config::ProxyPool {
            enabled: true,
            config: "no-such-config.toml".into(),
            regions: None,
        };
        let error = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(load_config(root, &pool))
            .unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("no-such-config.toml"), "{message}");
    }
}

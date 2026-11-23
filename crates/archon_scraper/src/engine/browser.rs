//! Chromium lifecycle: proxy verification, launch configuration, session.
//!
//! Nothing here is exercised by the current milestone — `launch` is the entry
//! point the scrape workflow will call once site modules exist.

use anyhow::{Context, Result, bail};
use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::Page;
use futures::StreamExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::config::{Browser as BrowserSettings, Proxy};
use crate::resolve_path;

pub struct Session {
    pub browser: Browser,
    pub page: Page,
    handler: tokio::task::JoinHandle<()>,
}

/// How long Chromium gets to answer a polite close before it is killed. A
/// page that never finishes loading can leave the CDP close unanswered, and a
/// run must never hang on teardown.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

impl Session {
    /// Whether the Chromium process behind this session is still running.
    ///
    /// The handler task ends when the browser's event stream ends, which is how
    /// a crashed or externally killed browser shows up here.
    pub fn is_alive(&self) -> bool {
        !self.handler.is_finished()
    }

    pub async fn close(mut self) -> Result<()> {
        let graceful = tokio::time::timeout(CLOSE_TIMEOUT, self.browser.close()).await;
        let outcome = match graceful {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(error)) => Err(error).context("Failed to close Chromium"),
            Err(_) => {
                eprintln!("chromium did not answer the close request; killing it");
                Ok(())
            }
        };
        // A browser that accepts the close can still stay alive — a page that
        // never finished loading is enough — and `Browser`'s own drop then
        // waits on it forever. Killing the process first is what guarantees the
        // run returns.
        self.kill_if_running();
        self.handler.abort();
        outcome
    }

    /// SIGKILL the Chromium process if it is still running.
    fn kill_if_running(&mut self) {
        if let Some(child) = self.browser.get_mut_child() {
            if matches!(child.try_wait(), Ok(None)) {
                let _ = child.as_mut_inner().kill();
            }
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.handler.abort();
        // Every exit path closes the session, but a panic or an early `?` must
        // not leave a browser behind.
        self.kill_if_running();
    }
}

/// The `--proxy-server` argument, if a proxy is configured.
pub fn proxy_arg(proxy: &Proxy) -> Result<Option<String>> {
    if !proxy.enabled {
        return Ok(None);
    }
    let endpoint = proxy
        .endpoints
        .first()
        .context("proxy.enabled requires at least one endpoint")?;
    Ok(Some(format!(
        "--proxy-server={}://{}:{}",
        proxy.proxy_type, endpoint.host, endpoint.port
    )))
}

/// Fail fast when a required proxy is not listening.
pub async fn verify_proxy(proxy: &Proxy) -> Result<()> {
    if !proxy.enabled {
        return Ok(());
    }
    let endpoint = proxy
        .endpoints
        .first()
        .context("proxy.enabled requires at least one endpoint")?;
    let address = format!("{}:{}", endpoint.host, endpoint.port);
    tokio::time::timeout(
        Duration::from_secs(3),
        tokio::net::TcpStream::connect((endpoint.host.as_str(), endpoint.port)),
    )
    .await
    .with_context(|| format!("Required proxy at {address} did not respond within 3 seconds"))?
    .with_context(|| format!("Cannot connect to required proxy at {address}"))?;
    Ok(())
}

/// Hosts contacted by Chromium itself or by the page's own analytics, none of
/// which this scraper can use. Resolving them to nothing keeps those tunnels,
/// and the strikes they cost a region, from leaving the machine at all.
const SILENCED_HOSTS: &[&str] = &[
    "static.cloudflareinsights.com",
    "www.googletagmanager.com",
    "www.google-analytics.com",
    "www.google.com",
    "accounts.google.com",
    "update.googleapis.com",
    "optimizationguide-pa.googleapis.com",
    "content-autofill.googleapis.com",
    "safebrowsing.googleapis.com",
    "clients2.google.com",
    "clients4.google.com",
];

fn host_resolver_rules() -> String {
    format!(
        "--host-resolver-rules={}",
        SILENCED_HOSTS
            .iter()
            .map(|host| format!("MAP {host} ~NOTFOUND"))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

pub fn build_config(
    settings: &BrowserSettings,
    proxy: &Proxy,
    root: &Path,
    user_agent_index: usize,
) -> Result<BrowserConfig> {
    let mut builder =
        BrowserConfig::builder().window_size(settings.window_width, settings.window_height);
    if !settings.headless {
        builder = builder.with_head();
    }
    if !settings.user_data_dir.is_empty() {
        let dir: PathBuf = resolve_path(root, &settings.user_data_dir);
        builder = builder.user_data_dir(dir);
    }
    if let Some(arg) = proxy_arg(proxy)? {
        builder = builder.arg(arg);
    }
    // Chromium's own traffic (component updates, account checks, telemetry,
    // Google endpoints) also goes through the proxy. Through a tunnel those
    // requests time out, and every one of them counted against a region: a live
    // run had background requests retire all 17 regions before the first
    // chapter finished loading. A scraping browser needs none of it.
    for flag in [
        "--disable-background-networking",
        "--disable-component-update",
        "--disable-domain-reliability",
        "--no-pings",
        "--no-default-browser-check",
    ] {
        builder = builder.arg(flag);
    }
    // `--disable-background-networking` covers Chromium's own updaters but not
    // the beacons a page ships. Note this rule only bites when Chromium resolves
    // names itself; through a proxy it sends CONNECT and the proxy resolves, so
    // `pia_proxy::noise` refuses the same hosts there as well.
    builder = builder.arg(host_resolver_rules());
    if !settings.user_agents.is_empty() {
        let agent = &settings.user_agents[user_agent_index % settings.user_agents.len()];
        builder = builder.arg(format!("--user-agent={agent}"));
    }
    if !settings.extensions_dir.is_empty() {
        let extension = resolve_path(root, &settings.extensions_dir).join("ublock_origin_lite");
        if !extension.is_dir() {
            bail!("Browser extension not found: {}", extension.display());
        }
        builder = builder.extension(extension.display().to_string());
    }
    builder
        .build()
        .map_err(|error| anyhow::anyhow!("Failed to build Chromium configuration: {error}"))
}

/// Chromium inherits our standard output, and a renderer helper can outlive
/// the browser just long enough to keep a caller's pipe open — so a run piped
/// into `tee` or `tail` appears to hang after the work is done. Standard output
/// points at `/dev/null` for the spawn only; the descriptors Chromium inherits
/// are the ones it keeps.
struct QuietStdout {
    saved: Option<libc::c_int>,
}

impl QuietStdout {
    fn new() -> Self {
        unsafe {
            let saved = libc::dup(libc::STDOUT_FILENO);
            if saved < 0 {
                return Self { saved: None };
            }
            let null = libc::open(c"/dev/null".as_ptr(), libc::O_WRONLY);
            if null >= 0 {
                libc::dup2(null, libc::STDOUT_FILENO);
                libc::close(null);
            }
            Self { saved: Some(saved) }
        }
    }
}

impl Drop for QuietStdout {
    fn drop(&mut self) {
        if let Some(saved) = self.saved.take() {
            unsafe {
                libc::dup2(saved, libc::STDOUT_FILENO);
                libc::close(saved);
            }
        }
    }
}

/// Chromium guards its profile with `SingletonLock`, a symlink to
/// `hostname-pid`. A browser that was killed — or crashed — leaves it behind,
/// and every later launch then aborts with "Failed to create SingletonLock"
/// before it can report its DevTools address. Clear it when its owner is gone.
pub fn clear_stale_profile_lock(dir: &Path) {
    let lock = dir.join("SingletonLock");
    let Ok(target) = std::fs::read_link(&lock) else {
        return;
    };
    let owner = target
        .to_string_lossy()
        .rsplit_once('-')
        .and_then(|(_host, pid)| pid.parse::<i32>().ok());
    if let Some(pid) = owner {
        if pid <= 0 || unsafe { libc::kill(pid, 0) } == 0
            || std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH) {
            return;
        }
    }
    for name in ["SingletonLock", "SingletonSocket", "SingletonCookie"] {
        let _ = std::fs::remove_file(dir.join(name));
    }
}

/// Launch Chromium and open a blank page.
pub async fn launch(
    settings: &BrowserSettings,
    proxy: &Proxy,
    root: &Path,
    user_agent_index: usize,
) -> Result<Session> {
    verify_proxy(proxy).await?;
    if !settings.user_data_dir.is_empty() {
        let dir = resolve_path(root, &settings.user_data_dir);
        if dir.is_dir() {
            clear_stale_profile_lock(&dir);
        }
    }
    let config = build_config(settings, proxy, root, user_agent_index)?;
    let (browser, mut handler) = {
        let _quiet = QuietStdout::new();
        Browser::launch(config)
            .await
            .context("Failed to launch Chromium")?
    };
    let handler = tokio::spawn(async move { while handler.next().await.is_some() {} });
    let page = browser
        .new_page("about:blank")
        .await
        .context("Failed to open a Chromium page")?;
    Ok(Session {
        browser,
        page,
        handler,
    })
}

/// Advance to the next user agent in the list, if there is one.
pub async fn rotate_user_agent(
    page: &Page,
    agents: &[String],
    index: &mut usize,
) -> Result<()> {
    if agents.is_empty() {
        return Ok(());
    }
    *index = (*index + 1) % agents.len();
    page.set_user_agent(&agents[*index])
        .await
        .context("Failed to rotate browser user agent")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProxyEndpoint;

    fn proxy(enabled: bool, endpoints: Vec<ProxyEndpoint>) -> Proxy {
        Proxy {
            enabled,
            proxy_type: "http".into(),
            endpoints,
        }
    }

    #[test]
    fn builds_the_proxy_argument() {
        let proxy = proxy(
            true,
            vec![ProxyEndpoint {
                host: "localhost".into(),
                port: 8889,
                region: "Proxy Multiplier".into(),
            }],
        );
        assert_eq!(
            proxy_arg(&proxy).unwrap().as_deref(),
            Some("--proxy-server=http://localhost:8889")
        );
    }

    #[test]
    fn no_argument_when_the_proxy_is_disabled() {
        assert_eq!(proxy_arg(&proxy(false, Vec::new())).unwrap(), None);
    }

    #[test]
    fn enabled_proxy_without_endpoint_is_an_error() {
        assert!(proxy_arg(&proxy(true, Vec::new())).is_err());
    }

    #[test]
    fn clears_a_lock_left_by_a_dead_browser() {
        let dir = std::env::temp_dir().join(format!("scraper-lock-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // PID 0 is never a live process we could have spawned.
        std::os::unix::fs::symlink("somehost-0", dir.join("SingletonLock")).unwrap();

        clear_stale_profile_lock(&dir);

        assert!(!dir.join("SingletonLock").exists(), "stale lock removed");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn clears_a_lock_whose_owner_is_not_a_browser() {
        // A recycled pid must never be killed, and its lock is not ours to keep.
        let dir = std::env::temp_dir().join(format!("scraper-live-lock-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = format!("somehost-{}", std::process::id());
        std::os::unix::fs::symlink(&target, dir.join("SingletonLock")).unwrap();

        clear_stale_profile_lock(&dir);

        assert!(!dir.join("SingletonLock").exists(), "non-browser lock removed");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn page_and_browser_telemetry_hosts_resolve_to_nothing() {
        let rule = host_resolver_rules();
        assert!(rule.starts_with("--host-resolver-rules="));
        for host in [
            "static.cloudflareinsights.com",
            "www.google.com",
            "optimizationguide-pa.googleapis.com",
        ] {
            assert!(
                rule.contains(&format!("MAP {host} ~NOTFOUND")),
                "{host} should be silenced: {rule}"
            );
        }
    }

    #[test]
    fn launch_config_builds_without_a_proxy() {
        let settings = BrowserSettings {
            headless: true,
            window_width: 1280,
            window_height: 720,
            user_data_dir: String::new(),
            extensions_dir: String::new(),
            user_agents: vec!["agent-a".into(), "agent-b".into()],
        };
        // BrowserConfig keeps its fields private, so this asserts that the
        // settings translate into a valid launch configuration at all.
        build_config(&settings, &proxy(false, Vec::new()), Path::new("/tmp"), 1)
            .expect("config");
    }
}

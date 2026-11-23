//! Library-first proxy for Private Internet Access.
//!
//! Each configured region gets a userspace WireGuard tunnel (boringtun) with its
//! own userspace TCP/IP stack (smoltcp), so per-region egress works with no TUN
//! device, no root and no routing changes. A round-robin pool hands client
//! connections to tunnels, and one listener accepts SOCKS5 and HTTP CONNECT.
//!
//! ```no_run
//! # async fn demo() -> pia_proxy::Result<()> {
//! let config = pia_proxy::Config::load("config.toml").await?;
//! pia_proxy::serve(config).await
//! # }
//! ```
//!
//! Using it as a library, without the binary:
//!
//! ```no_run
//! # async fn demo() -> pia_proxy::Result<()> {
//! use pia_proxy::{Config, Proxy, Target};
//! let config = Config::load("config.toml").await?;
//! let proxy = Proxy::new(&config).await?;
//! let (_region, mut stream) = proxy.connect_labelled(&Target::new("example.com", 443)).await?;
//! // stream: impl AsyncRead + AsyncWrite
//! # Ok(())
//! # }
//! ```

pub mod config;
pub mod error;
pub mod noise;
pub mod pia;
pub mod pool;
pub mod proxy;
pub mod server;
pub mod socks5;
pub mod tunnel;
pub mod upstream;

pub use config::{Config, Region};
pub use error::{ProxyError, Result};
pub use pia::{PiaClient, TunnelConfig};
pub use pool::Pool;
pub use proxy::Proxy;
pub use server::serve;
pub use tunnel::Conn;
pub use upstream::{Stream, Upstream};

/// A destination a client asked for.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Target {
    pub host: String,
    pub port: u16,
}

impl Target {
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            port,
        }
    }
}

impl std::fmt::Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.host, self.port)
    }
}

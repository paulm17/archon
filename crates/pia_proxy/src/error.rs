use thiserror::Error;

/// Everything the library can fail with.
#[derive(Debug, Error)]
pub enum ProxyError {
    #[error("config: {0}")]
    Config(String),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("http: {0}")]
    Http(#[from] reqwest::Error),

    #[error("pia: {0}")]
    Pia(String),

    #[error("tunnel: {0}")]
    Tunnel(String),

    #[error("socks5: {0}")]
    Socks5(String),

    #[error("protocol: {0}")]
    Protocol(String),

    #[error("no usable upstream: {0}")]
    NoUpstream(String),

    /// A request the proxy declines to send anywhere: browser telemetry and
    /// advertising hosts, which no region should ever be judged on.
    #[error("blocked: {0}")]
    Blocked(String),
}

pub type Result<T> = std::result::Result<T, ProxyError>;

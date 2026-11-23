//! Upstreams: a regional WireGuard tunnel, or PIA's SOCKS5 proxy.

use crate::error::Result;
use crate::socks5;
use crate::tunnel::{Conn, Tunnel};
use crate::Target;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;

/// A connection to a target, whichever upstream produced it.
pub enum Stream {
    Tunnel(Conn),
    Tcp(TcpStream),
}

impl std::fmt::Debug for Stream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Stream::Tunnel(_) => f.write_str("Stream::Tunnel"),
            Stream::Tcp(_) => f.write_str("Stream::Tcp"),
        }
    }
}

impl AsyncRead for Stream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Stream::Tunnel(conn) => Pin::new(conn).poll_read(cx, buf),
            Stream::Tcp(stream) => Pin::new(stream).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for Stream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Stream::Tunnel(conn) => Pin::new(conn).poll_write(cx, data),
            Stream::Tcp(stream) => Pin::new(stream).poll_write(cx, data),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Stream::Tunnel(conn) => Pin::new(conn).poll_flush(cx),
            Stream::Tcp(stream) => Pin::new(stream).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Stream::Tunnel(conn) => Pin::new(conn).poll_shutdown(cx),
            Stream::Tcp(stream) => Pin::new(stream).poll_shutdown(cx),
        }
    }
}

/// Something traffic can be sent through.
pub enum Upstream {
    WireGuard {
        name: String,
        tunnel: Tunnel,
    },
    Socks5 {
        name: String,
        endpoint: String,
        credentials: Option<(String, String)>,
    },
}

impl Upstream {
    pub fn wireguard(name: impl Into<String>, tunnel: Tunnel) -> Self {
        Self::WireGuard {
            name: name.into(),
            tunnel,
        }
    }

    pub fn socks5(
        name: impl Into<String>,
        endpoint: impl Into<String>,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        Self::Socks5 {
            name: name.into(),
            endpoint: endpoint.into(),
            credentials: Some((username.into(), password.into())),
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Upstream::WireGuard { name, .. } => name,
            Upstream::Socks5 { name, .. } => name,
        }
    }

    /// Open a connection to `target` through this upstream.
    pub async fn connect(&self, target: &Target) -> Result<Stream> {
        match self {
            Upstream::WireGuard { tunnel, .. } => {
                Ok(Stream::Tunnel(tunnel.connect(target).await?))
            }
            Upstream::Socks5 {
                endpoint,
                credentials,
                ..
            } => {
                let borrowed = credentials
                    .as_ref()
                    .map(|(user, pass)| (user.as_str(), pass.as_str()));
                let stream = socks5::connect(endpoint, borrowed, target).await?;
                Ok(Stream::Tcp(stream))
            }
        }
    }
}

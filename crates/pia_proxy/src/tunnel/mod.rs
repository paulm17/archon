//! A userspace WireGuard tunnel per PIA region, exposed as a byte stream.

pub mod device;
pub mod dns;
pub mod stack;
pub mod wireguard;

use crate::error::{ProxyError, Result};
use crate::pia::TunnelConfig;
use crate::Target;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::PollSender;

/// Control messages for a running tunnel.
pub enum Command {
    Connect {
        target: Target,
        reply: oneshot::Sender<Result<Conn>>,
    },
}

/// A bidirectional byte stream carried over a regional WireGuard tunnel.
///
/// Implements `AsyncRead`/`AsyncWrite`, so it splices into any TCP client with
/// `copy_bidirectional`.
pub struct Conn {
    incoming: mpsc::Receiver<Vec<u8>>,
    outgoing: PollSender<Vec<u8>>,
    current: Option<Vec<u8>>,
}

impl Conn {
    pub(crate) fn new(incoming: mpsc::Receiver<Vec<u8>>, outgoing: mpsc::Sender<Vec<u8>>) -> Self {
        Self {
            incoming,
            outgoing: PollSender::new(outgoing),
            current: None,
        }
    }
}

impl std::fmt::Debug for Conn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Conn").finish_non_exhaustive()
    }
}

impl AsyncRead for Conn {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if buf.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }

        if let Some(chunk) = self.current.take() {
            if !chunk.is_empty() {
                let take = chunk.len().min(buf.remaining());
                buf.put_slice(&chunk[..take]);
                if take < chunk.len() {
                    self.current = Some(chunk[take..].to_vec());
                }
                return Poll::Ready(Ok(()));
            }
        }

        match self.incoming.poll_recv(cx) {
            Poll::Ready(Some(chunk)) => {
                let take = chunk.len().min(buf.remaining());
                buf.put_slice(&chunk[..take]);
                if take < chunk.len() {
                    self.current = Some(chunk[take..].to_vec());
                }
                Poll::Ready(Ok(()))
            }
            // Peer closed: a zero-length read is EOF.
            Poll::Ready(None) => Poll::Ready(Ok(())),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl AsyncWrite for Conn {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        if data.is_empty() {
            return Poll::Ready(Ok(0));
        }
        match self.outgoing.poll_reserve(cx) {
            Poll::Ready(Ok(())) => {
                let len = data.len();
                let owned = data.to_vec();
                match self.outgoing.send_item(owned) {
                    Ok(()) => Poll::Ready(Ok(len)),
                    Err(_) => Poll::Ready(Err(io::Error::from(io::ErrorKind::BrokenPipe))),
                }
            }
            Poll::Ready(Err(_)) => Poll::Ready(Err(io::Error::from(io::ErrorKind::BrokenPipe))),
            Poll::Pending => Poll::Pending,
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.outgoing.close();
        Poll::Ready(Ok(()))
    }
}

/// Handle to a running regional tunnel.
#[derive(Clone)]
pub struct Tunnel {
    region: String,
    control: mpsc::Sender<Command>,
}

impl Tunnel {
    /// Start a region's WireGuard and TCP/IP stack tasks.
    pub fn spawn(region_id: &str, config: TunnelConfig) -> Self {
        let (control, commands) = mpsc::channel(128);
        let io = wireguard::WireGuardIo::new();

        let region = region_id.to_string();
        let label = region.clone();
        let wireguard_io = io.clone();
        let wireguard_config = config.clone();
        tokio::spawn(async move {
            if let Err(error) = wireguard::run(wireguard_config, wireguard_io).await {
                tracing::error!(region = %label, error = %error, "WireGuard task stopped");
            }
        });

        let stack_config = config.clone();
        tokio::spawn(async move {
            stack::run(stack_config, io, commands).await;
        });

        Self { region, control }
    }

    /// Convenience accessor used in logs.
    pub fn region(&self) -> &str {
        &self.region
    }

    /// Open a connection to `target` through this tunnel.
    pub async fn connect(&self, target: &Target) -> Result<Conn> {
        let (reply, response) = oneshot::channel();
        self.control
            .send(Command::Connect {
                target: target.clone(),
                reply,
            })
            .await
            .map_err(|_| ProxyError::Tunnel(format!("tunnel for region {} is not running", self.region)))?;

        response
            .await
            .map_err(|_| ProxyError::Tunnel(format!("tunnel for region {} dropped the request", self.region)))?
    }
}

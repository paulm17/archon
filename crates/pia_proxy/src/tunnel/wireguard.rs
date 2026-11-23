//! WireGuard I/O: owns the UDP socket and the boringtun session.
//!
//! Runs as its own task so the TCP/IP stack task never has to borrow the socket
//! or await on the network while polling the interface.

use crate::error::Result;
use crate::pia::TunnelConfig;
use crate::tunnel::device::PacketQueue;
use boringtun::noise::{Tunn, TunnResult};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::sync::Notify;
use tracing::{debug, trace, warn};

const MAX_PACKET: usize = 65_535;
const KEEPALIVE_SECS: u16 = 25;

/// Shared queues between the WireGuard task and the TCP/IP stack task.
#[derive(Clone)]
pub struct WireGuardIo {
    pub inbound: PacketQueue,
    pub outbound: PacketQueue,
    pub wake: Arc<Notify>,
}

impl WireGuardIo {
    pub fn new() -> Self {
        Self {
            inbound: PacketQueue::default(),
            outbound: PacketQueue::default(),
            wake: Arc::new(Notify::new()),
        }
    }
}

impl Default for WireGuardIo {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use boringtun::x25519::{PublicKey, StaticSecret};
    use rand::rngs::OsRng;

    /// Two boringtun peers, no network: proves we drive the API correctly.
    #[test]
    fn handshake_then_carry_an_ip_packet() {
        let our_private = StaticSecret::random_from_rng(OsRng);
        let our_public = PublicKey::from(&our_private);
        let peer_private = StaticSecret::random_from_rng(OsRng);
        let peer_public = PublicKey::from(&peer_private);

        let mut ours = Tunn::new(our_private, peer_public, None, None, 0, None);
        let mut theirs = Tunn::new(peer_private, our_public, None, None, 1, None);

        let mut buffer = [0u8; 2048];

        let initiation = match ours.format_handshake_initiation(&mut buffer, false) {
            TunnResult::WriteToNetwork(packet) => packet.to_vec(),
            _ => panic!("expected a handshake initiation"),
        };

        let mut second = [0u8; 2048];
        let response = match theirs.decapsulate(None, &initiation, &mut second) {
            TunnResult::WriteToNetwork(packet) => packet.to_vec(),
            _ => panic!("peer did not answer the handshake"),
        };

        let mut third = [0u8; 2048];
        let confirmation = match ours.decapsulate(None, &response, &mut third) {
            TunnResult::WriteToNetwork(packet) => packet.to_vec(),
            _ => panic!("we did not confirm the handshake"),
        };
        let mut fourth = [0u8; 2048];
        let _ = theirs.decapsulate(None, &confirmation, &mut fourth);

        // A well-formed IPv4 header: boringtun validates the version nibble and
        // truncates to the total-length field, so both must be correct.
        let mut ip_packet = vec![0u8; 20];
        ip_packet[0] = 0x45; // version 4, header length 5 words
        ip_packet[2..4].copy_from_slice(&20u16.to_be_bytes()); // total length
        ip_packet[8] = 64; // TTL
        ip_packet[9] = 6; // TCP
        ip_packet[12..16].copy_from_slice(&[10, 0, 0, 1]);
        ip_packet[16..20].copy_from_slice(&[10, 0, 0, 2]);

        // Sessions confirm in both directions a packet or two in, so retry
        // rather than assuming the very first data packet is accepted.
        let mut delivered: Option<Vec<u8>> = None;
        for attempt in 0..4 {
            let mut outbound = [0u8; 2048];
            let ciphertext = match ours.encapsulate(&ip_packet, &mut outbound) {
                TunnResult::WriteToNetwork(packet) => packet.to_vec(),
                _ => break,
            };

            let mut inbound = [0u8; 2048];
            match theirs.decapsulate(None, &ciphertext, &mut inbound) {
                TunnResult::WriteToTunnelV4(packet, _) if !packet.is_empty() => {
                    delivered = Some(packet.to_vec());
                    break;
                }
                other => {
                    assert!(
                        attempt < 3,
                        "peer never recovered an IPv4 payload; last result was {}",
                        match other {
                            TunnResult::Done => "done",
                            TunnResult::Err(_) => "error",
                            TunnResult::WriteToNetwork(_) => "write-to-network",
                            _ => "empty tunnel packet",
                        }
                    );
                }
            }
        }

        let delivered = delivered.expect("peer never received the data packet");
        assert_eq!(delivered, ip_packet, "payload survived the tunnel intact");
    }
}

/// Drive one WireGuard session until the task is dropped.
pub async fn run(config: TunnelConfig, io: WireGuardIo) -> Result<()> {
    let socket = UdpSocket::bind("0.0.0.0:0").await?;
    socket.connect(config.endpoint).await?;

    let mut tunnel = Tunn::new(
        config.keypair.private.clone(),
        config.peer_public,
        None,
        Some(KEEPALIVE_SECS),
        0,
        None,
    );

    let mut receive_buffer = vec![0u8; MAX_PACKET];
    let mut work_buffer = vec![0u8; MAX_PACKET];

    // Kick the handshake off immediately rather than waiting for the first packet.
    if let TunnResult::WriteToNetwork(packet) =
        tunnel.format_handshake_initiation(&mut work_buffer, false)
    {
        let _ = socket.send(packet).await;
        debug!(region = %config.region_id, "sent WireGuard handshake initiation");
    }

    let mut timer = tokio::time::interval(Duration::from_millis(250));

    loop {
        tokio::select! {
            received = socket.recv(&mut receive_buffer) => {
                let size = match received {
                    Ok(0) => continue,
                    Ok(size) => size,
                    Err(e) => {
                        warn!(error = %e, "WireGuard UDP receive failed");
                        continue;
                    }
                };

                match tunnel.decapsulate(None, &receive_buffer[..size], &mut work_buffer) {
                    TunnResult::WriteToNetwork(packet) => {
                        let owned = packet.to_vec();
                        let _ = socket.send(&owned).await;
                    }
                    TunnResult::WriteToTunnelV4(packet, _)
                    | TunnResult::WriteToTunnelV6(packet, _) => {
                        io.inbound.push(packet.to_vec());
                        trace!(region = %config.region_id, "packet decrypted from tunnel");
                    }
                    TunnResult::Err(error) => debug!(error = ?error, "WireGuard decapsulate error"),
                    TunnResult::Done => {}
                }
            }

            _ = io.wake.notified() => {
                while let Some(packet) = io.outbound.pop() {
                    match tunnel.encapsulate(&packet, &mut work_buffer) {
                        TunnResult::WriteToNetwork(encrypted) => {
                            let owned = encrypted.to_vec();
                            if let Err(e) = socket.send(&owned).await {
                                warn!(error = %e, "WireGuard UDP send failed");
                                break;
                            }
                        }
                        TunnResult::Err(error) => {
                            debug!(error = ?error, "WireGuard encapsulate error");
                        }
                        _ => {}
                    }
                }
            }

            _ = timer.tick() => {
                // Drive rekeying, handshake retries, and persistent keepalives.
                if let TunnResult::WriteToNetwork(packet) = tunnel.update_timers(&mut work_buffer) {
                    let owned = packet.to_vec();
                    let _ = socket.send(&owned).await;
                }
                // Nudge the stack task in case it has outbound packets queued.
                io.wake.notify_one();
            }
        }
    }
}

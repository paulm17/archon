//! The userspace TCP/IP stack: smoltcp over the WireGuard device.
//!
//! One task owns the `Interface` and `SocketSet`. Client connections arrive as
//! commands; each becomes a smoltcp TCP socket whose bytes are spliced to and
//! from the client through bounded channels.

use crate::error::{ProxyError, Result};
use crate::pia::TunnelConfig;
use crate::tunnel::dns;
use crate::tunnel::wireguard::WireGuardIo;
use crate::tunnel::{Command, Conn};
use smoltcp::iface::{Config as InterfaceConfig, Interface, SocketHandle, SocketSet};
use smoltcp::socket::tcp;
use smoltcp::time::Instant;
use smoltcp::wire::{HardwareAddress, IpAddress, IpCidr, IpEndpoint, Ipv4Address};
use std::net::Ipv4Addr;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{debug, trace, warn};

use super::device::TunDevice;

/// WireGuard adds 32 bytes of header plus a 16 byte tag to a 1500 byte path MTU.
const TUNNEL_MTU: usize = 1420;
const SOCKET_RX_BYTES: usize = 128 * 1024;
const SOCKET_TX_BYTES: usize = 128 * 1024;
const CLIENT_QUEUE: usize = 64;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
const DNS_TIMEOUT: Duration = Duration::from_secs(6);
const POLL_INTERVAL: Duration = Duration::from_millis(2);

struct Active {
    handle: SocketHandle,
    to_client: mpsc::Sender<Vec<u8>>,
    from_client: mpsc::Receiver<Vec<u8>>,
    pending_write: Vec<u8>,
}

fn now() -> Instant {
    Instant::now()
}

/// smoltcp has its own address newtype; convert at the boundary.
fn smoltcp_ip(ip: Ipv4Addr) -> IpAddress {
    let octets = ip.octets();
    IpAddress::Ipv4(Ipv4Address::new(
        octets[0], octets[1], octets[2], octets[3],
    ))
}

/// Run the stack for one tunnel until the control channel closes.
pub async fn run(config: TunnelConfig, io: WireGuardIo, mut ctrl: mpsc::Receiver<Command>) {
    let mut device = TunDevice::new(TUNNEL_MTU, io.inbound.clone(), io.outbound.clone());

    let mut iface_config = InterfaceConfig::new(HardwareAddress::Ip);
    iface_config.random_seed = rand::random();

    let mut iface = Interface::new(iface_config, &mut device, now());
    iface.update_ip_addrs(|addrs| {
        let _ = addrs.push(IpCidr::new(smoltcp_ip(config.peer_ip), 32));
    });
    if let Err(_e) = iface
        .routes_mut()
        .add_default_ipv4_route(Ipv4Address::UNSPECIFIED)
    {
        warn!(region = %config.region_id, "cannot install default route in userspace stack");
        return;
    }

    let mut sockets = SocketSet::new(Vec::new());
    let mut active: Vec<Active> = Vec::new();
    let mut next_port: u16 = 20_000;
    let mut ticker = tokio::time::interval(POLL_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    debug!(region = %config.region_id, peer_ip = %config.peer_ip, "userspace TCP/IP stack ready");

    loop {
        tokio::select! {
            command = ctrl.recv() => {
                match command {
                    Some(Command::Connect { target, reply }) => {
                        match open_connection(&config, &mut iface, &mut device, &mut sockets, &target, &mut next_port, &mut active).await {
                            Ok(connection) => {
                                let _ = reply.send(Ok(connection));
                            }
                            Err(error) => {
                                let _ = reply.send(Err(error));
                            }
                        }
                    }
                    None => break,
                }
            }
            _ = ticker.tick() => {}
        }

        pump(&mut iface, &mut device, &mut sockets, &mut active);
        // Outbound packets are queued by the device's TxToken; tell the
        // WireGuard task there is work to encrypt and send.
        io.wake.notify_one();
    }

    debug!(region = %config.region_id, "userspace TCP/IP stack stopped");
}

/// Move bytes between client channels and their smoltcp sockets.
fn pump(
    iface: &mut Interface,
    device: &mut TunDevice,
    sockets: &mut SocketSet<'_>,
    active: &mut Vec<Active>,
) {
    active.retain_mut(|connection| {
        let socket = sockets.get_mut::<tcp::Socket>(connection.handle);

        // client -> tunnel
        loop {
            if !connection.pending_write.is_empty() {
                match socket.send_slice(&connection.pending_write) {
                    Ok(0) => break,
                    Ok(written) => {
                        connection.pending_write.drain(..written);
                    }
                    Err(_) => break,
                }
            }
            if !socket.can_send() {
                break;
            }
            match connection.from_client.try_recv() {
                Ok(chunk) => connection.pending_write.extend_from_slice(&chunk),
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    // Client finished writing: half close towards the peer.
                    socket.close();
                    break;
                }
            }
        }

        // tunnel -> client
        while socket.can_recv() && connection.to_client.capacity() > 0 {
            let mut buffer = vec![0u8; 16 * 1024];
            match socket.recv_slice(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    buffer.truncate(read);
                    if connection.to_client.try_send(buffer).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }

        if connection.to_client.is_closed() {
            socket.abort();
            return false;
        }

        let state = socket.state();
        let finished = matches!(state, tcp::State::Closed | tcp::State::TimeWait)
            && connection.pending_write.is_empty();
        if finished {
            return false;
        }
        true
    });

    iface.poll(now(), device, sockets);
}

/// Resolve, connect and register a new client connection.
async fn open_connection(
    config: &TunnelConfig,
    iface: &mut Interface,
    device: &mut TunDevice,
    sockets: &mut SocketSet<'_>,
    target: &crate::Target,
    next_port: &mut u16,
    active: &mut Vec<Active>,
) -> Result<Conn> {
    let ip = match target.host.parse::<Ipv4Addr>() {
        Ok(ip) => ip,
        Err(_) => resolve(config, iface, device, sockets, &target.host, next_port).await?,
    };

    let endpoint = IpEndpoint::new(smoltcp_ip(ip), target.port);
    let handle = connect_tcp(iface, device, sockets, endpoint, next_port).await?;

    let (to_client, client_rx) = mpsc::channel(CLIENT_QUEUE);
    let (client_tx, from_client) = mpsc::channel(CLIENT_QUEUE);

    active.push(Active {
        handle,
        to_client,
        from_client,
        pending_write: Vec::new(),
    });

    trace!(target = %target, "tunnel connection established");
    Ok(Conn::new(client_rx, client_tx))
}

/// Resolve a hostname through the tunnel using PIA's assigned resolver.
async fn resolve(
    config: &TunnelConfig,
    iface: &mut Interface,
    device: &mut TunDevice,
    sockets: &mut SocketSet<'_>,
    host: &str,
    next_port: &mut u16,
) -> Result<Ipv4Addr> {
    let resolver = *config.dns.first().ok_or_else(|| {
        ProxyError::Tunnel("tunnel has no DNS resolver configured".into())
    })?;

    let transaction_id: u16 = rand::random();
    let query = dns::build_query(host, transaction_id)?;

    let handle = connect_tcp(
        iface,
        device,
        sockets,
        IpEndpoint::new(smoltcp_ip(resolver), 53),
        next_port,
    )
    .await
    .map_err(|e| ProxyError::Tunnel(format!("cannot reach DNS resolver {resolver}: {e}")))?;

    // Send the framed query.
    let mut remaining = query;
    let deadline = tokio::time::Instant::now() + DNS_TIMEOUT;
    while !remaining.is_empty() {
        if tokio::time::Instant::now() > deadline {
            sockets.remove(handle);
            return Err(ProxyError::Tunnel(format!("timed out sending DNS query for {host}")));
        }
        let socket = sockets.get_mut::<tcp::Socket>(handle);
        match socket.send_slice(&remaining) {
            Ok(0) => {}
            Ok(written) => {
                remaining.drain(..written);
            }
            Err(e) => {
                sockets.remove(handle);
                return Err(ProxyError::Tunnel(format!("DNS send failed: {e}")));
            }
        }
        iface.poll(now(), device, sockets);
        tokio::time::sleep(POLL_INTERVAL).await;
    }

    // Read the 2 byte length prefix, then the message.
    let mut framed = Vec::new();
    let mut expected: Option<usize> = None;
    let mut buffer = vec![0u8; 4096];

    while tokio::time::Instant::now() < deadline {
        iface.poll(now(), device, sockets);
        loop {
            let socket = sockets.get_mut::<tcp::Socket>(handle);
            if !socket.can_recv() {
                break;
            }
            match socket.recv_slice(&mut buffer) {
                Ok(0) => break,
                Ok(read) => framed.extend_from_slice(&buffer[..read]),
                Err(_) => break,
            }
        }
        if expected.is_none() && framed.len() >= 2 {
            expected = Some(u16::from_be_bytes([framed[0], framed[1]]) as usize);
        }
        if let Some(size) = expected {
            if framed.len() >= size + 2 {
                sockets.remove(handle);
                return dns::parse_response(&framed[2..2 + size], transaction_id)
                    .map(|mut addresses| addresses.remove(0));
            }
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }

    sockets.remove(handle);
    Err(ProxyError::Tunnel(format!("timed out resolving {host}")))
}

/// Open a TCP socket through the tunnel and wait for it to establish.
async fn connect_tcp(
    iface: &mut Interface,
    device: &mut TunDevice,
    sockets: &mut SocketSet<'_>,
    endpoint: IpEndpoint,
    next_port: &mut u16,
) -> Result<SocketHandle> {
    // Buffers are owned by the socket, so they live as long as the socket does
    // and nothing borrows from this stack frame.
    let mut socket = tcp::Socket::new(
        tcp::SocketBuffer::new(vec![0u8; SOCKET_RX_BYTES]),
        tcp::SocketBuffer::new(vec![0u8; SOCKET_TX_BYTES]),
    );

    let local_port = *next_port;
    *next_port = if *next_port >= 60_000 { 20_000 } else { *next_port + 1 };

    {
        let context = iface.context();
        socket
            .connect(context, endpoint, local_port)
            .map_err(|e| ProxyError::Tunnel(format!("cannot start TCP connect: {e}")))?;
    }

    let handle = sockets.add(socket);
    let deadline = tokio::time::Instant::now() + CONNECT_TIMEOUT;

    loop {
        iface.poll(now(), device, sockets);
        {
            let socket = sockets.get_mut::<tcp::Socket>(handle);
            match socket.state() {
                tcp::State::Established => return Ok(handle),
                tcp::State::Closed | tcp::State::TimeWait => {
                    sockets.remove(handle);
                    return Err(ProxyError::Tunnel(format!(
                        "connection to {endpoint} was refused or dropped"
                    )));
                }
                _ => {}
            }
        }
        if tokio::time::Instant::now() > deadline {
            sockets.remove(handle);
            return Err(ProxyError::Tunnel(format!(
                "connection to {endpoint} timed out"
            )));
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tunnel::device::PacketQueue;

    fn build_interface(ip: Ipv4Addr, device: &mut TunDevice) -> Interface {
        let mut config = InterfaceConfig::new(HardwareAddress::Ip);
        config.random_seed = 7;
        let mut iface = Interface::new(config, device, now());
        iface.update_ip_addrs(|addrs| {
            let _ = addrs.push(IpCidr::new(smoltcp_ip(ip), 32));
        });
        iface
            .routes_mut()
            .add_default_ipv4_route(Ipv4Address::UNSPECIFIED)
            .expect("default route");
        iface
    }

    /// Two independent userspace stacks, wired packet-to-packet with no
    /// WireGuard and no kernel: connect, then move bytes both ways.
    #[tokio::test]
    async fn connects_and_echoes_between_two_userspace_stacks() {
        let a_to_b = PacketQueue::default();
        let b_to_a = PacketQueue::default();

        let mut device_a = TunDevice::new(TUNNEL_MTU, b_to_a.clone(), a_to_b.clone());
        let mut device_b = TunDevice::new(TUNNEL_MTU, a_to_b.clone(), b_to_a.clone());

        let a_ip = Ipv4Addr::new(10, 0, 0, 1);
        let b_ip = Ipv4Addr::new(10, 0, 0, 2);

        let mut iface_a = build_interface(a_ip, &mut device_a);
        let mut iface_b = build_interface(b_ip, &mut device_b);

        let mut sockets_a = SocketSet::new(Vec::new());
        let mut sockets_b = SocketSet::new(Vec::new());

        // B accepts on 8080 and echoes whatever it receives.
        let mut server = tcp::Socket::new(
            tcp::SocketBuffer::new(vec![0u8; 8192]),
            tcp::SocketBuffer::new(vec![0u8; 8192]),
        );
        server.listen(8080).expect("listen");
        let server_handle = sockets_b.add(server);

        let driver = tokio::spawn(async move {
            let mut buffer = vec![0u8; 4096];
            for _ in 0..500 {
                iface_b.poll(now(), &mut device_b, &mut sockets_b);
                let socket = sockets_b.get_mut::<tcp::Socket>(server_handle);
                if socket.can_recv() {
                    if let Ok(read) = socket.recv_slice(&mut buffer) {
                        let _ = socket.send_slice(&buffer[..read]);
                    }
                }
                tokio::time::sleep(POLL_INTERVAL).await;
            }
        });

        let mut next_port = 40_000u16;
        let handle = connect_tcp(
            &mut iface_a,
            &mut device_a,
            &mut sockets_a,
            IpEndpoint::new(smoltcp_ip(b_ip), 8080),
            &mut next_port,
        )
        .await
        .expect("TCP connect through the userspace stack");

        {
            let socket = sockets_a.get_mut::<tcp::Socket>(handle);
            socket.send_slice(b"ping").expect("queue payload");
        }

        let mut received = Vec::new();
        for _ in 0..500 {
            iface_a.poll(now(), &mut device_a, &mut sockets_a);
            let socket = sockets_a.get_mut::<tcp::Socket>(handle);
            if socket.can_recv() {
                let mut buffer = [0u8; 64];
                if let Ok(read) = socket.recv_slice(&mut buffer) {
                    received.extend_from_slice(&buffer[..read]);
                }
                if received.len() >= 4 {
                    break;
                }
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }

        driver.abort();
        assert_eq!(received, b"ping", "payload was echoed by the peer stack");
    }
}

//! A smoltcp `Device` with no kernel behind it.
//!
//! Packets we "transmit" are queued for the WireGuard task to encrypt; packets
//! it decrypts are queued for smoltcp to receive. Nothing touches a TUN device,
//! so this needs no privileges and cannot disturb the host's routing table.

use smoltcp::phy::{ChecksumCapabilities, Device, DeviceCapabilities, Medium, RxToken, TxToken};
use smoltcp::time::Instant;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// Packet queue shared between the stack task and the WireGuard task.
#[derive(Clone, Default)]
pub struct PacketQueue(Arc<Mutex<VecDeque<Vec<u8>>>>);

impl PacketQueue {
    pub fn push(&self, packet: Vec<u8>) {
        let mut queue = self.0.lock().expect("packet queue poisoned");
        // Bound the queue so a stalled peer cannot grow memory without limit.
        if queue.len() >= 4096 {
            queue.pop_front();
        }
        queue.push_back(packet);
    }

    pub fn pop(&self) -> Option<Vec<u8>> {
        self.0.lock().expect("packet queue poisoned").pop_front()
    }

}

pub struct TunDevice {
    mtu: usize,
    incoming: PacketQueue,
    outgoing: PacketQueue,
}

impl TunDevice {
    pub fn new(mtu: usize, incoming: PacketQueue, outgoing: PacketQueue) -> Self {
        Self {
            mtu,
            incoming,
            outgoing,
        }
    }
}

pub struct OwnedRxToken(Vec<u8>);

impl RxToken for OwnedRxToken {
    fn consume<R, F>(self, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut buffer = self.0;
        f(&mut buffer)
    }
}

pub struct OwnedTxToken {
    queue: PacketQueue,
}

impl TxToken for OwnedTxToken {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut buffer = vec![0u8; len];
        let result = f(&mut buffer);
        self.queue.push(buffer);
        result
    }
}

impl Device for TunDevice {
    type RxToken<'a> = OwnedRxToken where Self: 'a;
    type TxToken<'a> = OwnedTxToken where Self: 'a;

    fn capabilities(&self) -> DeviceCapabilities {
        let mut capabilities = DeviceCapabilities::default();
        capabilities.medium = Medium::Ip;
        capabilities.max_transmission_unit = self.mtu;
        // No hardware offload in userspace: let smoltcp compute every checksum.
        capabilities.checksum = ChecksumCapabilities::default();
        capabilities
    }

    fn receive(&mut self, _timestamp: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        let packet = self.incoming.pop()?;
        Some((
            OwnedRxToken(packet),
            OwnedTxToken {
                queue: self.outgoing.clone(),
            },
        ))
    }

    fn transmit(&mut self, _timestamp: Instant) -> Option<Self::TxToken<'_>> {
        Some(OwnedTxToken {
            queue: self.outgoing.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moves_packets_between_the_queues() {
        let incoming = PacketQueue::default();
        let outgoing = PacketQueue::default();
        incoming.push(vec![1, 2, 3]);

        let mut device = TunDevice::new(1420, incoming.clone(), outgoing.clone());

        let (rx, _tx) = device.receive(Instant::now()).expect("inbound packet");
        rx.consume(|buffer| assert_eq!(buffer, &[1, 2, 3]));

        let tx = device.transmit(Instant::now()).expect("transmit token");
        tx.consume(4, |buffer| buffer.copy_from_slice(&[9, 9, 9, 9]));

        assert_eq!(outgoing.pop(), Some(vec![9, 9, 9, 9]));
        assert!(device.receive(Instant::now()).is_none());
    }

    #[test]
    fn reports_a_userspace_mtu_and_ip_medium() {
        let device = TunDevice::new(1420, PacketQueue::default(), PacketQueue::default());
        let capabilities = device.capabilities();
        assert_eq!(capabilities.max_transmission_unit, 1420);
        assert_eq!(capabilities.medium, Medium::Ip);
    }
}

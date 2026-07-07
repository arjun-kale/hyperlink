//! Transport abstraction layer.
//!
//! Provides a trait so the bench harness can work over different transports
//! (raw UDP now, QUIC in Phase 1) without rewriting measurement logic.

use std::io;
use std::net::SocketAddr;
use tokio::net::UdpSocket;

/// Abstraction over a datagram-oriented transport.
///
/// Both the clock sync and echo tools use this trait, so swapping from raw UDP
/// to QUIC (Phase 1) requires only a new `Transport` implementation.
#[async_trait::async_trait]
pub trait Transport: Send + Sync {
    /// Send a datagram to the peer.
    async fn send(&self, buf: &[u8]) -> io::Result<usize>;

    /// Receive a datagram from the peer. Returns (bytes_read, sender_addr).
    async fn recv(&self, buf: &mut [u8]) -> io::Result<(usize, SocketAddr)>;

    /// Send a datagram to a specific address.
    async fn send_to(&self, buf: &[u8], addr: SocketAddr) -> io::Result<usize>;
}

/// UDP transport implementation using `tokio::net::UdpSocket`.
pub struct UdpTransport {
    socket: UdpSocket,
}

impl UdpTransport {
    /// Bind a new UDP socket to the given address.
    pub async fn bind(addr: SocketAddr) -> io::Result<Self> {
        let socket = UdpSocket::bind(addr).await?;
        Ok(Self { socket })
    }

    /// Connect the UDP socket to a specific peer address.
    /// After this, `send()` sends to the connected peer.
    pub async fn connect(&mut self, addr: SocketAddr) -> io::Result<()> {
        self.socket.connect(addr).await
    }

    /// Get the local address this socket is bound to.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }
}

#[async_trait::async_trait]
impl Transport for UdpTransport {
    async fn send(&self, buf: &[u8]) -> io::Result<usize> {
        self.socket.send(buf).await
    }

    async fn recv(&self, buf: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
        self.socket.recv_from(buf).await
    }

    async fn send_to(&self, buf: &[u8], addr: SocketAddr) -> io::Result<usize> {
        self.socket.send_to(buf, addr).await
    }
}

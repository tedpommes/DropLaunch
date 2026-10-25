//! Small network helpers: this PC's network address.

use std::net::{IpAddr, UdpSocket};

/// The address other PCs on the network reach this one at (the one the default route uses).
pub fn lan_ip() -> Option<IpAddr> {
    let socket = UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    socket.connect(("8.8.8.8", 80)).ok()?;
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_unspecified() && !ip.is_loopback()).then_some(ip)
}

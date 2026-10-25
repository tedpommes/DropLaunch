//! Steam's server query (A2S_INFO): how Steam finds out which game a server runs, and on which
//! port it plays, before it connects to it.

use std::net::{IpAddr, UdpSocket};
use std::time::Duration;

const REQUEST: &[u8] = b"\xFF\xFF\xFF\xFFTSource Engine Query\0";

/// The game port a server reports when asked at `ip`:`query_port`, if it answers there.
pub fn game_port_at(ip: IpAddr, query_port: u16) -> Option<u16> {
    let socket = UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    socket.set_read_timeout(Some(Duration::from_millis(400))).ok()?;
    socket.connect((ip, query_port)).ok()?;
    let mut request = REQUEST.to_vec();
    let mut buf = [0u8; 1400];
    for _ in 0..3 {
        socket.send(&request).ok()?;
        let n = socket.recv(&mut buf).ok()?;
        let reply = &buf[..n];
        match reply.get(4) {
            // A challenge: ask again with it.
            Some(b'A') if n >= 9 => request = [REQUEST, &reply[5..9]].concat(),
            Some(b'I') => return reported_game_port(reply),
            _ => return None,
        }
    }
    None
}

/// The game port in an A2S_INFO reply (in its extra data), if it has one.
fn reported_game_port(reply: &[u8]) -> Option<u16> {
    let mut i = 6; // header, 'I', protocol
    for _ in 0..4 {
        // name, map, folder, game
        i += reply.get(i..)?.iter().position(|&b| b == 0)? + 1;
    }
    i += 2 + 3 + 4; // app id; players, max players, bots; type, OS, password, VAC
    i += reply.get(i..)?.iter().position(|&b| b == 0)? + 1; // version
    let extra = *reply.get(i)?;
    if extra & 0x80 == 0 {
        return None;
    }
    let port = reply.get(i + 1..i + 3)?;
    Some(u16::from_le_bytes([port[0], port[1]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_game_port_from_a_rust_reply() {
        // What a local Rust server sent on its query port (28017).
        let mut reply = b"\xFF\xFF\xFF\xFFI\x11DropLaunch test\0Procedural Map\0rust\0Rust\0".to_vec();
        reply.extend_from_slice(&[0, 0, 0, 10, 0, b'd', b'w', 0, 1]);
        reply.extend_from_slice(b"2634\0");
        reply.push(0x80 | 0x10 | 0x20 | 0x01);
        reply.extend_from_slice(&28015u16.to_le_bytes());
        reply.extend_from_slice(&90294310225950740u64.to_le_bytes());
        reply.extend_from_slice(b"mp10,cp0\0");
        reply.extend_from_slice(&252490u64.to_le_bytes());
        assert_eq!(reported_game_port(&reply), Some(28015));
        assert_eq!(reported_game_port(&reply[..20]), None);
    }

    /// Needs a DropLaunch server running on this PC: cargo test live_query -- --ignored
    #[test]
    #[ignore]
    fn live_query() {
        use crate::startfile::{DEFAULT_PORT, DEFAULT_QUERY_PORT};
        let localhost = IpAddr::from([127, 0, 0, 1]);
        assert_eq!(game_port_at(localhost, DEFAULT_QUERY_PORT), Some(DEFAULT_PORT));
        assert_eq!(game_port_at(localhost, DEFAULT_PORT), None, "the game port doesn't answer queries");
        let lan = crate::net::lan_ip().expect("on a network");
        assert_eq!(game_port_at(lan, DEFAULT_QUERY_PORT), Some(DEFAULT_PORT));
    }
}

//! Minimal STUN binding client — asks a public STUN server what this
//! socket looks like from the internet.
//!
//! str0m is Sans-IO and does no discovery of its own, so for relay-signaled
//! peers the daemon performs one binding round trip and advertises the
//! answer as a server-reflexive candidate. The request must leave on the
//! same socket the peer will use: the NAT mapping is per socket, and a
//! mapping discovered on any other socket would advertise an address that
//! routes nowhere.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;
use tokio::net::UdpSocket;

const STUN_SERVERS: &[&str] = &["stun.l.google.com:19302", "stun.cloudflare.com:3478"];
const MAGIC_COOKIE: u32 = 0x2112_A442;
const BINDING_REQUEST: u16 = 0x0001;
const BINDING_SUCCESS: u16 = 0x0101;
const ATTR_MAPPED_ADDRESS: u16 = 0x0001;
const ATTR_XOR_MAPPED_ADDRESS: u16 = 0x0020;
const QUERY_TIMEOUT: Duration = Duration::from_millis(1500);

/// Discover this socket's public address, trying each server in turn.
/// `None` means no server answered — the caller proceeds LAN-only.
pub async fn discover_public_addr(socket: &UdpSocket) -> Option<SocketAddr> {
    for server in STUN_SERVERS {
        match query(socket, server).await {
            Ok(addr) => return Some(addr),
            Err(e) => tracing::debug!("STUN query to {server} failed: {e}"),
        }
    }
    None
}

async fn query(socket: &UdpSocket, server: &str) -> anyhow::Result<SocketAddr> {
    let txid: [u8; 12] = std::array::from_fn(|_| rand::random::<u8>());
    socket.send_to(&binding_request(&txid), server).await?;

    // Read until our transaction's success response or the timeout; anything
    // else on the wire (a late reply from a previous server) is skipped.
    let deadline = tokio::time::Instant::now() + QUERY_TIMEOUT;
    let mut buf = [0u8; 512];
    loop {
        let recv = tokio::time::timeout_at(deadline, socket.recv_from(&mut buf));
        let (n, _) = recv.await.map_err(|_| anyhow::anyhow!("timed out"))??;
        if let Some(addr) = parse_binding_response(&buf[..n], &txid) {
            return Ok(addr);
        }
    }
}

fn binding_request(txid: &[u8; 12]) -> [u8; 20] {
    let mut msg = [0u8; 20];
    msg[0..2].copy_from_slice(&BINDING_REQUEST.to_be_bytes());
    // length stays 0 — no attributes
    msg[4..8].copy_from_slice(&MAGIC_COOKIE.to_be_bytes());
    msg[8..20].copy_from_slice(txid);
    msg
}

/// Extract the mapped address from a binding success response, or `None`
/// if the packet is not the answer to `txid`. Only IPv4 is handled — the
/// peer socket binds 0.0.0.0, so an IPv6 mapping could never pair with it.
fn parse_binding_response(msg: &[u8], txid: &[u8; 12]) -> Option<SocketAddr> {
    if msg.len() < 20
        || u16::from_be_bytes([msg[0], msg[1]]) != BINDING_SUCCESS
        || msg[4..8] != MAGIC_COOKIE.to_be_bytes()
        || &msg[8..20] != txid
    {
        return None;
    }

    let mut attrs = &msg[20..];
    while attrs.len() >= 4 {
        let attr_type = u16::from_be_bytes([attrs[0], attrs[1]]);
        let attr_len = u16::from_be_bytes([attrs[2], attrs[3]]) as usize;
        let value = attrs.get(4..4 + attr_len)?;

        match attr_type {
            ATTR_XOR_MAPPED_ADDRESS => {
                if let Some(addr) = decode_address(value, true) {
                    return Some(addr);
                }
            }
            ATTR_MAPPED_ADDRESS => {
                if let Some(addr) = decode_address(value, false) {
                    return Some(addr);
                }
            }
            _ => {}
        }

        // Attributes are padded to 4-byte boundaries.
        attrs = attrs.get(4 + attr_len.div_ceil(4) * 4..)?;
    }
    None
}

fn decode_address(value: &[u8], xored: bool) -> Option<SocketAddr> {
    if value.len() < 8 || value[1] != 0x01 {
        return None; // family 0x01 = IPv4
    }
    let mut port = u16::from_be_bytes([value[2], value[3]]);
    let mut ip = u32::from_be_bytes([value[4], value[5], value[6], value[7]]);
    if xored {
        port ^= (MAGIC_COOKIE >> 16) as u16;
        ip ^= MAGIC_COOKIE;
    }
    Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::from(ip)), port))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(txid: &[u8; 12], attrs: &[u8]) -> Vec<u8> {
        let mut msg = Vec::new();
        msg.extend_from_slice(&BINDING_SUCCESS.to_be_bytes());
        msg.extend_from_slice(&(attrs.len() as u16).to_be_bytes());
        msg.extend_from_slice(&MAGIC_COOKIE.to_be_bytes());
        msg.extend_from_slice(txid);
        msg.extend_from_slice(attrs);
        msg
    }

    fn xor_mapped_attr(addr: SocketAddr) -> Vec<u8> {
        let SocketAddr::V4(v4) = addr else { panic!() };
        let port = addr.port() ^ (MAGIC_COOKIE >> 16) as u16;
        let ip = u32::from(*v4.ip()) ^ MAGIC_COOKIE;
        let mut attr = vec![0u8, 0x01];
        attr.extend_from_slice(&port.to_be_bytes());
        attr.extend_from_slice(&ip.to_be_bytes());
        let mut out = ATTR_XOR_MAPPED_ADDRESS.to_be_bytes().to_vec();
        out.extend_from_slice(&(attr.len() as u16).to_be_bytes());
        out.extend_from_slice(&attr);
        out
    }

    #[test]
    fn parses_xor_mapped_address() {
        let txid = [7u8; 12];
        let addr: SocketAddr = "203.0.113.7:54321".parse().unwrap();
        let msg = response(&txid, &xor_mapped_attr(addr));
        assert_eq!(parse_binding_response(&msg, &txid), Some(addr));
    }

    #[test]
    fn rejects_wrong_transaction_id() {
        let txid = [7u8; 12];
        let addr: SocketAddr = "203.0.113.7:54321".parse().unwrap();
        let msg = response(&txid, &xor_mapped_attr(addr));
        assert_eq!(parse_binding_response(&msg, &[8u8; 12]), None);
    }

    #[test]
    fn skips_unknown_attributes() {
        let txid = [3u8; 12];
        let addr: SocketAddr = "198.51.100.2:19302".parse().unwrap();
        // SOFTWARE attribute (0x8022) with 5 bytes of value, padded to 8.
        let mut attrs = vec![0x80, 0x22, 0x00, 0x05];
        attrs.extend_from_slice(b"ling\0\0\0\0");
        attrs.extend_from_slice(&xor_mapped_attr(addr));
        let msg = response(&txid, &attrs);
        assert_eq!(parse_binding_response(&msg, &txid), Some(addr));
    }

    #[test]
    fn request_is_well_formed() {
        let txid = [9u8; 12];
        let req = binding_request(&txid);
        assert_eq!(u16::from_be_bytes([req[0], req[1]]), BINDING_REQUEST);
        assert_eq!(u16::from_be_bytes([req[2], req[3]]), 0);
        assert_eq!(req[4..8], MAGIC_COOKIE.to_be_bytes());
        assert_eq!(req[8..20], txid);
    }
}

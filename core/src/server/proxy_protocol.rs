/*
 * Copyright 2026 Antonio Cantallops Alba
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */
//! PROXY protocol v1/v2 parser — client address discovery behind LBs.
//!
//! Specification: https://www.haproxy.org/download/1.8/doc/proxy-protocol.txt

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxyProtocolVersion {
    V1,
    V2,
}

#[derive(Debug, Clone)]
pub struct ProxyHeader {
    pub version: ProxyProtocolVersion,
    pub source: SocketAddr,
    pub destination: SocketAddr,
}

#[derive(Debug)]
pub enum ProxyParseError {
    Incomplete { needed: usize, got: usize },
    InvalidV1Signature,
    InvalidV2Signature,
    UnsupportedVersion(u8),
    InvalidCommand(u8),
    InvalidFamily(u8),
    InvalidTransport(u8),
    InvalidAddress,
    MalformedV1,
}

impl fmt::Display for ProxyParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProxyParseError::Incomplete { needed, got } => {
                write!(f, "incomplete header: need {needed} bytes, got {got}")
            }
            ProxyParseError::InvalidV1Signature => write!(f, "invalid v1 signature"),
            ProxyParseError::InvalidV2Signature => write!(f, "invalid v2 signature"),
            ProxyParseError::UnsupportedVersion(v) => write!(f, "unsupported version: {v}"),
            ProxyParseError::InvalidCommand(c) => write!(f, "invalid v2 command: {c}"),
            ProxyParseError::InvalidFamily(fam) => write!(f, "invalid address family: {fam}"),
            ProxyParseError::InvalidTransport(t) => write!(f, "invalid transport protocol: {t}"),
            ProxyParseError::InvalidAddress => write!(f, "invalid address format"),
            ProxyParseError::MalformedV1 => write!(f, "malformed v1 header"),
        }
    }
}

impl std::error::Error for ProxyParseError {}

pub const V2_SIGNATURE: &[u8] = b"\x0D\x0A\x0D\x0A\x00\x0D\x0A\x51\x55\x49\x54\x0A";
pub const V1_PREFIX: &[u8] = b"PROXY ";

const V2_COMMAND_LOCAL: u8 = 0x00;
const V2_COMMAND_PROXY: u8 = 0x01;

const V2_FAMILY_UNSPEC: u8 = 0x00;
const V2_FAMILY_INET: u8 = 0x01;
const V2_FAMILY_INET6: u8 = 0x02;
const V2_FAMILY_UNIX: u8 = 0x03;

#[allow(dead_code)]
const V2_TRANSPORT_UNSPEC: u8 = 0x00;
const V2_TRANSPORT_STREAM: u8 = 0x01;
#[allow(dead_code)]
const V2_TRANSPORT_DGRAM: u8 = 0x02;

/// Parse PROXY protocol header from incoming stream bytes.
///
/// Returns `(header, bytes_consumed)` on success.
pub fn parse(buf: &[u8]) -> Result<(ProxyHeader, usize), ProxyParseError> {
    if buf.starts_with(V1_PREFIX) {
        parse_v1(buf)
    } else if buf.starts_with(V2_SIGNATURE) {
        parse_v2(buf)
    } else {
        Err(ProxyParseError::InvalidV1Signature)
    }
}

fn parse_v1(buf: &[u8]) -> Result<(ProxyHeader, usize), ProxyParseError> {
    // v1 format: "PROXY TCP4 <src_ip> <dst_ip> <src_port> <dst_port>\r\n"
    let end = buf
        .iter()
        .position(|b| *b == b'\r')
        .ok_or(ProxyParseError::MalformedV1)?;
    let header_end = end + 1; // include \r\n
    if header_end >= buf.len() || buf[header_end] != b'\n' {
        return Err(ProxyParseError::MalformedV1);
    }

    let line = std::str::from_utf8(&buf[..end]).map_err(|_| ProxyParseError::MalformedV1)?;
    let mut parts = line.split_whitespace();

    if parts.next() != Some("PROXY") {
        return Err(ProxyParseError::MalformedV1);
    }

    let proto = parts.next().ok_or(ProxyParseError::MalformedV1)?;
    let (family, _transport) = match proto {
        "TCP4" => (V2_FAMILY_INET, V2_TRANSPORT_STREAM),
        "TCP6" => (V2_FAMILY_INET6, V2_TRANSPORT_STREAM),
        "UNKNOWN" => {
            // Skip parse, return generic header
            return Ok((
                ProxyHeader {
                    version: ProxyProtocolVersion::V1,
                    source: SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0),
                    destination: SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0),
                },
                header_end + 1,
            ));
        }
        _ => return Err(ProxyParseError::MalformedV1),
    };

    let src_ip = parts.next().ok_or(ProxyParseError::MalformedV1)?;
    let dst_ip = parts.next().ok_or(ProxyParseError::MalformedV1)?;
    let src_port: u16 = parts
        .next()
        .ok_or(ProxyParseError::MalformedV1)?
        .parse()
        .map_err(|_| ProxyParseError::MalformedV1)?;
    let dst_port: u16 = parts
        .next()
        .ok_or(ProxyParseError::MalformedV1)?
        .parse()
        .map_err(|_| ProxyParseError::MalformedV1)?;

    let (src_addr, dst_addr) = match family {
        V2_FAMILY_INET => {
            let src_ip: Ipv4Addr = src_ip
                .parse()
                .map_err(|_| ProxyParseError::InvalidAddress)?;
            let dst_ip: Ipv4Addr = dst_ip
                .parse()
                .map_err(|_| ProxyParseError::InvalidAddress)?;
            (
                SocketAddr::new(IpAddr::V4(src_ip), src_port),
                SocketAddr::new(IpAddr::V4(dst_ip), dst_port),
            )
        }
        V2_FAMILY_INET6 => {
            let src_ip: Ipv6Addr = src_ip
                .parse()
                .map_err(|_| ProxyParseError::InvalidAddress)?;
            let dst_ip: Ipv6Addr = dst_ip
                .parse()
                .map_err(|_| ProxyParseError::InvalidAddress)?;
            (
                SocketAddr::new(IpAddr::V6(src_ip), src_port),
                SocketAddr::new(IpAddr::V6(dst_ip), dst_port),
            )
        }
        _ => return Err(ProxyParseError::InvalidAddress),
    };

    Ok((
        ProxyHeader {
            version: ProxyProtocolVersion::V1,
            source: src_addr,
            destination: dst_addr,
        },
        header_end + 1,
    ))
}

fn parse_v2(buf: &[u8]) -> Result<(ProxyHeader, usize), ProxyParseError> {
    if buf.len() < 16 {
        return Err(ProxyParseError::Incomplete {
            needed: 16,
            got: buf.len(),
        });
    }

    if !buf.starts_with(V2_SIGNATURE) {
        return Err(ProxyParseError::InvalidV2Signature);
    }

    let ver_cmd = buf[12];
    if ver_cmd >> 4 != 0x02 {
        return Err(ProxyParseError::UnsupportedVersion(ver_cmd >> 4));
    }

    let command = ver_cmd & 0x0f;
    if command != V2_COMMAND_PROXY && command != V2_COMMAND_LOCAL {
        return Err(ProxyParseError::InvalidCommand(command));
    }

    let fam_trans = buf[13];
    let family = fam_trans >> 4;
    let transport = fam_trans & 0x0f;

    let len = u16::from_be_bytes([buf[14], buf[15]]) as usize;
    let total_len = 16 + len;

    if buf.len() < total_len {
        return Err(ProxyParseError::Incomplete {
            needed: total_len,
            got: buf.len(),
        });
    }

    // LOCAL: the receiver must ignore the address block and use the real
    // connection peer (HAProxy health checks use this command).
    if command == V2_COMMAND_LOCAL {
        let unspecified = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0);
        return Ok((
            ProxyHeader {
                version: ProxyProtocolVersion::V2,
                source: unspecified,
                destination: unspecified,
            },
            total_len,
        ));
    }

    let (source, destination) = match (family, transport) {
        (V2_FAMILY_INET, V2_TRANSPORT_STREAM) => parse_v2_inet(&buf[16..])?,
        (V2_FAMILY_INET6, V2_TRANSPORT_STREAM) => parse_v2_inet6(&buf[16..])?,
        (V2_FAMILY_UNIX, _) => parse_v2_unix(&buf[16..])?,
        (V2_FAMILY_UNSPEC, _) => {
            return Ok((
                ProxyHeader {
                    version: ProxyProtocolVersion::V2,
                    source: SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0),
                    destination: SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0),
                },
                total_len,
            ))
        }
        (V2_FAMILY_INET | V2_FAMILY_INET6, t) => {
            return Err(ProxyParseError::InvalidTransport(t));
        }
        (f, _) => {
            return Err(ProxyParseError::InvalidFamily(f));
        }
    };

    Ok((
        ProxyHeader {
            version: ProxyProtocolVersion::V2,
            source,
            destination,
        },
        total_len,
    ))
}

fn parse_v2_inet(buf: &[u8]) -> Result<(SocketAddr, SocketAddr), ProxyParseError> {
    if buf.len() < 12 {
        return Err(ProxyParseError::Incomplete {
            needed: 12,
            got: buf.len(),
        });
    }

    let src_ip = Ipv4Addr::new(buf[0], buf[1], buf[2], buf[3]);
    let dst_ip = Ipv4Addr::new(buf[4], buf[5], buf[6], buf[7]);
    let src_port = u16::from_be_bytes([buf[8], buf[9]]);
    let dst_port = u16::from_be_bytes([buf[10], buf[11]]);

    Ok((
        SocketAddr::new(IpAddr::V4(src_ip), src_port),
        SocketAddr::new(IpAddr::V4(dst_ip), dst_port),
    ))
}

fn parse_v2_inet6(buf: &[u8]) -> Result<(SocketAddr, SocketAddr), ProxyParseError> {
    if buf.len() < 36 {
        return Err(ProxyParseError::Incomplete {
            needed: 36,
            got: buf.len(),
        });
    }

    let src_ip = Ipv6Addr::from(
        <[u8; 16]>::try_from(&buf[0..16]).map_err(|_| ProxyParseError::InvalidAddress)?,
    );
    let dst_ip = Ipv6Addr::from(
        <[u8; 16]>::try_from(&buf[16..32]).map_err(|_| ProxyParseError::InvalidAddress)?,
    );
    let src_port = u16::from_be_bytes([buf[32], buf[33]]);
    let dst_port = u16::from_be_bytes([buf[34], buf[35]]);

    Ok((
        SocketAddr::new(IpAddr::V6(src_ip), src_port),
        SocketAddr::new(IpAddr::V6(dst_ip), dst_port),
    ))
}

fn parse_v2_unix(_buf: &[u8]) -> Result<(SocketAddr, SocketAddr), ProxyParseError> {
    // Unix sockets have no network address; return unspecified
    Ok((
        SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0),
        SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_tcp4_parses() {
        let input = b"PROXY TCP4 192.168.1.1 10.0.0.1 56324 443\r\n";
        let (hdr, consumed) = parse(input).unwrap();
        assert_eq!(hdr.version, ProxyProtocolVersion::V1);
        assert_eq!(
            hdr.source,
            "192.168.1.1:56324".parse::<SocketAddr>().unwrap()
        );
        assert_eq!(consumed, input.len());
    }

    #[test]
    fn v1_tcp6_parses() {
        let input = b"PROXY TCP6 2001:db8::1 2001:db8::2 56324 443\r\n";
        let (hdr, consumed) = parse(input).unwrap();
        assert_eq!(hdr.version, ProxyProtocolVersion::V1);
        assert_eq!(
            hdr.source,
            "[2001:db8::1]:56324".parse::<SocketAddr>().unwrap()
        );
        assert_eq!(consumed, input.len());
    }

    #[test]
    fn v1_unknown_parses() {
        let input = b"PROXY UNKNOWN\r\n";
        let (hdr, _) = parse(input).unwrap();
        assert_eq!(hdr.version, ProxyProtocolVersion::V1);
    }

    #[test]
    fn v2_tcp4_parses() {
        // Signature + ver/cmd + fam/trans + len + addrs
        let mut buf = Vec::new();
        buf.extend_from_slice(V2_SIGNATURE);
        buf.push(0x21); // v2 + PROXY
        buf.push(0x11); // INET + STREAM
        buf.extend_from_slice(&[0x00, 0x0C]); // len = 12
        buf.extend_from_slice(&[192, 168, 1, 1]); // src ip
        buf.extend_from_slice(&[10, 0, 0, 1]); // dst ip
        buf.extend_from_slice(&[0xDC, 0x04]); // src port 56324
        buf.extend_from_slice(&[0x01, 0xBB]); // dst port 443

        let (hdr, consumed) = parse(&buf).unwrap();
        assert_eq!(hdr.version, ProxyProtocolVersion::V2);
        assert_eq!(
            hdr.source,
            "192.168.1.1:56324".parse::<SocketAddr>().unwrap()
        );
        assert_eq!(consumed, buf.len());
    }

    #[test]
    fn v2_local_command_keeps_real_peer() {
        // LOCAL: valid per spec; receiver ignores the address block and
        // keeps the real connection peer (unspecified addrs signal that).
        let mut buf = Vec::new();
        buf.extend_from_slice(V2_SIGNATURE);
        buf.push(0x20); // v2 + LOCAL
        buf.push(0x00); // UNSPEC
        buf.extend_from_slice(&[0x00, 0x00]); // len = 0

        let (hdr, consumed) = parse(&buf).unwrap();
        assert_eq!(hdr.version, ProxyProtocolVersion::V2);
        assert!(hdr.source.ip().is_unspecified());
        assert_eq!(consumed, buf.len());
    }

    #[test]
    fn v2_rejects_invalid_command() {
        let mut buf = Vec::new();
        buf.extend_from_slice(V2_SIGNATURE);
        buf.push(0x22); // v2 + undefined command 0x2
        buf.push(0x00); // UNSPEC
        buf.extend_from_slice(&[0x00, 0x00]); // len = 0

        assert!(matches!(
            parse(&buf),
            Err(ProxyParseError::InvalidCommand(0x02))
        ));
    }
}

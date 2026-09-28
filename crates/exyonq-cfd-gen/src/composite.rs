//! Composite generation payload: route table + optional WAF projection.
//!
//! ```text
//! magic[8] = b"CFDCOM01"
//! route_len: u32
//! route: [u8; route_len]   # CFDRT005/004/003 (or legacy CFDRT002) bytes (may be empty)
//! waf_len: u32
//! waf: [u8; waf_len]       # CFDWAF01 bytes (0 = WAF off / absent)
//! ```
//!
//! Legacy payloads that begin with `CFDRT002`, `CFDRT003`, `CFDRT004`, or `CFDRT005` remain valid (WAF absent).

use crate::route_table::{RouteTable, RouteTableError};
use crate::waf_proj::{CfdWafProjection, WafProjError};
use thiserror::Error;

const MAGIC: &[u8; 8] = b"CFDCOM01";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompositeProjection {
    pub routes: RouteTable,
    pub waf: Option<CfdWafProjection>,
}

#[derive(Debug, Error)]
pub enum CompositeError {
    #[error("invalid magic")]
    InvalidMagic,
    #[error("truncated composite")]
    Truncated,
    #[error("route: {0}")]
    Route(#[from] RouteTableError),
    #[error("waf: {0}")]
    Waf(#[from] WafProjError),
    #[error("route section empty but required")]
    EmptyRoute,
}

impl CompositeProjection {
    pub fn encode(&self) -> Result<Vec<u8>, CompositeError> {
        let route = self.routes.encode()?;
        let waf = match &self.waf {
            Some(w) => w.encode()?,
            None => Vec::new(),
        };
        let mut buf = Vec::with_capacity(8 + 8 + route.len() + waf.len());
        buf.extend_from_slice(MAGIC);
        buf.extend_from_slice(&(route.len() as u32).to_le_bytes());
        buf.extend_from_slice(&route);
        buf.extend_from_slice(&(waf.len() as u32).to_le_bytes());
        buf.extend_from_slice(&waf);
        Ok(buf)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, CompositeError> {
        if bytes.len() >= 8
            && (&bytes[0..8] == b"CFDRT002"
                || &bytes[0..8] == b"CFDRT003"
                || &bytes[0..8] == b"CFDRT004"
                || &bytes[0..8] == b"CFDRT005")
        {
            // Route-only payload (Phase-2/6B/ADR-043).
            let routes = RouteTable::decode(bytes)?;
            return Ok(Self { routes, waf: None });
        }
        if bytes.len() < 8 + 4 {
            return Err(CompositeError::Truncated);
        }
        if &bytes[0..8] != MAGIC {
            return Err(CompositeError::InvalidMagic);
        }
        let mut i = 8usize;
        let route_len = u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap()) as usize;
        i += 4;
        if i + route_len + 4 > bytes.len() {
            return Err(CompositeError::Truncated);
        }
        let route_bytes = &bytes[i..i + route_len];
        i += route_len;
        let routes = if route_bytes.is_empty() {
            RouteTable::default()
        } else {
            RouteTable::decode(route_bytes)?
        };
        let waf_len = u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap()) as usize;
        i += 4;
        if i + waf_len > bytes.len() {
            return Err(CompositeError::Truncated);
        }
        let waf_bytes = &bytes[i..i + waf_len];
        let waf = if waf_bytes.is_empty() {
            None
        } else {
            Some(CfdWafProjection::decode(waf_bytes)?)
        };
        Ok(Self { routes, waf })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::route_table::{BackendKind, CompiledRoute, CompiledUpstream};
    use crate::waf_proj::{CfdBuiltinToggles, CfdIpFilter, CfdWafRule};
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    #[test]
    fn composite_roundtrip() {
        let routes = RouteTable {
            upstreams: vec![CompiledUpstream {
                id: 1,
                connect: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 9000),
                authority_host: "upstream".into(),
            }],
            fcgi_pools: Vec::new(),
            static_policies: Vec::new(),
            routes: vec![CompiledRoute {
                route_id: 1,
                host: None,
                path: "/api".into(),
                backend_kind: BackendKind::Proxy,
                backend_id: 1,
            }],
        };
        let waf = CfdWafProjection {
            enabled: true,
            mode: 2,
            builtins: CfdBuiltinToggles::default(),
            ip_filter: CfdIpFilter::default(),
            rules: vec![CfdWafRule {
                id: "r1".into(),
                enabled: true,
                phases_mask: 1,
                target_kind: 0,
                header_name: String::new(),
                pattern: "/nope".into(),
                action: 2,
            }],
            exclusions: vec![],
        };
        let c = CompositeProjection {
            routes: routes.clone(),
            waf: Some(waf.clone()),
        };
        let b = c.encode().unwrap();
        let d = CompositeProjection::decode(&b).unwrap();
        assert_eq!(d.routes, routes);
        assert_eq!(d.waf, Some(waf));
    }

    #[test]
    fn legacy_route_only() {
        let routes = RouteTable {
            upstreams: vec![CompiledUpstream {
                id: 1,
                connect: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 9000),
                authority_host: "u".into(),
            }],
            fcgi_pools: Vec::new(),
            static_policies: Vec::new(),
            routes: vec![CompiledRoute {
                route_id: 1,
                host: None,
                path: "/api".into(),
                backend_kind: BackendKind::Proxy,
                backend_id: 1,
            }],
        };
        let raw = routes.encode().unwrap();
        let d = CompositeProjection::decode(&raw).unwrap();
        assert_eq!(d.routes, routes);
        assert!(d.waf.is_none());
    }
}

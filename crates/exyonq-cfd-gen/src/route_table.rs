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

//! Compiled routing projection payload (Phase 6B).
//!
//! Cap033 semantics mirrored (not product execution):
//! exact host > wildcard > hostless, then longest path prefix.
//!
//! ```text
//! CLIENT_HOST_POLICY = ROUTING_KEY_ONLY (Cap033 ranking)
//! UPSTREAM_HOST_POLICY = GET_REPLACE_WITH_PEER_AUTHORITY_HOST
//! MAGIC CFDRT005 — BackendKind + CompiledFcgiPool + static policies (ADR-044)
//! MAGIC CFDRT004 — BackendKind + CompiledFcgiPool + directory-index + front-controller (ADR-043)
//! MAGIC CFDRT003 — BackendKind + CompiledFcgiPool (decoded with empty DI/FC)
//! MAGIC CFDRT002 — proxy-only legacy (decoded into BackendKind::Proxy)
//! ```

use std::net::SocketAddr;
use std::path::PathBuf;

use thiserror::Error;

use crate::GenError;

const PAYLOAD_MAGIC_V5: &[u8; 8] = b"CFDRT005";
const PAYLOAD_MAGIC_V4: &[u8; 8] = b"CFDRT004";
const PAYLOAD_MAGIC_V3: &[u8; 8] = b"CFDRT003";
const PAYLOAD_MAGIC_V2: &[u8; 8] = b"CFDRT002";
const MAX_STR: usize = 1024;
const MAX_ENTRIES: usize = 4096;
/// Mirrors `module-api` Plan 11 bound (DIRECTORY_INDEX_SSOT).
pub const MAX_DIRECTORY_INDEX_CANDIDATES: usize = 32;
pub const MAX_DIRECTORY_INDEX_CANDIDATE_LEN: usize = 255;
pub const MAX_FRONT_CONTROLLER_TARGET_LEN: usize = 2048;

/// Backend kind on a compiled route.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    Proxy = 0,
    Fastcgi = 1,
    Reject = 2,
    Static = 3,
}

impl BackendKind {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Proxy),
            1 => Some(Self::Fastcgi),
            2 => Some(Self::Reject),
            3 => Some(Self::Static),
            _ => None,
        }
    }
}

/// FastCGI transport endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FcgiTransport {
    Unix(PathBuf),
    Tcp(SocketAddr),
}

/// Compiled front-controller policy (ADR-043; semantics aligned with Plan 11 overlay).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledFrontControllerPolicy {
    pub target_uri: String,
    pub require_not_file: bool,
    pub require_not_directory: bool,
    pub preserve_query: bool,
}

/// Compiled FastCGI pool metadata in the generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledFcgiPool {
    pub id: u32,
    pub transport: FcgiTransport,
    pub document_root: String,
    /// e.g. default index mapping helper field or empty.
    pub script_suffix: String,
    /// CFD supported project config requires `1` (ADR-042). Runtime uses this as a
    /// **per-shard idle retention cap** at `FcgiPoolMap::put` / `ensure_pool` shrink —
    /// not as concurrent busy+idle FastCGI capacity under serial `execute_get`.
    pub max_connections: u32,
    pub idle_timeout_ms: u32,
    pub connect_timeout_ms: u32,
    pub read_timeout_ms: u32,
    pub write_timeout_ms: u32,
    pub total_timeout_ms: u32,
    /// Basename-only DirectoryIndex candidates (ADR-043). Empty = no DI stage.
    pub directory_index: Vec<String>,
    /// Optional front-controller fallback (ADR-043).
    pub front_controller: Option<CompiledFrontControllerPolicy>,
}

/// Compiled static filesystem policy metadata in the generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledStaticPolicy {
    pub id: u32,
    pub document_root: String,
    /// Reserved ADR-044 bits. MVP supports only `0`.
    pub flags: u32,
    /// Basename-only DirectoryIndex candidates. Empty = no DI stage.
    pub index: Vec<String>,
}

/// Validate one directory-index candidate filename (basename only; ADR-043 / module-api parity).
pub fn validate_directory_index_candidate(name: &str) -> bool {
    if name.is_empty() || name.len() > MAX_DIRECTORY_INDEX_CANDIDATE_LEN {
        return false;
    }
    if name.starts_with('/')
        || name.contains("..")
        || name.contains('\0')
        || name.contains('\\')
        || name.contains('/')
        || name.contains('%')
        || name.contains('?')
        || name.contains('#')
    {
        return false;
    }
    true
}

/// Static directory-index basename: general DI rules plus MVP deny dotfiles and `.php`.
fn validate_static_index_candidate(name: &str) -> bool {
    validate_directory_index_candidate(name)
        && !name.starts_with('.')
        && !name.to_ascii_lowercase().ends_with(".php")
}

/// Validate a compiled front-controller target URI (local, literal, bounded).
pub fn validate_front_controller_target(target: &str) -> bool {
    if target.is_empty()
        || target.len() > MAX_FRONT_CONTROLLER_TARGET_LEN
        || !target.starts_with('/')
        || target == "/"
        || target.contains("..")
        || target.contains('\0')
        || target.contains('$')
        || target.contains('%')
        || target.contains('?')
        || target.contains('#')
        || target.starts_with("http://")
        || target.starts_with("https://")
    {
        return false;
    }
    // Executable classification for FastCGI FC: must end with .php (case-sensitive).
    target.ends_with(".php")
}

/// Join a directory URI ending in `/` with a safe candidate → child URI.
pub fn join_directory_index_uri(dir_uri: &str, candidate: &str) -> Option<String> {
    if !dir_uri.ends_with('/') || !validate_directory_index_candidate(candidate) {
        return None;
    }
    if dir_uri == "/" {
        Some(format!("/{candidate}"))
    } else {
        Some(format!("{dir_uri}{candidate}"))
    }
}

/// Upstream target in the compiled generation (proxy).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledUpstream {
    pub id: u32,
    /// TCP connect address (ip:port or hostname:port resolved by control at publish).
    pub connect: SocketAddr,
    /// Host header authority sent upstream on GET (peer authority).
    pub authority_host: String,
}

/// One Cap033-style route row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledRoute {
    pub route_id: u32,
    /// `None` = hostless; `Some("*.x")` = wildcard; else exact (normalized at encode).
    pub host: Option<String>,
    pub path: String,
    pub backend_kind: BackendKind,
    /// For Proxy: upstream id; for Fastcgi: pool id; for Reject: unused (0).
    pub backend_id: u32,
}

/// Lookup result against the compiled generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendTarget<'a> {
    Proxy(&'a CompiledUpstream),
    Fastcgi(&'a CompiledFcgiPool),
    Static(&'a CompiledStaticPolicy),
    Reject,
}

/// Immutable route table projected into a generation payload.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RouteTable {
    pub upstreams: Vec<CompiledUpstream>,
    pub fcgi_pools: Vec<CompiledFcgiPool>,
    pub static_policies: Vec<CompiledStaticPolicy>,
    pub routes: Vec<CompiledRoute>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RouteTableError {
    #[error("invalid route table magic")]
    InvalidMagic,
    #[error("truncated route table")]
    Truncated,
    #[error("string too long")]
    StringTooLong,
    #[error("too many entries")]
    TooManyEntries,
    #[error("unknown upstream id {0}")]
    UnknownUpstream(u32),
    #[error("unknown fcgi pool id {0}")]
    UnknownFcgiPool(u32),
    #[error("invalid socket addr")]
    InvalidAddr,
    #[error("invalid backend kind")]
    InvalidBackendKind,
    #[error("invalid fcgi transport")]
    InvalidTransport,
    #[error("script_suffix unsupported (ADR-041); must be empty")]
    UnsupportedScriptSuffix,
    #[error("invalid directory_index candidate")]
    InvalidDirectoryIndex,
    #[error("invalid front_controller policy")]
    InvalidFrontController,
    #[error("invalid static document_root")]
    InvalidStaticDocumentRoot,
    #[error("invalid static flags")]
    InvalidStaticFlags,
    #[error("unknown static policy id {0}")]
    UnknownStaticPolicy(u32),
    #[error("utf8")]
    Utf8,
}

impl From<RouteTableError> for GenError {
    fn from(e: RouteTableError) -> Self {
        GenError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            e.to_string(),
        ))
    }
}

impl RouteTable {
    pub fn encode(&self) -> Result<Vec<u8>, RouteTableError> {
        if self.upstreams.len() > MAX_ENTRIES
            || self.routes.len() > MAX_ENTRIES
            || self.fcgi_pools.len() > MAX_ENTRIES
            || self.static_policies.len() > MAX_ENTRIES
        {
            return Err(RouteTableError::TooManyEntries);
        }
        let mut buf = Vec::with_capacity(64 + self.routes.len() * 64);
        buf.extend_from_slice(PAYLOAD_MAGIC_V5);
        put_u16(&mut buf, self.upstreams.len() as u16);
        for u in &self.upstreams {
            put_u32(&mut buf, u.id);
            put_str(&mut buf, &u.connect.to_string())?;
            put_str(&mut buf, &u.authority_host)?;
        }
        put_u16(&mut buf, self.fcgi_pools.len() as u16);
        for p in &self.fcgi_pools {
            put_u32(&mut buf, p.id);
            encode_transport(&mut buf, &p.transport)?;
            put_str(&mut buf, &p.document_root)?;
            if !p.script_suffix.is_empty() {
                return Err(RouteTableError::UnsupportedScriptSuffix);
            }
            put_str(&mut buf, &p.script_suffix)?;
            put_u32(&mut buf, p.max_connections);
            put_u32(&mut buf, p.idle_timeout_ms);
            put_u32(&mut buf, p.connect_timeout_ms);
            put_u32(&mut buf, p.read_timeout_ms);
            put_u32(&mut buf, p.write_timeout_ms);
            put_u32(&mut buf, p.total_timeout_ms);
            encode_directory_index(&mut buf, &p.directory_index)?;
            encode_front_controller(&mut buf, p.front_controller.as_ref())?;
        }
        put_u16(&mut buf, self.static_policies.len() as u16);
        for policy in &self.static_policies {
            encode_static_policy(&mut buf, policy)?;
        }
        put_u16(&mut buf, self.routes.len() as u16);
        for r in &self.routes {
            if r.backend_kind == BackendKind::Static
                && !self.static_policies.iter().any(|p| p.id == r.backend_id)
            {
                return Err(RouteTableError::UnknownStaticPolicy(r.backend_id));
            }
            put_u32(&mut buf, r.route_id);
            match &r.host {
                None => put_str(&mut buf, "")?,
                Some(h) => put_str(&mut buf, &normalize_route_host(h))?,
            }
            put_str(&mut buf, &normalize_path(&r.path))?;
            buf.push(r.backend_kind as u8);
            put_u32(&mut buf, r.backend_id);
        }
        Ok(buf)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, RouteTableError> {
        if bytes.len() < 8 + 2 {
            return Err(RouteTableError::Truncated);
        }
        let magic = &bytes[0..8];
        if magic == PAYLOAD_MAGIC_V2 {
            return decode_v2_legacy(bytes);
        }
        let layout = if magic == PAYLOAD_MAGIC_V5 {
            RoutePayloadLayout::V5
        } else if magic == PAYLOAD_MAGIC_V4 {
            RoutePayloadLayout::V4
        } else if magic == PAYLOAD_MAGIC_V3 {
            RoutePayloadLayout::V3
        } else {
            return Err(RouteTableError::InvalidMagic);
        };
        let mut i = 8usize;
        let n_up = get_u16(bytes, &mut i)?;
        if n_up as usize > MAX_ENTRIES {
            return Err(RouteTableError::TooManyEntries);
        }
        let mut upstreams = Vec::with_capacity(n_up as usize);
        for _ in 0..n_up {
            let id = get_u32(bytes, &mut i)?;
            let connect_s = get_str(bytes, &mut i)?;
            let authority_host = get_str(bytes, &mut i)?;
            let connect: SocketAddr = connect_s
                .parse()
                .map_err(|_| RouteTableError::InvalidAddr)?;
            upstreams.push(CompiledUpstream {
                id,
                connect,
                authority_host,
            });
        }
        let n_fcgi = get_u16(bytes, &mut i)?;
        if n_fcgi as usize > MAX_ENTRIES {
            return Err(RouteTableError::TooManyEntries);
        }
        let mut fcgi_pools = Vec::with_capacity(n_fcgi as usize);
        for _ in 0..n_fcgi {
            let id = get_u32(bytes, &mut i)?;
            let transport = decode_transport(bytes, &mut i)?;
            let document_root = get_str(bytes, &mut i)?;
            let script_suffix = get_str(bytes, &mut i)?;
            if !script_suffix.is_empty() {
                return Err(RouteTableError::UnsupportedScriptSuffix);
            }
            let max_connections = get_u32(bytes, &mut i)?;
            let idle_timeout_ms = get_u32(bytes, &mut i)?;
            let connect_timeout_ms = get_u32(bytes, &mut i)?;
            let read_timeout_ms = get_u32(bytes, &mut i)?;
            let write_timeout_ms = get_u32(bytes, &mut i)?;
            let total_timeout_ms = get_u32(bytes, &mut i)?;
            let (directory_index, front_controller) = if layout.has_fcgi_routing_policy() {
                (
                    decode_directory_index(bytes, &mut i)?,
                    decode_front_controller(bytes, &mut i)?,
                )
            } else {
                (Vec::new(), None)
            };
            fcgi_pools.push(CompiledFcgiPool {
                id,
                transport,
                document_root,
                script_suffix,
                max_connections,
                idle_timeout_ms,
                connect_timeout_ms,
                read_timeout_ms,
                write_timeout_ms,
                total_timeout_ms,
                directory_index,
                front_controller,
            });
        }
        let static_policies = if layout.has_static_policy_section() {
            let n_static = get_u16(bytes, &mut i)?;
            if n_static as usize > MAX_ENTRIES {
                return Err(RouteTableError::TooManyEntries);
            }
            let mut policies = Vec::with_capacity(n_static as usize);
            for _ in 0..n_static {
                policies.push(decode_static_policy(bytes, &mut i)?);
            }
            policies
        } else {
            Vec::new()
        };
        let n_rt = get_u16(bytes, &mut i)?;
        if n_rt as usize > MAX_ENTRIES {
            return Err(RouteTableError::TooManyEntries);
        }
        let mut routes = Vec::with_capacity(n_rt as usize);
        for _ in 0..n_rt {
            let route_id = get_u32(bytes, &mut i)?;
            let host_s = get_str(bytes, &mut i)?;
            let path = get_str(bytes, &mut i)?;
            if i >= bytes.len() {
                return Err(RouteTableError::Truncated);
            }
            let kind = BackendKind::from_u8(bytes[i]).ok_or(RouteTableError::InvalidBackendKind)?;
            i += 1;
            let backend_id = get_u32(bytes, &mut i)?;
            if kind == BackendKind::Static {
                if !layout.has_static_policy_section() {
                    return Err(RouteTableError::InvalidBackendKind);
                }
                if !static_policies.iter().any(|p| p.id == backend_id) {
                    return Err(RouteTableError::UnknownStaticPolicy(backend_id));
                }
            }
            let host = if host_s.is_empty() {
                None
            } else {
                Some(host_s)
            };
            routes.push(CompiledRoute {
                route_id,
                host,
                path,
                backend_kind: kind,
                backend_id,
            });
        }
        if i != bytes.len() {
            return Err(RouteTableError::Truncated);
        }
        Ok(Self {
            upstreams,
            fcgi_pools,
            static_policies,
            routes,
        })
    }

    /// Cap033 host+path ranking. Returns `(route, backend target)`.
    pub fn lookup<'a>(
        &'a self,
        path: &str,
        host: Option<&str>,
    ) -> Option<(&'a CompiledRoute, BackendTarget<'a>)> {
        let path = normalize_path(path);
        let best = self
            .routes
            .iter()
            .filter(|r| path_under_route_prefix(&path, &r.path))
            .filter(|r| host_matches_route(r.host.as_deref(), host))
            .max_by_key(|r| (host_match_rank(r.host.as_deref()), r.path.len()))?;
        let target = match best.backend_kind {
            BackendKind::Proxy => {
                let up = self.upstreams.iter().find(|u| u.id == best.backend_id)?;
                BackendTarget::Proxy(up)
            }
            BackendKind::Fastcgi => {
                let pool = self.fcgi_pools.iter().find(|p| p.id == best.backend_id)?;
                BackendTarget::Fastcgi(pool)
            }
            BackendKind::Static => {
                let policy = self
                    .static_policies
                    .iter()
                    .find(|p| p.id == best.backend_id)?;
                BackendTarget::Static(policy)
            }
            BackendKind::Reject => BackendTarget::Reject,
        };
        Some((best, target))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RoutePayloadLayout {
    V3,
    V4,
    V5,
}

impl RoutePayloadLayout {
    fn has_fcgi_routing_policy(self) -> bool {
        match self {
            Self::V3 => false,
            Self::V4 | Self::V5 => true,
        }
    }

    fn has_static_policy_section(self) -> bool {
        match self {
            Self::V3 | Self::V4 => false,
            Self::V5 => true,
        }
    }
}

/// Decode CFDRT002 proxy-only legacy into CFDRT003-shaped table.
fn decode_v2_legacy(bytes: &[u8]) -> Result<RouteTable, RouteTableError> {
    let mut i = 8usize;
    let n_up = get_u16(bytes, &mut i)?;
    if n_up as usize > MAX_ENTRIES {
        return Err(RouteTableError::TooManyEntries);
    }
    let mut upstreams = Vec::with_capacity(n_up as usize);
    for _ in 0..n_up {
        let id = get_u32(bytes, &mut i)?;
        let connect_s = get_str(bytes, &mut i)?;
        let authority_host = get_str(bytes, &mut i)?;
        let connect: SocketAddr = connect_s
            .parse()
            .map_err(|_| RouteTableError::InvalidAddr)?;
        upstreams.push(CompiledUpstream {
            id,
            connect,
            authority_host,
        });
    }
    let n_rt = get_u16(bytes, &mut i)?;
    if n_rt as usize > MAX_ENTRIES {
        return Err(RouteTableError::TooManyEntries);
    }
    let mut routes = Vec::with_capacity(n_rt as usize);
    for _ in 0..n_rt {
        let route_id = get_u32(bytes, &mut i)?;
        let host_s = get_str(bytes, &mut i)?;
        let path = get_str(bytes, &mut i)?;
        let upstream_id = get_u32(bytes, &mut i)?;
        let host = if host_s.is_empty() {
            None
        } else {
            Some(host_s)
        };
        routes.push(CompiledRoute {
            route_id,
            host,
            path,
            backend_kind: BackendKind::Proxy,
            backend_id: upstream_id,
        });
    }
    if i != bytes.len() {
        return Err(RouteTableError::Truncated);
    }
    Ok(RouteTable {
        upstreams,
        fcgi_pools: Vec::new(),
        static_policies: Vec::new(),
        routes,
    })
}

fn encode_static_policy(
    buf: &mut Vec<u8>,
    policy: &CompiledStaticPolicy,
) -> Result<(), RouteTableError> {
    validate_static_policy(policy)?;
    put_u32(buf, policy.id);
    put_str(buf, &policy.document_root)?;
    put_u32(buf, policy.flags);
    encode_directory_index(buf, &policy.index)?;
    Ok(())
}

fn decode_static_policy(
    bytes: &[u8],
    i: &mut usize,
) -> Result<CompiledStaticPolicy, RouteTableError> {
    let policy = CompiledStaticPolicy {
        id: get_u32(bytes, i)?,
        document_root: get_str(bytes, i)?,
        flags: get_u32(bytes, i)?,
        index: decode_directory_index(bytes, i)?,
    };
    validate_static_policy(&policy)?;
    Ok(policy)
}

fn validate_static_policy(policy: &CompiledStaticPolicy) -> Result<(), RouteTableError> {
    if policy.document_root.is_empty()
        || !policy.document_root.starts_with('/')
        || policy.document_root.contains('\0')
    {
        return Err(RouteTableError::InvalidStaticDocumentRoot);
    }
    if policy.flags != 0 {
        return Err(RouteTableError::InvalidStaticFlags);
    }
    for candidate in &policy.index {
        if !validate_static_index_candidate(candidate) {
            return Err(RouteTableError::InvalidDirectoryIndex);
        }
    }
    Ok(())
}

fn encode_directory_index(buf: &mut Vec<u8>, names: &[String]) -> Result<(), RouteTableError> {
    if names.len() > MAX_DIRECTORY_INDEX_CANDIDATES {
        return Err(RouteTableError::InvalidDirectoryIndex);
    }
    put_u16(buf, names.len() as u16);
    for name in names {
        if !validate_directory_index_candidate(name) {
            return Err(RouteTableError::InvalidDirectoryIndex);
        }
        put_str(buf, name)?;
    }
    Ok(())
}

fn decode_directory_index(bytes: &[u8], i: &mut usize) -> Result<Vec<String>, RouteTableError> {
    let n = get_u16(bytes, i)? as usize;
    if n > MAX_DIRECTORY_INDEX_CANDIDATES {
        return Err(RouteTableError::InvalidDirectoryIndex);
    }
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let name = get_str(bytes, i)?;
        if !validate_directory_index_candidate(&name) {
            return Err(RouteTableError::InvalidDirectoryIndex);
        }
        out.push(name);
    }
    Ok(out)
}

fn encode_front_controller(
    buf: &mut Vec<u8>,
    fc: Option<&CompiledFrontControllerPolicy>,
) -> Result<(), RouteTableError> {
    match fc {
        None => {
            buf.push(0);
            Ok(())
        }
        Some(fc) => {
            if !validate_front_controller_target(&fc.target_uri)
                || !fc.require_not_file
                || !fc.require_not_directory
            {
                return Err(RouteTableError::InvalidFrontController);
            }
            buf.push(1);
            put_str(buf, &fc.target_uri)?;
            buf.push(u8::from(fc.require_not_file));
            buf.push(u8::from(fc.require_not_directory));
            buf.push(u8::from(fc.preserve_query));
            Ok(())
        }
    }
}

fn decode_front_controller(
    bytes: &[u8],
    i: &mut usize,
) -> Result<Option<CompiledFrontControllerPolicy>, RouteTableError> {
    if *i >= bytes.len() {
        return Err(RouteTableError::Truncated);
    }
    let tag = bytes[*i];
    *i += 1;
    match tag {
        0 => Ok(None),
        1 => {
            let target_uri = get_str(bytes, i)?;
            if *i + 3 > bytes.len() {
                return Err(RouteTableError::Truncated);
            }
            let require_not_file = bytes[*i] != 0;
            let require_not_directory = bytes[*i + 1] != 0;
            let preserve_query = bytes[*i + 2] != 0;
            *i += 3;
            let fc = CompiledFrontControllerPolicy {
                target_uri,
                require_not_file,
                require_not_directory,
                preserve_query,
            };
            if !validate_front_controller_target(&fc.target_uri)
                || !fc.require_not_file
                || !fc.require_not_directory
            {
                return Err(RouteTableError::InvalidFrontController);
            }
            Ok(Some(fc))
        }
        _ => Err(RouteTableError::InvalidFrontController),
    }
}

fn encode_transport(buf: &mut Vec<u8>, t: &FcgiTransport) -> Result<(), RouteTableError> {
    match t {
        FcgiTransport::Unix(path) => {
            buf.push(0);
            put_str(buf, &path.to_string_lossy())?;
        }
        FcgiTransport::Tcp(addr) => {
            buf.push(1);
            put_str(buf, &addr.to_string())?;
        }
    }
    Ok(())
}

fn decode_transport(bytes: &[u8], i: &mut usize) -> Result<FcgiTransport, RouteTableError> {
    if *i >= bytes.len() {
        return Err(RouteTableError::Truncated);
    }
    let tag = bytes[*i];
    *i += 1;
    let s = get_str(bytes, i)?;
    match tag {
        0 => Ok(FcgiTransport::Unix(PathBuf::from(s))),
        1 => {
            let addr: SocketAddr = s.parse().map_err(|_| RouteTableError::InvalidAddr)?;
            Ok(FcgiTransport::Tcp(addr))
        }
        _ => Err(RouteTableError::InvalidTransport),
    }
}

fn host_match_rank(configured: Option<&str>) -> u8 {
    match configured {
        Some(h) if h.starts_with('*') => 1,
        Some(_) => 2,
        None => 0,
    }
}

fn host_matches_route(configured: Option<&str>, host: Option<&str>) -> bool {
    match (configured, host) {
        (None, _) => true,
        (Some(expected), Some(actual)) => host_matches(expected, actual),
        (Some(_), None) => false,
    }
}

fn host_matches(expected: &str, actual: &str) -> bool {
    let actual_n = normalize_route_host(actual);
    if let Some(suffix) = expected.strip_prefix('*') {
        let suffix_n = normalize_route_host(suffix);
        return actual_n.ends_with(&suffix_n);
    }
    normalize_route_host(expected) == actual_n
}

/// Cap033: DNS labels case-insensitive; trailing FQDN dots equivalent.
pub fn normalize_route_host(host: &str) -> String {
    host.trim_end_matches('.').to_ascii_lowercase()
}

pub fn normalize_path(path: &str) -> String {
    if path.is_empty() {
        return "/".to_string();
    }
    if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    }
}

fn path_under_route_prefix(path: &str, route_path: &str) -> bool {
    let prefix = route_path.trim_end_matches('/');
    if prefix.is_empty() {
        return path.starts_with('/');
    }
    path == prefix || path.starts_with(&format!("{prefix}/"))
}

fn put_u16(buf: &mut Vec<u8>, v: u16) {
    buf.extend_from_slice(&v.to_le_bytes());
}
fn put_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}
fn put_str(buf: &mut Vec<u8>, s: &str) -> Result<(), RouteTableError> {
    if s.len() > MAX_STR {
        return Err(RouteTableError::StringTooLong);
    }
    put_u16(buf, s.len() as u16);
    buf.extend_from_slice(s.as_bytes());
    Ok(())
}
fn get_u16(bytes: &[u8], i: &mut usize) -> Result<u16, RouteTableError> {
    if *i + 2 > bytes.len() {
        return Err(RouteTableError::Truncated);
    }
    let v = u16::from_le_bytes(bytes[*i..*i + 2].try_into().unwrap());
    *i += 2;
    Ok(v)
}
fn get_u32(bytes: &[u8], i: &mut usize) -> Result<u32, RouteTableError> {
    if *i + 4 > bytes.len() {
        return Err(RouteTableError::Truncated);
    }
    let v = u32::from_le_bytes(bytes[*i..*i + 4].try_into().unwrap());
    *i += 4;
    Ok(v)
}
fn get_str(bytes: &[u8], i: &mut usize) -> Result<String, RouteTableError> {
    let len = get_u16(bytes, i)? as usize;
    if len > MAX_STR {
        return Err(RouteTableError::StringTooLong);
    }
    if *i + len > bytes.len() {
        return Err(RouteTableError::Truncated);
    }
    let s = std::str::from_utf8(&bytes[*i..*i + len]).map_err(|_| RouteTableError::Utf8)?;
    *i += len;
    Ok(s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    fn sample() -> RouteTable {
        let a = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 9001);
        let b = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 9002);
        let c = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 9003);
        RouteTable {
            upstreams: vec![
                CompiledUpstream {
                    id: 1,
                    connect: a,
                    authority_host: "127.0.0.1".into(),
                },
                CompiledUpstream {
                    id: 2,
                    connect: b,
                    authority_host: "127.0.0.1".into(),
                },
                CompiledUpstream {
                    id: 3,
                    connect: c,
                    authority_host: "127.0.0.1".into(),
                },
            ],
            fcgi_pools: Vec::new(),
            static_policies: Vec::new(),
            routes: vec![
                CompiledRoute {
                    route_id: 10,
                    host: Some("host-a.example".into()),
                    path: "/a".into(),
                    backend_kind: BackendKind::Proxy,
                    backend_id: 1,
                },
                CompiledRoute {
                    route_id: 11,
                    host: Some("host-a.example".into()),
                    path: "/b".into(),
                    backend_kind: BackendKind::Proxy,
                    backend_id: 2,
                },
                CompiledRoute {
                    route_id: 12,
                    host: Some("host-b.example".into()),
                    path: "/a".into(),
                    backend_kind: BackendKind::Proxy,
                    backend_id: 3,
                },
            ],
        }
    }

    #[test]
    fn roundtrip_and_matrix() {
        let t = sample();
        let bytes = t.encode().unwrap();
        assert_eq!(&bytes[0..8], b"CFDRT005");
        let d = RouteTable::decode(&bytes).unwrap();
        assert_eq!(d, t);
        match d.lookup("/a", Some("host-a.example")).unwrap() {
            (r, BackendTarget::Proxy(u)) => {
                assert_eq!(r.route_id, 10);
                assert_eq!(u.id, 1);
            }
            _ => panic!("expected proxy"),
        }
        assert!(d.lookup("/a", Some("unknown.example")).is_none());
    }

    #[test]
    fn fcgi_pool_roundtrip_and_lookup() {
        let t = RouteTable {
            upstreams: Vec::new(),
            fcgi_pools: vec![CompiledFcgiPool {
                id: 7,
                transport: FcgiTransport::Tcp(SocketAddr::new(
                    IpAddr::V4(Ipv4Addr::LOCALHOST),
                    9000,
                )),
                document_root: "/var/www".into(),
                script_suffix: String::new(),
                max_connections: 8,
                idle_timeout_ms: 60_000,
                connect_timeout_ms: 2_000,
                read_timeout_ms: 120_000,
                write_timeout_ms: 60_000,
                total_timeout_ms: 180_000,
                directory_index: Vec::new(),
                front_controller: None,
            }],
            static_policies: Vec::new(),
            routes: vec![CompiledRoute {
                route_id: 1,
                host: None,
                path: "/app".into(),
                backend_kind: BackendKind::Fastcgi,
                backend_id: 7,
            }],
        };
        let d = RouteTable::decode(&t.encode().unwrap()).unwrap();
        match d.lookup("/app/index.php", None).unwrap().1 {
            BackendTarget::Fastcgi(p) => assert_eq!(p.id, 7),
            _ => panic!("expected fcgi"),
        }
    }

    #[test]
    fn fcgi_pool_rejects_nonempty_script_suffix() {
        let t = RouteTable {
            upstreams: Vec::new(),
            fcgi_pools: vec![CompiledFcgiPool {
                id: 7,
                transport: FcgiTransport::Tcp(SocketAddr::new(
                    IpAddr::V4(Ipv4Addr::LOCALHOST),
                    9000,
                )),
                document_root: "/var/www".into(),
                script_suffix: ".php".into(),
                max_connections: 8,
                idle_timeout_ms: 60_000,
                connect_timeout_ms: 2_000,
                read_timeout_ms: 120_000,
                write_timeout_ms: 60_000,
                total_timeout_ms: 180_000,
                directory_index: Vec::new(),
                front_controller: None,
            }],
            static_policies: Vec::new(),
            routes: Vec::new(),
        };
        assert!(matches!(
            t.encode(),
            Err(RouteTableError::UnsupportedScriptSuffix)
        ));
    }

    #[test]
    fn legacy_cfdrt002_decodes_as_proxy() {
        // Manually build CFDRT002 bytes via temporary encode of v2 shape.
        let a = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 1);
        let mut buf = Vec::new();
        buf.extend_from_slice(b"CFDRT002");
        put_u16(&mut buf, 1);
        put_u32(&mut buf, 1);
        put_str(&mut buf, &a.to_string()).unwrap();
        put_str(&mut buf, "h").unwrap();
        put_u16(&mut buf, 1);
        put_u32(&mut buf, 1);
        put_str(&mut buf, "h").unwrap();
        put_str(&mut buf, "/api").unwrap();
        put_u32(&mut buf, 1);
        let t = RouteTable::decode(&buf).unwrap();
        assert!(t.fcgi_pools.is_empty());
        assert!(t.static_policies.is_empty());
        assert_eq!(t.routes[0].backend_kind, BackendKind::Proxy);
        assert_eq!(t.routes[0].backend_id, 1);
    }

    #[test]
    fn static_policy_roundtrip_and_lookup() {
        let t = RouteTable {
            upstreams: Vec::new(),
            fcgi_pools: Vec::new(),
            static_policies: vec![CompiledStaticPolicy {
                id: 17,
                document_root: "/srv/www".into(),
                flags: 0,
                index: vec!["index.html".into(), "index.htm".into()],
            }],
            routes: vec![CompiledRoute {
                route_id: 42,
                host: Some("STATIC.EXAMPLE".into()),
                path: "/assets".into(),
                backend_kind: BackendKind::Static,
                backend_id: 17,
            }],
        };
        let encoded = t.encode().unwrap();
        assert_eq!(&encoded[0..8], b"CFDRT005");
        let decoded = RouteTable::decode(&encoded).unwrap();
        assert_eq!(
            decoded.static_policies[0].index,
            ["index.html", "index.htm"]
        );
        match decoded
            .lookup("/assets/logo.png", Some("static.example"))
            .unwrap()
            .1
        {
            BackendTarget::Static(policy) => assert_eq!(policy.document_root, "/srv/www"),
            other => panic!("expected static policy, got {other:?}"),
        }
    }

    #[test]
    fn cfdrt005_decodes_proxy_fastcgi_static_handlers() {
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 9000);
        let t = RouteTable {
            upstreams: vec![CompiledUpstream {
                id: 1,
                connect: addr,
                authority_host: "upstream".into(),
            }],
            fcgi_pools: vec![CompiledFcgiPool {
                id: 2,
                transport: FcgiTransport::Tcp(addr),
                document_root: "/srv/php".into(),
                script_suffix: String::new(),
                max_connections: 1,
                idle_timeout_ms: 60_000,
                connect_timeout_ms: 2_000,
                read_timeout_ms: 120_000,
                write_timeout_ms: 60_000,
                total_timeout_ms: 180_000,
                directory_index: Vec::new(),
                front_controller: None,
            }],
            static_policies: vec![CompiledStaticPolicy {
                id: 3,
                document_root: "/srv/static".into(),
                flags: 0,
                index: vec!["index.html".into()],
            }],
            routes: vec![
                CompiledRoute {
                    route_id: 1,
                    host: None,
                    path: "/api".into(),
                    backend_kind: BackendKind::Proxy,
                    backend_id: 1,
                },
                CompiledRoute {
                    route_id: 2,
                    host: None,
                    path: "/php".into(),
                    backend_kind: BackendKind::Fastcgi,
                    backend_id: 2,
                },
                CompiledRoute {
                    route_id: 3,
                    host: None,
                    path: "/static".into(),
                    backend_kind: BackendKind::Static,
                    backend_id: 3,
                },
            ],
        };
        let decoded = RouteTable::decode(&t.encode().unwrap()).unwrap();
        assert!(matches!(
            decoded.lookup("/api/users", None).unwrap().1,
            BackendTarget::Proxy(_)
        ));
        assert!(matches!(
            decoded.lookup("/php/index.php", None).unwrap().1,
            BackendTarget::Fastcgi(_)
        ));
        assert!(matches!(
            decoded.lookup("/static/app.css", None).unwrap().1,
            BackendTarget::Static(_)
        ));
    }

    #[test]
    fn static_policy_validation_rejects_bad_inputs() {
        let mut t = RouteTable {
            upstreams: Vec::new(),
            fcgi_pools: Vec::new(),
            static_policies: vec![CompiledStaticPolicy {
                id: 1,
                document_root: String::new(),
                flags: 0,
                index: Vec::new(),
            }],
            routes: Vec::new(),
        };
        assert_eq!(
            t.encode().unwrap_err(),
            RouteTableError::InvalidStaticDocumentRoot
        );

        t.static_policies[0].document_root = "relative".into();
        assert_eq!(
            t.encode().unwrap_err(),
            RouteTableError::InvalidStaticDocumentRoot
        );

        t.static_policies[0].document_root = "/srv/www".into();
        t.static_policies[0].flags = 1;
        assert_eq!(t.encode().unwrap_err(), RouteTableError::InvalidStaticFlags);

        t.static_policies[0].flags = 0;
        t.static_policies[0].index = vec!["../index.html".into()];
        assert_eq!(
            t.encode().unwrap_err(),
            RouteTableError::InvalidDirectoryIndex
        );

        t.static_policies[0].index = vec!["index.php".into()];
        assert_eq!(
            t.encode().unwrap_err(),
            RouteTableError::InvalidDirectoryIndex
        );
    }

    #[test]
    fn static_routes_require_known_policy() {
        let t = RouteTable {
            upstreams: Vec::new(),
            fcgi_pools: Vec::new(),
            static_policies: Vec::new(),
            routes: vec![CompiledRoute {
                route_id: 1,
                host: None,
                path: "/static".into(),
                backend_kind: BackendKind::Static,
                backend_id: 999,
            }],
        };
        assert_eq!(
            t.encode().unwrap_err(),
            RouteTableError::UnknownStaticPolicy(999)
        );
    }

    #[test]
    fn legacy_cfdrt004_decodes_with_empty_static_policies() {
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 9000);
        let mut buf = Vec::new();
        buf.extend_from_slice(b"CFDRT004");
        put_u16(&mut buf, 0);
        put_u16(&mut buf, 1);
        put_u32(&mut buf, 7);
        encode_transport(&mut buf, &FcgiTransport::Tcp(addr)).unwrap();
        put_str(&mut buf, "/srv/www").unwrap();
        put_str(&mut buf, "").unwrap();
        put_u32(&mut buf, 1);
        put_u32(&mut buf, 60_000);
        put_u32(&mut buf, 2_000);
        put_u32(&mut buf, 120_000);
        put_u32(&mut buf, 60_000);
        put_u32(&mut buf, 180_000);
        encode_directory_index(&mut buf, &["index.php".to_owned()]).unwrap();
        encode_front_controller(&mut buf, None).unwrap();
        put_u16(&mut buf, 1);
        put_u32(&mut buf, 1);
        put_str(&mut buf, "").unwrap();
        put_str(&mut buf, "/app").unwrap();
        buf.push(BackendKind::Fastcgi as u8);
        put_u32(&mut buf, 7);

        let decoded = RouteTable::decode(&buf).unwrap();
        assert!(decoded.static_policies.is_empty());
        assert_eq!(decoded.fcgi_pools[0].directory_index, ["index.php"]);
    }

    #[test]
    fn legacy_cfdrt004_rejects_static_backend_kind() {
        let mut buf = Vec::new();
        buf.extend_from_slice(b"CFDRT004");
        put_u16(&mut buf, 0);
        put_u16(&mut buf, 0);
        put_u16(&mut buf, 1);
        put_u32(&mut buf, 1);
        put_str(&mut buf, "").unwrap();
        put_str(&mut buf, "/static").unwrap();
        buf.push(BackendKind::Static as u8);
        put_u32(&mut buf, 1);

        assert_eq!(
            RouteTable::decode(&buf).unwrap_err(),
            RouteTableError::InvalidBackendKind
        );
    }

    #[test]
    fn cfdrt005_decode_rejects_nonzero_static_flags() {
        let mut buf = Vec::new();
        buf.extend_from_slice(b"CFDRT005");
        put_u16(&mut buf, 0);
        put_u16(&mut buf, 0);
        put_u16(&mut buf, 1);
        put_u32(&mut buf, 1);
        put_str(&mut buf, "/srv/www").unwrap();
        put_u32(&mut buf, 1);
        put_u16(&mut buf, 0);
        put_u16(&mut buf, 0);

        assert_eq!(
            RouteTable::decode(&buf).unwrap_err(),
            RouteTableError::InvalidStaticFlags
        );
    }

    #[test]
    fn longest_prefix_wins() {
        let a = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 1);
        let t = RouteTable {
            upstreams: vec![
                CompiledUpstream {
                    id: 1,
                    connect: a,
                    authority_host: "h".into(),
                },
                CompiledUpstream {
                    id: 2,
                    connect: a,
                    authority_host: "h".into(),
                },
            ],
            fcgi_pools: Vec::new(),
            static_policies: Vec::new(),
            routes: vec![
                CompiledRoute {
                    route_id: 1,
                    host: None,
                    path: "/api".into(),
                    backend_kind: BackendKind::Proxy,
                    backend_id: 1,
                },
                CompiledRoute {
                    route_id: 2,
                    host: None,
                    path: "/api/v2".into(),
                    backend_kind: BackendKind::Proxy,
                    backend_id: 2,
                },
            ],
        };
        assert_eq!(t.lookup("/api/v2/x", None).unwrap().0.route_id, 2);
        assert!(t.lookup("/api2", None).is_none());
    }
}

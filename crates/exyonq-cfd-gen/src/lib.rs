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

//! Immutable generation blob for Competitive Frontier dataplane.
//!
//! On-disk layout (little-endian):
//! ```text
//! magic[8] = b"EXYQCFD1"
//! schema_version: u32
//! generation_id: u64
//! published_unix_ms: u64
//! payload_len: u32
//! payload: [u8; payload_len]   # RouteTable CFDRT002..005; Phase 3: CFDCOM01
//! crc32: u32   # IEEE over all preceding bytes
//! ```

mod composite;
mod route_table;
mod waf_proj;

pub use composite::{CompositeError, CompositeProjection};
pub use route_table::{
    join_directory_index_uri, normalize_path, normalize_route_host,
    validate_directory_index_candidate, validate_front_controller_target, BackendKind,
    BackendTarget, CompiledFcgiPool, CompiledFrontControllerPolicy, CompiledRoute,
    CompiledStaticPolicy, CompiledUpstream, FcgiTransport, RouteTable, RouteTableError,
    MAX_DIRECTORY_INDEX_CANDIDATES, MAX_DIRECTORY_INDEX_CANDIDATE_LEN,
    MAX_FRONT_CONTROLLER_TARGET_LEN,
};
pub use waf_proj::{
    CfdBuiltinToggles, CfdIpFilter, CfdToggle, CfdWafExclusion, CfdWafProjection, CfdWafRule,
    WafProjError,
};

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use thiserror::Error;

/// On-wire / on-disk schema version for control↔dataplane compatibility.
/// Bumped to 2 for Phase 6B FastCGI route IR (old CFD fails closed).
pub const SCHEMA_VERSION: u32 = 2;
const MAGIC: &[u8; 8] = b"EXYQCFD1";
const MAX_PAYLOAD: u32 = 1024 * 1024;

/// Immutable generation (identity + compiled payload; Phase 2 = RouteTable bytes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Generation {
    pub schema_version: u32,
    pub generation_id: u64,
    pub published_unix_ms: u64,
    pub payload: Vec<u8>,
}

#[derive(Debug, Error)]
pub enum GenError {
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("invalid magic")]
    InvalidMagic,
    #[error("incompatible schema version {got} (need {SCHEMA_VERSION})")]
    IncompatibleSchema { got: u32 },
    #[error("payload too large ({0})")]
    PayloadTooLarge(u32),
    #[error("truncated generation blob")]
    Truncated,
    #[error("crc mismatch")]
    CrcMismatch,
    #[error("generation id must be non-zero")]
    ZeroGenerationId,
    #[error("composite payload: {0}")]
    Composite(String),
}

impl Generation {
    pub fn new(generation_id: u64, payload: Vec<u8>) -> Result<Self, GenError> {
        if generation_id == 0 {
            return Err(GenError::ZeroGenerationId);
        }
        if payload.len() as u32 > MAX_PAYLOAD {
            return Err(GenError::PayloadTooLarge(payload.len() as u32));
        }
        let published_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        Ok(Self {
            schema_version: SCHEMA_VERSION,
            generation_id,
            published_unix_ms,
            payload,
        })
    }

    /// Publish a generation whose payload is a compiled [`RouteTable`] (WAF off).
    pub fn from_route_table(generation_id: u64, table: &RouteTable) -> Result<Self, GenError> {
        let payload = table.encode().map_err(GenError::from)?;
        Self::new(generation_id, payload)
    }

    /// Publish routes + optional WAF projection (Phase 3 composite).
    pub fn from_composite(
        generation_id: u64,
        composite: &CompositeProjection,
    ) -> Result<Self, GenError> {
        let payload = composite
            .encode()
            .map_err(|e| GenError::Composite(e.to_string()))?;
        Self::new(generation_id, payload)
    }

    /// Decode composite (routes + optional WAF). Accepts legacy CFDRT002.
    pub fn composite(&self) -> Result<Option<CompositeProjection>, CompositeError> {
        if self.payload.is_empty() {
            return Ok(None);
        }
        CompositeProjection::decode(&self.payload).map(Some)
    }

    /// Decode payload as RouteTable when present (Phase 2 / Phase 3 routes section).
    ///
    /// - Empty payload → `Ok(None)`.
    /// - Unknown magic → `Ok(None)`.
    /// - Known magic but undecodable route → `Err`.
    pub fn route_table(&self) -> Result<Option<RouteTable>, RouteTableError> {
        if self.payload.is_empty() {
            return Ok(None);
        }
        match CompositeProjection::decode(&self.payload) {
            Ok(c) => Ok(Some(c.routes)),
            Err(CompositeError::InvalidMagic) => Ok(None),
            Err(CompositeError::Route(e)) => Err(e),
            Err(CompositeError::Truncated) => Err(RouteTableError::Truncated),
            Err(CompositeError::EmptyRoute) => Ok(Some(RouteTable::default())),
            Err(CompositeError::Waf(_)) => Err(RouteTableError::Truncated),
        }
    }

    pub fn encode(&self) -> Result<Vec<u8>, GenError> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(GenError::IncompatibleSchema {
                got: self.schema_version,
            });
        }
        if self.payload.len() as u32 > MAX_PAYLOAD {
            return Err(GenError::PayloadTooLarge(self.payload.len() as u32));
        }
        let mut buf = Vec::with_capacity(8 + 4 + 8 + 8 + 4 + self.payload.len() + 4);
        buf.extend_from_slice(MAGIC);
        buf.extend_from_slice(&self.schema_version.to_le_bytes());
        buf.extend_from_slice(&self.generation_id.to_le_bytes());
        buf.extend_from_slice(&self.published_unix_ms.to_le_bytes());
        let plen = self.payload.len() as u32;
        buf.extend_from_slice(&plen.to_le_bytes());
        buf.extend_from_slice(&self.payload);
        let crc = crc32(&buf);
        buf.extend_from_slice(&crc.to_le_bytes());
        Ok(buf)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, GenError> {
        if bytes.len() < 8 + 4 + 8 + 8 + 4 + 4 {
            return Err(GenError::Truncated);
        }
        if &bytes[0..8] != MAGIC {
            return Err(GenError::InvalidMagic);
        }
        let schema_version = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        if schema_version != SCHEMA_VERSION {
            return Err(GenError::IncompatibleSchema {
                got: schema_version,
            });
        }
        let generation_id = u64::from_le_bytes(bytes[12..20].try_into().unwrap());
        let published_unix_ms = u64::from_le_bytes(bytes[20..28].try_into().unwrap());
        let payload_len = u32::from_le_bytes(bytes[28..32].try_into().unwrap());
        if payload_len > MAX_PAYLOAD {
            return Err(GenError::PayloadTooLarge(payload_len));
        }
        let body_end = 32 + payload_len as usize;
        let crc_end = body_end + 4;
        if bytes.len() != crc_end {
            return Err(GenError::Truncated);
        }
        let expect = u32::from_le_bytes(bytes[body_end..crc_end].try_into().unwrap());
        let got = crc32(&bytes[..body_end]);
        if expect != got {
            return Err(GenError::CrcMismatch);
        }
        if generation_id == 0 {
            return Err(GenError::ZeroGenerationId);
        }
        Ok(Self {
            schema_version,
            generation_id,
            published_unix_ms,
            payload: bytes[32..body_end].to_vec(),
        })
    }
}

/// Directory layout for atomic generation publication.
#[derive(Debug, Clone)]
pub struct GenDir {
    root: PathBuf,
}

impl GenDir {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn generation_path(&self) -> PathBuf {
        self.root.join("generation.bin")
    }

    pub fn tmp_path(&self) -> PathBuf {
        self.root.join("generation.bin.tmp")
    }

    pub fn status_path(&self) -> PathBuf {
        self.root.join("status")
    }

    pub fn ensure(&self) -> io::Result<()> {
        fs::create_dir_all(&self.root)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = fs::metadata(&self.root)?;
            let mut perms = meta.permissions();
            perms.set_mode(0o700);
            fs::set_permissions(&self.root, perms)?;
        }
        Ok(())
    }

    /// Atomic publish: write tmp → sync → rename over `generation.bin`.
    pub fn publish(&self, gen: &Generation) -> Result<(), GenError> {
        self.ensure()?;
        let bytes = gen.encode()?;
        let tmp = self.tmp_path();
        let final_path = self.generation_path();
        {
            let mut f = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&tmp)?;
            f.write_all(&bytes)?;
            f.sync_all()?;
        }
        fs::rename(&tmp, &final_path)?;
        // Best-effort directory sync for durability on crash mid-rename.
        if let Ok(dir) = File::open(&self.root) {
            let _ = dir.sync_all();
        }
        Ok(())
    }

    pub fn load(&self) -> Result<Generation, GenError> {
        let mut f = File::open(self.generation_path())?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;
        Generation::decode(&buf)
    }
}

/// IEEE CRC-32 (same polynomial as PNG/ZIP).
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xffff_ffff;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            let mask = (!(crc & 1)).wrapping_add(1);
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn roundtrip_encode_decode() {
        let g = Generation::new(7, b"phase1".to_vec()).unwrap();
        let bytes = g.encode().unwrap();
        let d = Generation::decode(&bytes).unwrap();
        assert_eq!(d.generation_id, 7);
        assert_eq!(d.payload, b"phase1");
        assert_eq!(d.schema_version, SCHEMA_VERSION);
    }

    #[test]
    fn corrupt_crc_rejected() {
        let g = Generation::new(1, b"x".to_vec()).unwrap();
        let mut bytes = g.encode().unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        assert!(matches!(
            Generation::decode(&bytes),
            Err(GenError::CrcMismatch)
        ));
    }

    #[test]
    fn incompatible_schema_rejected() {
        let g = Generation::new(1, Vec::new()).unwrap();
        let mut bytes = g.encode().unwrap();
        bytes[8..12].copy_from_slice(&99u32.to_le_bytes());
        // Fix CRC so schema check is what fails.
        let body_end = bytes.len() - 4;
        let c = crc32(&bytes[..body_end]);
        bytes[body_end..].copy_from_slice(&c.to_le_bytes());
        assert!(matches!(
            Generation::decode(&bytes),
            Err(GenError::IncompatibleSchema { got: 99 })
        ));
    }

    #[test]
    fn atomic_publish_load() {
        let dir = tempdir().unwrap();
        let gd = GenDir::new(dir.path());
        let g1 = Generation::new(1, b"g1".to_vec()).unwrap();
        gd.publish(&g1).unwrap();
        let loaded = gd.load().unwrap();
        assert_eq!(loaded.generation_id, 1);
        let g2 = Generation::new(2, b"g2".to_vec()).unwrap();
        gd.publish(&g2).unwrap();
        assert_eq!(gd.load().unwrap().generation_id, 2);
    }
}

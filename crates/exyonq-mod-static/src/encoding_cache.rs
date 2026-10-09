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
//! Cap067 static encoding cache (ADR-046 Option A).
//!
//! Compress-once into a managed directory; Cap067 sendfiles coded bytes with
//! `Content-Encoding` + `Vary` — no hot-path zlib on hit.
//!
//! Default OFF. Env `EXYONQ_STATIC_ENCODING_CACHE=1` forces enable; `=0` forces off.

use crate::conditional::{StaticValidators, ValidatorIdentity};
use crate::sendfile::SendfileAsset;
use crate::wire;
use brotli::enc::backward_references::BrotliEncoderMode;
use brotli::enc::{BrotliCompress, BrotliEncoderParams};
use flate2::write::GzEncoder;
use flate2::Compression;
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// Default managed cache directory (matches IR).
pub const DEFAULT_CACHE_DIR: &str = "/var/cache/exyonq/static-encoding";
/// OLS-parity default compression level for static cache objects.
pub const DEFAULT_LEVEL: u32 = 6;
/// Minimum source size (bytes) before cache engages.
pub const DEFAULT_MIN_BYTES: u64 = 300;
/// Maximum source size (bytes) accepted into the cache.
pub const DEFAULT_MAX_BYTES: u64 = 10_485_760;
/// Default max coded objects retained in the cache directory.
pub const DEFAULT_MAX_ENTRIES: u64 = 4096;
/// Default max total bytes of coded objects (1 GiB).
pub const DEFAULT_MAX_TOTAL_BYTES: u64 = 1_073_741_824;
/// Cap concurrent A1 miss compressions (CPU DoS bound).
const MAX_MISS_INFLIGHT: usize = 2;

static MISS_INFLIGHT: AtomicUsize = AtomicUsize::new(0);

/// Content coding offered by the static encoding cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CacheCoding {
    Gzip,
    Brotli,
}

impl CacheCoding {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Gzip => "gzip",
            Self::Brotli => "br",
        }
    }

    pub fn etag_suffix(self) -> &'static str {
        match self {
            Self::Gzip => "gzip",
            Self::Brotli => "br",
        }
    }

    /// Cap067 FD-cache discriminant (must match `sendfile_fd_cache::CODING_*`).
    pub fn fd_cache_tag(self) -> u8 {
        match self {
            Self::Gzip => 1,
            Self::Brotli => 2,
        }
    }
}

/// Runtime config for Cap067 static encoding cache (mirrors IR; no config-ir dep).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodingCacheConfig {
    pub enabled: bool,
    pub cache_dir: PathBuf,
    pub level: u32,
    pub min_bytes: u64,
    pub max_bytes: u64,
    pub max_entries: u64,
    pub max_total_bytes: u64,
    pub gzip: bool,
    pub brotli: bool,
}

impl Default for EncodingCacheConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            cache_dir: PathBuf::from(DEFAULT_CACHE_DIR),
            level: DEFAULT_LEVEL,
            min_bytes: DEFAULT_MIN_BYTES,
            max_bytes: DEFAULT_MAX_BYTES,
            max_entries: DEFAULT_MAX_ENTRIES,
            max_total_bytes: DEFAULT_MAX_TOTAL_BYTES,
            gzip: true,
            brotli: false,
        }
    }
}

static INSTALLED: OnceLock<EncodingCacheConfig> = OnceLock::new();

/// Install config from composition root (CLI / tests). First call wins for process lifetime.
pub fn install_encoding_cache_config(cfg: EncodingCacheConfig) {
    let _ = INSTALLED.set(cfg);
}

/// Test helper: replace installed config (or seed defaults) by rebuilding from env+arg.
#[cfg(test)]
pub fn install_encoding_cache_config_for_tests(cfg: EncodingCacheConfig) {
    // OnceLock cannot reset; tests that need custom cfg pass it explicitly to helpers.
    let _ = INSTALLED.set(cfg);
}

/// Effective config: installed IR (or defaults) with env overrides.
pub fn effective_config() -> EncodingCacheConfig {
    let mut cfg = INSTALLED.get().cloned().unwrap_or_default();
    apply_env_overrides(&mut cfg);
    cfg
}

fn apply_env_overrides(cfg: &mut EncodingCacheConfig) {
    if let Ok(v) = std::env::var("EXYONQ_STATIC_ENCODING_CACHE") {
        if env_flag_truthy(&v) {
            cfg.enabled = true;
        } else if env_flag_falsy(&v) {
            cfg.enabled = false;
        }
    }
    if let Ok(dir) = std::env::var("EXYONQ_STATIC_ENCODING_CACHE_DIR") {
        if !dir.is_empty() {
            cfg.cache_dir = PathBuf::from(dir);
        }
    }
}

fn env_flag_truthy(v: &str) -> bool {
    matches!(
        v.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn env_flag_falsy(v: &str) -> bool {
    matches!(
        v.trim().to_ascii_lowercase().as_str(),
        "0" | "false" | "no" | "off"
    )
}

/// True when the feature is on (IR/install + env) and at least one coding is enabled.
pub fn feature_enabled() -> bool {
    let cfg = effective_config();
    cfg.enabled && (cfg.gzip || cfg.brotli)
}

/// Select a cache coding from raw Accept-Encoding (Cap022 rank: br > gzip among offered).
///
/// Returns `None` when the feature is off, AE absent/identity-only, or no offered coding matches.
pub fn select_coding_from_accept_encoding(
    accept_encoding: Option<&str>,
    cfg: &EncodingCacheConfig,
) -> Option<CacheCoding> {
    if !cfg.enabled {
        return None;
    }
    let raw = accept_encoding?.trim();
    if raw.is_empty() {
        return None;
    }
    let br_q = coding_q(raw, "br");
    let gzip_q = coding_q(raw, "gzip");
    // Cap022 server preference on equal q: br > gzip (among static-cache offerings).
    if cfg.brotli {
        if let Some(q) = br_q {
            if q > 0.0 {
                if let Some(gq) = gzip_q {
                    if cfg.gzip && gq > q {
                        return Some(CacheCoding::Gzip);
                    }
                }
                return Some(CacheCoding::Brotli);
            }
        }
    }
    if cfg.gzip {
        if let Some(q) = gzip_q {
            if q > 0.0 {
                return Some(CacheCoding::Gzip);
            }
        }
        // Wildcard `*` with q>0 covers gzip when listed implicitly.
        if let Some(q) = coding_q(raw, "*") {
            if q > 0.0 {
                return Some(CacheCoding::Gzip);
            }
        }
    }
    None
}

/// `identity;q=0` with no other coding offered at q>0.
///
/// Deflate/gzip/br still belong to the compressor. This is only the case
/// where serving the raw file would violate the client's only rule.
pub fn identity_q_zero_without_other_coding(accept_encoding: Option<&str>) -> bool {
    let Some(raw) = accept_encoding.map(str::trim).filter(|s| !s.is_empty()) else {
        return false;
    };
    let identity_q = coding_q(raw, "identity");
    let star_q = coding_q(raw, "*");
    let identity_forbidden = match identity_q {
        Some(q) => q == 0.0,
        None => star_q == Some(0.0),
    };
    if !identity_forbidden {
        return false;
    }
    for part in raw.split(',') {
        let name = part.split(';').next().unwrap_or("").trim();
        if name.is_empty() || name.eq_ignore_ascii_case("identity") || name == "*" {
            continue;
        }
        if coding_q(raw, name).unwrap_or(0.0) > 0.0 {
            return false;
        }
    }
    true
}

/// Parse q-value for a coding token; absent → `None` (not offered).
fn coding_q(header: &str, token: &str) -> Option<f32> {
    for part in header.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let mut name = part;
        let mut q = 1.0_f32;
        if let Some((n, params)) = part.split_once(';') {
            name = n.trim();
            for p in params.split(';') {
                let p = p.trim();
                if let Some(rest) = p.strip_prefix("q=").or_else(|| p.strip_prefix("Q=")) {
                    if let Ok(v) = rest.trim().parse::<f32>() {
                        q = v.clamp(0.0, 1.0);
                    }
                }
            }
        }
        if name.eq_ignore_ascii_case(token) {
            return Some(q);
        }
    }
    None
}

/// Stable hex cache filename for source identity + coding + level (no path traversal).
pub fn cache_object_filename(
    identity: &ValidatorIdentity,
    coding: CacheCoding,
    level: u32,
) -> String {
    let mut hasher = Sha256::new();
    #[cfg(unix)]
    {
        hasher.update(identity.dev.to_le_bytes());
        hasher.update(identity.ino.to_le_bytes());
    }
    #[cfg(not(unix))]
    {
        hasher.update(0u64.to_le_bytes());
        hasher.update(0u64.to_le_bytes());
    }
    hasher.update(identity.mtime_secs.to_le_bytes());
    hasher.update(identity.len.to_le_bytes());
    hasher.update(coding.as_str().as_bytes());
    hasher.update(level.to_le_bytes());
    let digest = hasher.finalize();
    let mut out = String::with_capacity(64);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// ETag for an encoded representation: Cap020 source identity + coding suffix.
pub fn encoded_etag(source: &StaticValidators, coding: CacheCoding) -> String {
    let base = source.etag.trim_end_matches('"');
    format!("{base}-{}\"", coding.etag_suffix())
}

/// Compress `plain` with the selected coding at `level`.
pub fn compress_bytes(plain: &[u8], coding: CacheCoding, level: u32) -> io::Result<Vec<u8>> {
    match coding {
        CacheCoding::Gzip => {
            let lvl = Compression::new(level.min(9));
            let mut enc = GzEncoder::new(Vec::new(), lvl);
            enc.write_all(plain)?;
            enc.finish()
        }
        CacheCoding::Brotli => {
            let quality = i32::try_from(level.min(11)).unwrap_or(6);
            let params = BrotliEncoderParams {
                quality,
                lgwin: 22,
                mode: BrotliEncoderMode::BROTLI_MODE_GENERIC,
                ..Default::default()
            };
            let mut input = io::Cursor::new(plain);
            let mut output = Vec::new();
            BrotliCompress(&mut input, &mut output, &params)?;
            Ok(output)
        }
    }
}

/// Refuse world-writable cache directories (trust-on-hit / TOCTOU surface).
pub fn assert_cache_dir_safe(cache_dir: &Path) -> io::Result<()> {
    let meta = fs::metadata(cache_dir)?;
    if !meta.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "encoding cache_dir is not a directory",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = meta.permissions().mode();
        if mode & 0o002 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "encoding cache_dir is world-writable",
            ));
        }
    }
    Ok(())
}

/// Count hex-named coded objects and their total size under `cache_dir`.
fn cache_usage(cache_dir: &Path) -> io::Result<(u64, u64)> {
    let mut entries: u64 = 0;
    let mut total: u64 = 0;
    for ent in fs::read_dir(cache_dir)? {
        let ent = ent?;
        let name = ent.file_name();
        let Some(s) = name.to_str() else {
            continue;
        };
        if s.starts_with('.') {
            continue;
        }
        if !s.chars().all(|c| c.is_ascii_hexdigit()) {
            continue;
        }
        let meta = ent.metadata()?;
        if !meta.is_file() {
            continue;
        }
        entries = entries.saturating_add(1);
        total = total.saturating_add(meta.len());
    }
    Ok((entries, total))
}

struct MissSlot;

impl MissSlot {
    fn try_acquire() -> Option<Self> {
        loop {
            let cur = MISS_INFLIGHT.load(Ordering::Relaxed);
            if cur >= MAX_MISS_INFLIGHT {
                return None;
            }
            if MISS_INFLIGHT
                .compare_exchange_weak(cur, cur + 1, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
            {
                return Some(Self);
            }
        }
    }
}

impl Drop for MissSlot {
    fn drop(&mut self) {
        MISS_INFLIGHT.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Ensure a coded object exists under `cache_dir` (A1 sync compress-once + atomic publish).
///
/// Returns the absolute path of the coded file. Enforces directory safety, quotas, and
/// bounded concurrent miss compression.
pub fn ensure_coded_object(
    cfg: &EncodingCacheConfig,
    identity: &ValidatorIdentity,
    coding: CacheCoding,
    source_plain: &[u8],
) -> io::Result<PathBuf> {
    fs::create_dir_all(&cfg.cache_dir)?;
    assert_cache_dir_safe(&cfg.cache_dir)?;
    let name = cache_object_filename(identity, coding, cfg.level);
    // Hex-only name — refuse anything that is not pure hex (defense in depth).
    if !name.chars().all(|c| c.is_ascii_hexdigit()) || name.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "encoding cache key is not hex",
        ));
    }
    let final_path = cfg.cache_dir.join(&name);
    if final_path.is_file() {
        return Ok(final_path);
    }
    let (entries, total) = cache_usage(&cfg.cache_dir)?;
    if entries >= cfg.max_entries {
        return Err(io::Error::new(
            io::ErrorKind::StorageFull,
            "encoding cache max_entries exceeded",
        ));
    }
    if total >= cfg.max_total_bytes {
        return Err(io::Error::new(
            io::ErrorKind::StorageFull,
            "encoding cache max_total_bytes exceeded",
        ));
    }
    let _slot = MissSlot::try_acquire().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::ResourceBusy,
            "encoding cache miss compress concurrency limit",
        )
    })?;
    // Re-check after acquiring the miss slot (another worker may have published).
    if final_path.is_file() {
        return Ok(final_path);
    }
    let coded = compress_bytes(source_plain, coding, cfg.level)?;
    let coded_len = coded.len() as u64;
    if total.saturating_add(coded_len) > cfg.max_total_bytes {
        return Err(io::Error::new(
            io::ErrorKind::StorageFull,
            "encoding cache max_total_bytes would be exceeded",
        ));
    }
    atomic_write_file(&final_path, &coded)?;
    Ok(final_path)
}

/// Write `bytes` via `.tmp-{pid}-{nanos}` + fsync + rename.
pub fn atomic_write_file(final_path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = final_path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "cache path has no parent"))?;
    fs::create_dir_all(parent)?;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp_name = format!(".tmp-{}-{nanos}", std::process::id());
    let tmp_path = parent.join(tmp_name);
    {
        let mut f = File::create(&tmp_path)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp_path, final_path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp_path);
    })?;
    Ok(())
}

/// Open a coded cache file as a Cap067 [`SendfileAsset`] with CE/Vary headers.
///
/// On Linux, opens with `O_NOFOLLOW` so a symlink planted in `cache_dir` cannot
/// redirect the sendfile FD (trust-on-hit remediation).
pub fn open_encoded_sendfile_asset(
    coded_path: &Path,
    source_validators: &StaticValidators,
    coding: CacheCoding,
    content_type: &'static str,
) -> io::Result<SendfileAsset> {
    let file = open_cache_file_nofollow(coded_path)?;
    let meta = file.metadata()?;
    if !meta.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "encoding cache object is not a regular file",
        ));
    }
    let coded_len = usize::try_from(meta.len()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "coded object too large for sendfile",
        )
    })?;
    let etag = encoded_etag(source_validators, coding);
    let lm = source_validators.last_modified.as_deref();
    let header_200 = Arc::new(wire::ok_header_encoded_with_validators(
        coded_len,
        content_type,
        coding.as_str(),
        Some(&etag),
        lm,
    ));
    let header_304 = Arc::new(wire::not_modified_header(&etag, lm));
    let mut validators = source_validators.clone();
    validators.etag = etag;
    Ok(SendfileAsset {
        file: Arc::new(file),
        header: header_200,
        header_304,
        body_len: coded_len,
        content_type,
        validators: Arc::new(validators),
    })
}

fn open_cache_file_nofollow(path: &Path) -> io::Result<File> {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)
    }
    #[cfg(not(target_os = "linux"))]
    {
        OpenOptions::new().read(true).open(path)
    }
}

/// Read source file bytes (bounded by caller size check).
pub fn read_source_plain(path: &Path, expected_len: u64) -> io::Result<Vec<u8>> {
    let mut f = File::open(path)?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf)?;
    if buf.len() as u64 != expected_len {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "source size changed during encoding cache miss",
        ));
    }
    Ok(buf)
}

/// Resolve or create a coded sendfile asset for Cap067 (A1 miss policy).
///
/// **Hit path:** open the coded object only — never re-read the source body
/// (P9 hot-path tax: reading 64 KiB on every CE:gzip hit was the dominant cost).
/// **Miss path:** read source once, compress, atomic publish, then open.
///
/// Returns `None` when size is out of bounds or I/O/compress fails (caller may Hyper-fallback).
pub fn try_coded_sendfile_asset(
    cfg: &EncodingCacheConfig,
    coding: CacheCoding,
    source_path: &Path,
    identity: &ValidatorIdentity,
    source_validators: &StaticValidators,
    content_type: &'static str,
) -> Option<SendfileAsset> {
    if identity.len < cfg.min_bytes || identity.len > cfg.max_bytes {
        return None;
    }
    let name = cache_object_filename(identity, coding, cfg.level);
    if !name.chars().all(|c| c.is_ascii_hexdigit()) || name.is_empty() {
        return None;
    }
    let coded_path = cfg.cache_dir.join(&name);
    // Warm hit: coded bytes already on disk — skip source read + compress.
    if coded_path.is_file() {
        return open_encoded_sendfile_asset(&coded_path, source_validators, coding, content_type)
            .ok();
    }
    let plain = read_source_plain(source_path, identity.len).ok()?;
    let coded_path = ensure_coded_object(cfg, identity, coding, &plain).ok()?;
    open_encoded_sendfile_asset(&coded_path, source_validators, coding, content_type).ok()
}

/// Open a warm coded cache object if present (no source read, no compress).
pub fn try_open_cached_coded(
    cfg: &EncodingCacheConfig,
    coding: CacheCoding,
    identity: &ValidatorIdentity,
    source_validators: &StaticValidators,
    content_type: &'static str,
) -> Option<SendfileAsset> {
    if identity.len < cfg.min_bytes || identity.len > cfg.max_bytes {
        return None;
    }
    let name = cache_object_filename(identity, coding, cfg.level);
    if !name.chars().all(|c| c.is_ascii_hexdigit()) || name.is_empty() {
        return None;
    }
    let coded_path = cfg.cache_dir.join(&name);
    if !coded_path.is_file() {
        return None;
    }
    open_encoded_sendfile_asset(&coded_path, source_validators, coding, content_type).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::read::GzDecoder;

    fn sample_identity() -> ValidatorIdentity {
        #[cfg(unix)]
        {
            ValidatorIdentity {
                dev: 0x10,
                ino: 0x20,
                len: 1024,
                mtime_secs: 1_700_000_000,
                mtime_nsecs: 0,
            }
        }
        #[cfg(not(unix))]
        {
            ValidatorIdentity {
                len: 1024,
                mtime_secs: 1_700_000_000,
                mtime_nsecs: 0,
            }
        }
    }

    #[test]
    fn cache_path_key_stable() {
        let id = sample_identity();
        let a = cache_object_filename(&id, CacheCoding::Gzip, 6);
        let b = cache_object_filename(&id, CacheCoding::Gzip, 6);
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        let other_level = cache_object_filename(&id, CacheCoding::Gzip, 1);
        assert_ne!(a, other_level);
        let other_coding = cache_object_filename(&id, CacheCoding::Brotli, 6);
        assert_ne!(a, other_coding);
    }

    #[test]
    fn compress_gzip_roundtrip() {
        let plain = b"adr046-static-encoding-cache-payload-xxxxxxxxxxxxxxxx";
        let coded = compress_bytes(plain, CacheCoding::Gzip, 6).expect("gzip");
        assert!(!coded.is_empty());
        let mut dec = GzDecoder::new(coded.as_slice());
        let mut out = Vec::new();
        dec.read_to_end(&mut out).expect("gunzip");
        assert_eq!(out.as_slice(), plain.as_slice());
    }

    #[test]
    fn compress_brotli_roundtrip() {
        let plain = b"adr046-brotli-static-cache-payload-yyyyyyyyyyyyyyyy";
        let coded = compress_bytes(plain, CacheCoding::Brotli, 6).expect("br");
        assert!(!coded.is_empty());
        let mut out = Vec::new();
        brotli::BrotliDecompress(&mut io::Cursor::new(&coded), &mut out).expect("debr");
        assert_eq!(out.as_slice(), plain.as_slice());
    }

    #[test]
    fn disabled_selects_no_coding() {
        let cfg = EncodingCacheConfig {
            enabled: false,
            ..EncodingCacheConfig::default()
        };
        assert!(select_coding_from_accept_encoding(Some("gzip"), &cfg).is_none());
    }

    #[test]
    fn enabled_selects_gzip() {
        let cfg = EncodingCacheConfig {
            enabled: true,
            gzip: true,
            brotli: false,
            ..EncodingCacheConfig::default()
        };
        assert_eq!(
            select_coding_from_accept_encoding(Some("gzip"), &cfg),
            Some(CacheCoding::Gzip)
        );
        assert_eq!(
            select_coding_from_accept_encoding(Some("br, gzip"), &cfg),
            Some(CacheCoding::Gzip)
        );
    }

    #[test]
    fn enabled_prefers_brotli_when_offered() {
        let cfg = EncodingCacheConfig {
            enabled: true,
            gzip: true,
            brotli: true,
            ..EncodingCacheConfig::default()
        };
        assert_eq!(
            select_coding_from_accept_encoding(Some("gzip, br"), &cfg),
            Some(CacheCoding::Brotli)
        );
    }

    #[test]
    fn atomic_publish_and_ensure_hit() {
        let dir = tempfile::tempdir().expect("tmpdir");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(dir.path()).expect("meta").permissions();
            perms.set_mode(0o755);
            fs::set_permissions(dir.path(), perms).expect("chmod");
        }
        let id = sample_identity();
        let plain = vec![b'x'; 512];
        let cfg = EncodingCacheConfig {
            enabled: true,
            cache_dir: dir.path().to_path_buf(),
            level: 6,
            ..EncodingCacheConfig::default()
        };
        let p1 = ensure_coded_object(&cfg, &id, CacheCoding::Gzip, &plain).expect("miss");
        let p2 = ensure_coded_object(&cfg, &id, CacheCoding::Gzip, &plain).expect("hit");
        assert_eq!(p1, p2);
        assert!(p1.is_file());
        let coded = fs::read(&p1).expect("read");
        let mut dec = GzDecoder::new(coded.as_slice());
        let mut out = Vec::new();
        dec.read_to_end(&mut out).expect("gunzip");
        assert_eq!(out, plain);
    }

    #[test]
    fn quota_rejects_new_entry_when_max_entries_zero_effective() {
        let dir = tempfile::tempdir().expect("tmpdir");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(dir.path()).expect("meta").permissions();
            perms.set_mode(0o755);
            fs::set_permissions(dir.path(), perms).expect("chmod");
        }
        let id = sample_identity();
        let plain = vec![b'y'; 400];
        let cfg = EncodingCacheConfig {
            enabled: true,
            cache_dir: dir.path().to_path_buf(),
            max_entries: 0,
            ..EncodingCacheConfig::default()
        };
        let err = ensure_coded_object(&cfg, &id, CacheCoding::Gzip, &plain).expect_err("quota");
        assert_eq!(err.kind(), io::ErrorKind::StorageFull);
    }

    #[test]
    fn world_writable_cache_dir_rejected() {
        let dir = tempfile::tempdir().expect("tmpdir");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(dir.path()).expect("meta").permissions();
            perms.set_mode(0o777);
            fs::set_permissions(dir.path(), perms).expect("chmod");
            let err = assert_cache_dir_safe(dir.path()).expect_err("world-writable");
            assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        }
        #[cfg(not(unix))]
        {
            let _ = dir;
        }
    }

    #[test]
    fn warm_hit_skips_source_reread() {
        let dir = tempfile::tempdir().expect("tmpdir");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(dir.path()).expect("meta").permissions();
            perms.set_mode(0o755);
            fs::set_permissions(dir.path(), perms).expect("chmod");
        }
        let id = sample_identity();
        let plain = vec![b'z'; 512];
        let cfg = EncodingCacheConfig {
            enabled: true,
            cache_dir: dir.path().to_path_buf(),
            level: 6,
            ..EncodingCacheConfig::default()
        };
        let p1 = ensure_coded_object(&cfg, &id, CacheCoding::Gzip, &plain).expect("miss");
        assert!(p1.is_file());
        // Hit via try_open_cached_coded must not need the source path.
        let validators = StaticValidators {
            etag: "W/\"exq-test\"".into(),
            last_modified: None,
            mtime_secs: Some(1_700_000_000),
        };
        let asset = try_open_cached_coded(&cfg, CacheCoding::Gzip, &id, &validators, "text/plain")
            .expect("warm hit");
        assert_eq!(
            asset.body_len,
            fs::metadata(&p1).expect("meta").len() as usize
        );
        // try_coded_sendfile_asset on warm hit also works without reading a real source.
        let missing_source = dir.path().join("no-such-source.bin");
        let asset2 = try_coded_sendfile_asset(
            &cfg,
            CacheCoding::Gzip,
            &missing_source,
            &id,
            &validators,
            "text/plain",
        )
        .expect("warm hit without source");
        assert_eq!(asset2.body_len, asset.body_len);
    }

    #[test]
    fn encoded_header_has_ce_and_vary() {
        let hdr = wire::ok_header_encoded_with_validators(
            42,
            "text/plain",
            "gzip",
            Some("W/\"exq-1-2-3-4-5-gzip\""),
            None,
        );
        let s = std::str::from_utf8(&hdr).expect("utf8");
        assert!(s.contains("Content-Encoding: gzip\r\n"));
        assert!(s.contains("Vary: Accept-Encoding\r\n"));
        assert!(s.contains("Content-Length: 42\r\n"));
        assert!(s.contains("ETag: W/\"exq-1-2-3-4-5-gzip\"\r\n"));
    }
}

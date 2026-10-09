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
//! WC3 — CachePurgePort over SharedServerState.fpc_cache (off request hot path).

use crate::reload::{self, SharedServerState};
use exyonq_cache::{
    build_storage_cache_key, normalize_host, note_fpc_purge_rejected, note_fpc_purge_request,
    note_fpc_purge_success, CacheKeyParts,
};
use exyonq_module_api::cache_purge::{CachePurgeOp, CachePurgeOutcome, CachePurgePort};
use exyonq_module_api::{fpc_canonicalize_path, fpc_query_decision};
use std::sync::Arc;
use std::time::Instant;

pub struct CoreCachePurgePort {
    pub shared: SharedServerState,
}

impl CoreCachePurgePort {
    fn resolve_route_for_site(
        state: &crate::server::state::ServerState,
        site_id: u64,
    ) -> Option<usize> {
        state
            .snapshot
            .full_page_cache
            .route_site_ids
            .iter()
            .position(|&id| id == site_id)
    }

    fn purge_committed(&self, op: CachePurgeOp) -> CachePurgeOutcome {
        note_fpc_purge_request();
        let started = Instant::now();
        let state = reload::read_state(&self.shared);
        let generation = state.generation;

        let Some(cache) = state.fpc_cache.as_ref() else {
            note_fpc_purge_rejected("internal_error");
            let site = match &op {
                CachePurgeOp::Url { site_id, .. }
                | CachePurgeOp::Site { site_id }
                | CachePurgeOp::Generation { site_id, .. }
                | CachePurgeOp::Tag { site_id, .. } => *site_id,
            };
            return CachePurgeOutcome::fail(op_name(&op), site, generation, "internal_error");
        };

        match op {
            CachePurgeOp::Tag { site_id, tag } => {
                if !acceptable_tag(&tag) {
                    note_fpc_purge_rejected("invalid_key");
                    return CachePurgeOutcome::fail(
                        "purge.tag",
                        site_id,
                        generation,
                        "invalid_key",
                    );
                }
                if Self::resolve_route_for_site(&state, site_id).is_none() {
                    note_fpc_purge_rejected("unauthorized");
                    return CachePurgeOutcome::fail(
                        "purge.tag",
                        site_id,
                        generation,
                        "unauthorized",
                    );
                }
                let stats = cache.invalidate_site_tag(site_id, &tag);
                let us = started.elapsed().as_micros() as u64;
                note_fpc_purge_success(stats.purged_entries, stats.purged_bytes, us);
                CachePurgeOutcome::success(
                    "purge.tag",
                    site_id,
                    stats.purged_entries,
                    stats.purged_bytes,
                    generation,
                )
            }
            CachePurgeOp::Site { site_id } => {
                if Self::resolve_route_for_site(&state, site_id).is_none() {
                    note_fpc_purge_rejected("unauthorized");
                    return CachePurgeOutcome::fail(
                        "purge.site",
                        site_id,
                        generation,
                        "unauthorized",
                    );
                }
                let stats = cache.invalidate_site(site_id);
                let us = started.elapsed().as_micros() as u64;
                note_fpc_purge_success(stats.purged_entries, stats.purged_bytes, us);
                CachePurgeOutcome::success(
                    "purge.site",
                    site_id,
                    stats.purged_entries,
                    stats.purged_bytes,
                    generation,
                )
            }
            CachePurgeOp::Generation {
                site_id,
                generation: target_gen,
            } => {
                if Self::resolve_route_for_site(&state, site_id).is_none() {
                    note_fpc_purge_rejected("unauthorized");
                    return CachePurgeOutcome::fail(
                        "purge.generation",
                        site_id,
                        generation,
                        "unauthorized",
                    );
                }
                let stats = cache.invalidate_site_runtime_generation(site_id, target_gen);
                let us = started.elapsed().as_micros() as u64;
                note_fpc_purge_success(stats.purged_entries, stats.purged_bytes, us);
                CachePurgeOutcome::success(
                    "purge.generation",
                    site_id,
                    stats.purged_entries,
                    stats.purged_bytes,
                    generation,
                )
            }
            CachePurgeOp::Url {
                site_id,
                scheme,
                host,
                path,
                query,
            } => {
                let Some(route_idx) = Self::resolve_route_for_site(&state, site_id) else {
                    note_fpc_purge_rejected("unauthorized");
                    return CachePurgeOutcome::fail(
                        "purge.url",
                        site_id,
                        generation,
                        "unauthorized",
                    );
                };
                let Some(backend_id) = state.snapshot.route_backend_id(route_idx) else {
                    note_fpc_purge_rejected("invalid_scope");
                    return CachePurgeOutcome::fail(
                        "purge.url",
                        site_id,
                        generation,
                        "invalid_scope",
                    );
                };
                let Some(canon_path) = fpc_canonicalize_path(&path) else {
                    note_fpc_purge_rejected("invalid_key");
                    return CachePurgeOutcome::fail(
                        "purge.url",
                        site_id,
                        generation,
                        "invalid_key",
                    );
                };
                let canon_query = match fpc_query_decision(&query, &[]) {
                    exyonq_module_api::FpcQueryDecision::Eligible { canonical } => canonical,
                    exyonq_module_api::FpcQueryDecision::Bypass(_) => {
                        note_fpc_purge_rejected("invalid_key");
                        return CachePurgeOutcome::fail(
                            "purge.url",
                            site_id,
                            generation,
                            "invalid_key",
                        );
                    }
                };
                let key = build_storage_cache_key(CacheKeyParts {
                    site_id,
                    namespace: state.snapshot.full_page_cache.namespace,
                    backend_id: backend_id.index(),
                    runtime_generation: generation,
                    policy_generation: 0,
                    route_idx,
                    method: "GET".into(),
                    scheme,
                    host: normalize_host(Some(&host)),
                    path: canon_path,
                    query: canon_query,
                    content_encoding: "identity".into(),
                });
                let stats = cache.invalidate_key(&key);
                let us = started.elapsed().as_micros() as u64;
                // Cap057 LA-CAP057-003: purge.url must not report operator success when
                // zero live entries were removed (wrong scheme/host/key → no-op).
                if stats.purged_entries == 0 {
                    note_fpc_purge_rejected("not_found");
                    return CachePurgeOutcome::fail("purge.url", site_id, generation, "not_found");
                }
                note_fpc_purge_success(stats.purged_entries, stats.purged_bytes, us);
                CachePurgeOutcome::success(
                    "purge.url",
                    site_id,
                    stats.purged_entries,
                    stats.purged_bytes,
                    generation,
                )
            }
        }
    }
}

fn acceptable_tag(tag: &str) -> bool {
    let mut chars = tag.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if tag.len() > 32 || !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
        return false;
    }
    tag.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

impl CachePurgePort for CoreCachePurgePort {
    fn purge(&self, op: CachePurgeOp) -> CachePurgeOutcome {
        let outcome = self.purge_committed(op);
        // Cap061: AUDIT_SUCCESS only after invalidate_* returned (committed deletion stats).
        let detail = format!(
            "site_id={} purged_entries={} err={}",
            outcome.site_id,
            outcome.purged_entries,
            outcome.error.unwrap_or("")
        );
        crate::observability::emit_audit(crate::observability::AuditEvent {
            action: outcome.operation,
            result: if outcome.ok { "success" } else { "failure" },
            detail: Some(&detail),
            request_id: None,
        });
        outcome
    }
}

fn op_name(op: &CachePurgeOp) -> &'static str {
    match op {
        CachePurgeOp::Url { .. } => "purge.url",
        CachePurgeOp::Site { .. } => "purge.site",
        CachePurgeOp::Generation { .. } => "purge.generation",
        CachePurgeOp::Tag { .. } => "purge.tag",
    }
}

/// Token the WordPress plugin will read from `cache-purge.token`.
/// Empty, too long, or characters outside `[A-Za-z0-9._~+-]` are rejected.
pub fn purge_token_acceptable(token: &str) -> bool {
    !token.is_empty()
        && token.len() <= 256
        && token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'~' | b'+' | b'-'))
}

/// 32 random bytes, hex-encoded. The value is not logged.
pub fn generate_purge_token() -> Result<String, String> {
    use std::io::Read;
    let mut bytes = [0u8; 32];
    let mut urandom = std::fs::File::open("/dev/urandom")
        .map_err(|err| format!("open /dev/urandom: {err}"))?;
    urandom
        .read_exact(&mut bytes)
        .map_err(|err| format!("read /dev/urandom: {err}"))?;
    let mut token = String::with_capacity(64);
    for byte in bytes {
        token.push_str(&format!("{byte:02x}"));
    }
    Ok(token)
}

/// `route-name<TAB>site-id` lines. PHP reads this file and must not recompute the id.
pub fn render_purge_site_index(sites: &[(&str, u64)]) -> Result<String, String> {
    let mut out = String::new();
    for (name, id) in sites {
        if name.is_empty()
            || name
                .bytes()
                .any(|b| b.is_ascii_whitespace() || b == b'\\' || b == b'/')
        {
            return Err("purge site index refused a route name".into());
        }
        if *id == 0 {
            return Err("purge site id must be non-zero".into());
        }
        out.push_str(name);
        out.push('\t');
        out.push_str(&id.to_string());
        out.push('\n');
    }
    Ok(out)
}

/// Write `<socket-dir>/cache-purge.token` (mode 0600) and `<socket>.sites` (mode 0640).
///
/// Called when the purge socket is about to listen. The token is the same secret
/// the socket will compare. Neither path is logged with the token bytes.
pub fn publish_purge_sidecars(
    socket_path: &std::path::Path,
    token: &str,
    sites: &[(&str, u64)],
) -> Result<(), String> {
    if !purge_token_acceptable(token) {
        return Err(
            "purge token is empty or has characters the WordPress plugin will not read".into(),
        );
    }
    let parent = socket_path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .ok_or_else(|| "purge socket path has no directory".to_string())?;
    std::fs::create_dir_all(parent)
        .map_err(|err| format!("create purge directory {}: {err}", parent.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o750)).map_err(
            |err| format!("chmod purge directory {}: {err}", parent.display()),
        )?;
    }
    let index = render_purge_site_index(sites)?;
    let token_path = parent.join("cache-purge.token");
    let mut sites_name = socket_path.as_os_str().to_owned();
    sites_name.push(".sites");
    let sites_path = std::path::PathBuf::from(sites_name);
    write_private_file(&token_path, token.as_bytes(), 0o600)?;
    write_private_file(&sites_path, index.as_bytes(), 0o640)?;
    Ok(())
}

fn write_private_file(path: &std::path::Path, bytes: &[u8], mode: u32) -> Result<(), String> {
    let mut partial_name = path.as_os_str().to_owned();
    partial_name.push(".partial");
    let partial = std::path::PathBuf::from(partial_name);
    let result = write_private_file_inner(&partial, path, bytes, mode);
    if result.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
    result
}

fn write_private_file_inner(
    partial: &std::path::Path,
    path: &std::path::Path,
    bytes: &[u8],
    mode: u32,
) -> Result<(), String> {
    use std::io::Write;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(partial)
            .map_err(|err| format!("open {}: {err}", partial.display()))?;
        file.write_all(bytes)
            .map_err(|err| format!("write {}: {err}", partial.display()))?;
        file.sync_all()
            .map_err(|err| format!("sync {}: {err}", partial.display()))?;
        std::fs::set_permissions(partial, std::fs::Permissions::from_mode(mode))
            .map_err(|err| format!("chmod {}: {err}", partial.display()))?;
    }
    #[cfg(not(unix))]
    {
        let _ = mode;
        let mut file = std::fs::File::create(partial)
            .map_err(|err| format!("open {}: {err}", partial.display()))?;
        file.write_all(bytes)
            .map_err(|err| format!("write {}: {err}", partial.display()))?;
        file.sync_all()
            .map_err(|err| format!("sync {}: {err}", partial.display()))?;
    }
    std::fs::rename(partial, path)
        .map_err(|err| format!("rename {} -> {}: {err}", partial.display(), path.display()))
}

/// Build a purge port handle for the live shared state (tests / composition).
pub fn purge_port(shared: SharedServerState) -> Arc<dyn CachePurgePort> {
    Arc::new(CoreCachePurgePort { shared })
}

/// WC7B1 — wrap local purge with best-effort invalidation publish (fail-open).
pub fn purge_port_with_coordination(
    shared: SharedServerState,
    publisher: Option<Arc<dyn exyonq_module_api::InvalidationPublisher>>,
    node_id: impl Into<String>,
    invalidation_enabled: bool,
) -> Arc<dyn CachePurgePort> {
    let inner = purge_port(shared);
    if !invalidation_enabled || publisher.is_none() {
        return inner;
    }
    Arc::new(
        crate::coordinating_purge_port::CoordinatingCachePurgePort::new(
            inner, publisher, node_id, true,
        ),
    )
}

#[cfg(test)]
mod purge_sidecar_tests {
    use super::{publish_purge_sidecars, purge_token_acceptable, render_purge_site_index};
    use exyonq_runtime_plan::stable_fpc_site_id;

    #[test]
    fn site_index_uses_the_runtime_site_id() {
        let id = stable_fpc_site_id("wordpress");
        let text = render_purge_site_index(&[("wordpress", id), ("wp-content", 7)]).unwrap();
        assert_eq!(text, format!("wordpress\t{id}\nwp-content\t7\n"));
        assert!(render_purge_site_index(&[("wp includes", 1)]).is_err());
        assert!(render_purge_site_index(&[("wordpress", 0)]).is_err());
        assert!(!purge_token_acceptable(""));
        assert!(!purge_token_acceptable("has space"));
        assert!(purge_token_acceptable("abcDEF0123._~+-"));
    }

    #[test]
    fn sidecars_are_owner_readable_and_name_the_wordpress_route() {
        let id = stable_fpc_site_id("wordpress");
        let dir = std::env::temp_dir().join(format!(
            "exyonq-purge-sidecars-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let socket = dir.join("cache-purge.sock");
        publish_purge_sidecars(socket.as_path(), "token-value", &[("wordpress", id)]).unwrap();

        let token = std::fs::read_to_string(dir.join("cache-purge.token")).unwrap();
        assert_eq!(token, "token-value");
        let mut sites_name = socket.as_os_str().to_owned();
        sites_name.push(".sites");
        let sites_path = std::path::PathBuf::from(sites_name);
        let sites = std::fs::read_to_string(&sites_path).unwrap();
        assert_eq!(sites, format!("wordpress\t{id}\n"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let token_mode = std::fs::metadata(dir.join("cache-purge.token"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(token_mode, 0o600);
            let sites_mode = std::fs::metadata(&sites_path).unwrap().permissions().mode() & 0o777;
            assert_eq!(sites_mode, 0o640);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

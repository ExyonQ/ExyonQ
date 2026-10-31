# ADR-066 — ACME HTTP-01 challenge store shared across listener processes

**Status:** Accepted (implemented, deployed and verified on Oracle arm64)
**Date:** 2026-10-31
**Depends on:** `exyonq-acme` (`ChallengeStore`, `ensure_certificates`), `core/src/acme_http01_adapter.rs`, unconditional `register_acme_integration()` at startup
**Contract:** the `:80` plaintext process must be able to answer HTTP-01 lookups for challenges registered by the ACME-running `:443` process, without merging configs or listener topologies

## Context

ExyonQ deployments may run two sibling processes: a plaintext `:80` instance (redirect-only) and a TLS `:443` instance that owns the `[[server]]` blocks and runs the ACME bootstrap/renewal loop.

The HTTP-01 challenge store was process-local (`Arc<RwLock<HashMap>>`). The `:443` process registered tokens during an order, but the CA fetches them on `:80` — a different process with an empty map. DNS-01 was the only viable path, which forces a DNS provider API token for what should be a dependency-free renewal.

Route merging is not an option: route-match identity `(host, kind, pattern)` is global across listeners in one config, so a `:80` redirect route shadows the `:443` content route for the same host (regex outranks prefix; prefix collides in dedup). Verified empirically as a redirect loop (`https://host` → `Location: https://host`).

## Decision

`ChallengeStore` gains an optional filesystem-backed share selected by `EXYONQ_ACME_CHALLENGE_DIR`:

- `set(token, key_auth)` writes `<dir>/<token>` atomically (tmp + `rename`, mode `0600`) in addition to the memory map.
- `get(token)` checks memory first, then reads `<dir>/<token>`.
- `remove(token)` deletes both; `clear()` sweeps files for tokens it owns.
- Unset env var → memory-only store (single-process layouts unchanged).
- IO failures degrade to memory with `warn!`, never panic: a read-only dir cannot break an order that the same process can still answer itself.

Both processes instantiate their store via `ChallengeStore::new()`, which reads the env var — `register_acme_integration()` is unconditional in `main.rs`, so the `:80` process already holds a service; it just needed tokens it can see. `try_http01_response` runs before route dispatch for GET, so redirect routes never shadow the challenge path.

## Security

- Only base64url token names (`[A-Za-z0-9_-]`, 1–256) ever become filenames — `..`, separators and dots cannot escape `dir` (defense in depth: `lookup_token` already rejects `/` at the URL layer).
- Files are `0600`; the dir is expected same-owner (both units run the same `User=`).
- Anything else a same-uid peer writes there is servable only under `/.well-known/acme-challenge/<token>` — the trust domain is the service account, not the filesystem.

## Deployment contract

Point both units at one directory **outside** `RuntimeDirectory` management: systemd removes a `RuntimeDirectory` on any referencing unit's stop (no cross-unit refcount — observed wiping `/run/exyonq` on a single-unit restart, orphaning the sibling's control-socket path). A stable path (e.g. `/var/lib/exyonq-http01`, `0750`, service user) survives restarts; `create_dir_all` self-heals if it is removed.

```ini
# /etc/systemd/system/exyonq-{http,sites}.service.d/acme.conf
[Service]
Environment=EXYONQ_ACME_CHALLENGE_DIR=/var/lib/exyonq-http01
```

## Evidence (Oracle arm64, 2026-10-31)

- `cargo test -p exyonq-acme`: 10/10 (macOS + Oracle) — incl. `shares_tokens_through_directory` (cross-instance visibility+cleanup) and `unsafe_token_names_never_touch_disk`.
- Token file dropped as `www-data` → `GET http://exyonq.org/.well-known/acme-challenge/<tok>` externally → exact body; unknown token → falls through to redirect.
- Forced production issuance (cert moved aside + restart): `ACME certificate issued` ~4 s after boot via HTTP-01, LE YE1, SAN `exyonq.org`+`www`; challenge dir empty after `poll_ready` (cleanup verified).

## Non-goals

- Host-to-host sharing: the share is local-filesystem IPC for sibling processes on one machine, not a network protocol.
- Route scoping per listener (the unrelated global-matcher limitation stays; two processes keep separate configs).
- Garbage collection of orphaned token files (e.g. process killed between `set` and `remove`): stale files are single-use random names served only on the well-known prefix; bounded in size and harmless.

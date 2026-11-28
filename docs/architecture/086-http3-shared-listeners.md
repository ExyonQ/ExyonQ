# 086 — HTTP/3 listeners: one UDP socket per listener, shared by every host

Status: implemented in 0.9.4 (2026-11-28). Supersedes the 0.9.3 rule that
`http3_listen` was only honored on the first `[[server]]` block.

## Why

QUIC runs on UDP, so an HTTP/3 endpoint can use the same port number as the TCP
`listen` (443/TCP and 443/UDP are different sockets). The 0.9.3 runtime read
`http3_listen` from `config.primary_server()` only and rejected the field on any
other block (`http3_listen is only honored on the first [[server]] block`), so a
multi-host config could not serve HTTP/3 for its other virtual servers at all.

HTTP/3 belongs to the *listener*, not to a server block: a QUIC endpoint is
bound to one UDP address, terminates TLS with SNI-selected certificates and then
routes the request through the same host/route logic as the TCP listener.

## Semantics

- **Global**: `[http3] listen = "0.0.0.0:443"` sets the endpoint for every TLS
  listener in the config. One UDP socket serves all of them.
- **Per listener**: any `[[server]]` may set `http3_listen`; it overrides the
  global value for that TCP listen group. Two groups may therefore have their
  own sockets.
- **Shared by the port's hosts**: every server whose `listen` equals that group
  uses the same UDP socket, including blocks that omit `http3_listen` — they
  inherit it, so the old "declare it on the first block" shape keeps working.
  Two explicit, different `http3_listen` values inside one listen group are a
  config error.
- **Certificates**: the endpoint carries the TLS pair of every server in the
  group; the provider selects by SNI. Requests then follow the normal host
  routing of that listener.
- **Validation**: an HTTP/3 endpoint requires TLS on every server of the group
  and rejects the TLS options the QUIC providers do not honor yet
  (`strict_sni`, `protocols`, `ciphers`, `curves`, `ocsp_stapling`,
  `must_staple`, `client_auth`, `client_ca`, `crl`, `min_version`). `[[stream]]`
  UDP listeners cannot duplicate the effective HTTP/3 address.
- **Reload**: changing `listen`, `http3_listen` or the global `[http3] listen`
  still requires a restart (`EXY-RELOAD-0005`); certificates are read when the
  endpoint starts (restart after the first ACME issuance to enable HTTP/3).

## Example

```toml
[http3]
listen = "0.0.0.0:443"          # one UDP endpoint for every TLS host

[[server]]
listen = "0.0.0.0:443"
server_name = "a.example"
routes = ["a"]
[server.tls]
cert = "/etc/exyonq/a.pem"
key = "/etc/exyonq/a.key"

[[server]]
listen = "0.0.0.0:443"
server_name = "b.example"
routes = ["b"]
[server.tls]
cert = "/etc/exyonq/b.pem"
key = "/etc/exyonq/b.key"
```

Both hosts answer HTTP/3 on UDP/443 from the same socket; `a.example` and
`b.example` are distinguished by SNI and by the normal `server_name` routing.
A non-TLS listener (plain HTTP on :80) keeps running without HTTP/3: the global
value only applies to listener groups where every server has TLS.

## Evidence

`scripts/e2e/http3-shared-listener-e2e.sh` (Linux + Docker curl with HTTP/3)
asserts, against the real binary and a real QUIC client:

1. two `[[server]]` blocks sharing one TCP listen with the global `[http3]
   listen` produce **one** UDP socket (`ss -lunp`) and both hosts return their
   own body over `--http3-only`;
2. two listeners with their own `http3_listen` produce **two** UDP sockets and
   each host is served on its own port, including when the field is declared on
   a non-primary block.

Unit coverage lives in `config/ir/src/lib.rs`
(`http3_listen_is_honored_on_any_server_block`,
`http3_global_listen_is_shared_by_every_host`,
`http3_per_server_override_wins_over_global_listen`,
`http3_listen_must_agree_within_one_tcp_listen`,
`http3_listen_requires_tls_and_rejects_unsupported_crypto`).

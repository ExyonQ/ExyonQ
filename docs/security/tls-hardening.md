# TLS hardening guide

```text
DOCUMENT = docs/security/tls-hardening.md
SCOPE = TLS listener configuration: versions, ciphers, curves, OCSP stapling,
        SNI, session resumption, ACME automation, status visibility
APPLIES_TO = ExyonQ 0.8+
ENGINE = rustls 0.23 (ring provider)
```

This guide documents the TLS surface ExyonQ actually implements. It does not
claim knobs that the engine does not have (see [Limits](#limits)).

## Baseline (SSL Labs A+ target)

```toml
[[server]]
listen = "0.0.0.0:443"
server_name = ["example.com", "www.example.com"]
routes = ["site"]

[server.tls]
cert = "/etc/exyonq/tls/example.com.pem"
key  = "/etc/exyonq/tls/example.com.key"

# TLS 1.2 + 1.3 only (rustls has no older versions at all).
protocols = ["TLSv1.2", "TLSv1.3"]

# AEAD-only suite allowlist. IANA or OpenSSL spellings; ':'-separated accepted.
ciphers = [
  "TLS13_AES_256_GCM_SHA384",
  "TLS13_CHACHA20_POLY1305_SHA256",
  "ECDHE-ECDSA-AES256-GCM-SHA384",
  "ECDHE-RSA-AES256-GCM-SHA384",
]

# ECDHE groups in preference order.
curves = ["X25519", "secp256r1"]

# Reject handshakes whose SNI matches no configured server_name.
strict_sni = true

# Optional: staple a DER OCSP response (see "OCSP" below).
# ocsp_stapling = "/etc/exyonq/tls/example.com.ocsp.der"
# must_staple = true
```

HSTS belongs to response headers, not TLS:

```toml
[[route.response_headers]]
name = "Strict-Transport-Security"
value = "max-age=31536000; includeSubDomains"
```

(Headers are route-scoped in the IR — ADR-062.)

## Protocol versions and cipher suites

`[server.tls] protocols`, `ciphers`, `curves` apply to the TLS listener that
owns the socket (the first `[[server]]` on that `listen`). The values are
validated fail-closed:

- unknown protocol / suite / group → config rejected (`exyonq validate`)
- a cipher list that leaves no usable suite for the enabled versions → rejected
- the same fields on a non-primary `[[server]]` block → rejected (they would be
  silently ignored)

Supported suites are exactly the rustls `ring` AEAD set:

| TLS 1.3 | TLS 1.2 |
|---|---|
| `TLS13_AES_256_GCM_SHA384` | `TLS_ECDHE_{ECDSA,RSA}_WITH_AES_256_GCM_SHA384` |
| `TLS13_AES_128_GCM_SHA256` | `TLS_ECDHE_{ECDSA,RSA}_WITH_AES_128_GCM_SHA256` |
| `TLS13_CHACHA20_POLY1305_SHA256` | `TLS_ECDHE_{ECDSA,RSA}_WITH_CHACHA20_POLY1305_SHA256` |

Curves: `X25519`, `secp256r1` (`prime256v1`/`P-256`), `secp384r1` (`P-384`).

Empty lists mean "rustls default" (both versions, every suite and group).

## SNI: one listener, many certificates

Every `[[server]]` that shares a `listen` contributes its `cert`/`key` and its
`server_name` set to one TLS listener:

```toml
[[server]]
listen = "0.0.0.0:443"
server_name = ["example.com", "www.example.com"]
routes = ["site"]

[server.tls]
cert = "/etc/exyonq/tls/example.com.pem"
key  = "/etc/exyonq/tls/example.com.key"

[[server]]
listen = "0.0.0.0:443"
server_name = ["api.example.com"]
routes = ["api"]

[server.tls]
cert = "/etc/exyonq/tls/api.example.com.pem"
key  = "/etc/exyonq/tls/api.example.com.key"
```

Resolution is exact name → `*.suffix` wildcard → first block's certificate
(deterministic default). `strict_sni = true` turns "unknown name" into a
handshake abort instead of the default certificate, and requires at least one
`server_name`.

## Session resumption

Session IDs and stateless tickets are shared across acceptor rebuilds: a
reload does not break resumption, and ticket keys rotate on the rustls
schedule (6h issuance, 12h acceptance). No configuration is required.

## OCSP stapling

ExyonQ does not fetch from the responder. Point `ocsp_stapling` at a
DER-encoded response produced by your PKI tooling (nginx `ssl_stapling_file`
equivalent); the file is read when the acceptor is built, so replace it and
run `exyonqctl reload` — even with an unchanged TOML the TLS material is
re-read.

```toml
ocsp_stapling = "/etc/exyonq/tls/example.com.ocsp.der"
must_staple = true   # fail-closed: requires ocsp_stapling
```

`must_staple` requires the path to be set; a missing or empty file fails the
acceptor build (the previous acceptor stays live). It does not verify the
response's signature, status or validity window — that is the PKI's job.

## ACME: zero-touch certificates

HTTP-01 (default):

```toml
[server.tls.acme]
email = "ops@example.com"
domains = ["example.com", "www.example.com"]
```

DNS-01 (required for wildcards):

```toml
[server.tls.acme]
email = "ops@example.com"
domains = ["example.com", "*.example.com"]
challenge = "dns-01"
dns_hook = "/usr/local/bin/exyonq-acme-dns"
dns_propagation_secs = 30
```

The hook is run with `sh -c` and these variables:

| Variable | Value |
|---|---|
| `EXYONQ_ACME_ACTION` | `present` or `cleanup` |
| `EXYONQ_ACME_DOMAIN` | bare domain (no `*.`); the record owner is `_acme-challenge.$EXYONQ_ACME_DOMAIN` |
| `EXYONQ_ACME_TOKEN` | challenge token |
| `EXYONQ_ACME_TXT_VALUE` | TXT value to publish |

`present` must **add** the TXT value to the RRset (not replace it): a wildcard
and its apex share the same `_acme-challenge` name. `cleanup` runs after the
order completes, best-effort.

The ACME account is persisted (`account_path`, or `<cert>.acme-account.json`)
with mode `0600`, and reused across runs. Renewal uses ACME Renewal Information
(ARI) when the CA supports it, and otherwise renews 30 days before expiry.
Certificates are written atomically and the live TLS acceptor is rebuilt from
the current config.

## Verify

```bash
# Negotiated version + suite, TLS 1.3 pinned config
openssl s_client -connect example.com:443 -servername example.com -tls1_3 </dev/null 2>/dev/null \
  | grep -E "Protocol|Cipher"

# TLS 1.2 must be refused when protocols = ["TLSv1.3"]
openssl s_client -connect example.com:443 -servername example.com -tls1_2 </dev/null 2>&1 \
  | grep -i alert

# Per-name certificate on a shared listener
openssl s_client -connect example.com:443 -servername api.example.com </dev/null 2>/dev/null \
  | openssl x509 -noout -subject

# Stapling
openssl s_client -connect example.com:443 -servername example.com -status </dev/null 2>/dev/null \
  | grep -A2 "OCSP response"

# Expiry as the server sees it
exyonqctl status | grep tls_cert
exyonqctl status --format json | grep -o '"tls_cert_not_after_epoch":[0-9]*'
```

## Limits

- **No renegotiation.** rustls does not implement TLS renegotiation; there is
  no `SSLRenegBufferSize` equivalent. `SSLStrictSNIVHostCheck` semantics are
  covered by `strict_sni`.
- **HTTP/3 ignores these fields.** QUIC providers consume only `cert`/`key`;
  `exyonq validate` rejects `http3_listen` combined with `strict_sni`,
  `protocols`, `ciphers`, `curves`, `ocsp_stapling` or `must_staple` rather
  than silently ignoring them.
- **No `ssl_ciphers` expression language.** Only the explicit suite names
  above; no `HIGH:!aNULL` style macros.
- **Stapling is file-based.** No automatic responder fetch, no
  `ssl_stapling_verify`.
- **One TLS listener per process.** Fields apply to the first `[[server]]`;
  other listeners are plain HTTP.

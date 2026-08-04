# ExyonQ real E2E gates (authoritative)

```text
AUTHORITY = QUALIFICATION / CAPABILITY CLOSE (when dual-arch PASS)
SMOKE = NON_AUTHORITATIVE — do not use scripts/smoke/** to close capabilities
POLICY = EXYONQ_NO_SMOKE_POLICY + scripts/gates/no-smoke-as-proof-gate.sh
LINUX_EVIDENCE = Netcup amd64 + Oracle arm64 (rule 126)
```

These scripts prove real product behavior end-to-end. They replace
smoke-named scripts as the closing path for capability qualification.

| Script | Capability | Class target |
|--------|------------|--------------|
| `static-e2e.sh` | static serving | C+ |
| `proxy-e2e.sh` | reverse proxy | C+ (controlled HTTP peer) |
| `fastcgi-php-fpm-e2e.sh` | FastCGI + real PHP-FPM | C+ |
| `tls-e2e.sh` | TLS ALPN + HTTPS GET + reload | C |
| `http2-e2e.sh` | HTTP/2 multiplex + body integrity | C |
| `http3-protocol-e2e.sh` | HTTP/3 wire product path | C |
| `oci-runtime-e2e.sh` | container runtime (not version-only) | C |
| `run-linux-matrix.sh` | dual-arch orchestrator | — |

```text
R3_RESUME_ALLOWED remains NO until owner accepts NO-SMOKE gaps closed.
```

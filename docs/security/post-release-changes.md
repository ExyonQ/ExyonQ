# Post-release changes — after v0.9.1

**Status: PREPARED, NOT RELEASED.** These changes are in the working tree for
the next release candidate. There is no `v0.9.2` tag, GitHub Release or image;
this file is not evidence that any of them were published.

The five changes below close the audit findings tracked for the next release.
They must be re-verified against the exact tagged commit and recorded in
`audit-vX.Y.Z.md`; this page is a change register, not a release audit.

| ID | Change | Why it matters | Real verification |
| --- | --- | --- | --- |
| H-01 | The request target is canonicalized once, before auth/WAF/route selection, on Hyper request paths. The raw static wire refuses non-canonical spellings and sensitive names, falling back to Hyper. | Prevents `/./admin`, `//admin`, `/%61dmin` or an encoded sensitive name from selecting a different route than the one the policy gates inspect. | `scripts/e2e/h01-l01-canonical-host-e2e.sh`; unit coverage in the core path-canonicalization tests. |
| M-01 | Module map keys are validated as route names at config validation; an unknown name fails instead of silently disabling a module for that route. | A typo cannot quietly remove auth, limits or a transform from a production route. | `cargo nextest run -p exyonq-config-ir` (module route-name validation). |
| M-02 | The raw proxy wire forwards `Cookie`, `Authorization`, `Host`, `Range` and `Accept-Encoding` to the upstream; the raw relay stays disabled until its header parity is proven. Hop-by-hop headers and `Content-Length` on an empty GET are not forwarded. | The wire path and the Hyper path must send the same end-to-end request to the origin; selection affinity must not be mistaken for header forwarding. | `cargo test -p exyonq-mod-proxy --test m02_header_parity --locked` against two real loopback upstreams. |
| L-01 | HTTP/1.x origin-form requests without `Host` are not admitted to the static wire when a host-scoped route can match; they reach Hyper, which rejects them. | A host-scoped listener must not serve a static file without evaluating its host policy. | `scripts/e2e/h01-l01-canonical-host-e2e.sh` (missing `Host` → `400`, correct `Host` → `200`). |
| L-04 | The control Unix socket is bound normally and chmodded to owner-only mode; no process-global `umask()` mutation races with other threads. | The socket must not be accessible to other local users, and the service must not change the process-wide umask. | Control-socket unit tests; inspect the inode mode after bind. |

## Not claimed

- The release audit for the next version is **NOT EXECUTED** until the tag
  exists and `scripts/verify-release-audit.sh X.Y.Z` passes against that tag's
  commit.
- The H-01/L-01 E2E is not release evidence until it runs on native Linux
  amd64 and arm64 with the binary hash recorded by the harness.
- No performance improvement is claimed for the wire path; the header-parity
  test is a correctness test, not a benchmark.

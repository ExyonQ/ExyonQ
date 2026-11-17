# COMPAT — Apache httpd → ExyonQ (0.9.x)

<!-- compat-matrix
source=apache
exact=0
approximated=1
unsupported=0
directives=17
p1_open=0
p2_open=0
-->

Measured with `exyonq-compat migrate --from apache` over
`tests/fixtures/compat-matrix/apache.conf`, a two-vhost file covering the
documented equivalent-importer scope (`docs/compat/compatibility-scope-v1.md`).
The importer is fail-closed: a directive outside the allowlist refuses the whole
migration (a `<Directory>` block with `Options`/`AllowOverride` did exactly that
while this fixture was being written, and was removed because it is out of the
declared scope, not because it was silently dropped).

`exact` is 0 because the Apache report counts *notes* rather than exact entries:
3 `migrated` notes and 1 `approximated` over 17 directive instances in the
fixture. `scripts/gates/compat-matrices-gate.sh` re-measures it on every run
(the gate is not wired into `cargo xtask ci` yet — run it by hand) and fails if
the block above drifts or drops a metric.

| Directive | Instances | Status | Note |
|---|---|---|---|
| `Listen` | 2 | ✅ migrated | Two listeners |
| `<VirtualHost>` | 2 | ✅ migrated | Mapped to ExyonQ servers |
| `ServerName` | 2 | ✅ migrated | `match.host` |
| `ServerAlias` | 1 | ✅ migrated | Additional host match |
| `DocumentRoot` | 2 | ✅ migrated | Static root per vhost |
| `DirectoryIndex` | 1 | ⚠️ approximated | Only the first filename reaches `route.index` |
| `Alias` | 1 | ✅ migrated | Extra static route |
| `Redirect` | 1 | ✅ migrated | Permanent redirect route |
| `RewriteEngine` | 1 | ✅ migrated | Enables the rewrite block |
| `RewriteCond` | 1 | ✅ migrated | `rewrite.when` (ADR-048) |
| `RewriteRule` | 1 | ✅ migrated | `match.regex` + rewrite rule |
| `SSLEngine` | 1 | ✅ migrated | TLS on the vhost |
| `SSLCertificateFile` | 1 | ✅ migrated | `[server.tls]` cert path |
| `SSLCertificateKeyFile` | 1 | ✅ migrated | `[server.tls]` key path |
| `ProxyPass` | 1 | ✅ migrated | Upstream + proxy route |

## Open gaps

`P1_OPEN=0 P2_OPEN=0`. `DirectoryIndex` is P3 **with a workaround that has to be
applied explicitly**: the runtime resolves a single index per route
(`crates/exyonq-mod-static/src/root.rs`), so the importer sets `route.index` to
the first filename and the remaining filenames are *not* served — generic static
resolution does not fall back to them. List the primary filename first, or add
an explicit route per extra index; an earlier revision of this document claimed
the other filenames resolved statically, which is wrong.

## Out of scope

`<Directory>`, `Options`, `AllowOverride`, `Require` and the `.htaccess` overlay
are not part of this importer's allowlist; `.htaccess` has its own offline UX
(`exyonqctl config htaccess`). Anything outside the allowlist fails closed.

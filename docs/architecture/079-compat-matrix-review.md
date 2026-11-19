# COMPAT matrices review (Transversal — matrices P1/P2)

The roadmap asks for the three compatibility matrices to carry zero open P1/P2
gaps, with an independent review of all three. This document records what is
measured from this checkout and what the measured gap surface is. The measured
half is at zero open P1/P2 as of 2026-11-16; the independent review is still
external by definition.

## What the matrices are here

The three matrices the roadmap names did not exist as versioned docs: `git log
--all` on those paths is empty. Two copies existed on this machine — an
untracked `ROADMAP EXYONQ/COMPAT_*.md` set and a snapshot under
`.exyonq-local/act-final-20261104/` — both written for v0.4.x (they describe
`migrate_to_exyonq.py` and `ghcr.io/exyonq/exyonq:0.4.13`). They were not
restored: a v0.4.x matrix is not the 0.9.2 artifact of record.

The matrices were **rebuilt from measurement** as `docs/compat/COMPAT_NGINX.md`,
`docs/compat/COMPAT_APACHE.md` and `docs/compat/COMPAT_OPENLITESPEED.md`. Each
carries a machine-readable `<!-- compat-matrix -->` block with the counts and
`p1_open` / `p2_open`, and a table of the directives the importer actually
reported. `scripts/gates/compat-matrices-gate.sh` re-measures the three sources
and compares. The gate is **not** wired into `cargo xtask ci`; run it by hand.

| Source | Artifact measured | Command |
|---|---|---|
| NGINX | `tests/fixtures/migrations/*.conf` (5 real configs) | `exyonqctl config migrate-nginx --report` |
| Apache httpd | `tests/fixtures/compat-matrix/apache.conf` | `exyonq-compat migrate --from apache` |
| OpenLiteSpeed | `tests/fixtures/compat-matrix/ols-throttle.conf` | `exyonq-compat ols-throttle` |

## What the gate enforces (2026-11-16)

1. **No silent approximation.** A `gap` line is emitted for every approximated
   (LOSSY) *and* every refused (UNSUPPORTED) entry the importers report; each one
   needs a `tests/fixtures/compat-matrix/gaps.txt` line with a priority and a
   status. Before this change only refusals were surfaced, which is how 21 lossy
   entries lived outside `gaps.txt`.
2. **No open P1/P2.** A `P1 open` or `P2 open` line fails the run.
3. **No drift.** The measurement must match `expectations.txt`.
4. **No publishing shortcuts.** Every metric in `expectations.txt` must appear in
   that source's published block *and* match it; deleting `exact=` from a block
   is a failure, not a pass. `p1_open` / `p2_open` are per source, not a global
   number repeated three times.

`--selftest` measures the real corpus and then injects five defects: an open P1,
an undeclared lossy entry, a drifted expectation, a matrix that contradicts the
measurement, and a block that dropped a metric. All five are rejected; the
baseline (same gaps downgraded to `workaround` in a temporary copy) passes.

## Measured result (2026-11-16, debug binaries)

| Source | exact | approximated (LOSSY) | refused (unsupported) | exit |
|---|---|---|---|---|
| NGINX (5 configs) | 79 | 13 | 5 | 0 each |
| Apache httpd | — | 1 (note) | 0 | 0 |
| OpenLiteSpeed | — | 0 | 0 (9 fields emitted) | 0 |

Four changes moved entries from lossy to exact: `ssl_certificate` +
`ssl_certificate_key` map to `[server.tls]` (and the `ssl` listen flag is served
by it), an `location =` is no longer reported lossy when it maps to
`match.exact`, and — the last one — `return <status> "<body>"` now maps to
`route.respond`, a first-class literal response in the IR.

The Apache import reports 4 notes (3 `migrated`, 1 `approximated`:
`DirectoryIndex` keeps only the first filename) and 0 unsupported. The OLS
translation covers the whole `perClientConnLimit` block into
`[modules.ratelimit.client]`; `exact=0` there counts report entries, while the
nine ✅ rows count emitted fields — the matrix says so explicitly.

## The gap surface: no open P1/P2

| Source | Directive | Priority | Status | Why |
|---|---|---|---|---|
| nginx | `location` (`~`/`~*` precedence) | P2 | workaround | Declare the same set as explicit routes; a bare prefix route can answer what nginx sent to the regex block |
| nginx | `expires` | P3 | workaround | Positive `max_age` only |
| nginx | `index` | P3 | workaround | Single index per route; list the primary first |
| nginx | `try_files` | P3 | workaround | Variable forms → `front_controller`; `=404` is the default 404 |
| nginx | `add_header Cache-Control` | P3 | workaround | Cache policy in `[cache]`/`expires` (ADR-062) |
| nginx | `client_max_body_size` | P3 | workaround | Route body budget |
| nginx | `proxy_set_header` (Upgrade/Connection) | P3 | workaround | Hop-by-hop, runtime-owned (ADR-051) |
| nginx | `keepalive` | P3 | workaround | IR pool policy |
| apache | `DirectoryIndex` | P3 | workaround | First filename → `route.index`; the others are **not** served statically |

`return <status> "<body>"` is no longer a gap. The IR gained `route.respond`
(`status`, `body`, optional `content_type`) with schema, canonical form and
validation; the handler serves it after ACL/auth/zone-limit/request-WAF and
before static or proxy resolution, so a literal route wins over an index file at
the same path and never takes the wire fast path (it is excluded there). Real
HTTP evidence: `tests/integration/literal_response.rs` asserts `200` + body `ok`
+ `text/plain; charset=utf-8` for `/healthz` while a file of the same name exists
in the root, a `418` + `application/json` literal, and both IR rejections
(`respond` with a backend, `respond.status = 204`). Importer evidence:
`compat/nginx/tests/literal_response_mapping.rs` covers the mapped body, the
variable body (stays lossy) and `return 204` (stays refused).

`P1_OPEN=0 P2_OPEN=0` and the gate now exits 0:

```
MATRICES=3 P1_OPEN=0 P2_OPEN=0
RESULT=PASS COMPAT_MATRICES_GATE                    exit 0
```

## What is not closed here

1. The **artifact of record**: which matrices the reviewer signs is an owner
   decision. `docs/compat/*` is the only candidate in the tree; the v0.4.x
   copies under `.exyonq-local/` are stale and untracked.
2. The **independent reviewer** is external by definition: «Revisión
   independiente de las tres matrices COMPAT P1/P2 | Requiere un revisor externo
   al árbol de trabajo».

## Negative control (real runs, 2026-11-16)

```
# gate against the real artifacts
MATRICES=3 P1_OPEN=0 P2_OPEN=0
RESULT=PASS COMPAT_MATRICES_GATE                exit 0

# --selftest: real measurement + six injected defects
PASS selftest baseline (real measurement, expectations and matrices agree)
PASS selftest open-P1 (gate rejected)
PASS selftest undeclared-gap (gate rejected)
PASS selftest count-drift (gate rejected)
PASS selftest matrix-drift (gate rejected)
PASS selftest block-completeness (gate rejected)
PASS selftest return-p1 (gate rejected an unmapped return body)
RESULT=PASS COMPAT_MATRICES_GATE_SELFTEST       exit 0
```

The `return-p1` control is the one that keeps this honest: it feeds the gate the
real facts plus a synthetic lossy `return` entry (what the importer would report
if `route.respond` stopped being emitted) with the manifest declaring it `P1
open`, and requires the gate to reopen P1. It fails if the green baseline is
taken on trust.

# ExyonQ — Documentation Index (canonical)

```text
DOCUMENT = DOCUMENTATION_INDEX.md
DOCUMENT_ROLE = CANONICAL_DOCUMENTATION_MAP
AUTHORITY = SOLE inventory of documentation roles for this worktree
LAST_UPDATED = 2026-10-03
BRANCH = main
PUBLIC_RELEASE = v0.4.7
HEAD = 99bd7c547cb4d7c7821a1e131dfca5cf8d8db987
```

Companions: [`PROJECT_STATUS.md`](PROJECT_STATUS.md) · [`ROADMAP.md`](ROADMAP.md)

```text
DOCS_SOT = /docs/* ignored locally; canonical exceptions:
  docs/security/** , docs/governance/** , docs/releases/**
```

Local / gitignored documentation (other `/docs/` subtrees) appears as:

```text
LOCAL_ONLY
NOT_RELEASE_CONTENT
NOT_CANONICAL_UNLESS_PROMOTED
```

---

## Phase ID taxonomy

| Prefix | Meaning | Global phase authority? |
|--------|---------|-------------------------|
| `R*` | Global roadmap (`ROADMAP.md`) | **YES — only these** |
| `BV04-P*` | Benchmark v0.4.x | No |
| `K8S-P*` | Kubernetes wedge | No |
| `P14*` | Release/signing historical | No |
| `K*` / `KD*` | Kernel/core historical | No |
| `MR*` | Modern-rival validation | No |

```text
Only R* determines the current global project phase.
```

---

## Classification vocabulary

| STATUS | Meaning |
|--------|---------|
| `CANONICAL` | Sole authority for its function |
| `IMPLEMENTED_REFERENCE` | Describes behavior backed by code/tests/accepted close |
| `IMPLEMENTATION_IN_PROGRESS` | Active design/build; not finished |
| `FUTURE_DESIGN` | Not implemented; not current product behavior |
| `HISTORICAL_EVIDENCE` | Past audit/close/result; not current ops instructions |
| `SUPERSEDED` | Replaced; successor named |
| `GENERATED_REPORT` | Regenerable output |
| `DUPLICATE` | Same content role as another path |
| `UNKNOWN_REVIEW` | Conflict or insufficient evidence |

Recommended actions (proposal only): `KEEP_ACTIVE` · `ADD_STATUS_HEADER` · `LINK_FROM_CANONICAL_INDEX` · `MOVE_TO_HISTORY_LATER` · `DELETE_LATER` · `KEEP_LOCAL_ONLY` · `REVIEW`

---

## A. Canonical authorities (new)

| PATH | TITLE | TRACKED_OR_LOCAL | AREA | DOCUMENT_TYPE | STATUS | CANONICAL_SUCCESSOR | IMPLEMENTATION_STATUS | LAST_EVIDENCE_DATE | REFERENCED_BY | RECOMMENDED_ACTION |
|------|-------|------------------|------|---------------|--------|---------------------|----------------------|--------------------|---------------|--------------------|
| `PROJECT_STATUS.md` | Project Status | TRACKED (this commit) | project | status | CANONICAL | — | current | 2026-07-31 | ROADMAP, this index | KEEP_ACTIVE |
| `ROADMAP.md` | Global Roadmap R0–R6 | TRACKED (this commit) | project | roadmap | CANONICAL | — | current | 2026-07-31 | PROJECT_STATUS | KEEP_ACTIVE |
| `DOCUMENTATION_INDEX.md` | Documentation Index | TRACKED (this commit) | project | index | CANONICAL | — | current | 2026-07-31 | PROJECT_STATUS | KEEP_ACTIVE |

---

## B. Tracked product / maintainer documentation

| PATH | TITLE | TRACKED_OR_LOCAL | AREA | DOCUMENT_TYPE | STATUS | CANONICAL_SUCCESSOR | IMPLEMENTATION_STATUS | LAST_EVIDENCE_DATE | REFERENCED_BY | RECOMMENDED_ACTION |
|------|-------|------------------|------|---------------|--------|---------------------|----------------------|--------------------|---------------|--------------------|
| `README.md` | ExyonQ product README | TRACKED | product | overview | IMPLEMENTED_REFERENCE | PROJECT_STATUS for phase | v0.4.2 line | 2026-07 | public entry | KEEP_ACTIVE; ADD_STATUS_HEADER linking PROJECT_STATUS |
| `ARCHITECTURE.md` | Source-tree architecture | TRACKED | architecture | overview | IMPLEMENTED_REFERENCE | — | layout current | 2026-07 | README | KEEP_ACTIVE |
| `CHANGELOG.md` | Changelog | TRACKED | release | changelog | HISTORICAL_EVIDENCE | — | partial vs HEAD | UNKNOWN | packaging | REVIEW vs Changeset A/B |
| `CONTRIBUTING.md` | Contributing | TRACKED | process | guide | IMPLEMENTED_REFERENCE | — | process | UNKNOWN | README | KEEP_ACTIVE |
| `SECURITY.md` | Security policy | TRACKED | security | policy | CANONICAL | docs/security/README | current | 2026-08-03 | release | KEEP_ACTIVE |
| `docs/security/README.md` | Security SoT index | TRACKED | security | index | CANONICAL | — | GRC-R5 | 2026-08-03 | SECURITY.md | KEEP_ACTIVE |
| `docs/security/audit-template.md` | Release audit template | TRACKED | security | template | CANONICAL | audit-vX.Y.Z | GRC-R5 | 2026-08-03 | xtask, verify-release-audit | KEEP_ACTIVE |
| `docs/security/release-checklist.md` | Release security checklist | TRACKED | security | checklist | CANONICAL | — | GRC-R5 | 2026-08-03 | SECURITY.md | KEEP_ACTIVE |
| `docs/security/private-material-zero.md` | PMZ policy | TRACKED | security | policy | CANONICAL | — | GRC-R5 | 2026-08-03 | scanner/CI | KEEP_ACTIVE |
| `docs/governance/README.md` | Governance SoT index | TRACKED | governance | index | CANONICAL | — | GRC-R5 | 2026-08-03 | pre-R5 gate | KEEP_ACTIVE |
| `docs/governance/project-integrity.md` | Project integrity policy | TRACKED | governance | policy | CANONICAL | — | GRC-R5 | 2026-08-03 | integrity gate | KEEP_ACTIVE |
| `docs/governance/benchmark-integrity.md` | Benchmark integrity policy | TRACKED | governance | policy | CANONICAL | — | GRC-R5 | 2026-08-03 | integrity gate | KEEP_ACTIVE |
| `docs/governance/data-provenance.md` | Data provenance policy | TRACKED | governance | policy | CANONICAL | — | GRC-R5 | 2026-08-03 | provenance gate | KEEP_ACTIVE |
| `docs/governance/signing-readiness.md` | Signing readiness | TRACKED | governance | policy | CANONICAL | security/signing | GRC-R5 | 2026-08-03 | R5 later | KEEP_ACTIVE |
| `docs/governance/enforcement-matrix.md` | Enforcement matrix | TRACKED | governance | map | CANONICAL | — | GRC-R5 | 2026-08-03 | pre-R5 | KEEP_ACTIVE |
| `docs/governance/evidence/INDEX.md` | Evidence binder | TRACKED | governance | index | CANONICAL | — | GRC-R5 | 2026-08-03 | PROJECT_STATUS | KEEP_ACTIVE |
| `docs/governance/exceptions/RUSTSEC-2026-0222.md` | Wasmtime waiver | TRACKED | governance | exception | CANONICAL | — | ACTIVE_WAIVER | 2026-08-03 | deny/audit | KEEP_ACTIVE |
| `THIRD_PARTY_NOTICES.md` | Third-party notices | TRACKED | legal | notices | IMPLEMENTED_REFERENCE | — | — | UNKNOWN | packaging | KEEP_ACTIVE |
| `compat/README.md` | Compat layer | TRACKED | compat | module | IMPLEMENTED_REFERENCE | — | present | UNKNOWN | architecture | KEEP_ACTIVE |
| `crates/exyonq-mod-fastcgi/README.md` | FastCGI module | TRACKED | fastcgi | module | IMPLEMENTED_REFERENCE | — | module present | UNKNOWN | crates | KEEP_ACTIVE |
| `modules/_template/README.md` | Module template | TRACKED | modules | template | IMPLEMENTED_REFERENCE | — | template | UNKNOWN | modules | KEEP_ACTIVE |
| `packaging/README.md` | Packaging | TRACKED | packaging | guide | IMPLEMENTED_REFERENCE | — | packaging | UNKNOWN | release | KEEP_ACTIVE |
| `packaging/config/distributed-cache-health.example.md` | Dist-cache health example | TRACKED | packaging | example | FUTURE_DESIGN / UNKNOWN_REVIEW | — | example only | UNKNOWN | packaging | ADD_STATUS_HEADER NOT IMPLEMENTED if not wired |
| `integrations/wordpress/REDIS_OBJECT_CACHE.md` | WP Redis notes | TRACKED | integrations | guide | UNKNOWN_REVIEW | — | integration | UNKNOWN | — | REVIEW |
| `integrations/wordpress/exyonq-cache/README.md` | WP cache plugin | TRACKED | integrations | module | UNKNOWN_REVIEW | — | plugin tree | UNKNOWN | — | REVIEW |
| `scripts/architecture/README.md` | Architecture scripts | TRACKED | architecture | tooling | IMPLEMENTED_REFERENCE | study reports (local) | gates present | 2026-06+ | verify-phase0 | KEEP_ACTIVE; note study LOCAL_ONLY |
| `scripts/architecture/fixtures/7b-negative/README.md` | Negative fixtures | TRACKED | architecture | test | IMPLEMENTED_REFERENCE | — | fixtures | UNKNOWN | architecture lint | KEEP_ACTIVE |
| `scripts/release/tests/fixtures/README.md` | Release fixtures | TRACKED | release | test | IMPLEMENTED_REFERENCE | — | fixtures | UNKNOWN | release tests | KEEP_ACTIVE |
| `scripts/smoke/K0.4-core-smoke-matrix.md` | K0.4 smoke matrix | TRACKED | smoke | historical-runbook | HISTORICAL_EVIDENCE | ROADMAP R2 for current qual | k0 matrix still referenced | UNKNOWN | k0 scripts | KEEP_ACTIVE or ADD_STATUS_HEADER |
| `security/signing/README.md` | Signing README | TRACKED | security | runbook | HISTORICAL_EVIDENCE | TRUST-POLICY + PROJECT_STATUS | living/stale mix | UNKNOWN | release | REVIEW; ADD_STATUS_HEADER |
| `security/signing/TRUST-POLICY.md` | Trust policy | TRACKED | security | policy | IMPLEMENTED_REFERENCE | — | policy | UNKNOWN | signing | KEEP_ACTIVE |
| `tests/fixtures/README.md` | Test fixtures | TRACKED | tests | guide | IMPLEMENTED_REFERENCE | — | fixtures | UNKNOWN | tests | KEEP_ACTIVE |
| `tests/fixtures/tls/README.md` | TLS fixtures policy | TRACKED | security | guide | IMPLEMENTED_REFERENCE | ephemeral TLS scripts | PMZ | UNKNOWN | smokes | KEEP_ACTIVE |
| `.github/pull_request_template.md` | PR template | TRACKED | process | template | IMPLEMENTED_REFERENCE | — | — | UNKNOWN | GitHub | KEEP_ACTIVE |

### P14 release ledgers (tracked under `docs/releases/`)

Note: `/docs/*` is ignored with explicit exceptions for `security/`, `governance/`, and `releases/`.

| PATH | TITLE | TRACKED_OR_LOCAL | AREA | DOCUMENT_TYPE | STATUS | CANONICAL_SUCCESSOR | IMPLEMENTATION_STATUS | LAST_EVIDENCE_DATE | REFERENCED_BY | RECOMMENDED_ACTION |
|------|-------|------------------|------|---------------|--------|---------------------|----------------------|--------------------|---------------|--------------------|
| `docs/releases/p14v041-scope-audit.md` | P14V041 scope audit | TRACKED | P14 | audit | HISTORICAL_EVIDENCE / SUPERSEDED | PROJECT_STATUS + ROADMAP | frozen | ~2026-07 | p14v042 | KEEP_ACTIVE (history) |
| `docs/releases/p14v041-execution.md` | P14V041 execution | TRACKED | P14 | ledger | HISTORICAL_EVIDENCE | — | frozen | ~2026-07 | close | KEEP_ACTIVE |
| `docs/releases/p14v041-close.md` | P14V041 close | TRACKED | P14 | close | HISTORICAL_EVIDENCE | — | frozen | ~2026-07 | — | KEEP_ACTIVE |
| `docs/releases/p14v042-scope-proposal.md` | P14V042 proposal | TRACKED | P14 | proposal | HISTORICAL_EVIDENCE / SUPERSEDED | p14v042-scope-audit | frozen | ~2026-07 | audit | KEEP_ACTIVE |
| `docs/releases/p14v042-scope-audit.md` | P14V042 scope audit | TRACKED | P14 | audit | HISTORICAL_EVIDENCE | PROJECT_STATUS for current | frozen | ~2026-07 | deps | KEEP_ACTIVE |
| `docs/releases/p14v042-execution.md` | P14V042 execution | TRACKED | P14 | ledger | HISTORICAL_EVIDENCE | — | frozen | ~2026-07 | — | KEEP_ACTIVE |
| `docs/releases/p14v042-dependency-decisions.md` | P14V042 deps | TRACKED | P14 | decisions | HISTORICAL_EVIDENCE | — | frozen | ~2026-07 | — | KEEP_ACTIVE |

```text
DOCUMENTS_TRACKED_INVENTORIED ≈ 29 pre-existing + 3 new canonical = 32 after commit
```

---

## C. Local-only / ignored documentation

```text
TRACKED_OR_LOCAL = LOCAL_ONLY
NOT_RELEASE_CONTENT = YES
NOT_CANONICAL_UNLESS_PROMOTED = YES
```

### C1. `study/reports/**` (untracked; referenced by tracked architecture scripts)

| PATH | TITLE | AREA | DOCUMENT_TYPE | STATUS | CANONICAL_SUCCESSOR | RECOMMENDED_ACTION |
|------|-------|------|---------------|--------|---------------------|--------------------|
| `study/reports/11-phase0-kernel-freeze-signoff.md` | Phase 0 freeze signoff | architecture | contract | HISTORICAL_EVIDENCE / IMPLEMENTED_REFERENCE (local) | promote candidate | KEEP_LOCAL_ONLY; REVIEW promote |
| `study/reports/KERNEL-AUDIT-current-vs-phase0.md` | Kernel audit current vs P0 | architecture | audit | HISTORICAL_EVIDENCE | — | KEEP_LOCAL_ONLY; REVIEW promote |
| `study/reports/KERNEL-AUDIT-v1-archive.md` | Kernel audit v1 archive | architecture | audit | HISTORICAL_EVIDENCE | — | KEEP_LOCAL_ONLY |
| `study/reports/12-frozen-vs-swappable-checklist.md` | Frozen vs swappable | architecture | checklist | HISTORICAL_EVIDENCE | — | KEEP_LOCAL_ONLY; REVIEW promote |
| `study/reports/HANDOFF-kernel-official-developer.md` | Kernel handoff | architecture | handoff | HISTORICAL_EVIDENCE | — | KEEP_LOCAL_ONLY |
| `study/reports/PR-3-EPOLL-GENERATION-SMOKE-AMD64-ARM.md` | PR-3 dual-arch smoke | architecture | evidence | HISTORICAL_EVIDENCE | — | KEEP_LOCAL_ONLY |
| `study/reports/pr3-smoke-packets/*.tar.gz` | Packed smoke evidence | architecture | packet | GENERATED_REPORT | — | KEEP_LOCAL_ONLY |

### C2. Ignored `docs/**` on disk (beyond force-tracked releases)

| PATH | TITLE | AREA | STATUS | RECOMMENDED_ACTION |
|------|-------|------|--------|--------------------|
| `docs/build/ALLOCATOR_SELECTION.md` | Allocator selection | build | UNKNOWN_REVIEW / DUPLICATE risk vs product | REVIEW; possible promote |
| `docs/product/build/ALLOCATOR_SELECTION.md` | Allocator selection (product copy) | build | DUPLICATE | REVIEW vs docs/build |
| `docs/architecture/**` | Architecture panel/local | architecture | UNKNOWN_REVIEW | REVIEW |
| `docs/benchmarks/**` | Benchmark methodology locals | bench | GENERATED_REPORT / UNKNOWN | KEEP_LOCAL_ONLY |
| other `docs/*` dirs (roadmap, observability, …) | sparse / empty shells | mixed | UNKNOWN_REVIEW | REVIEW |

### C3. Cursor IDE (`.cursor/**`) — LOCAL_ONLY

| GROUP | COUNT (approx) | STATUS | RECOMMENDED_ACTION |
|-------|----------------|--------|--------------------|
| `.cursor/rules/*.mdc` | ~35 | IMPLEMENTED_REFERENCE (local governance) | KEEP_LOCAL_ONLY; not GitHub release content |
| `.cursor/agents/*.md` | ~17 | LOCAL tooling | KEEP_LOCAL_ONLY |
| `.cursor/plans/**` | ~50+ | mixed FUTURE_DESIGN / HISTORICAL / SUPERSEDED | KEEP_LOCAL_ONLY; do not treat as R\* |
| `.cursor/plans/realizados/**` | archived plans | SUPERSEDED / HISTORICAL_EVIDENCE | KEEP_LOCAL_ONLY |
| `.cursor/skills/**` | mem0/skills | tooling | KEEP_LOCAL_ONLY |

Notable FUTURE_DESIGN plans (examples; **NOT IMPLEMENTED / NOT CURRENT PRODUCT BEHAVIOR** unless separately proven): `13-distributed-discovery-p2p`, `80-admin-api-control-plane`, `90-exyapp-exyphp`, `91-wasm-h3-mtls-admin-ui`, `94-grpc-lb-ai-gateway`.

Notable historical/active local plans: `00-roadmap-core-modules-migration`, `08-fastcgi-php-fpm`, `12-cache-microcache`, `phase1-strategic-closure` — classify individually before any promote; none override `ROADMAP.md`.

### C4. Other local / generated

| PATH | STATUS | RECOMMENDED_ACTION |
|------|--------|--------------------|
| `graphify-out/**` | GENERATED_REPORT | KEEP_LOCAL_ONLY / DELETE_LATER |
| `benchmarks/results/**/*.md` | GENERATED_REPORT / HISTORICAL_EVIDENCE | KEEP_LOCAL_ONLY |
| `waf/exyonq-waf-package-20260726/.cursor/**` | UNKNOWN_REVIEW (empty package scaffold) | REVIEW; KEEP_LOCAL_ONLY |
| `.pytest_cache/README.md` | GENERATED_REPORT | DELETE_LATER |
| `dist/sim/THIRD_PARTY_NOTICES.md` | UNKNOWN_REVIEW | REVIEW |
| `.cursor-local/incidents/**` | HISTORICAL_EVIDENCE | KEEP_LOCAL_ONLY |

---

## D. Proposed promotions (later; not this commit)

| Candidate | Why |
|-----------|-----|
| `study/reports/11-phase0-kernel-freeze-signoff.md` | Referenced as contract by tracked `verify-phase0-kernel.sh` |
| `study/reports/KERNEL-AUDIT-current-vs-phase0.md` | Linked from tracked architecture README |
| `docs/build/ALLOCATOR_SELECTION.md` (if still accurate) | Build policy may belong in tracked docs after allowlist |

Promotion requires: owner allowlist, secret scan, and decision whether `/docs/` ignore policy changes (separate authorization).

---

## E. Proposed future moves / deletions (later)

| Action | Paths | Notes |
|--------|-------|-------|
| MOVE_TO_HISTORY_LATER | none yet as repo `docs/history/` | Prefer SUPERSEDED headers over new `Archivo/` in git |
| DELETE_LATER | `graphify-out/**`, `.pytest_cache/**`, duplicate allocator copy if confirmed | After owner auth |
| DELETE_LATER (local bench) | see prior benchmark storage triage | Not documentation |

---

## F. Owner review required

1. Whether Phase 0 `study/reports` should be promoted to tracked tree or remain LOCAL_ONLY with scripts still pointing there.  
2. Whether `CHANGELOG.md` must mention Changeset A/B before any v0.4.3 freeze.  
3. Whether ignored `docs/architecture` / `docs/benchmarks` content is stale vs clean publication README claims.  
4. Cursor plans that still look “active” vs `ROADMAP.md` R\* — close or retarget without creating parallel roadmaps.  

---

## G. Counts (approximate inventory snapshot 2026-07-31)

```text
DOCUMENTS_TOTAL ≈ 198 paths matching *.md/*.mdc/*.html in scoped walk
DOCUMENTS_TRACKED = 29 (before this commit) → 32 after adding 3 canonical
DOCUMENTS_LOCAL_ONLY ≈ 169 (incl. .cursor, study, ignored docs, graphify, waf cursor)

DOCUMENTS_CANONICAL = 3 (PROJECT_STATUS, ROADMAP, DOCUMENTATION_INDEX)
DOCUMENTS_IMPLEMENTED_REFERENCE ≈ 20 tracked product/tooling READMEs + local rules
DOCUMENTS_FUTURE_DESIGN ≈ Cursor plans 13/80/90/91/94 class + examples TBD
DOCUMENTS_HISTORICAL ≈ P14 ledgers + study reports + realizados plans
DOCUMENTS_SUPERSEDED ≈ realizados/obsoleto-* + older P14 proposal vs audit
DOCUMENTS_GENERATED ≈ graphify-out, bench result markdown, smoke packets
DOCUMENTS_UNKNOWN ≈ WP integrations, packaging distributed-cache example, waf scaffold, ignored docs shells
```

Exact per-file classification for every `.cursor/plans/*` name is deferred to follow-up REVIEW passes; none of those files are global-phase authorities.

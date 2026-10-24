# Governance documentation (canonical SoT)

```text
DOCS_GOVERNANCE_ROLE = CANONICAL_TRACKED_SOT
TRACKING = force-tracked under /docs/governance/** (see .gitignore exceptions)
```

| Path | Purpose |
|------|---------|
| [project-integrity.md](project-integrity.md) | Project integrity policy |
| [CLAIM_EVIDENCE_INTEGRITY_GUARD.md](CLAIM_EVIDENCE_INTEGRITY_GUARD.md) | Claim/evidence integrity guard — development governance; a claim may not exceed its evidence in strength, scope, execution class, platform, client or source state |
| [git-worktree-policy.md](git-worktree-policy.md) | Git worktree isolation: extra trees do not replace canonical lab, share mutable state, or authorize a second WIP |
| [benchmark-integrity.md](benchmark-integrity.md) | Benchmark integrity policy |
| [data-provenance.md](data-provenance.md) | Data provenance policy |
| [signing-readiness.md](signing-readiness.md) | Signing design readiness (no production sign) |
| [enforcement-matrix.md](enforcement-matrix.md) | Policy ↔ enforcement map |
| [exceptions/](exceptions/) | Release waiver register (minimal) |
| [evidence/INDEX.md](evidence/INDEX.md) | Evidence binder |
| [integrity/mrc-let-current-state.md](integrity/mrc-let-current-state.md) | MRC-LET subroot closures — current terminal state |
| [v044-p1-cpu-efficiency-causal.md](v044-p1-cpu-efficiency-causal.md) | P1 RAW WAF-off CPU-efficiency causal model (throughput baseline remains closed) |
| [v044-p1-cpu-efficiency-repair-root1.md](v044-p1-cpu-efficiency-repair-root1.md) | Root1 TLS StaticRuntime pin — CPU cost/req repair CLOSED_PROVEN |
| [v044-p1-cpu-efficiency-repair-root2.md](v044-p1-cpu-efficiency-repair-root2.md) | Root2 registry bypass — FALSIFIED_OR_TOO_SMALL; mutation reverted |
| [v044-p1-cpu-efficiency-repair-root3.md](v044-p1-cpu-efficiency-repair-root3.md) | Root3 reduce Cap067 `read_state` — CLOSED_PROVEN (−8.5% core-ms/req vs Root1) |
| [v044-p1-cpu-efficiency-live-3way.md](v044-p1-cpu-efficiency-live-3way.md) | Live ExyonQ/NGINX/OLS CPU-efficiency reconciliation after Root1+Root3 |
| [v044-p1-ols-fd-geometry-cost-attribution.md](v044-p1-ols-fd-geometry-cost-attribution.md) | P1 OLS FD geometry µs/req cost attribution (CASE A · sealed) |
| [v044-p1-fd-reuse-repair.md](v044-p1-fd-reuse-repair.md) | Cap067 FD reuse repair — CASE R1 material success; FD_REUSE_CAUSAL≠miss hang; see miss/NeedHyper hang doc |
| [v044-p1-cap067-miss-needhyper-hang.md](v044-p1-cap067-miss-needhyper-hang.md) | Cap067 miss/NeedHyper hang — CASE H1 CLOSED dual-arch; Alternative B epoll terminals; FD_REUSE_CAUSAL=NO |
| [performance-competitive-admission.md](performance-competitive-admission.md) | Competitive performance governance — ENGINEERING_BASELINE ≠ COMPETITIVE_WIN ≠ MIGRATION_VALUE |
| [v044-p1-final-post-repair-baseline.md](v044-p1-final-post-repair-baseline.md) | P1 final post-H1 authoritative rebaseline — CASE S1 CLOSED_CURRENT_COMPETITIVE_BASELINE (engineering) |
| [v044-p1-competitive-deficit-attribution.md](v044-p1-competitive-deficit-attribution.md) | P1 competitive deficit attribution — CASE C2; RECOMMENDED_ROOT=NONE; P2 later authorized |
| [v044-p2-static-64k-competitive-baseline.md](v044-p2-static-64k-competitive-baseline.md) | P2 static 64 KiB competitive admission — CASE P2-C2; COMPETITIVE_WIN=NOT_PROVEN |
| [v044-p3-static-1mib-competitive-baseline.md](v044-p3-static-1mib-competitive-baseline.md) | P3 static 1 MiB competitive admission — CASE P3-C1; COMPETITIVE_WIN=YES; migration PARTIAL |
| [v044-architecture-fitness-review-before-p4.md](v044-architecture-fitness-review-before-p4.md) | Architecture fitness before P4 — TARGETED_ARCHITECTURAL_EVOLUTION; no major rewrite |
| [v044-p1-fixed-stage-and-parallelism-ab.md](v044-p1-fixed-stage-and-parallelism-ab.md) | P1 fixed-stage/parallelism A/B — CASE AB3; PARALLELISM_UNDERUTIL STRONGLY_SUPPORTED |
| [v044-p1-default-worker-geometry-8.md](v044-p1-default-worker-geometry-8.md) | Default worker geometry SoT — CASE G1 CLOSED_VERIFIED_REAL_PRODUCTION |
| [v044-p4-proxy-1k-competitive-baseline.md](v044-p4-proxy-1k-competitive-baseline.md) | P4 proxy 1 KiB competitive admission — CASE P4-C2; COMPETITIVE_WIN=NOT_PROVEN; valid run 115113Z |
| [v044-cross-profile-performance-root-cause-review-p1-p4.md](v044-cross-profile-performance-root-cause-review-p1-p4.md) | Cross-profile P1–P4 root cause review — CASE B; multiple bounded roots; P5–P14 frozen |
| [v044-p4-proxy-path-causal-profiling.md](v044-p4-proxy-path-causal-profiling.md) | P4 proxy path causal profiling — CASE P4P-B; ROOT_1 decomposed; HAProxy governance reconciled |
| [v044-cap034-proxy-framing-keepalive-contract-review.md](v044-cap034-proxy-framing-keepalive-contract-review.md) | Cap034 framing/keep-alive contract review — CASE C34-C; safe evolution plausible; value NOT_BOUNDED |
| [v044-cap034-framing-keepalive-bounded-value-ab.md](v044-cap034-framing-keepalive-bounded-value-ab.md) | Cap034 A/B experiment — CASE AB-C34-B; +5.3% P4 RPS (NON_PRODUCTION_EXPERIMENT) |
| [v044-p4-proxy-execution-tax-decomposition.md](v044-p4-proxy-execution-tax-decomposition.md) | P4 proxy execution tax decomposition — CASE PX-B; ROOT_1A/1F/1G bounded; targeted evolution |
| [v044-data-plane-architecture-strategic-review.md](v044-data-plane-architecture-strategic-review.md) | Data plane architecture strategic review — CASE DP-C; proxy execution shell design; body-pipeline HOLD |
| [v044-proxy-execution-data-plane-architecture-design.md](v044-proxy-execution-data-plane-architecture-design.md) | Proxy execution DP design — **CASE PXDP-B**; worker-local ProxyConnection shell (design only) |
| [v044-proxy-execution-data-plane-contracts-ir.md](v044-proxy-execution-data-plane-contracts-ir.md) | Proxy execution contracts & IR — **CASE P0-A**; frozen PXDP-P0 authority (no product code) |
| [v044-p4-proxy-wire-body-pipeline-slim.md](v044-p4-proxy-wire-body-pipeline-slim.md) | P4 body-pipeline slim experiment — **HOLD** pending implementation gate |
| [v044-exyonq-tokio-worker-geometry-ab.md](v044-exyonq-tokio-worker-geometry-ab.md) | Tokio worker geometry A/B — **WG-B** / EC-A complete |
| [v044-runtime-topology-parallelism-budget-audit.md](v044-runtime-topology-parallelism-budget-audit.md) | Runtime topology audit — **RT-C**; Block B = Cap067 OS pool; multi-concurrency HOLD |
| [v044-p4-cap067-process-residency-ab.md](v044-p4-cap067-process-residency-ab.md) | Cap067 P4 residency A/B — **CP-C** NOT_MATERIAL |
| [v044-global-execution-architecture-strategic-review.md](v044-global-execution-architecture-strategic-review.md) | Global execution architecture — **GA-B** hybrid; P4 micro-opt STOPPED; PXDP-P6 / body-relay HOLD |
| [v044-specialized-proxy-event-loop-control.md](v044-specialized-proxy-event-loop-control.md) | Specialized mio TPC proxy prototype — **EP-C** competitive breakthrough; integration design next (not started) |
| [v044-specialized-proxy-dataplane-integration-design.md](v044-specialized-proxy-dataplane-integration-design.md) | Specialized dataplane contracts — **SD-B**; SDP-P1 not authorized |
| [v044-phase6a-fastcgi-module-architecture-admission.md](v044-phase6a-fastcgi-module-architecture-admission.md) | Phase 6A FastCGI/Module architecture admission — **CASE P6A-C**; product mutation NO; Phase 6B was later authorized separately |
| [v044-phase6b-sync-cfd-native-fastcgi-mvp.md](v044-phase6b-sync-cfd-native-fastcgi-mvp.md) | Phase 6B sync CFD-native FastCGI MVP — **CASE P6B-B**; ADR-038 |
| [v044-phase6b-fastcgi-mvp-terminal-seal.md](v044-phase6b-fastcgi-mvp-terminal-seal.md) | Phase 6B terminal seal — **CASE P6BSEAL-B**; causal KEEP_CONN Netcup; ADR021 governance INCOMPLETE (POSTHOC); Phase 6C NOT authorized at seal time |
| [v044-phase6c-fcgi-race01-bounded-hardening.md](v044-phase6c-fcgi-race01-bounded-hardening.md) | Phase 6C FCGI-RACE-01 — **CASE P6C-B**; ADR-039; pre-mutation PASS; residual post-probe window qualified nonblocking |
| [v044-phase6d-fastcgi-resource-concurrency-stress-qualification.md](v044-phase6d-fastcgi-resource-concurrency-stress-qualification.md) | Phase 6D FastCGI resource/concurrency stress — **CASE P6D-C**; Netcup qualification; PRODUCT_REPAIR_REQUIRED (P6D-L1-001 sticky pool idle/max); no product mutation in-WIP |
| [v044-p6d-l1-001-fastcgi-pool-reload-refresh-repair.md](v044-p6d-l1-001-fastcgi-pool-reload-refresh-repair.md) | P6D-L1-001 repair — ADR-040 Design D `ensure_pool` policy refresh; Netcup S8 eviction + KEEP_CONN; harness SUMMARY honesty |
| [v044-phase6d-residual-requalification-close.md](v044-phase6d-residual-requalification-close.md) | Phase 6D residual requalification — **CASE P6DRC-B**; P6D current = CLOSED_REQUALIFIED_WITH_NONBLOCKING_QUALIFICATIONS; parents P6D-C/P6DR-B preserved |
| [v044-phase6-fastcgi-pre-wordpress-blocker-repair-and-attribution-close.md](v044-phase6-fastcgi-pre-wordpress-blocker-repair-and-attribution-close.md) | Pre-WordPress FastCGI blockers — **CASE P6PREWP-B**; ADR-041 A2/A3 honesty; Track B attributable WS test PASS; WORDPRESS=NO |
| [v044-phase6-fastcgi-strong-reload-oracles-close.md](v044-phase6-fastcgi-strong-reload-oracles-close.md) | Strong reload oracles — **CASE P6ORACLE-C**; TTL+GEN STRONG; MAX INSUFFICIENT_EVIDENCE under serial FCGI |
| [v044-phase6-fastcgi-max-connections-strong-oracle-close.md](v044-phase6-fastcgi-max-connections-strong-oracle-close.md) | Max connections oracle close — **CASE P6MAX-C**; max>1 inoperative on serial HTTP; CONFIG MISLEADING; PRODUCT_MUTATION=NO |
| [v044-phase6-fastcgi-max-connections-config-honesty-close.md](v044-phase6-fastcgi-max-connections-config-honesty-close.md) | Max_conn config honesty — **CASE P6MAXHONEST-A**; ADR-042 reject gt1; WORDPRESS feasible then authorized as 6E |
| [v044-phase6e-current-head-real-wordpress-qualification.md](v044-phase6e-current-head-real-wordpress-qualification.md) | Phase 6E real WordPress on CURRENT CFD FastCGI — **CASE P6E-C** (immutable historical); pretty/`/wp-admin/` gaps at that terminal |
| [v044-phase6e-wordpress-routing-repair.md](v044-phase6e-wordpress-routing-repair.md) | Phase 6E routing repair — **CASE P6EROUTE-B** (immutable historical); ADR-043; DEFECT_1/2 CLOSED |
| [v044-phase6e-wordpress-routing-symlink-same-binary-reseal.md](v044-phase6e-wordpress-routing-symlink-same-binary-reseal.md) | Symlink same-binary reseal — **CASE P6EROUTE-A**; Netcup binary `5d4b6bdf…`; Phase 6E current = CLOSED_VERIFIED_REAL_PRODUCTION_PATH |
| [v044-phase6f-dual-arch-wordpress-qualification.md](v044-phase6f-dual-arch-wordpress-qualification.md) | Phase 6F dual-arch WordPress — **CASE P6F-A**; native Oracle ARM64 `7fae16d…`; `WORDPRESS_ARCHITECTURE_SCOPE=LINUX_AMD64_AND_LINUX_ARM64_VERIFIED`; STOP |
| [v044-phase6g-additional-generic-php-app-qualification.md](v044-phase6g-additional-generic-php-app-qualification.md) | Phase 6G second real PHP app (Laravel 13) — **CASE P6G-A**; Netcup AMD64; `GENERIC_PHP_APPLICATION_SCOPE=WORDPRESS_PLUS_SECOND_REAL_FRAMEWORK_VERIFIED`; STOP |
| [v044-phase7-static-proxy-coexistence.md](v044-phase7-static-proxy-coexistence.md) | Phase 7 static+proxy+FastCGI coexistence — **CASE P7-E**; native `BackendKind::Static` ABSENT; Proxy+FastCGI same-gen PASS; STOP |
| [v044-cfd-native-static-filesystem-handler-architecture-admission.md](v044-cfd-native-static-filesystem-handler-architecture-admission.md) | CFD native Static architecture admission — **CASE P7STATICADM-B**; ADR-044 `BackendKind::Static`; Cap067 remingle REJECT; parent of implementation WIP |
| [v044-cfd-native-static-filesystem-handler-implementation.md](v044-cfd-native-static-filesystem-handler-implementation.md) | CFD native Static implementation — **CASE P7STATICIMPL-B**; CFDRT005 + StaticSend + Netcup bounded reality (not P7 requal) |
| [v044-phase7-static-proxy-fastcgi-requalification.md](v044-phase7-static-proxy-fastcgi-requalification.md) | Phase 7 requalification — **CASE P7R-B**; native Static + Proxy + FastCGI same-gen on Netcup; parent P7STATICIMPL-B |
| [v044-static-dual-arch-qualification.md](v044-static-dual-arch-qualification.md) | Static dual-arch qualification — **CASE STATICDUAL-B**; native Oracle ARM64 Static parity with Netcup amd64; parent P7R-B |
| [v044-phase8-completeness-and-adversarial-security.md](v044-phase8-completeness-and-adversarial-security.md) | Phase 8 completeness + adversarial security — **CASE P8-C**; FIFO `openat2` shard hang MUST_FIX; STOP; no product repair |
| [v044-phase8-residual-ledger.md](v044-phase8-residual-ledger.md) | Phase 8 residual ledger R1–R9 + P8-STATIC-FIFO-001 |
| [v044-static-openat2-nonblock-special-file-repair.md](v044-static-openat2-nonblock-special-file-repair.md) | P8R repair — **CASE P8R-B**; FIFO defect closed dual Linux; product terminal unchanged |
| [v044-static-eagain-partial-sendfile-toctou-reoracle.md](v044-static-eagain-partial-sendfile-toctou-reoracle.md) | Static reoracle — **P8R-BR** MODE A dual Linux; EAGAIN/partial/sendfile/TOCTOU measured; not P8R-A |
| [v044-static-p8rbr-evidence-accounting-documentation-integration.md](v044-static-p8rbr-evidence-accounting-documentation-integration.md) | P8R-BR evidence accounting + docs candidate — **CASE P8RBRDOC-A**; FD/RSS reconciled; canonical merge NO |
| [v044-phase8-remainder-and-source-cleanliness-terminal.md](v044-phase8-remainder-and-source-cleanliness-terminal.md) | Phase 8 remainder + clean source — **CASE P8REM-D** blocked (clippy/oracle incomplete); dual-Linux builds PASS |
| [v044-p8rem-candidate-consistency-and-blocker-attribution.md](v044-p8rem-candidate-consistency-and-blocker-attribution.md) | P8REM candidate reconciliation — **CASE P8RECON-A**; identity/P3/Anti-K/blockers attributed; Phase 8 still open |
| [v044-phase8-clippy-all-features-and-oracle-completion.md](v044-phase8-clippy-all-features-and-oracle-completion.md) | Phase 8 clippy+oracle WIP — **CASE P8TECH-D**; clippy `--all-features` PASS; EAGAIN/partial/MAP/AK advanced; pread+trailing still incomplete; Phase 8 open |
| [v044-phase8-fcgi-trailing-data-causal-closure.md](v044-phase8-fcgi-trailing-data-causal-closure.md) | FastCGI trailing_data causal WIP — **CASE FCGITD-A**; harness race closed; dual-Linux 20/20 + CORE_MATRIX PASS; pread still NOT_MEASURED; Phase 8 open |
| [v044-phase8-pread-fallback-reality-admission.md](v044-phase8-pread-fallback-reality-admission.md) | Phase 8 pread fallback reality admission — **CASE PREAD-D**; aborted entry identity mismatch (expected `82ac7969…` absent; actual tip `82ac4ec2…`); historical WIP abort |
| [v044-phase8-pread-engineering-results-first.md](v044-phase8-pread-engineering-results-first.md) | Phase 8 pread engineering — **CASE PREAD-A**; dual-Linux controlled oracle `20260903T143458Z-seal`; natural EINVAL/ENOSYS not observed; test-utils seam only |
| [v044-phase8-canonical-documentation-seal.md](v044-phase8-canonical-documentation-seal.md) | Phase 8 canonical documentation seal — **CASE P8DOCSEAL-A**; `PHASE_8_CLOSED=YES` at `46b53b4f`; Phase 9 / official bench not authorized |
| [v044-fastcgi-application-compatibility-matrix.md](v044-fastcgi-application-compatibility-matrix.md) | CFD FastCGI app compatibility matrix (WordPress P6F-A + Laravel P6G-A) |

Gates:

```bash
bash scripts/gates/project-integrity-gate.sh --selftest
bash scripts/gates/benchmark-integrity-gate.sh --selftest
bash scripts/gates/data-provenance-gate.sh --selftest
bash scripts/gates/waiver-expiry-gate.sh --check
bash scripts/gates/pre-r5-grc-readiness-gate.sh
bash scripts/integrity/claim-evidence-check.sh --selftest
```

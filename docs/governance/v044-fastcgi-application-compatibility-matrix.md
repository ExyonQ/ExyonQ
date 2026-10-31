# FastCGI application compatibility (v0.4.4 CFD)

Evidence-bound matrix for the **current** CFD FastCGI path (ADR-043 front controller /
directory index). Not a performance ranking.

| Application | Version pin | Platforms | Terminal | Notes |
| --- | --- | --- | --- | --- |
| WordPress | 7.1 | Linux AMD64 Netcup + Linux ARM64 Oracle | P6F-A | Pretty permalinks, `/wp-admin/`, MariaDB, KEEP_CONN |
| Laravel | skeleton `laravel/laravel:13.10.1` / framework `v13.29.0` | Linux AMD64 Netcup | P6G-A | `public/` docroot; sessions/CSRF/ORM; owner initial close single-arch |

Phase 7 coexistence — **historical (P7-E, immutable):** same-generation **Proxy + FastCGI**
isolation PASS on Netcup; **native CFD static filesystem handler ABSENT**
(`BackendKind` = Proxy\|Fastcgi\|Reject only at that HEAD). See
[`v044-phase7-static-proxy-coexistence.md`](v044-phase7-static-proxy-coexistence.md).

Phase 7 requalification — **current (P7R-B, parent P7STATICIMPL-B):** one CFDRT005
generation with **native Static + Proxy + FastCGI** on Netcup amd64;
`CORE_MATRIX=PASS`, `ROUTE_CROSS_CONTAMINATION=MEASURED_0` (RUN
`20260901T140000Z`). Fixture docroot + minimal PHP app — not WordPress/Laravel
under this three-way topology. See
[`v044-phase7-static-proxy-fastcgi-requalification.md`](v044-phase7-static-proxy-fastcgi-requalification.md).

P6G “static asset” via proxy-to-Python fileserver is **not** native static proof.

```text
GENERIC_PHP_APPLICATION_SCOPE = WORDPRESS_PLUS_SECOND_REAL_FRAMEWORK_VERIFIED
TARGET_APP_SPECIFIC_PRODUCT_FAST_PATH = NO
FCGI_MAX_CONN = 1_ONLY
SYMLINK_TOCTOU_STATUS = BOUNDED_RESIDUAL
PHASE7_THREE_HANDLER_COEXISTENCE = P7R-B_BOUNDED_NETCUP_AMD64
PHASE7_HISTORICAL_STATIC_ABSENT = P7-E
PHASE8_THREE_HANDLER_ADVERSARIAL = P8-C_FIFO_MUST_FIX_NOT_A_FASTCGI_APP_FINDING
```

Historical terminals P6E-C / P6EROUTE-A / P6F-A / P6G-A remain immutable.

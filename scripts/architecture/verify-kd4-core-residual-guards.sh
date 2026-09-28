#!/usr/bin/env bash
# KD4 — Residual core census guards (baseline captures current inventory; no pre-existing debt = fail).
#
# Usage:
#   bash scripts/architecture/verify-kd4-core-residual-guards.sh baseline
#   bash scripts/architecture/verify-kd4-core-residual-guards.sh kd4_6
#   bash scripts/architecture/verify-kd4-core-residual-guards.sh kd4_7
#   bash scripts/architecture/verify-kd4-core-residual-guards.sh kd4_9
#   bash scripts/architecture/verify-kd4-core-residual-guards.sh kd4_10
#   bash scripts/architecture/verify-kd4-core-residual-guards.sh kd4_11
#   bash scripts/architecture/verify-kd4-core-residual-guards.sh kd4_12
#   bash scripts/architecture/verify-kd4-core-residual-guards.sh kd4_14
#   bash scripts/architecture/verify-kd4-core-residual-guards.sh closure   # future — strictest
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

MODE="${1:-${KD4_CORE_GUARD_MODE:-baseline}}"

if ! command -v rg >/dev/null 2>&1; then
  echo "verify-kd4-core-residual-guards: ripgrep required" >&2
  exit 2
fi

VIOLATIONS=0
DEBTS=0

fail() {
  VIOLATIONS=$((VIOLATIONS + 1))
  echo "verify-kd4-core-residual-guards: VIOLATION: $*" >&2
}

pass() {
  echo "verify-kd4-core-residual-guards: PASS: $*"
}

debt() {
  DEBTS=$((DEBTS + 1))
  echo "verify-kd4-core-residual-guards: DEBT (baseline allowed): $*"
}

record_inventory() {
  local label="$1"
  local count
  count="$(find core/src -name '*.rs' | wc -l | tr -d ' ')"
  pass "inventory $label: core/src *.rs files=$count"
  if command -v wc >/dev/null 2>&1; then
    local loc
    loc="$(find core/src -name '*.rs' -print0 | xargs -0 wc -l | tail -1 | awk '{print $1}')"
    pass "inventory $label: core/src LOC(wc -l)=$loc"
  fi
}

echo "verify-kd4-core-residual-guards: MODE=$MODE"

record_inventory "$MODE"

# --- KD3 closure invariants must remain green ---
if [[ -x scripts/architecture/verify-kd3-proxy-guards.sh ]]; then
  if bash scripts/architecture/verify-kd3-proxy-guards.sh closure >/dev/null 2>&1; then
    pass "KD3 closure guards still green (proxy boundary frozen)"
  else
    fail "KD3 closure guards regressed — fix before KD4 work"
  fi
fi

# --- No new backend-specific runtime in core (beyond allowlisted seams) ---
if rg -n 'mod proxy|core/src/proxy/' core/src --type rust 2>/dev/null | rg -v 'proxy_compiled|proxy_cluster|ProxyClient|proxy_dispatch|proxy_wire' >/dev/null 2>&1; then
  fail "resurrected core/src/proxy runtime surface"
else
  pass "no resurrected proxy runtime module in core"
fi

if rg -n 'mod fastcgi|FcgiRecord|connect_unix' core/src --type rust 2>/dev/null | rg -v 'fcgi_script|fastcgi_cache|FcgiDispatch|execute_backend' >/dev/null 2>&1; then
  debt "residual FastCGI symbols in core (pre-existing glue — review in KD4 census)"
else
  pass "no obvious FastCGI transport runtime in core"
fi

# --- Inverse module dependencies forbidden ---
for mod_crate in exyonq-mod-proxy exyonq-mod-static exyonq-mod-fastcgi exyonq-mod-htaccess exyonq-mod-tls exyonq-mod-http3; do
  if [[ ! -d "crates/$mod_crate" ]]; then
    continue
  fi
  if rg -n '^exyonq-core\s*=' "crates/$mod_crate/Cargo.toml" 2>/dev/null \
    | rg -v 'optional\s*=\s*true' >/dev/null 2>&1; then
    fail "$mod_crate has mandatory exyonq-core dependency"
  elif rg -n 'exyonq-core' "crates/$mod_crate/Cargo.toml" >/dev/null 2>&1; then
    debt "$mod_crate optional core-bridge feature (test/dev only — review)"
  else
    pass "$mod_crate has no Cargo.toml dependency on exyonq-core"
  fi
done

# --- Snapshot must stay slot/metadata-only (no upstream/proxy runtime maps) ---
if rg -n 'upstream_targets|proxy_slots\s*:' core/src/snapshot/mod.rs >/dev/null 2>&1; then
  fail "snapshot reintroduced upstream_targets/proxy_slots runtime fields"
else
  pass "snapshot avoids upstream_targets/proxy_slots runtime fields (KD3.6 invariant)"
fi

# --- No new service locators beyond documented registries ---
ALLOWLIST='htaccess_runtime_registry|contract_service_registry|fcgi_cache_registry|register_.*_service|register_htaccess'
new_locators="$(rg -n 'OnceLock|lazy_static|RwLock<Option<.*Fn' core/src --type rust 2>/dev/null \
  | rg -v "$ALLOWLIST" || true)"
if [[ -n "$new_locators" ]]; then
  debt "service-locator patterns outside allowlist (review): $(echo "$new_locators" | wc -l | tr -d ' ') hits"
else
  pass "no unexpected service locator patterns beyond allowlist"
fi

# --- Rule/overlay policy should not sprawl into new core files ---
OVERLAY_CORE='htaccess_runtime_registry|htaccess_probes|router/|routing\.rs|handler\.rs|snapshot/'
if rg -l 'lookup_overlay|apply_overlay|RewriteRule|Redirect 301' core/src --type rust 2>/dev/null \
  | rg -v "$OVERLAY_CORE" >/dev/null 2>&1; then
  debt "overlay/rule evaluation outside known core files — review for dispersion"
else
  pass "overlay/rule evaluation confined to known core files"
fi

# --- module-api must not gain hyper/tokio/epoll in rule contracts ---
if rg -n 'use (hyper|tokio|libc)::' module-api/src/htaccess_overlay.rs module-api/src/routing.rs module-api/src/route_rules.rs module-api/src/cross_cutting_pipeline.rs 2>/dev/null; then
  fail "module-api rule contracts import runtime internals"
else
  pass "module-api htaccess/routing contracts stay runtime-agnostic"
fi

# --- htaccess compiler stays in mod-htaccess, not core ---
if rg -n 'parse_htaccess|compile_vhost_overlay|RewriteCond' core/src --type rust >/dev/null 2>&1; then
  fail "htaccess parsing/compilation must not enter core/src"
else
  pass "htaccess parse/compile not in core/src"
fi

if [[ "$MODE" == "kd4_2" ]] || [[ "$MODE" == "kd4_3" ]] || [[ "$MODE" == "kd4_4" ]] || [[ "$MODE" == "kd4_5" ]] || [[ "$MODE" == "kd4_6" ]] || [[ "$MODE" == "kd4_7" ]] || [[ "$MODE" == "kd4_9" ]] || [[ "$MODE" == "kd4_10" ]] || [[ "$MODE" == "kd4_11" ]] || [[ "$MODE" == "closure" ]]; then
  if [[ -f core/src/overlay_lookup.rs ]] || [[ -f core/src/overlay_transform.rs ]] \
    || [[ -f core/src/htaccess_registry.rs ]] || [[ -f core/src/directory_index.rs ]]; then
    fail "legacy overlay runtime modules still present in core (KD4.2)"
  else
    pass "legacy overlay runtime modules removed from core (KD4.2)"
  fi

  if rg -n 'apply_overlay_for_site|overlay_directory_index_candidates|resolve_directory_index_dispatch|resolve_overlay_fcgi_script_path|note_directory_index_' core/src/server/handler.rs >/dev/null 2>&1; then
    fail "handler.rs still contains overlay policy symbols (KD4.2)"
  else
    pass "handler.rs has no overlay policy symbols (KD4.2)"
  fi

  if rg -n 'htaccess_directory_index_hits_total|htaccess_overlay_redirect_total|OVERLAY_REDIRECT_TOTAL' core/src --type rust \
    | rg -v 'htaccess_runtime_registry|lib\.rs' >/dev/null 2>&1; then
    fail "overlay metrics defined outside htaccess_runtime_registry (KD4.2)"
  else
    pass "overlay metrics only in registry seam (KD4.2)"
  fi

  if rg -n 'register_htaccess_overlay_getter|clear_htaccess_overlay_getter' core/src --type rust >/dev/null 2>&1; then
    fail "legacy htaccess overlay registry still present (KD4.2)"
  else
    pass "legacy htaccess overlay registry removed (KD4.2)"
  fi

  if rg -n 'use (hyper|tokio|libc)::' module-api/src/htaccess_runtime.rs module-api/src/htaccess_overlay.rs 2>/dev/null; then
    fail "module-api htaccess runtime contract imports runtime internals (KD4.2)"
  else
    pass "module-api htaccess runtime contract stays runtime-agnostic (KD4.2)"
  fi

  if rg -n 'lookup_overlay|apply_overlay|RewriteRule|Redirect 301' core/src --type rust 2>/dev/null \
    | rg -v 'htaccess_runtime_registry|htaccess_probes|handler\.rs|server/handler' >/dev/null 2>&1; then
    fail "overlay/rule evaluation outside allowed core seams (KD4.2)"
  else
    pass "overlay evaluation confined to handler/registry/probes seams (KD4.2)"
  fi
fi

if [[ "$MODE" == "kd4_3" ]] || [[ "$MODE" == "kd4_4" ]] || [[ "$MODE" == "kd4_5" ]] || [[ "$MODE" == "kd4_6" ]] || [[ "$MODE" == "kd4_7" ]] || [[ "$MODE" == "kd4_9" ]] || [[ "$MODE" == "kd4_10" ]] || [[ "$MODE" == "kd4_11" ]] || [[ "$MODE" == "closure" ]]; then
  if rg -n 'route\.redirect|route\.rewrite|&route\.redirect|&route\.rewrite' core/src/server/handler.rs >/dev/null 2>&1; then
    fail "handler.rs still reads structural redirect/rewrite policy directly (KD4.3)"
  else
    pass "handler.rs has no direct route.redirect/route.rewrite policy (KD4.3)"
  fi

  if rg -n 'if let Some\(redirect\)|if let Some\(rewrite\)' core/src/server/handler.rs >/dev/null 2>&1; then
    fail "handler.rs still contains structural redirect/rewrite conditionals (KD4.3)"
  else
    pass "handler.rs has no structural redirect/rewrite conditionals (KD4.3)"
  fi

  if rg -n 'evaluate_structural_route_rules|RouteRuleOutcome|structural_route_rules' core/src/server/handler.rs >/dev/null 2>&1; then
    pass "handler uses structural_route_rules seam (KD4.3)"
  else
    fail "handler.rs must invoke structural_route_rules seam (KD4.3)"
  fi

  if rg -n 'evaluate_structural_route_rules|RouteRuleInput' core/src --type rust \
    | rg -v 'structural_route_rules\.rs|lib\.rs' >/dev/null 2>&1; then
    fail "structural rule evaluation duplicated outside structural_route_rules seam (KD4.3)"
  else
    pass "structural rule evaluation confined to seam (KD4.3)"
  fi

  if rg -n 'use (hyper|tokio|libc)::' module-api/src/route_rules.rs 2>/dev/null; then
    fail "module-api route_rules contract imports runtime internals (KD4.3)"
  else
    pass "module-api route_rules contract stays runtime-agnostic (KD4.3)"
  fi

  if rg -n 'evaluate_structural_route_rules|RouteRuleOutcome' crates/exyonq-mod-htaccess >/dev/null 2>&1; then
    fail "structural route rules must not live in exyonq-mod-htaccess (KD4.3)"
  else
    pass "exyonq-mod-htaccess does not own structural route rules (KD4.3)"
  fi

  if rg -n 'FROZEN_DISPATCH_STAGE_ORDER' core/tests module-api/src/route_rules.rs >/dev/null 2>&1; then
    pass "dispatch precedence order is documented in tests/constants (KD4.3)"
  else
    fail "missing FROZEN_DISPATCH_STAGE_ORDER precedence guard (KD4.3)"
  fi
fi

if [[ "$MODE" == "kd4_1" ]] || [[ "$MODE" == "kd4_4" ]] || [[ "$MODE" == "kd4_5" ]] || [[ "$MODE" == "kd4_6" ]] || [[ "$MODE" == "kd4_7" ]] || [[ "$MODE" == "kd4_9" ]] || [[ "$MODE" == "kd4_10" ]] || [[ "$MODE" == "closure" ]]; then
  KD4_1_ALLOW='http_cache|lib\.rs|execute_backend|handler\.rs|reload/|fcgi_cache_registry'
  if rg -n 'ResponseCache|Singleflight|serve_with_cache|insert_get_representation|GLOBAL_RESPONSE_CACHE' core/src --type rust \
    | rg -v "$KD4_1_ALLOW" >/dev/null 2>&1; then
    fail "core/src still contains cache store/singleflight implementation (KD4.1)"
  else
    pass "core/src has zero cache store/singleflight LOC (KD4.1)"
  fi

  prod_manifest="$(sed '/\[dev-dependencies\]/,$d' core/Cargo.toml)"
  if echo "$prod_manifest" | rg -n '^exyonq-mod-fastcgi\s*=' >/dev/null 2>&1; then
    fail "core production deps must not depend on exyonq-mod-fastcgi (use fcgi_cache_registry)"
  else
    pass "core has no production exyonq-mod-fastcgi dependency (KD4.1)"
  fi

  if rg -n 'fn serve_proxy_with_cache' core/src --type rust >/dev/null 2>&1; then
    fail "serve_proxy_with_cache must not be defined in core (KD4.1)"
  else
    pass "no serve_proxy_with_cache definition in core (KD4.1)"
  fi

  if rg -n 'fn serve_fastcgi_with_cache' core/src --type rust \
    | rg -v 'fcgi_cache_registry' >/dev/null 2>&1; then
    fail "serve_fastcgi_with_cache defined outside fcgi_cache_registry (KD4.1)"
  else
    pass "serve_fastcgi_with_cache only in fcgi_cache_registry seam (KD4.1)"
  fi

  if rg -n 'CacheOrigin::(Proxy|Static|Fastcgi)' core/src crates/exyonq-cache/src --type rust >/dev/null 2>&1; then
    fail "backend-specific CacheOrigin enum must not remain (KD4.1)"
  else
    pass "no backend-specific CacheOrigin symbols (KD4.1)"
  fi

  if rg -n 'Proxy|Static|Fastcgi|BackendId' crates/exyonq-cache/src --type rust \
    | rg -vi 'namespace|comment|test|CacheNamespace|module-api' >/dev/null 2>&1; then
    debt "review exyonq-cache for backend-specific symbols (KD4.1)"
  else
    pass "exyonq-cache avoids backend-specific policy symbols (KD4.1)"
  fi

  if [[ -f core/src/cache.rs ]] || [[ -f core/src/cache_key.rs ]] || [[ -f core/src/fastcgi_cache.rs ]]; then
    fail "legacy core cache modules still present (cache.rs/cache_key.rs/fastcgi_cache.rs)"
  else
    pass "legacy core cache modules removed"
  fi

  if rg -n '^exyonq-core\s*=' crates/exyonq-cache/Cargo.toml >/dev/null 2>&1; then
    fail "exyonq-cache must not depend on exyonq-core"
  else
    pass "exyonq-cache has no exyonq-core dependency"
  fi
fi

if [[ "$MODE" == "kd4_4" ]] || [[ "$MODE" == "kd4_5" ]] || [[ "$MODE" == "kd4_6" ]] || [[ "$MODE" == "kd4_7" ]] || [[ "$MODE" == "kd4_9" ]] || [[ "$MODE" == "kd4_10" ]] || [[ "$MODE" == "closure" ]]; then
  prod_manifest="$(sed '/\[dev-dependencies\]/,$d' core/Cargo.toml)"
  for dep in exyonq-compression exyonq-ratelimit exyonq-metrics; do
    if echo "$prod_manifest" | rg -n "^${dep}\s*=" >/dev/null 2>&1; then
      fail "core production deps must not depend on $dep (KD4.4 — use exyonq-module-pipeline)"
    else
      pass "core has no production $dep dependency (KD4.4)"
    fi
  done

  if [[ -d core/src/modules ]] || [[ -f core/src/modules/mod.rs ]]; then
    fail "legacy core/src/modules pipeline still present (KD4.4)"
  else
    pass "legacy core/src/modules removed (KD4.4)"
  fi

  if rg -n 'CompressionModule|RateLimitModule|MetricsModule|ModuleRegistry::new' core/src --type rust >/dev/null 2>&1; then
    fail "module orchestration/registry construction still in core (KD4.4)"
  else
    pass "no module orchestration construction in core (KD4.4)"
  fi

  if rg -n 'on_route_shortcircuit|transform_response|on_module_request|\.on_request\(&req\)' core/src/server/handler.rs >/dev/null 2>&1; then
    fail "handler.rs still orchestrates module pipeline directly (KD4.4)"
  else
    pass "handler uses pipeline_registry seam only (KD4.4)"
  fi

  if rg -n 'accepts_gzip|compressible|allow_key|requests_per_second' core/src --type rust >/dev/null 2>&1; then
    fail "compression/ratelimit policy symbols in core (KD4.4)"
  else
    pass "no compression/ratelimit policy in core (KD4.4)"
  fi

  if rg -n '^exyonq-core\s*=' crates/exyonq-module-pipeline/Cargo.toml >/dev/null 2>&1; then
    fail "exyonq-module-pipeline must not depend on exyonq-core (KD4.4)"
  else
    pass "exyonq-module-pipeline has no exyonq-core dependency (KD4.4)"
  fi

  if rg -n 'use (hyper|tokio|libc)::' module-api/src/cross_cutting_pipeline.rs 2>/dev/null; then
    fail "module-api cross_cutting_pipeline imports runtime internals (KD4.4)"
  else
    pass "module-api cross_cutting_pipeline contract stays runtime-agnostic (KD4.4)"
  fi

  if rg -n 'FROZEN_PIPELINE_AROUND_CORE_ORDER' core/tests module-api/src/cross_cutting_pipeline.rs >/dev/null 2>&1; then
    pass "pipeline precedence order documented in tests/constants (KD4.4)"
  else
    fail "missing FROZEN_PIPELINE_AROUND_CORE_ORDER precedence guard (KD4.4)"
  fi
fi

if [[ "$MODE" == "kd4_5" ]] || [[ "$MODE" == "kd4_6" ]] || [[ "$MODE" == "kd4_7" ]] || [[ "$MODE" == "kd4_9" ]] || [[ "$MODE" == "kd4_10" ]] || [[ "$MODE" == "kd4_11" ]] || [[ "$MODE" == "closure" ]]; then
  prod_manifest="$(sed '/\[dev-dependencies\]/,$d' core/Cargo.toml)"
  for dep in rustls rustls-pemfile tokio-rustls quinn quinn-proto h3 h3-quinn; do
    if echo "$prod_manifest" | rg -n "^${dep}\s*=" >/dev/null 2>&1; then
      fail "core production deps must not depend on $dep (KD4.5 — use exyonq-mod-tls/http3)"
    else
      pass "core has no production $dep dependency (KD4.5)"
    fi
  done

  if [[ -d core/src/tls ]] || [[ -f core/src/tls/mod.rs ]]; then
    fail "legacy core/src/tls runtime still present (KD4.5)"
  else
    pass "legacy core/src/tls runtime removed (KD4.5)"
  fi

  if [[ -d core/src/http3 ]] || [[ -f core/src/http3/mod.rs ]]; then
    fail "legacy core/src/http3 runtime still present (KD4.5)"
  else
    pass "legacy core/src/http3 runtime removed (KD4.5)"
  fi

  KD4_5_TLS_ALLOW='http3_runtime_registry|lib\.rs|reload/|control/|server/mod|certificate_publication_port|acme_http01_adapter'
  if rg -n 'rustls::|tokio_rustls|rustls_pemfile|load_rustls_config|load_certs|load_private_key|CertificateDer|PrivateKeyDer|TlsAcceptor::from|install_rustls_provider' core/src --type rust \
    | rg -v "$KD4_5_TLS_ALLOW" >/dev/null 2>&1; then
    fail "TLS runtime/policy symbols remain in core outside allowlisted seams (KD4.5)"
  else
    pass "no TLS runtime/policy in core outside seams (KD4.5)"
  fi

  KD4_5_H3_ALLOW='http3_runtime_registry|lib\.rs|pipeline_registry|handler\.rs|server/mod'
  if rg -n 'quinn|h3::|h3_quinn|Http3Settings|serve_quic|write_h3_response' core/src --type rust \
    | rg -v "$KD4_5_H3_ALLOW" >/dev/null 2>&1; then
    fail "HTTP/3 runtime symbols remain in core outside allowlisted seams (KD4.5)"
  else
    pass "no HTTP/3 runtime in core outside seams (KD4.5)"
  fi

  if rg -n '^(use |pub use ).*(rustls|quinn|h3::|h3_quinn|tokio_rustls|tokio-rustls)' module-api/src/tls_runtime.rs module-api/src/http3_runtime.rs 2>/dev/null; then
    fail "module-api TLS/H3 contracts leak implementation types (KD4.5)"
  else
    pass "module-api TLS/H3 contracts stay implementation-agnostic (KD4.5)"
  fi

  for mod_crate in exyonq-mod-tls exyonq-mod-http3; do
    if [[ ! -d "crates/$mod_crate" ]]; then
      fail "missing crate crates/$mod_crate (KD4.5)"
      continue
    fi
    if rg -n '^exyonq-core\s*=' "crates/$mod_crate/Cargo.toml" 2>/dev/null \
      | rg -v 'optional\s*=\s*true' >/dev/null 2>&1; then
      fail "$mod_crate has mandatory exyonq-core dependency (KD4.5)"
    else
      pass "$mod_crate has no mandatory exyonq-core dependency (KD4.5)"
    fi
  done

  if rg -n 'serve_http3_request|ConnectionContext' crates/exyonq-mod-http3 >/dev/null 2>&1; then
    fail "exyonq-mod-http3 must not import core handler types (KD4.5)"
  else
    pass "exyonq-mod-http3 has no core handler coupling (KD4.5)"
  fi
fi

if [[ "$MODE" == "kd4_6" ]] || [[ "$MODE" == "kd4_7" ]] || [[ "$MODE" == "kd4_9" ]] || [[ "$MODE" == "kd4_10" ]] || [[ "$MODE" == "closure" ]]; then
  # --- RuntimePlan compiler must not live in core ---
  if rg -n 'pub (type|struct) Runtime(Plan|Snapshot)|compile_(snapshot|runtime_plan_from_ir)|CrossCuttingPipeline::from_config|matchit::Router' core/src --type rust >/dev/null 2>&1; then
    fail "runtime-plan compiler/types must not be defined in core/src (KD4.6)"
  else
    pass "no runtime-plan compiler/type definitions in core/src (KD4.6)"
  fi

  # --- Core must not import config merge/parsing crates directly ---
  if rg -n 'exyonq_config_merge|toml::from_str|AppConfig::parse_str\\(' core/src --type rust >/dev/null 2>&1; then
    fail "core/src must not parse/merge config directly (KD4.6)"
  else
    pass "core/src does not parse/merge config directly (KD4.6)"
  fi

  # --- Snapshot compiler tests must not live in core ---
  if rg -n 'compile_is_deterministic|backend_table_populated|minimal\\.toml' core/src --type rust core/tests 2>/dev/null \
    | rg -n 'core/src' >/dev/null 2>&1; then
    fail "compiler/determinism tests must not live in core/src (KD4.6)"
  else
    pass "no compiler/determinism tests in core/src (KD4.6)"
  fi
fi

if [[ "$MODE" == "kd4_7" ]] || [[ "$MODE" == "kd4_9" ]] || [[ "$MODE" == "kd4_10" ]] || [[ "$MODE" == "kd4_11" ]] || [[ "$MODE" == "closure" ]]; then
  # --- FastCGI script resolution policy must not live in core ---
  if [[ -f core/src/fcgi_script.rs ]]; then
    fail "core/src/fcgi_script.rs must be removed (KD4.7)"
  else
    pass "core/src/fcgi_script.rs removed (KD4.7)"
  fi

  if rg -n 'resolve_script_paths|probe_document_file|RequestedResourceKind|FcgiScriptError' core/src --type rust >/dev/null 2>&1; then
    fail "FastCGI script resolver symbols must not exist in core/src (KD4.7)"
  else
    pass "no FastCGI script resolver symbols in core/src (KD4.7)"
  fi

  if rg -n 'EXYONQ_FCGI_DOCUMENT_ROOT|resolve_fcgi_document_root' core/src --type rust >/dev/null 2>&1; then
    fail "FastCGI document_root fallback policy must not exist in core/src (KD4.7)"
  else
    pass "no FastCGI document_root fallback policy in core/src (KD4.7)"
  fi

  if rg -n 'SCRIPT_FILENAME|SCRIPT_NAME|PATH_INFO' core/src --type rust >/dev/null 2>&1; then
    fail "FastCGI SCRIPT_* / PATH_INFO policy must not exist in core/src (KD4.7)"
  else
    pass "no FastCGI SCRIPT_* / PATH_INFO policy in core/src (KD4.7)"
  fi

  if rg -n 'directory_index_not_found' core/src/server/handler.rs core/src/htaccess_runtime_registry.rs >/dev/null 2>&1; then
    fail "handler/registry must not encode directory-index policy booleans (KD4.7)"
  else
    pass "handler/registry avoid directory-index policy booleans (KD4.7)"
  fi
fi

if [[ "$MODE" == "kd4_9" ]] || [[ "$MODE" == "kd4_10" ]] || [[ "$MODE" == "kd4_11" ]] || [[ "$MODE" == "closure" ]]; then
  # --- Control plane / ops runtime must not live in core ---
  if [[ -f core/src/control/mod.rs ]] || [[ -d core/src/control ]]; then
    fail "core/src/control/ must be removed (KD4.9)"
  else
    pass "core/src/control/ removed (KD4.9)"
  fi

  if [[ -f core/src/ops/mod.rs ]]; then
    fail "core/src/ops/mod.rs must be removed (KD4.9)"
  else
    pass "core/src/ops/mod.rs removed (KD4.9)"
  fi

  if rg -n 'pub mod (control|ops)' core/src/lib.rs >/dev/null 2>&1; then
    fail "core must not expose control/ops modules (KD4.9)"
  else
    pass "core does not expose control/ops modules (KD4.9)"
  fi

  if rg -n 'parse_command_line|UnixListener::bind|ControlResponse|serde_json::to_string' core/src --type rust >/dev/null 2>&1; then
    fail "admin command parsing/formatting must not exist in core/src (KD4.9)"
  else
    pass "no admin command parsing/formatting in core/src (KD4.9)"
  fi

  if rg -n 'serde_json' core/Cargo.toml >/dev/null 2>&1; then
    fail "core must not depend on serde_json for ops formatting (KD4.9)"
  else
    pass "core has no serde_json dependency (KD4.9)"
  fi

  if rg -n 'exyonq-core' crates/exyonq-ops-runtime/Cargo.toml >/dev/null 2>&1; then
    fail "exyonq-ops-runtime must not depend on exyonq-core (KD4.9)"
  else
    pass "exyonq-ops-runtime has no exyonq-core dependency (KD4.9)"
  fi

  if rg -n 'ServerState|OpsState|epoll_worker|RuntimeSnapshot' module-api/src/kernel_control.rs >/dev/null 2>&1; then
    fail "kernel_control contract must not leak core implementation types (KD4.9)"
  else
    pass "kernel_control contract has no implementation leaks (KD4.9)"
  fi

  if ! rg -n 'KernelControlPort|ControlPlaneService|OpsCommand' module-api/src/kernel_control.rs >/dev/null 2>&1; then
    fail "module-api must define kernel control contracts (KD4.9)"
  else
    pass "module-api defines kernel control contracts (KD4.9)"
  fi

  if ! rg -n 'lifecycle::LifecycleState|kernel_control_port::CoreKernelControlPort' core/src/server/mod.rs >/dev/null 2>&1; then
    fail "core server must wire lifecycle + kernel control port (KD4.9)"
  else
    pass "core server wires lifecycle + kernel control port (KD4.9)"
  fi
fi

if [[ "$MODE" == "kd4_10" ]] || [[ "$MODE" == "kd4_11" ]] || [[ "$MODE" == "closure" ]]; then
  # --- Discovery runtime must not live in core ---
  if [[ -f core/src/discovery/mod.rs ]] || [[ -d core/src/discovery ]]; then
    fail "core/src/discovery/ must be removed (KD4.10)"
  else
    pass "core/src/discovery/ removed (KD4.10)"
  fi

  if rg -n 'pub mod discovery[^_]' core/src/lib.rs >/dev/null 2>&1; then
    fail "core must not expose discovery module (KD4.10)"
  else
    pass "core does not expose discovery module (KD4.10)"
  fi

  if rg -n 'apply_file_discovery|DiscoveryOverlay::from_json|discovery file' core/src --type rust >/dev/null 2>&1; then
    fail "discovery file overlay policy must not exist in core/src (KD4.10)"
  else
    pass "no discovery file overlay policy in core/src (KD4.10)"
  fi

  if rg -n 'EXYONQ_DISCOVERY_' core/src --type rust >/dev/null 2>&1; then
    fail "discovery env policy must not exist in core/src (KD4.10)"
  else
    pass "no discovery env policy in core/src (KD4.10)"
  fi

  if rg -n 'exyonq-core' crates/exyonq-discovery-runtime/Cargo.toml >/dev/null 2>&1; then
    fail "exyonq-discovery-runtime must not depend on exyonq-core (KD4.10)"
  else
    pass "exyonq-discovery-runtime has no exyonq-core dependency (KD4.10)"
  fi

  if rg -n 'ServerState|RuntimePlan|ProxyClient|tokio::spawn' module-api/src/discovery_runtime.rs >/dev/null 2>&1; then
    fail "discovery_runtime contract must not leak implementation types (KD4.10)"
  else
    pass "discovery_runtime contract has no implementation leaks (KD4.10)"
  fi

  if ! rg -n 'DiscoveryConfigOverlayService|discovery_runtime_service' module-api/src/discovery_runtime.rs >/dev/null 2>&1; then
    fail "module-api must define discovery runtime contracts (KD4.10)"
  else
    pass "module-api defines discovery runtime contracts (KD4.10)"
  fi

  if ! rg -n 'discovery_overlay::apply_env_discovery_overlay' core/src/server/mod.rs core/src/reload/mod.rs >/dev/null 2>&1; then
    fail "core must use mechanical discovery overlay adapter (KD4.10)"
  else
    pass "core uses mechanical discovery overlay adapter (KD4.10)"
  fi
fi

if [[ "$MODE" == "kd4_11" ]] || [[ "$MODE" == "kd4_12" ]] || [[ "$MODE" == "closure" ]]; then
  # --- ACME integration runtime must not live in core ---
  if [[ -f core/src/acme/mod.rs ]] || [[ -d core/src/acme ]]; then
    fail "core/src/acme/ must be removed (KD4.11)"
  else
    pass "core/src/acme/ removed (KD4.11)"
  fi

  if rg -n 'pub mod acme[^_]|mod acme[^_]' core/src/lib.rs >/dev/null 2>&1; then
    fail "core must not expose acme module (KD4.11)"
  else
    pass "core does not expose acme module (KD4.11)"
  fi

  if rg -n 'exyonq-acme|exyonq_acme::' core/Cargo.toml core/src --type rust >/dev/null 2>&1; then
    fail "core must not depend on or call exyonq-acme directly (KD4.11)"
  else
    pass "core has no exyonq-acme dependency or calls (KD4.11)"
  fi

  ACME_FORBIDDEN='AcmeRuntime|SharedAcmeRuntime|ChallengeStore|ensure_certificates|spawn_renewal_loop|HTTP01_PREFIX|instant_acme|LetsEncrypt|account_key|renewal_loop'
  if rg -n "$ACME_FORBIDDEN" core/src --type rust >/dev/null 2>&1; then
    fail "ACME runtime/policy symbols must not exist in core/src (KD4.11)"
  else
    pass "no ACME runtime/policy symbols in core/src (KD4.11)"
  fi

  if rg -n 'state\.acme|ServerState.*acme' core/src --type rust >/dev/null 2>&1; then
    fail "ServerState must not carry ACME runtime (KD4.11)"
  else
    pass "ServerState has no ACME runtime field (KD4.11)"
  fi

  if rg -n 'exyonq-core' modules/acme/Cargo.toml >/dev/null 2>&1; then
    fail "exyonq-acme must not depend on exyonq-core (KD4.11)"
  else
    pass "exyonq-acme has no exyonq-core dependency (KD4.11)"
  fi

  if rg -n 'ServerState|RuntimePlan|rustls|instant_acme|tokio::spawn' module-api/src/acme_integration.rs >/dev/null 2>&1; then
    fail "acme_integration contract must not leak implementation types (KD4.11)"
  else
    pass "acme_integration contract has no implementation leaks (KD4.11)"
  fi

  if ! rg -n 'AcmeIntegrationService|CertificatePublicationPort|acme_integration_service' module-api/src/acme_integration.rs >/dev/null 2>&1; then
    fail "module-api must define ACME integration contracts (KD4.11)"
  else
    pass "module-api defines ACME integration contracts (KD4.11)"
  fi

  if ! rg -n 'acme_http01_adapter::try_http01_response|acme_integration_service|CoreCertificatePublicationPort' core/src/server/mod.rs core/src/server/handler.rs core/src/certificate_publication_port.rs >/dev/null 2>&1; then
    fail "core must use mechanical ACME/TLS publication adapters (KD4.11)"
  else
    pass "core uses mechanical ACME/TLS publication adapters (KD4.11)"
  fi

  if ! rg -n 'register_acme_integration' cli/exyonq/src/main.rs tests/integration/bootstrap.rs >/dev/null 2>&1; then
    fail "composition root must register ACME integration (KD4.11)"
  else
    pass "composition root registers ACME integration (KD4.11)"
  fi
fi

if [[ "$MODE" == "kd4_12" ]] || [[ "$MODE" == "closure" ]]; then
  if [[ -f core/src/bench_trace.rs ]] || [[ -f core/src/contract_backend_http_metrics.rs ]]; then
    fail "legacy observability modules must be removed from core (KD4.12)"
  else
    pass "legacy bench_trace/contract_backend_http_metrics removed (KD4.12)"
  fi

  if rg -n 'tracing_subscriber|EnvFilter|fmt::layer|set_global_default' core/src --type rust >/dev/null 2>&1; then
    fail "tracing subscriber setup must not live in core (KD4.12)"
  else
    pass "no tracing subscriber setup in core (KD4.12)"
  fi

  OBS_FORBIDDEN='prometheus_lines|append_prometheus|# TYPE |# HELP |openmetrics-text|render_prometheus|MetricsModule'
  if rg -n "$OBS_FORBIDDEN" core/src --type rust >/dev/null 2>&1; then
    fail "metrics export/formatting must not live in core (KD4.12)"
  else
    pass "no metrics export/formatting in core (KD4.12)"
  fi

  if rg -n 'fn (static|proxy|fcgi)_metrics_snapshot|fn fcgi_responses_[0-9]+_total|fn htaccess_.*_total|fn cache_fcgi_(hits|misses|insertions|rejections)_total' core/src --type rust >/dev/null 2>&1; then
    fail "backend/module metric accessors must not live in core (KD4.12)"
  else
    pass "no backend/module metric accessors in core (KD4.12)"
  fi

  if rg -n 'register_linux_epoll_prometheus|register_runtime_prometheus' core/src --type rust >/dev/null 2>&1; then
    fail "observability registration must not live in core startup (KD4.12)"
  else
    pass "no observability registration in core startup (KD4.12)"
  fi

  if ! rg -n 'KernelObservationService|register_kernel_observation_service|register_prometheus_appender' module-api/src/kernel_observation.rs module-api/src/observability_runtime.rs >/dev/null 2>&1; then
    fail "module-api must define observability contracts (KD4.12)"
  else
    pass "module-api defines observability contracts (KD4.12)"
  fi

  if ! rg -n 'register_observability_runtime' cli/exyonq/src/main.rs tests/integration/bootstrap.rs modules/metrics/src/observability_runtime.rs >/dev/null 2>&1; then
    fail "composition root must register observability runtime (KD4.12)"
  else
    pass "composition root registers observability runtime (KD4.12)"
  fi

  if rg -n 'note_(fastcgi|static|proxy)_http_501' core/src/execute_backend.rs core/src/server/handler.rs >/dev/null 2>&1; then
    pass "core uses kernel_observation note_* for shell 501 (KD4.12)"
  else
    fail "core must emit shell 501 via kernel_observation (KD4.12)"
  fi

  prod_manifest="$(sed '/\[dev-dependencies\]/,$d' core/Cargo.toml)"
  if echo "$prod_manifest" | rg -n '^exyonq-metrics\s*=' >/dev/null 2>&1; then
    fail "core production deps must not depend on exyonq-metrics (KD4.12)"
  else
    pass "core has no production exyonq-metrics dependency (KD4.12)"
  fi
fi

if [[ "$MODE" == "kd4_14" ]] || [[ "$MODE" == "closure" ]]; then
  if rg -n 'notify::|RecommendedWatcher|RecursiveMode|notify::Config' core/src --type rust >/dev/null 2>&1; then
    fail "notify watcher runtime must not live in core (KD4.14)"
  else
    pass "no notify watcher runtime in core (KD4.14)"
  fi

  if rg -n 'spawn_config_watcher' core/src --type rust >/dev/null 2>&1; then
    fail "spawn_config_watcher must not live in core (KD4.14)"
  else
    pass "no spawn_config_watcher in core (KD4.14)"
  fi

  if rg -n 'tokio::signal|ctrl_c|shutdown_signal' core/src --type rust >/dev/null 2>&1; then
    fail "OS signal runtime must not live in core (KD4.14)"
  else
    pass "no OS signal runtime in core (KD4.14)"
  fi

  prod_manifest="$(sed '/\[dev-dependencies\]/,$d' core/Cargo.toml)"
  if echo "$prod_manifest" | rg -n '^notify\s*=' >/dev/null 2>&1; then
    fail "core production deps must not include notify (KD4.14)"
  else
    pass "core has no production notify dependency (KD4.14)"
  fi

  prod_reload="$(sed '/\[dev-dependencies\]/,$d' crates/exyonq-reload-runtime/Cargo.toml)"
  if echo "$prod_reload" | rg -n '^exyonq-core\s*=' >/dev/null 2>&1; then
    fail "exyonq-reload-runtime must not depend on exyonq-core (KD4.14)"
  else
    pass "exyonq-reload-runtime has no exyonq-core dependency (KD4.14)"
  fi

  if rg -n 'notify::|tokio::signal|RuntimePlan|ServerState' module-api/src/reload_runtime.rs >/dev/null 2>&1; then
    fail "reload_runtime contract must not leak implementation types (KD4.14)"
  else
    pass "reload_runtime contract has no implementation leaks (KD4.14)"
  fi

  if ! rg -n 'ReloadRuntimeService|register_reload_runtime_service|reload_runtime_service' module-api/src/reload_runtime.rs >/dev/null 2>&1; then
    fail "module-api must define reload runtime contract (KD4.14)"
  else
    pass "module-api defines reload runtime contract (KD4.14)"
  fi

  if ! rg -n 'register_reload_runtime' cli/exyonq/src/main.rs tests/integration/bootstrap.rs crates/exyonq-reload-runtime/src/service.rs >/dev/null 2>&1; then
    fail "composition root must register reload runtime (KD4.14)"
  else
    pass "composition root registers reload runtime (KD4.14)"
  fi

  if ! rg -n 'reload_runtime_service\(\)' core/src/server/mod.rs >/dev/null 2>&1; then
    fail "core must start reload runtime via module-api service (KD4.14)"
  else
    pass "core starts reload runtime via module-api service (KD4.14)"
  fi

  if ! rg -n 'wait_for_shutdown' core/src/server/mod.rs core/src/lifecycle.rs >/dev/null 2>&1; then
    fail "core must wait on lifecycle shutdown, not OS signals (KD4.14)"
  else
    pass "core waits on lifecycle shutdown (KD4.14)"
  fi

  if ! rg -n 'reload_from_path|CoreKernelControlPort|LifecycleState' core/src/reload/mod.rs core/src/kernel_control_port.rs core/src/lifecycle.rs >/dev/null 2>&1; then
    fail "core must retain reload/generation/control port/lifecycle mechanism (KD4.14)"
  else
    pass "core retains reload mechanism (KD4.14)"
  fi

  HOTPATH='crates/exyonq-platform-linux/src/epoll_worker.rs|core/src/server/wire_dispatch.rs|core/src/server/handler.rs|core/src/execute_backend.rs'
  if rg -n 'notify::|tokio::signal|ctrl_c|spawn_config_watcher' \
      crates/exyonq-platform-linux/src/epoll_worker.rs \
      core/src/server/wire_dispatch.rs \
      core/src/server/handler.rs \
      core/src/execute_backend.rs >/dev/null 2>&1; then
    fail "hot-path files must not import watcher/signal runtime (KD4.14)"
  else
    pass "hot-path files free of watcher/signal runtime (KD4.14)"
  fi
fi

if [[ "$MODE" == "baseline" ]] || [[ "$MODE" == "kd4_1" ]]; then
  pass "${MODE} mode: $DEBTS debts recorded, $VIOLATIONS new violations"
  echo "verify-kd4-core-residual-guards: OK (${MODE} — pre-existing debt not blocking)"
  exit 0
elif [[ "$MODE" == "kd4_2" ]]; then
  if [[ "$VIOLATIONS" -gt 0 ]]; then
    echo "verify-kd4-core-residual-guards: FAILED ($VIOLATIONS violations, $DEBTS debts)" >&2
    exit 1
  fi
  pass "kd4_2 mode checks complete"
  echo "verify-kd4-core-residual-guards: OK (kd4_2)"
  exit 0
elif [[ "$MODE" == "kd4_3" ]]; then
  if [[ "$VIOLATIONS" -gt 0 ]]; then
    echo "verify-kd4-core-residual-guards: FAILED ($VIOLATIONS violations, $DEBTS debts)" >&2
    exit 1
  fi
  pass "kd4_3 mode checks complete"
  echo "verify-kd4-core-residual-guards: OK (kd4_3)"
  exit 0
elif [[ "$MODE" == "kd4_4" ]]; then
  if [[ "$VIOLATIONS" -gt 0 ]]; then
    echo "verify-kd4-core-residual-guards: FAILED ($VIOLATIONS violations, $DEBTS debts)" >&2
    exit 1
  fi
  pass "kd4_4 mode checks complete"
  echo "verify-kd4-core-residual-guards: OK (kd4_4)"
  exit 0
elif [[ "$MODE" == "kd4_5" ]]; then
  if [[ "$VIOLATIONS" -gt 0 ]]; then
    echo "verify-kd4-core-residual-guards: FAILED ($VIOLATIONS violations, $DEBTS debts)" >&2
    exit 1
  fi
  pass "kd4_5 mode checks complete"
  echo "verify-kd4-core-residual-guards: OK (kd4_5)"
  exit 0
elif [[ "$MODE" == "kd4_6" ]]; then
  if [[ "$VIOLATIONS" -gt 0 ]]; then
    echo "verify-kd4-core-residual-guards: FAILED ($VIOLATIONS violations, $DEBTS debts)" >&2
    exit 1
  fi
  pass "kd4_6 mode checks complete"
  echo "verify-kd4-core-residual-guards: OK (kd4_6)"
  exit 0
elif [[ "$MODE" == "kd4_7" ]]; then
  if [[ "$VIOLATIONS" -gt 0 ]]; then
    echo "verify-kd4-core-residual-guards: FAILED ($VIOLATIONS violations, $DEBTS debts)" >&2
    exit 1
  fi
  pass "kd4_7 mode checks complete"
  echo "verify-kd4-core-residual-guards: OK (kd4_7)"
  exit 0
elif [[ "$MODE" == "kd4_9" ]]; then
  if [[ "$VIOLATIONS" -gt 0 ]]; then
    echo "verify-kd4-core-residual-guards: FAILED ($VIOLATIONS violations, $DEBTS debts)" >&2
    exit 1
  fi
  pass "kd4_9 mode checks complete"
  echo "verify-kd4-core-residual-guards: OK (kd4_9)"
  exit 0
elif [[ "$MODE" == "kd4_10" ]]; then
  if [[ "$VIOLATIONS" -gt 0 ]]; then
    echo "verify-kd4-core-residual-guards: FAILED ($VIOLATIONS violations, $DEBTS debts)" >&2
    exit 1
  fi
  pass "kd4_10 mode checks complete"
  echo "verify-kd4-core-residual-guards: OK (kd4_10)"
  exit 0
elif [[ "$MODE" == "kd4_11" ]]; then
  if [[ "$VIOLATIONS" -gt 0 ]]; then
    echo "verify-kd4-core-residual-guards: FAILED ($VIOLATIONS violations, $DEBTS debts)" >&2
    exit 1
  fi
  pass "kd4_11 mode checks complete"
  echo "verify-kd4-core-residual-guards: OK (kd4_11)"
  exit 0
elif [[ "$MODE" == "kd4_12" ]]; then
  if [[ "$VIOLATIONS" -gt 0 ]]; then
    echo "verify-kd4-core-residual-guards: FAILED ($VIOLATIONS violations, $DEBTS debts)" >&2
    exit 1
  fi
  pass "kd4_12 mode checks complete"
  echo "verify-kd4-core-residual-guards: OK (kd4_12)"
  exit 0
elif [[ "$MODE" == "kd4_14" ]]; then
  if [[ "$VIOLATIONS" -gt 0 ]]; then
    echo "verify-kd4-core-residual-guards: FAILED ($VIOLATIONS violations, $DEBTS debts)" >&2
    exit 1
  fi
  pass "kd4_14 mode checks complete"
  echo "verify-kd4-core-residual-guards: OK (kd4_14)"
  exit 0
elif [[ "$MODE" == "closure" ]]; then
  if [[ "$DEBTS" -gt 0 ]]; then
    fail "closure mode: $DEBTS baseline debts remain"
  fi
  pass "closure mode checks complete"
else
  fail "unknown mode: $MODE (expected baseline|kd4_1|kd4_2|kd4_3|kd4_4|kd4_5|kd4_6|kd4_7|kd4_9|kd4_10|kd4_11|kd4_12|kd4_14|closure)"
fi

if [[ "$VIOLATIONS" -gt 0 ]]; then
  echo "verify-kd4-core-residual-guards: FAILED ($VIOLATIONS violations, $DEBTS debts)" >&2
  exit 1
fi

echo "verify-kd4-core-residual-guards: OK ($DEBTS debts recorded)"
exit 0

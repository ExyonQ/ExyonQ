# V044 Phase 6 — FastCGI max_connections config honesty close

```text
WIP = V044_PHASE6_FASTCGI_MAX_CONNECTIONS_CONFIG_HONESTY_CLOSE
CASE = P6MAXHONEST-A
PARENT_TERMINAL = P6MAX-C (IMMUTABLE)
ADR = 042
ENTRY_HEAD = d94856663f42ffce16a3c0ccc1291feb4d7083e4
ENTRY_TREE = ac059750141ee48f141c543cccbd641652bf49fb
VALID_RUN = 20260831T134723Z
NETCUP_BINARY_SHA256 = 8da3d613e6597032b363dc9ddce233fbabdd73c6ecd9c48be79be4825f57af55
WORDPRESS_AUTHORIZED = NO
FASTCGI_CONCURRENCY_IMPLEMENTATION = NO
```

## Purpose

Make the CFD `max_conn` / `max_connections` contract truthful after P6MAX-C proved values >1 are inoperative under serial `execute_get`.

## Decision (ADR-042)

```text
CHOSEN_CONFIG_STRATEGY = REJECT_GT1
FAIL_CLOSED_GT1 = SELECTED
NEW_CONFIG_NAME = NONE
LEGACY_MAX_CONNECTIONS_POLICY = CFD_FIELD_RETAINED_VALUE_MUST_BE_1
VALUE_0_POLICY = REJECT
VALUE_1_POLICY = ACCEPT
VALUE_GT1_POLICY = REJECT
```

CFD project parse rejects `max_conn != 1` with an explicit ADR-042 message (serial per shard; not concurrent backend capacity).

`exyonq-mod-fastcgi` TOML `FcgiPoolConfig.max_connections` is **not** forced to 1 in this WIP (distinct module open-socket path; out of P6MAX-C evidence). IR docs no longer claim a universal busy+idle cap for all FastCGI surfaces.

## Current effective semantics (preserved)

```text
CURRENT_EFFECTIVE_SEMANTICS =
  PER_SHARD × PER_POOL_ID idle retention cap at PoolIdle.put / ensure_pool shrink
  under serial execute_get; published CFD config may only supply max_conn=1
IS_MAX_CONNECTIONS_NAME_TRUTHFUL_FOR_CURRENT_EFFECT = PARTIAL
  # name kept; fail-closed value=1 removes concurrent-budget illusion
RESOURCE_BOUND_PUBLIC_CLAIM_ALLOWED = NO
IMPORTER_CONTRACT_DRIFT = NO
```

## Netcup evidence (VALID)

| Check | Result |
| --- | --- |
| reject max_conn=6 | PASS |
| reject max_conn=0 | PASS |
| accept max_conn=1 + dynamic PHP | PASS |
| failed publish max=6 leaves gen active | PASS |
| gen reload max=1 | PASS |
| KEEP_CONN 200 / SYN=1 | PASS |

## Preserved parent claims

```text
TTL_RELOAD = STRONG_CAUSAL_PASS (preserved; not re-campaigned)
GENERATION_RELOAD = STRONG_CAUSAL_PASS (bounded gen reload max=1 PASS)
MAX_CONNECTIONS_GT1_SEMANTICS = INOPERATIVE_CURRENTLY_PRESERVED
P6MAX-C = IMMUTABLE (not rewritten as STRONG_CAUSAL_PASS)
```

## Audits

| Agent | Verdict |
| --- | --- |
| Orchestrator | CONDITIONAL_ACCEPT → executed REJECT_GT1 |
| Vertex | ARCHITECTURE_AUDIT=PASS; TRUTHFUL_ENFORCED_AT_PARSE_TIME |
| Security | SECURITY_ACCEPT; honesty PASS; RESOURCE_BOUND_PUBLIC_CLAIM_ALLOWED=NO |
| Research | PASS_WITH_REMEDIATION (rename advisory deferred) |
| GRC | PASS |
| Logic | PASS; OPEN_P0=0 OPEN_P1=0 MUST_FIX=0 |

## WordPress

```text
FASTCGI_CONFIG_HONESTY_BLOCKERS = 0
WORDPRESS_CURRENT_HEAD_QUALIFICATION_FEASIBLE = YES
WORDPRESS_AUTHORIZED = NO
FUTURE_FASTCGI_CONCURRENCY_WIP_RECOMMENDED = YES
FUTURE_FASTCGI_CONCURRENCY_AUTHORIZED = NO
```

## Evidence / docs

```text
EVIDENCE = .exyonq-local/evidence/phase6-fastcgi-max-connections-config-honesty-close/20260831T134723Z/
ADR = docs/adr/042-cfd-fastcgi-max-connections-config-honesty.md
HARNESS = scripts/reality/run-fcgi-max-connections-honesty-netcup.sh
TERMINAL = docs/governance/v044-phase6-fastcgi-max-connections-config-honesty-close-terminal-block.txt
```

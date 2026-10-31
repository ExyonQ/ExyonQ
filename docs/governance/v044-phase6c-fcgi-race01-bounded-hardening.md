# V044 Phase 6C — FCGI-RACE-01 bounded hardening

```text
WIP = V044_PHASE6C_FCGI_RACE01_BOUNDED_HARDENING
CASE = P6C-B
PARENT = P6BSEAL-B
ADR = docs/adr/039-phase6c-fcgi-race01-bounded-hardening.md
ENTRY_HEAD = d94856663f42ffce16a3c0ccc1291feb4d7083e4
ENTRY_TREE = ac059750141ee48f141c543cccbd641652bf49fb
```

## Purpose

Close or falsify **only** P6B-P3-03 / FCGI-RACE-01: TOCTOU between FastCGI pool checkout validation and `FCGI_BEGIN_REQUEST`.

## Process correction vs Phase 6B

```text
PHASE6B_PRE_MUTATION_ARCHITECTURE_REVIEW = FAIL   # historical; unchanged
P6C_PRE_MUTATION_ARCHITECTURE_REVIEW = PASS
VERTEX_PRE_MUTATION = PASS
SECURITY_PRE_MUTATION = PASS
GRC_PRE_MUTATION = PASS
ADR_039_CREATED_BEFORE_PRODUCT_MUTATION = YES
```

## Root cause

`FcgiPoolMap::take` probed Clean, then `execute_get` wrote BEGIN without final activation validation. Peer EOF / unexpected bytes / error in that window could reach BEGIN on a dirty socket.

## Chosen design

```text
CHOSEN_DESIGN = B_PRE_BEGIN_ALWAYS_PROBE
NO_NEW_PATTERN_ALTERNATIVE = SUFFICIENT
ORDER = take|connect → activate_before_begin(=probe) → apply_timeouts → exchange_once
DIRTY = DISCARD + fresh connect ≤1 while NotStarted
```

Rejected for P6C: A (status quo), C/G hold-READABLE, D/E newtype/RAII, F always-discard.

## Residual (nonblocking qualification)

```text
FCGI_RACE_01_PROBE_DETECTABLE_WINDOW = CLOSED
FCGI_RACE_01_RESIDUAL_POST_PROBE_TO_WRITE = ACKNOWLEDGED_NOT_CLOSED
CAN_BACKEND_LEGITIMATELY_SEND_FASTCGI_RESPONSE_BYTES_BEFORE_RECEIVING_A_NEW_BEGIN = NO
```

Contamination after residual → protocol/CGI fail → DISCARD → no put.

## Evidence

| Layer | Result |
| --- | --- |
| Adversarial component (EOF / pending byte / partial header / dirty put) | PASS |
| Netcup amd64 PHP-FPM 1000 req, SYN oracle | PASS — SYN=1, contamination=0 |
| BINARY_SHA256 | `5e5ff3c86abe28c4ac8befe30ac75ebe94d56b4b629efc0bf3d639877cf1936d` |
| Evidence dir | `.exyonq-local/evidence/phase6c-fcgi-race01-bounded-hardening/20260830T161953Z/` |

```text
DIRTY_CONNECTION_REUSE_COUNT (component) = 0
DIRTY_CONNECTION_REUSE_COUNT (PHP-FPM harness) = NOT_MEASURED_IN_PHP_FPM_SEAL
```

## P3 reconciliation

| ID | Status |
| --- | --- |
| P6B-P3-01 KEEP_CONN causal | CLOSED (parent seal) |
| P6B-P3-02 SG-FCGI-06 peercred | OPEN_NONBLOCKING / QUALIFIED — unchanged |
| P6B-P3-03 FCGI-RACE-01 | CLOSED_WITH_NONBLOCKING_QUALIFICATIONS (this WIP) |
| P6B-P3-04 script_suffix / reuse_ok | OPEN_NONBLOCKING — `P6B_P3_04_CAUSAL_TO_RACE01=NO` |

## Audits

```text
ARCHITECTURE_AUDIT = PASS
SECURITY_AUDIT = PASS_WITH_NONBLOCKING
GRC_AUDIT = PASS
INDEPENDENT_LOGIC_AUDIT = PASS_WITH_FINDINGS (P3 residual + evidence-class hygiene only)
ANTI_KOBAYASHI = PASS
ZERO_FAKE = PASS
NO_SMOKE = PASS
```

## Migration value

```text
DOES_P6C_INCREASE_MIGRATION_VALUE = YES
REASON = Final pooled→active boundary fail-closed under EOF/unexpected readable;
         KEEP_CONN causal reuse preserved; contamination risk reduced for PHP-FPM ops.
```

## Next tranche (recommendation only)

```text
PHASE_6D_AUTHORIZED = NO
PHASE_6D_FEASIBLE = CONDITIONAL
RECOMMENDED_NEXT_PHASE6_TRANCHE = D_FastCGI_resource_stress_OR_F_bounded_prerequisite
  # Prefer stress/resource qualification or residual-window adversarial fixture before WordPress/Module.
```

## Publication defaults

```text
CANONICAL_MERGE = NO
PUSH = NO
TAG = NO
RELEASE = NO
PUBLICATION = NO
NOTION_RECONCILIATION_REQUIRED = YES
```

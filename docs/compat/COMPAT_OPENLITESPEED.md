# COMPAT — OpenLiteSpeed → ExyonQ (0.9.x)

<!-- compat-matrix
source=openlitespeed
exact=0
approximated=0
unsupported=0
fields=9
p1_open=0
p2_open=0
-->

ExyonQ does **not** ship a full OpenLiteSpeed config importer. The only imported
OLS surface is the `perClientConnLimit` throttle block, translated by
`exyonq-compat ols-throttle` into `[modules.ratelimit.client]`. Measured over
`tests/fixtures/compat-matrix/ols-throttle.conf`: one block, 9 emitted fields
(1 `enabled` + 8 translated), 0 approximated, 0 unsupported, exit 0.

**How to read `exact=0` with nine ✅ rows below.** The translator emits one
fragment per block and its report has no per-directive taxonomy, so `exact` —
the count of report entries the importer labels exact — is 0 for this source.
`fields=9` is the number of keys in the emitted fragment. The ✅ column below
means "translated field-for-field into a same-named IR key", not "one of the
nine counted as an `exact` report entry"; the two numbers measure different
things, and publishing `exact=0` next to nine ✅ rows without this sentence is
exactly the confusion this note removes.

`scripts/gates/compat-matrices-gate.sh` re-runs the translation on every run,
requires the `[modules.ratelimit.client]` table to exist and fails if the block
above drifts or drops a metric.

| OLS field | ExyonQ field | Translation |
|---|---|---|
| `softLimit` | `soft_connections_per_ip` | ✅ field-for-field |
| `hardLimit` | `max_connections_per_ip` | ✅ field-for-field |
| `staticReqPerSec` | `static_requests_per_second` | ✅ field-for-field |
| `dynReqPerSec` | `dynamic_requests_per_second` | ✅ field-for-field |
| `outBandwidth` | `bandwidth_bytes_per_second` | ✅ field-for-field |
| `inBandwidth` | `input_bandwidth_bytes_per_second` | ✅ field-for-field |
| `banPeriod` | `ban_seconds` | ✅ field-for-field |
| `gracePeriod` | `grace_seconds` | ✅ field-for-field |
| `enabled` (implied) | `enabled = true` | ✅ field-for-field |

Unknown or multiple `perClientConnLimit` blocks fail closed.

## Open gaps

`P1_OPEN=0 P2_OPEN=0` — this matrix has no approximated and no refused field.

## Out of scope (not a claim of parity)

Everything else OpenLiteSpeed exposes — `vhconf`, rewrite rules, LSCache,
`extprocessor`, listeners, virtual host templates, `mod_security` wiring — has no
OLS importer here. nginx-style configs migrated through
`exyonqctl config migrate-nginx` are the supported path; OLS deployments are
expected to move to an ExyonQ-native config, not to be imported one-to-one.

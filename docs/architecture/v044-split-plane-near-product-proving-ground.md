# V044 Split-plane near-product proving ground

```text
WIP = V044_SPLIT_PLANE_NEAR_PRODUCT_PROVING_GROUND
SHORT = SPPG
MODE = ISOLATED_NEAR_PRODUCT_E2E_ARCHITECTURE_PROVING_GROUND
PARENT = IL-F
SOURCE_AUTHORITY_HEAD = d94856663f42ffce16a3c0ccc1291feb4d7083e4
PRODUCT_MUTATION = NO
CONTROL_IS_PRODUCT = NO
TERMINAL_CASE = SPPG-C
TERMINAL_RUN_ID = 20260827-234226
```

Governance: [`../governance/v044-split-plane-near-product-proving-ground.md`](../governance/v044-split-plane-near-product-proving-ground.md)  
Performance: [`../performance/v044-split-plane-near-product-proving-ground.md`](../performance/v044-split-plane-near-product-proving-ground.md)  
Experiment: `.exyonq-local/experiments/split-plane-near-product/`  
Evidence: `.exyonq-local/evidence/split-plane-near-product/20260827-234226/`

---

## Hypothesis

Can a **true split-plane** near-product dataplane (immutable control publication; mio TPC dataplane with **zero** per-request Tokio) retain enough competitive headroom vs best rival to justify continuing the extreme +25/−10 objective?

IL-F showed embedding EP-C geometry inside Cap067/Tokio erased the advantage. This proving ground forbids Cap067 process residency in the dataplane.

---

## ADR process model

**SELECTED = B_SEPARATE_DATAPLANE_PROCESS** (see experiment `ADR-process-model.md`).

| Option | Verdict |
| --- | --- |
| A same-process isolated domain | Reject for proving ground (IL-F contamination risk) |
| **B separate DP process + atomic gen publish** | **Selected** |
| C SHM snapshot | Deferred |

```text
TOKIO_IN_DATAPLANE_REQUEST_PATH = NO
CAP067_PROCESS_RESIDENCY_IN_SPPG_DATAPLANE = NO
PER_REQUEST_CONTROL_PLANE_INVOLVEMENT = 0
SELECTED_PROCESS_MODEL = SEPARATE_PROCESS
DATAPLANE_RUNTIME = mio thread-per-core (2 shards, SO_REUSEPORT)
CONTROL_PLANE_RUNTIME = blocking thread poll (gen file watch; no Tokio on request path)
```

---

## Near-product semantics implemented

Real TCP/H1 parse; client header forward + hop-by-hop strip; multi-host/path/upstream routing; immutable generation via `ArcSwap`; compiled WAF (host/path/header/IP) with real peer IP; shard-local upstream KA pools with poison guard; phase deadlines for timeouts; shard-local counters + periodic stderr drain.

**LOC:** 1318 Rust (under 2500 budget). **Unsafe:** 0.

Evidence gaps on Netcup terminal run: correctness matrix capture empty; generation reload probe `/g2/x` returned 404; security adversarial matrix not executed; 8K/64K diversity not measured.

---

## Terminal result (Netcup amd64, P4_PROXY_1K)

7 interleaved repetitions vs ExyonQ / NGINX / OLS / HAProxy. Median primary metrics:

| Product | RPS | CPU µs/req | RSS KiB | P99 µs |
| --- | --- | --- | --- | --- |
| **SPPG** | **94,145** | **17.68** | **2,984** | **3,710** |
| OLS (best RPS) | 86,133 | 22.94 | 3,608 | 4,320 |
| NGINX | 85,166 | 26.40 | 6,064 | 4,540 |
| ExyonQ | 59,370 | 41.76 | 37,288 | 4,610 |
| HAProxy | 35,470 | 36.73 | 18,672 | 7,270 |

**SPPG vs best rival (OLS):** RPS **+9.3%** (gate ≥+15% **FAIL**); CPU **+22.9% better** (**PASS**); memory **+17.3% better** (**PASS**); P95 **+16.5% better** (**PASS**); P99 **+14.1% better** (**PASS**).

30-minute endurance: FD/thread stable; RSS growth 4 KiB.

---

## Kill switch disposition

```text
CASE = SPPG-C (INSUFFICIENT_COMPETITIVE_HEADROOM)
MINIMUM_CONTINUATION_PASS = NO
EXTREME_TARGET_P4_PASS = NO
EXTREME_OBJECTIVE_DISPOSITION = STOP_PURSUING
```

The split-plane near-product proving ground did **not** retain enough competitive headroom. Based on current evidence, continuing to pursue the extreme +25% RPS / −10% CPU / −10% memory / −10% P95 / −10% P99 objective is **not** economically or technically justified.

**Interpretation:** True split-plane + mio TPC recovered most of EP-C-class efficiency vs current ExyonQ (+58.6% RPS, −57.7% CPU/req) and beats all rivals on CPU/memory/tails, but **does not clear the +15% RPS minimum** vs best rival (OLS). Productization headroom is insufficient for the extreme campaign.

---

## Lineage update

GA-B → EP-C → SD-B → SDP-P1/SP1-C → IL-F → **SPPG (terminal SPPG-C)** → **STOP extreme objective**

No automatic next architecture prototype. Owner may later redefine goals; this +25/−10 campaign ends here.

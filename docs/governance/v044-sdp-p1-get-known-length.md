# V044 SDP-P1 governance terminal

```text
STATUS = COMPLETE
CASE = SP1-C
INTEGRATION_VALUE_GATE = FAIL
STOP_ARCHITECTURE_INTEGRATION = YES
PRODUCT_IMPLEMENTATION_CONTINUE_RECOMMENDATION = NO
NEXT_PRODUCT_WIP_PROPOSAL = NONE
CANONICAL_MERGE = NO
PUSH = NO
```

Architecture: [`../architecture/v044-sdp-p1-get-known-length.md`](../architecture/v044-sdp-p1-get-known-length.md)  
Performance: [`../performance/v044-sdp-p1-get-known-length.md`](../performance/v044-sdp-p1-get-known-length.md)  
Evidence: `.exyonq-local/evidence/sdp-p1/20260827-210315/`

```text
PXDP = HOLD
REQUEST_BODY_RELAY = HOLD
PXDP_P6 = HOLD
```

Default feature gate remains OFF (`EXYONQ_SDP_P1` unset). Code may remain in-tree behind the gate for forensic/owner review; no further SDP feature work without new authorization after SP1-C.

Post-mortem attribution: [`v044-sdp-p1-integration-loss-attribution.md`](v044-sdp-p1-integration-loss-attribution.md) — **CASE=IL-F** → `SPECIALIZED_PRODUCT_INTEGRATION_DIRECTION=CLOSE`.

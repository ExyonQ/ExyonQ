# ExyonQ HTTP Control API

```text
CAPABILITY_ID = 016
FEATURE_ID = control-http-api
CONTROL_API_OWNER = EXYONQ
NEXUS_ROLE = CLIENT_AND_ADAPTER
```

Canonical contract:

- OpenAPI 3.1.2: [`openapi-v1.yaml`](openapi-v1.yaml)
- ADR: [`../adr/036-control-http-api.md`](../adr/036-control-http-api.md)

Runtime leaf: `crates/exyonq-control-api` (Hyper adapter + handwritten `ControlService`).

Transports:

- UDS `/run/exyonq/api.sock` (`EXYONQ_API_SOCKET`)
- loopback TCP `127.0.0.1:9090` (`EXYONQ_API_TCP`)
- auth token env `EXYONQ_SERVER_TOKEN` (`exq_sk_v1_…`)

Distinct from operational `control.sock` line protocol.

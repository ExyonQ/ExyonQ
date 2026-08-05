# r3-p4-mock

R3 P4 **benchmark infrastructure** only — deterministic HTTP mock upstream.

Not product code. Not part of the ExyonQ runtime.

## Contract

- `GET /health` → `200` `ok`
- `GET /api/` and `GET /api/*` (except echo/stream) → `200`, fixed **1024-byte** body
- Keep-alive enabled
- No cache headers that imply response memoization ambiguity
- Prefers `BENCH_WWW/1k.bin` when present; otherwise synthesizes 1024 `x` bytes

## Build / run

```bash
cargo build --release --manifest-path tools/r3-p4-mock/Cargo.toml
MOCK_PORT=9000 ./tools/r3-p4-mock/target/release/r3-p4-mock
```

Env: `MOCK_HOST` (default `0.0.0.0`), `MOCK_PORT` (default `9000`), `BENCH_WWW` (optional payload root).

Standalone package (`[workspace]` empty) — does **not** modify the product workspace `Cargo.toml` / `Cargo.lock`.

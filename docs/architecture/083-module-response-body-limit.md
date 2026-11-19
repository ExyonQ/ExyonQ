# 083 — Module response body limit (`[modules] response_body_limit_bytes`)

Status: implemented (0.9.3). Closes issue #86.

## Problem

Modules that transform a response (`compression`, `sub_filter`, `gunzip`,
`metrics`) need the whole body before `on_response` runs. Until 0.9.3 that
collection was capped by a hard-coded `MODULE_BODY_LIMIT` of 4 MiB in
`crates/exyonq-module-pipeline`, and any larger response failed with `500` whose
body was the internal error string. Large but legitimate downloads (video, PDF,
exports) were unservable with modules enabled and there was no supported way to
raise the ceiling.

## Contract

```toml
[modules]
# Ceiling for the response body the module chain collects before on_response.
# 1 KiB ..= 256 MiB. Omitted => 4 MiB.
response_body_limit_bytes = 16777216
```

- **Default:** 4 MiB (`4 * 1024 * 1024`), the historical value; existing configs
  keep their fingerprint because the default is omitted from the canonical IR.
- **Range:** `1024 ..= 268435456`; anything else is a config error
  (`exyonqctl config lint` fails), never a silent clamp.
- **Failure mode:** a body above the ceiling is **not** truncated. The module
  chain aborts, the client receives a generic `500` (`internal server error`)
  and the cause stays in the server log.
- **Observability:** the log entry carries `route`, `body_bytes` and
  `limit_bytes` (`event="module_body_limit"`), and `exyonq_module_body_limit_total`
  counts the rejected responses (exported when `[modules.metrics]` is enabled).
- **SSE:** `text/event-stream` responses never enter body collection (Cap032),
  so the ceiling does not apply to them.

## Cost

The ceiling is a memory bound, not a streaming switch: a response up to the
ceiling is buffered in RAM once per request that reaches the module chain.
Raising it to serve 100 MiB downloads costs 100 MiB of transient memory per
concurrent request; streaming those endpoints is the next step (follow-up in
issue #86), not something this knob provides.

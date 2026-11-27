# 083 — Module response body limit (`[modules] response_body_limit_bytes`)

Status: implemented (0.9.3). Closes issue #86.

## Problem

Modules that transform a response (`compression`, `sub_filter`, `gunzip`,
`metrics`) need the whole body before `on_response` runs. Until 0.9.3 that
collection was capped by a hard-coded `MODULE_BODY_LIMIT` of 4 MiB in
`crates/exyonq-module-pipeline`, and a larger response failed with `500` whose
body was the internal error string. Large but legitimate downloads (video, PDF,
exports) were unservable with modules enabled and there was no supported way to
raise the ceiling.

## Contract

```toml
[modules]
# Ceiling for the response body the module chain collects before on_response.
# 1 KiB ..= 1 GiB. Omitted => 4 MiB.
response_body_limit_bytes = 16777216
```

- **Default:** 4 MiB (`4 * 1024 * 1024`), the historical value; existing configs
  keep their fingerprint because the default is omitted from the canonical IR.
- **Range:** `1024 ..= 1073741824`; anything else is a config error
  (`exyonqctl config lint` fails), never a silent clamp.
- **Known-size bodies:** when the declared `Content-Length` (or exact body size
  hint) exceeds the ceiling, the body is **not** collected or truncated. Modules
  observe metadata only and the original bytes stream to the client.
- **Unknown-size bodies:** a body that grows past the ceiling without a declared
  size is aborted with `413 Payload Too Large`; the response names the configured
  limit and the `[modules] response_body_limit_bytes` knob, instead of the
  historical generic `500 module error`.
- **Observability:** the rejection logs `limit`, `observed`, `route` and
  `upstream` under `target = "exyonq::modules"`.
- **SSE:** `text/event-stream` responses never enter body collection (Cap032),
  so the ceiling does not apply to them.

## Cost

The ceiling is a memory bound, not a streaming switch: a response up to the
ceiling is buffered in RAM once per request that reaches the module chain.
Raising it to serve 100 MiB downloads costs 100 MiB of transient memory per
concurrent request; streaming those endpoints is the next step (follow-up in
issue #86), not something this knob provides.

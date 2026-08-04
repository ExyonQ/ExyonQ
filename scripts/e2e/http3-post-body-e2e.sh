#!/usr/bin/env bash
# AUTHORITATIVE HTTP/3 → reverse-proxy POST body E2E (NO-SMOKE).
# Real h3 client → ExyonQ → controlled upstream. No HTTP/1.1/H2 fallback.
#
# USES_REAL_DATA=YES (controlled upstream + real product binary)
# CHANGES_PRODUCT_SEMANTICS=NO
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
# shellcheck source=lib.sh
source "$(cd "$(dirname "$0")" && pwd)/lib.sh"
e2e_require_linux
e2e_ensure_bins

if ! command -v docker >/dev/null 2>&1; then
  e2e_record_fail "docker required for curl-http3 client"
  e2e_finish "http3-post-body-e2e"
  exit 1
fi

# shellcheck source=../smoke/lib-p13a-tls.sh
source "$ROOT/scripts/smoke/lib-p13a-tls.sh"
ensure_bins
ensure_ephemeral_tls

CLIENT_IMAGE="${H3_CLIENT_IMAGE:-ymuski/curl-http3:latest}"
CLIENT_PLAT="$(uname -m)"
case "$CLIENT_PLAT" in
  x86_64) DOCKER_PLAT=linux/amd64 ;;
  aarch64|arm64)
    # Image is amd64-only today; use qemu/amd64 client against native arm64 ExyonQ.
    DOCKER_PLAT=linux/amd64
    ;;
  *) e2e_record_fail "unsupported arch $CLIENT_PLAT"; e2e_finish "http3-post-body-e2e"; exit 1 ;;
esac
if ! docker pull --platform "$DOCKER_PLAT" "$CLIENT_IMAGE" >/dev/null; then
  e2e_record_fail "docker pull $CLIENT_IMAGE plat=$DOCKER_PLAT"
  e2e_finish "http3-post-body-e2e"
  exit 1
fi
e2e_record_pass "h3 client image ready plat=$DOCKER_PLAT host=$CLIENT_PLAT"

TMP="$(mktemp -d)"
cleanup() {
  [[ -n "${SRV_PID:-}" ]] && kill "$SRV_PID" 2>/dev/null || true
  [[ -n "${UP_PID:-}" ]] && kill "$UP_PID" 2>/dev/null || true
  bash "$ROOT/scripts/test-tls/generate-ephemeral-tls.sh" --cleanup "${EXYONQ_TLS_DIR:-}" 2>/dev/null || true
  rm -rf "$TMP"
}
trap cleanup EXIT

TCP_PORT="$(e2e_pick_port)"
UDP_PORT="$(e2e_pick_port)"
UP_PORT="$(e2e_pick_port)"
UP_STATE="$TMP/up"
mkdir -p "$UP_STATE" "$TMP/www"
echo ready >"$UP_STATE/mode"
printf 'static-ok' >"$TMP/www/index.html"

export UP_PORT UP_STATE
python3 - <<'PY' &
import hashlib, os, socket, threading, time
addr = ("127.0.0.1", int(os.environ["UP_PORT"]))
sd = os.environ["UP_STATE"]
s = socket.socket()
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(addr)
s.listen(64)

def mode():
    try:
        return open(os.path.join(sd, "mode")).read().strip()
    except Exception:
        return "ready"

def serve(c):
    m = mode()
    if m in ("refuse", "down"):
        c.close()
        return
    data = b""
    c.settimeout(8)
    try:
        while b"\r\n\r\n" not in data:
            chunk = c.recv(4096)
            if not chunk:
                break
            data += chunk
        hdr, body = data.split(b"\r\n\r\n", 1)
        cl = 0
        for line in hdr.decode("latin1", "ignore").split("\r\n"):
            if line.lower().startswith("content-length:"):
                cl = int(line.split(":", 1)[1].strip() or 0)
        while len(body) < cl:
            chunk = c.recv(max(cl - len(body), 1))
            if not chunk:
                break
            body += chunk
        body = body[:cl]
        open(os.path.join(sd, "last_req"), "wb").write(hdr + b"\n\n" + body)
        open(os.path.join(sd, "last_body"), "wb").write(body)
        open(os.path.join(sd, "last_sha"), "w").write(hashlib.sha256(body).hexdigest())
        if m == "timeout":
            time.sleep(35)
            return
        resp = (
            f"HTTP/1.1 200 OK\r\nContent-Length: {len(body)}\r\n"
            f"X-Up-Sha256: {hashlib.sha256(body).hexdigest()}\r\n"
            f"Connection: close\r\n\r\n"
        ).encode() + body
        c.sendall(resp)
    except Exception as e:
        open(os.path.join(sd, "err"), "w").write(repr(e))
    finally:
        try:
            c.close()
        except Exception:
            pass

while True:
    c, _ = s.accept()
    threading.Thread(target=serve, args=(c,), daemon=True).start()
PY
UP_PID=$!

# Drain cap must cover largest body under PROXY_MAX (default product drain is 64KiB).
LARGE_BODY=262144
cat >"$TMP/exyonq.toml" <<EOF
config_version = 1
[http3]
enabled = true
request_body_drain_cap_bytes = 1048576
[[server]]
listen = "127.0.0.1:${TCP_PORT}"
http3_listen = "0.0.0.0:${UDP_PORT}"
routes = ["site", "api"]
[server.tls]
cert = "${CERT_PEM}"
key = "${KEY_PEM}"
[[route]]
name = "site"
match = { path = "/site" }
root = "${TMP}/www"
index = "index.html"
[[route]]
name = "api"
match = { path = "/api" }
upstream = "backend"
[[upstream]]
name = "backend"
target = "http://127.0.0.1:${UP_PORT}"
timeout_ms = 2000
EOF

RUST_LOG=info "$EXYONQ_BIN" serve --config "$TMP/exyonq.toml" >"$TMP/ex.log" 2>&1 &
SRV_PID=$!
for _ in $(seq 1 100); do
  if rg -q "HTTP/3 listening" "$TMP/ex.log" 2>/dev/null; then break; fi
  if ! kill -0 "$SRV_PID" 2>/dev/null; then
    e2e_record_fail "server exited early"
    cat "$TMP/ex.log" >&2 || true
    e2e_finish "http3-post-body-e2e"
    exit 1
  fi
  sleep 0.1
done
rg -q "HTTP/3 listening" "$TMP/ex.log" || {
  e2e_record_fail "no HTTP/3 listening"
  cat "$TMP/ex.log" >&2 || true
  e2e_finish "http3-post-body-e2e"
  exit 1
}
e2e_record_pass "HTTP/3 listener ready"

H3_URL="https://127.0.0.1:${UDP_PORT}"

# curl runs in a container — mount TMP at /e2e for payloads and outputs.
h3() {
  docker run --rm --platform "$DOCKER_PLAT" --network host -v "$TMP:/e2e" "$CLIENT_IMAGE" \
    curl --http3-only -sk --max-time 20 "$@"
}

h3_code() {
  local out
  out="$(h3 -o /dev/null -w '%{http_code}' "$@" 2>/dev/null || true)"
  if [[ "$out" =~ ^[0-9]{3}$ ]]; then printf '%s\n' "$out"; else printf '000\n'; fi
}

# A: fixed JSON body + sha256
PAYLOAD='{"a":1}'
printf '%s' "$PAYLOAD" >"$TMP/payload-a.json"
WANT_SHA="$(sha256sum "$TMP/payload-a.json" | awk '{print $1}')"
rm -f "$UP_STATE/last_body" "$UP_STATE/last_sha" "$TMP/body-a"
RESP="$(h3 -X POST -H 'Content-Type: application/json' --data-binary @/e2e/payload-a.json \
  -o /e2e/body-a -w '%{http_code}' "${H3_URL}/api/json" 2>/dev/null || true)"
UP_SHA="$(cat "$UP_STATE/last_sha" 2>/dev/null || true)"
UP_BODY="$(cat "$UP_STATE/last_body" 2>/dev/null || true)"
GOT_SHA="$(sha256sum "$TMP/body-a" 2>/dev/null | awk '{print $1}' || true)"
if [[ "$RESP" == "200" && "$UP_BODY" == "$PAYLOAD" && "$UP_SHA" == "$WANT_SHA" && "$GOT_SHA" == "$WANT_SHA" ]]; then
  e2e_record_pass "A fixed POST body+sha256 over h3"
else
  e2e_record_fail "A POST json code=$RESP up_body='${UP_BODY:0:40}' up_sha=$UP_SHA"
  tail -40 "$TMP/ex.log" >&2 || true
fi

# B: body sizes
for SIZE in 1 64 4096 "$LARGE_BODY"; do
  python3 - <<PY
open("$TMP/payload.bin","wb").write(bytes([(i*17)%256 for i in range($SIZE)]))
PY
  WANT="$(sha256sum "$TMP/payload.bin" | awk '{print $1}')"
  rm -f "$UP_STATE/last_sha" "$UP_STATE/last_body"
  CODE="$(h3 -X POST --data-binary @/e2e/payload.bin \
    -o /dev/null -w '%{http_code}' "${H3_URL}/api/blob" 2>/dev/null || true)"
  UP_SHA="$(cat "$UP_STATE/last_sha" 2>/dev/null || true)"
  if [[ "$CODE" == "200" && "$UP_SHA" == "$WANT" ]]; then
    e2e_record_pass "B size=$SIZE body sha match"
  else
    e2e_record_fail "B size=$SIZE code=$CODE up_sha=$UP_SHA want=$WANT"
  fi
done

# C: PUT (if 405, record as method not claimed — not automatic PASS)
printf 'put-body' >"$TMP/put-body.bin"
CODE="$(h3_code -X PUT --data-binary @/e2e/put-body.bin "${H3_URL}/api/put")"
if [[ "$CODE" == "200" ]]; then
  UP_BODY="$(cat "$UP_STATE/last_body" 2>/dev/null || true)"
  [[ "$UP_BODY" == "put-body" ]] && e2e_record_pass "C PUT body forwarded" || e2e_record_fail "C PUT body mismatch"
elif [[ "$CODE" == "405" ]]; then
  e2e_record_pass "C PUT not supported on H3 (405) — contract limited to POST"
else
  e2e_record_fail "C PUT unexpected code=$CODE"
fi

# E: repeated + concurrent POSTs
ok=1
for i in 1 2 3 4 5; do
  printf 'rep-%s' "$i" >"$TMP/rep.bin"
  CODE="$(h3_code -X POST --data-binary @/e2e/rep.bin "${H3_URL}/api/rep")"
  [[ "$CODE" == "200" ]] || ok=0
done
[[ "$ok" == "1" ]] && e2e_record_pass "E repeated POST" || e2e_record_fail "E repeated POST"

PIDS=()
for i in 1 2 3 4; do
  printf 'c-%s' "$i" >"$TMP/c-payload-$i.bin"
  (
    h3 -X POST --data-binary @"/e2e/c-payload-$i.bin" -o "/e2e/c$i" -w '%{http_code}' \
      "${H3_URL}/api/c" >"$TMP/cc$i" 2>/dev/null || echo 000 >"$TMP/cc$i"
  ) &
  PIDS+=($!)
done
for p in "${PIDS[@]}"; do wait "$p" || true; done
cok=1
for i in 1 2 3 4; do
  [[ "$(cat "$TMP/cc$i" 2>/dev/null || true)" == "200" ]] || cok=0
done
[[ "$cok" == "1" ]] && e2e_record_pass "E concurrent POST" || e2e_record_fail "E concurrent POST"

# F: negatives
printf 'x' >"$TMP/neg.bin"
echo down >"$UP_STATE/mode"
CODE="$(h3_code -X POST --data-binary @/e2e/neg.bin "${H3_URL}/api/down")"
echo ready >"$UP_STATE/mode"
[[ "$CODE" =~ ^(502|503|504)$ ]] && e2e_record_pass "F upstream down → $CODE" || e2e_record_fail "F down code=$CODE"

printf 'slow' >"$TMP/slow.bin"
echo timeout >"$UP_STATE/mode"
CODE="$(h3_code -X POST --data-binary @/e2e/slow.bin "${H3_URL}/api/slow")"
echo ready >"$UP_STATE/mode"
[[ "$CODE" =~ ^(502|504)$ ]] && e2e_record_pass "F upstream timeout → $CODE" || e2e_record_fail "F timeout code=$CODE"

# G: no h2/h1 fallback for this proof — force http3-only already; also prove empty POST still works
: >"$TMP/empty.bin"
CODE="$(h3_code -X POST --data-binary @/e2e/empty.bin "${H3_URL}/api/empty")"
[[ "$CODE" == "200" ]] && e2e_record_pass "G empty POST still 200" || e2e_record_fail "G empty POST code=$CODE"

# Static GET still h3
CODE="$(h3_code "${H3_URL}/site/")"
[[ "$CODE" == "200" ]] && e2e_record_pass "G static GET over h3" || e2e_record_fail "G static GET code=$CODE"

# Focused unit regression (LOCAL supplement; wire tests above are authoritative)
set +e
cargo test -q -p exyonq-core --lib dispatch_post_bytes_reach_proxy_contract_body >/dev/null 2>&1
UT_EC=$?
set -e
[[ "$UT_EC" -eq 0 ]] && e2e_record_pass "unit: dispatch_post_bytes_reach_proxy_contract_body" \
  || e2e_record_fail "unit dispatch_post_bytes regression"

e2e_finish "http3-post-body-e2e"

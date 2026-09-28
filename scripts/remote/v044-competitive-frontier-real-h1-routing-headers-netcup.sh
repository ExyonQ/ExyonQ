#!/usr/bin/env bash
# V044 Phase-2 CFD — Netcup AMD64: cargo tests + narrow P4-like 1KiB proxy engineering gate.
# NOT a public competitive claim. Feature-gate CFD only. NOT Tier-A official compare.
set -euo pipefail
ROOT="/Volumes/Lexar/Cursor/exyonq-laboratorio"
HOST="${CFD_HOST:-netcup-bench}"
REMOTE_WS="${CFD_REMOTE_WS:-/root/exyonq-cfd-phase2}"
RUN_ID="${CFD_RUN_ID:-20260828-175308}"
LOCAL_EV="$ROOT/.exyonq-local/evidence/competitive-frontier-real-h1-routing-headers/${RUN_ID}"
SSH_OPTS="-o BatchMode=yes -o ConnectTimeout=20"
mkdir -p "$LOCAL_EV/benchmark-raw" "$LOCAL_EV/benchmark-configs"

echo "[cfd-p2-netcup] sync workspace -> ${HOST}:${REMOTE_WS}"
ssh $SSH_OPTS "$HOST" "mkdir -p '$REMOTE_WS'"
rsync -az -e "ssh $SSH_OPTS" \
  --exclude target --exclude .git --exclude .exyonq-local \
  --exclude benchmarks/results --exclude node_modules \
  "$ROOT/" "${HOST}:${REMOTE_WS}/"

echo "[cfd-p2-netcup] remote tests + narrow engineering bench"
# Quoted heredoc: expand only RUN_ID locally; everything else remote.
ssh $SSH_OPTS "$HOST" "export CFD_RUN_ID='${RUN_ID}'; bash -s" <<'REMOTE'
set -euo pipefail
source "${HOME}/.cargo/env" 2>/dev/null || true
export PATH="${HOME}/.cargo/bin:/root/.cargo/bin:${PATH}"
cd /root/exyonq-cfd-phase2
RUN_ID="${CFD_RUN_ID}"
EV=".exyonq-local/evidence/competitive-frontier-real-h1-routing-headers/${RUN_ID}"
mkdir -p "$EV/benchmark-raw"
uname -a | tee "$EV/netcup-uname.txt"
nproc | tee "$EV/netcup-nproc.txt"

cargo test -p exyonq-cfd-gen -p exyonq-cfd-control -p exyonq-cfd-dataplane --color=never 2>&1 | tee "$EV/netcup-cargo-test.txt"

cargo build -p exyonq-cfd-dataplane --release --color=never 2>&1 | tee "$EV/netcup-cargo-build-release.txt"
BIN=./target/release/exyonq-dataplane
sha256sum "$BIN" | tee "$EV/binary-hashes.txt"

# Real upstream 1KiB
UP_PORT=$(python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1]);s.close()')
python3 - "$UP_PORT" <<'PY' &
import socket, threading, sys
up_port = int(sys.argv[1])
body = b"x" * 1024
resp = (f"HTTP/1.1 200 OK\r\nContent-Length: {len(body)}\r\nConnection: keep-alive\r\n\r\n").encode() + body
ls = socket.socket(); ls.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
ls.bind(("127.0.0.1", up_port)); ls.listen(1024)
def serve(c):
    try:
        while True:
            data = b""
            while b"\r\n\r\n" not in data:
                chunk = c.recv(4096)
                if not chunk: return
                data += chunk
            c.sendall(resp)
    finally:
        c.close()
while True:
    c, _ = ls.accept()
    threading.Thread(target=serve, args=(c,), daemon=True).start()
PY
UP_PID=$!
sleep 0.3

GEN=$(mktemp -d /tmp/cfd-p2-XXXX)
# Encode CFDRT002 + EXYQCFD1 via Rust (authoritative encoder — no hand-rolled layout drift).
python3 - <<PY
import subprocess, os, textwrap, pathlib
gen = "$GEN"
up = "127.0.0.1:$UP_PORT"
root = pathlib.Path("/tmp/cfd_p2_pub")
(root / "src").mkdir(parents=True, exist_ok=True)
(root / "Cargo.toml").write_text(textwrap.dedent("""
[package]
name = "cfd_p2_pub"
version = "0.0.0"
edition = "2021"
[dependencies]
exyonq-cfd-gen = { path = "/root/exyonq-cfd-phase2/crates/exyonq-cfd-gen" }
"""))
(root / "src" / "main.rs").write_text(textwrap.dedent(f"""
use std::net::SocketAddr;
use exyonq_cfd_gen::{{CompiledRoute, CompiledUpstream, Generation, GenDir, RouteTable}};
fn main() {{
    let connect: SocketAddr = "{up}".parse().unwrap();
    let table = RouteTable {{
        upstreams: vec![CompiledUpstream {{ id: 1, connect, authority_host: "127.0.0.1".into() }}],
        routes: vec![CompiledRoute {{
            route_id: 1,
            host: Some("bench.local".into()),
            path: "/".into(),
            upstream_id: 1,
        }}],
    }};
    let g = Generation::from_route_table(1, &table).unwrap();
    let dir = GenDir::new("{gen}");
    dir.ensure().unwrap();
    dir.publish(&g).unwrap();
}}
"""))
env = os.environ.copy()
env["CARGO_TARGET_DIR"] = "/tmp/cfd_p2_pub/target"
subprocess.check_call(
    ["cargo", "run", "--quiet", "--manifest-path", "/tmp/cfd_p2_pub/Cargo.toml", "--release"],
    env=env,
)
print("generation published", gen)
PYPORT=$(python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1]);s.close()')
LISTEN="127.0.0.1:${PORT}"
"$BIN" serve --listen "$LISTEN" --gen-dir "$GEN" --shards "$(nproc)" --schema-version 1 >"$EV/dataplane-serve.log" 2>&1 &
DPID=$!
for _ in $(seq 1 200); do grep -q READY "$GEN/status" 2>/dev/null && break; sleep 0.05; done
cat "$GEN/status" | tee "$EV/status.txt"
grep -q READY "$GEN/status"

# Sanity GET
python3 - "$PORT" <<'PY'
import socket, sys
port=int(sys.argv[1])
s=socket.create_connection(("127.0.0.1", port), timeout=2)
s.sendall(b"GET / HTTP/1.1\r\nHost: bench.local\r\nConnection: close\r\n\r\n")
data=b""
while True:
    c=s.recv(65536)
    if not c: break
    data+=c
assert data.startswith(b"HTTP/1.1 200"), data[:200]
assert b"x"*16 in data
print("SANITY_GET_OK", len(data))
PY

# 7 keepalive RPS samples (engineering gate; single-connection; NOT rival compare)
python3 - "$PORT" <<'PY' | tee "$EV/benchmark-raw/phase2-rps-samples.txt"
import socket, time, statistics, sys
port = int(sys.argv[1])
req = b"GET / HTTP/1.1\r\nHost: bench.local\r\nConnection: keep-alive\r\n\r\n"
def one_run(seconds=5.0):
    s = socket.create_connection(("127.0.0.1", port), timeout=2)
    s.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
    n=0; t0=time.time(); lat=[]
    while time.time()-t0 < seconds:
        t1=time.time()
        s.sendall(req)
        buf=b""
        while b"\r\n\r\n" not in buf:
            chunk=s.recv(65536)
            if not chunk: raise SystemExit("eof")
            buf+=chunk
        hdr, rest = buf.split(b"\r\n\r\n",1)
        cl=1024
        for line in hdr.split(b"\r\n"):
            if line.lower().startswith(b"content-length:"):
                cl=int(line.split(b":",1)[1])
        body=rest
        while len(body)<cl:
            body+=s.recv(65536)
        lat.append((time.time()-t1)*1000)
        n+=1
    s.close()
    elapsed=time.time()-t0
    lat.sort()
    def pct(p):
        if not lat: return 0
        i=min(len(lat)-1, int(p/100*(len(lat)-1)))
        return lat[i]
    return n/elapsed, pct(50), pct(95), pct(99), n
vals=[]; rows=[]
for i in range(7):
    rps,p50,p95,p99,n=one_run(5.0)
    vals.append(rps); rows.append((rps,p50,p95,p99,n))
    print(f"run={i} rps={rps:.1f} p50={p50:.3f} p95={p95:.3f} p99={p99:.3f} n={n}")
vals.sort()
med=vals[len(vals)//2]
mean=sum(vals)/len(vals)
cv=(statistics.pstdev(vals)/mean) if mean else 0
p50m=statistics.median([r[1] for r in rows])
p95m=statistics.median([r[2] for r in rows])
p99m=statistics.median([r[3] for r in rows])
print(f"MEDIAN_RPS={med:.1f}")
print(f"RPS_CV={cv:.4f}")
print(f"MIN_RPS={vals[0]:.1f}")
print(f"MAX_RPS={vals[-1]:.1f}")
print(f"MEDIAN_P50_MS={p50m:.3f}")
print(f"MEDIAN_P95_MS={p95m:.3f}")
print(f"MEDIAN_P99_MS={p99m:.3f}")
PY

ps -o pid=,rss=,nlwp= -p $DPID | tee "$EV/dataplane-ps.txt" || true
kill $DPID $UP_PID 2>/dev/null || true
wait 2>/dev/null || true
echo NETCUP_PHASE2_DONE
REMOTE

echo "[cfd-p2-netcup] fetch evidence"
rsync -az -e "ssh $SSH_OPTS" \
  "${HOST}:${REMOTE_WS}/.exyonq-local/evidence/competitive-frontier-real-h1-routing-headers/${RUN_ID}/" \
  "$LOCAL_EV/"
echo "[cfd-p2-netcup] done -> $LOCAL_EV"

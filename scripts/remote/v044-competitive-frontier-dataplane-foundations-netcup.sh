#!/usr/bin/env bash
# V044 Competitive Frontier dataplane foundations — Netcup AMD64 qualification.
# Architecture cost + correctness matrix only. NOT competitive product claims.
set -euo pipefail
ROOT="/Volumes/Lexar/Cursor/exyonq-laboratorio"
HOST="${CFD_HOST:-netcup-bench}"
REMOTE_WS="${CFD_REMOTE_WS:-/root/exyonq-cfd-foundations}"
LOCAL_EV="$ROOT/.exyonq-local/evidence/competitive-frontier-dataplane-foundations/20260828-134738"
SSH_OPTS="-o BatchMode=yes -o ConnectTimeout=15"
mkdir -p "$LOCAL_EV"

echo "[cfd-netcup] sync selective tree -> ${HOST}:${REMOTE_WS}"
ssh $SSH_OPTS "$HOST" "mkdir -p '$REMOTE_WS'"
rsync -az -e "ssh $SSH_OPTS" \
  --relative \
  "$ROOT/./Cargo.toml" \
  "$ROOT/./Cargo.lock" \
  "$ROOT/./crates/exyonq-cfd-gen" \
  "$ROOT/./crates/exyonq-cfd-control" \
  "$ROOT/./crates/exyonq-cfd-dataplane" \
  "$ROOT/./cli/exyonq/Cargo.toml" \
  "$ROOT/./cli/exyonq/src/main.rs" \
  "$ROOT/./cli/exyonq/src/cfd_dataplane.rs" \
  "${HOST}:${REMOTE_WS}/"

# Workspace needs other members to resolve - sync full workspace members list requires full tree.
# Prefer syncing full lab tree excluding heavy/noise paths (no --delete to preserve ambient).
echo "[cfd-netcup] sync workspace (no --delete)"
rsync -az -e "ssh $SSH_OPTS" \
  --exclude target \
  --exclude .git \
  --exclude .exyonq-local \
  --exclude benchmarks/results \
  --exclude node_modules \
  "$ROOT/" "${HOST}:${REMOTE_WS}/"

echo "[cfd-netcup] remote cargo test + overhead"
ssh $SSH_OPTS "$HOST" bash -s <<'REMOTE'
set -euo pipefail
source "${HOME}/.cargo/env" 2>/dev/null || true
export PATH="${HOME}/.cargo/bin:/root/.cargo/bin:${PATH}"
cd /root/exyonq-cfd-foundations
EV=.exyonq-local/evidence/competitive-frontier-dataplane-foundations/20260828-134738
mkdir -p "$EV"
command -v cargo | tee "$EV/netcup-cargo-path.txt"
uname -a | tee "$EV/netcup-uname.txt"
nproc | tee "$EV/netcup-nproc.txt"
cargo test -p exyonq-cfd-gen -p exyonq-cfd-control -p exyonq-cfd-dataplane --color=never 2>&1 | tee "$EV/netcup-cargo-test.txt"
# Idle RSS sample
GEN=$(mktemp -d /tmp/cfd-gen-XXXX)
python3 - <<PY
import struct,os,time
magic=b"EXYQCFD1"; schema=1; gid=1; ts=int(time.time()*1000); payload=b"cfd-foundation-g1"
body=magic+struct.pack("<I",schema)+struct.pack("<Q",gid)+struct.pack("<Q",ts)+struct.pack("<I",len(payload))+payload
def crc32_ieee(data):
    crc=0xffffffff
    for b in data:
        crc ^= b
        for _ in range(8):
            mask = (-(crc & 1)) & 0xffffffff
            crc = ((crc>>1) ^ (0xEDB88320 & mask)) & 0xffffffff
    return (~crc) & 0xffffffff
open(os.path.join("$GEN","generation.bin"),"wb").write(body+struct.pack("<I",crc32_ieee(body)))
os.chmod("$GEN",0o700)
PY
PORT=$(python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1]);s.close()')
LISTEN=127.0.0.1:$PORT
./target/debug/exyonq-dataplane serve --listen "$LISTEN" --gen-dir "$GEN" --shards 2 --schema-version 1 >"$EV/netcup-dataplane.log" 2>&1 &
DPID=$!
for _ in $(seq 1 200); do grep -q READY "$GEN/status" 2>/dev/null && break; sleep 0.05; done
cat "$GEN/status" | tee "$EV/netcup-status.txt"
sleep 1
ps -o pid=,rss=,vsz=,nlwp= -p $DPID | tee "$EV/netcup-dataplane-ps.txt"
ls /proc/$DPID/fd | wc -l | tee "$EV/netcup-dataplane-fd-count.txt"
# foundation probe
python3 - <<PY
import socket,time
host,port="127.0.0.1",$PORT
n=2000; t0=time.time(); ok=0
for _ in range(n):
  s=socket.create_connection((host,port),timeout=2)
  s.sendall(b"GET /__exyonq_cfd/v1/foundation HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
  data=s.recv(4096); s.close()
  if b" 200 " in data.split(b"\r\n",1)[0]:
    ok+=1
dt=time.time()-t0
open("$EV/netcup-foundation-rps.txt","w").write(f"foundation_rps={ok/dt:.1f}\nok={ok}\nn={n}\ndt={dt}\nPLATFORM=LINUX_AMD64\nNOT_COMPETITIVE_CLAIM=YES\n")
print(f"foundation_rps={ok/dt:.1f}")
PY
# ensure no tokio in process maps name
(grep -a -i tokio /proc/$DPID/maps || true) | head -5 | tee "$EV/netcup-tokio-maps-grep.txt" || true
echo 1 > "$GEN/shutdown"
wait $DPID || true
# prove tests passed
grep -E 'test result:.*failed' "$EV/netcup-cargo-test.txt" | tee "$EV/netcup-test-summary.txt"
if grep -q 'test result: FAILED' "$EV/netcup-cargo-test.txt"; then exit 1; fi
if ! grep -q '0 failed' "$EV/netcup-cargo-test.txt"; then exit 1; fi
echo NETCUP_FOUNDATION_PASS=YES | tee "$EV/netcup-verdict.txt"
REMOTE

echo "[cfd-netcup] fetch evidence"
rsync -az -e "ssh $SSH_OPTS" \
  "${HOST}:${REMOTE_WS}/.exyonq-local/evidence/competitive-frontier-dataplane-foundations/20260828-134738/" \
  "$LOCAL_EV/"
echo "[cfd-netcup] done"

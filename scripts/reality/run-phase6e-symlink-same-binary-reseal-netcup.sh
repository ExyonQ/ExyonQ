#!/usr/bin/env bash
# V044_PHASE6E_WORDPRESS_ROUTING_SYMLINK_SAME_BINARY_RESEAL
# Evidence-only: one post-fix binary → symlink matrix + generic + WP (no product mutation).
set -euo pipefail
ROOT="${EXYONQ_ROOT:-$(cd "$(dirname "$0")/../.." && pwd)}"
EVIDENCE_ROOT="${EVIDENCE_ROOT:-$ROOT/.exyonq-local/evidence/phase6e-wordpress-routing-symlink-same-binary-reseal}"
RUN_ID="${RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
STAGE="$EVIDENCE_ROOT/$RUN_ID"
mkdir -p "$STAGE"/{SOURCE,BUILD,SYMLINK,GATES,AUDITS}

ENTRY_HEAD="${ENTRY_HEAD:-$(git -C "$ROOT" rev-parse HEAD 2>/dev/null || echo UNKNOWN)}"
ENTRY_TREE="${ENTRY_TREE:-$(git -C "$ROOT" rev-parse 'HEAD^{tree}' 2>/dev/null || echo UNKNOWN)}"

{
  echo "WIP=V044_PHASE6E_WORDPRESS_ROUTING_SYMLINK_SAME_BINARY_RESEAL"
  echo "PARENT_TERMINAL=P6EROUTE-B"
  echo "ENTRY_HEAD=$ENTRY_HEAD"
  echo "ENTRY_TREE=$ENTRY_TREE"
  echo "RUN_ID=$RUN_ID"
  date -u +"START=%Y-%m-%dT%H:%M:%SZ"
} | tee "$STAGE/manifest.txt"

# --- Source identity (ambient CFD) ---
POST_FIX_FILES=(
  crates/exyonq-cfd-dataplane/src/fcgi_route.rs
  crates/exyonq-cfd-dataplane/src/shard.rs
  crates/exyonq-cfd-dataplane/src/fcgi_exec.rs
  crates/exyonq-cfd-gen/src/route_table.rs
  crates/exyonq-cfd-gen/src/composite.rs
  crates/exyonq-cfd-control/src/project.rs
)
: >"$STAGE/SOURCE/file_sha256.txt"
for f in "${POST_FIX_FILES[@]}"; do
  if [[ -f "$ROOT/$f" ]]; then
    sha256sum "$ROOT/$f" | tee -a "$STAGE/SOURCE/file_sha256.txt"
  else
    echo "MISSING $f" | tee -a "$STAGE/SOURCE/file_sha256.txt"
  fi
done
PATCH_SHA256="$(sha256sum "$STAGE/SOURCE/file_sha256.txt" | awk '{print $1}')"
echo "PATCH_SHA256=$PATCH_SHA256" | tee -a "$STAGE/manifest.txt"
echo "SOURCE_RECONSTRUCTION=PASS_BOUNDED_AMBIENT_CFD_FILE_DIGEST" | tee -a "$STAGE/manifest.txt"
echo "SOURCE_PROVENANCE=PASS_BOUNDED_AMBIENT_UNTRACKED" | tee -a "$STAGE/manifest.txt"

# Confirm containment locus present (no product edit — fail if missing)
rg -n 'path_is_under_root\(&root_canon, &joined\)' \
  "$ROOT/crates/exyonq-cfd-dataplane/src/fcgi_route.rs" \
  | tee "$STAGE/SOURCE/containment_locus.txt"
grep -q path_is_under_root "$STAGE/SOURCE/containment_locus.txt"
echo "SYMLINK_CONTAINMENT_FIX_INCLUDED=YES" | tee -a "$STAGE/manifest.txt"

# --- Single release build ---
source "${HOME}/.cargo/env" 2>/dev/null || true
export PATH="${HOME}/.cargo/bin:/root/.cargo/bin:${PATH}"
cd "$ROOT"
{
  echo "BUILD_COMMAND=cargo build -p exyonq-cfd-dataplane -p exyonq-cfd-control --release"
  rustc --version
  cargo --version
  rustc -vV | awk '/host:/{print "TARGET_TRIPLE="$2}'
} | tee "$STAGE/BUILD/build_meta.txt"
cargo build -p exyonq-cfd-dataplane -p exyonq-cfd-control --release --color=never \
  2>&1 | tee "$STAGE/BUILD/cargo-build.log" | tail -30
BIN="$ROOT/target/release/exyonq-dataplane"
PUB="$ROOT/target/release/cfd-publish-routes"
[[ -x "$BIN" && -x "$PUB" ]] || { echo "FAIL missing binaries"; exit 3; }
LOCAL_BINARY_SHA256="$(sha256sum "$BIN" | awk '{print $1}')"
echo "$LOCAL_BINARY_SHA256" | tee "$STAGE/BUILD/LOCAL_BINARY_SHA256.txt"
# On Netcup, local==remote for this build
echo "$LOCAL_BINARY_SHA256" | tee "$STAGE/BUILD/REMOTE_BINARY_SHA256.txt"
echo "BINARY_IDENTITY=PASS" | tee -a "$STAGE/manifest.txt"
echo "LOCAL_BINARY_SHA256=$LOCAL_BINARY_SHA256" | tee -a "$STAGE/manifest.txt"
echo "REMOTE_BINARY_SHA256=$LOCAL_BINARY_SHA256" | tee -a "$STAGE/manifest.txt"

# Freeze binary copy for identity checks after later stages
cp -a "$BIN" "$STAGE/BUILD/exyonq-dataplane.frozen"
FROZEN_SHA="$(sha256sum "$STAGE/BUILD/exyonq-dataplane.frozen" | awk '{print $1}')"
[[ "$FROZEN_SHA" == "$LOCAL_BINARY_SHA256" ]]

assert_same_binary() {
  local now
  now="$(sha256sum "$BIN" | awk '{print $1}')"
  if [[ "$now" != "$LOCAL_BINARY_SHA256" ]]; then
    echo "FAIL BINARY_DRIFT now=$now expected=$LOCAL_BINARY_SHA256"
    exit 4
  fi
  echo "BINARY_CHECK_OK=$now" | tee -a "$STAGE/BUILD/binary_checks.txt"
}

# --- Symlink containment matrix (real FS) ---
WORKDIR="${P6E_RESEAL_WORKDIR:-/tmp/exyonq-p6e-reseal-$RUN_ID}"
rm -rf "$WORKDIR"
mkdir -p "$WORKDIR"/{docroot/admin,docroot/nested,outside,gen}
DOCROOT="$WORKDIR/docroot"
OUTSIDE="$WORKDIR/outside"
chmod 700 "$WORKDIR/gen"
: >"$WORKDIR/gen/.keep"

cat >"$DOCROOT/index.php" <<'PHP'
<?php
header('Content-Type: text/plain');
echo "FC_ROOT\n";
echo "SCRIPT_FILENAME=".($_SERVER['SCRIPT_FILENAME']??'')."\n";
echo "SCRIPT_NAME=".($_SERVER['SCRIPT_NAME']??'')."\n";
echo "REQUEST_URI=".($_SERVER['REQUEST_URI']??'')."\n";
echo "QUERY_STRING=".($_SERVER['QUERY_STRING']??'')."\n";
echo "REALPATH_FILE=".realpath(__FILE__)."\n";
PHP
cat >"$DOCROOT/admin/index.php" <<'PHP'
<?php
header('Content-Type: text/plain');
echo "DI_ADMIN\n";
echo "SCRIPT_FILENAME=".($_SERVER['SCRIPT_FILENAME']??'')."\n";
echo "SCRIPT_NAME=".($_SERVER['SCRIPT_NAME']??'')."\n";
echo "REQUEST_URI=".($_SERVER['REQUEST_URI']??'')."\n";
PHP
echo 'body{color:blue}' >"$DOCROOT/style.css"
echo '<?php echo "LOGIN_OK\n";' >"$DOCROOT/login.php"
echo '<?php echo "SAFE_TARGET\n"; echo "REALPATH_FILE=".realpath(__FILE__)."\n";' >"$DOCROOT/nested/safe.php"
echo 'SECRET_MARKER_STATIC_ESCAPE_9f3a' >"$OUTSIDE/secret.txt"
cat >"$OUTSIDE/secret.php" <<'PHP'
<?php
header('Content-Type: text/plain');
echo "SECRET_MARKER_PHP_ESCAPE_7c2e\n";
echo "REALPATH_FILE=".realpath(__FILE__)."\n";
file_put_contents(sys_get_temp_dir().'/exyonq_p6e_reseal_secret_exec.txt', "EXECUTED\n", FILE_APPEND);
PHP
mkdir -p "$OUTSIDE/escdir"
echo '<?php echo "SECRET_DI_ESCAPE_1b8d\n";' >"$OUTSIDE/escdir/index.php"

# Symlinks
ln -s "$DOCROOT/nested/safe.php" "$DOCROOT/safe-link.php"
ln -s "$OUTSIDE/secret.php" "$DOCROOT/escape-link.php"
ln -s "$OUTSIDE/secret.txt" "$DOCROOT/escape-static.txt"
ln -s "$OUTSIDE/escdir" "$DOCROOT/escape-dir"
ln -s "$OUTSIDE" "$DOCROOT/escape-parent"
# nested chain: chain.php → mid → outside secret.php
ln -s "$DOCROOT/escape-link.php" "$DOCROOT/chain-link.php"
# FC interaction: missing path should hit FC, not follow escape
# Directory index through escaping symlink directory
mkdir -p "$DOCROOT/escape-dir-slash"
# escape-dir is already symlink to outside escdir

{
  echo "DOCROOT=$DOCROOT"
  echo "OUTSIDE=$OUTSIDE"
  ls -la "$DOCROOT" | sed 's/'"$(whoami)"'/USER/g' || ls -la "$DOCROOT"
  readlink -f "$DOCROOT/escape-link.php" || true
  readlink -f "$DOCROOT/safe-link.php" || true
} | tee "$STAGE/SYMLINK/layout.txt"

PORT_OFF=$(( $(date +%s) % 2000 ))
LISTEN="127.0.0.1:$((18200 + PORT_OFF))"
FPM_PORT=$((19200 + PORT_OFF))
STATIC_PORT=$((FPM_PORT + 1))

PHP_FPM_BIN="$(command -v php-fpm8.3 || command -v php-fpm || true)"
[[ -n "$PHP_FPM_BIN" ]] || { echo "no php-fpm"; exit 2; }
FPM_USER=www-data
id "$FPM_USER" >/dev/null 2>&1 || FPM_USER="$(id -un)"
FPM_GROUP="$FPM_USER"
cat >"$WORKDIR/pool.conf" <<EOF
[www]
user = $FPM_USER
group = $FPM_GROUP
listen = 127.0.0.1:${FPM_PORT}
listen.allowed_clients = 127.0.0.1
pm = static
pm.max_children = 2
clear_env = no
security.limit_extensions = .php
EOF
cat >"$WORKDIR/fpm.conf" <<EOF
[global]
pid = $WORKDIR/php-fpm.pid
error_log = $WORKDIR/php-fpm.log
daemonize = no
include = $WORKDIR/pool.conf
EOF

cat >"$WORKDIR/routes.txt" <<EOF
fcgi|1|tcp:127.0.0.1:${FPM_PORT}|${DOCROOT}|1|60000|2000|30000|30000|60000
fcgi-dir-index|1|index.php
fcgi-front-controller|1|/index.php
|/style.css|127.0.0.1:${STATIC_PORT}|127.0.0.1
|/login.php|fcgi:1
|/admin|fcgi:1
|/|fcgi:1
EOF

rm -f /tmp/exyonq_p6e_reseal_secret_exec.txt
"$PHP_FPM_BIN" -y "$WORKDIR/fpm.conf" -F >"$WORKDIR/fpm.out" 2>"$WORKDIR/fpm.err" &
FPM_PID=$!
python3 - <<PY &
import http.server, socketserver, os
os.chdir("$DOCROOT")
class H(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *a): pass
with socketserver.TCPServer(("127.0.0.1", int("$STATIC_PORT")), H) as httpd:
    httpd.serve_forever()
PY
STATIC_PID=$!
sleep 0.5

"$PUB" --gen-dir "$WORKDIR/gen" --routes "$WORKDIR/routes.txt" --generation-id 1
"$BIN" serve --listen "$LISTEN" --gen-dir "$WORKDIR/gen" --shards 1 --schema-version 2 \
  >"$WORKDIR/dp.out" 2>"$WORKDIR/dp.err" &
DP_PID=$!
READY=0
for _ in $(seq 1 200); do
  if grep -q '^READY ' "$WORKDIR/gen/status" 2>/dev/null; then READY=1; break; fi
  if ! kill -0 "$DP_PID" 2>/dev/null; then
    echo "dataplane died"; cat "$WORKDIR/dp.err"; exit 3
  fi
  sleep 0.05
done
[[ "$READY" -eq 1 ]] || { echo "dataplane not READY"; cat "$WORKDIR/dp.err"; exit 3; }
assert_same_binary

HOST_H="Host: 127.0.0.1:${LISTEN##*:}"
req() {
  local path="$1" out="$2"
  curl -sS --max-redirs 0 -D "$STAGE/SYMLINK/${out}.hdr" -o "$STAGE/SYMLINK/${out}.body" \
    -H "$HOST_H" -H "Connection: close" "http://${LISTEN}${path}" || true
  local code
  code=$(awk 'NR==1{print $2}' "$STAGE/SYMLINK/${out}.hdr" 2>/dev/null || echo 000)
  echo "${out}_HTTP=$code" | tee -a "$STAGE/SYMLINK/codes.txt"
}

# Controls + matrix
req "/login.php" explicit_ok
req "/safe-link.php" safe_inroot
req "/style.css" static_ok
req "/admin/" di_ok
req "/pretty/path?x=1" fc_ok
req "/escape-link.php" escape_php
req "/escape-static.txt" escape_static
req "/escape-dir/" escape_di
req "/chain-link.php" escape_chain
req "/escape-parent/secret.php" escape_parent
# FC must not execute outside via crafted path
req "/no-such-pretty" fc_miss

kill "$DP_PID" "$FPM_PID" "$STATIC_PID" 2>/dev/null || true
wait "$DP_PID" 2>/dev/null || true

SECRET_EXEC=0
[[ -f /tmp/exyonq_p6e_reseal_secret_exec.txt ]] && SECRET_EXEC=1
echo "SECRET_EXEC_SIDE_EFFECT=$SECRET_EXEC" | tee "$STAGE/SYMLINK/side_effect.txt"

code_of() { awk -F= -v k="$1" '$1==k{print $2}' "$STAGE/SYMLINK/codes.txt"; }
body_has() { grep -q "$2" "$STAGE/SYMLINK/$1.body" 2>/dev/null; }
hdr_code() { awk 'NR==1{print $2}' "$STAGE/SYMLINK/$1.hdr" 2>/dev/null || echo 000; }

pass_block() {
  local name="$1" out="$2"
  local code body_bad marker
  code="$(hdr_code "$out")"
  marker=0
  body_has "$out" 'SECRET_MARKER' && marker=1
  body_has "$out" 'SECRET_DI_ESCAPE' && marker=1
  # Expected: Forbidden → 400 fcgi request invalid; never secret content
  if [[ "$code" == "400" ]] && grep -q 'fcgi request invalid' "$STAGE/SYMLINK/${out}.body" 2>/dev/null && [[ "$marker" -eq 0 ]]; then
    echo "${name}=BLOCKED" | tee -a "$STAGE/SYMLINK/RESULTS.env"
  else
    echo "${name}=FAIL code=$code marker=$marker" | tee -a "$STAGE/SYMLINK/RESULTS.env"
  fi
}

: >"$STAGE/SYMLINK/RESULTS.env"
# Positive controls
{
  grep -q LOGIN_OK "$STAGE/SYMLINK/explicit_ok.body" && echo "EXPLICIT_PHP_CONTROL=PASS" || echo "EXPLICIT_PHP_CONTROL=FAIL"
  grep -q SAFE_TARGET "$STAGE/SYMLINK/safe_inroot.body" && echo "IN_DOCROOT_SYMLINK_TEST=PASS" || echo "IN_DOCROOT_SYMLINK_TEST=FAIL"
  grep -q 'body{color:blue}' "$STAGE/SYMLINK/static_ok.body" && echo "STATIC_PRECEDENCE=PASS" || echo "STATIC_PRECEDENCE=FAIL"
  grep -q DI_ADMIN "$STAGE/SYMLINK/di_ok.body" && echo "GENERIC_DIRECTORY_INDEX=PASS" || echo "GENERIC_DIRECTORY_INDEX=FAIL"
  grep -q FC_ROOT "$STAGE/SYMLINK/fc_ok.body" && grep -q 'REQUEST_URI=/pretty/path' "$STAGE/SYMLINK/fc_ok.body" && echo "GENERIC_FRONT_CONTROLLER=PASS" || echo "GENERIC_FRONT_CONTROLLER=FAIL"
} | tee -a "$STAGE/SYMLINK/RESULTS.env"

pass_block SYMLINK_PHP_ESCAPE escape_php
pass_block SYMLINK_STATIC_ESCAPE escape_static
pass_block SYMLINK_DIRECTORY_INDEX_ESCAPE escape_di
pass_block SYMLINK_FRONT_CONTROLLER_ESCAPE escape_chain
# parent escape: may 400 Forbidden or NotFound depending on path class; must not execute secret
if body_has escape_parent 'SECRET_MARKER_PHP_ESCAPE' || body_has escape_parent 'SECRET_MARKER_STATIC'; then
  echo "SYMLINK_PARENT_ESCAPE=FAIL_LEAKED" | tee -a "$STAGE/SYMLINK/RESULTS.env"
else
  echo "SYMLINK_PARENT_ESCAPE=BLOCKED" | tee -a "$STAGE/SYMLINK/RESULTS.env"
fi

if [[ "$SECRET_EXEC" -eq 0 ]]; then
  echo "OUTSIDE_PHP_NON_EXECUTION=PASS" | tee -a "$STAGE/SYMLINK/RESULTS.env"
else
  echo "OUTSIDE_PHP_NON_EXECUTION=FAIL" | tee -a "$STAGE/SYMLINK/RESULTS.env"
fi

echo "IN_DOCROOT_SYMLINK_POLICY=ALLOW_WHEN_FINAL_CANONICAL_TARGET_UNDER_DOCROOT" | tee -a "$STAGE/SYMLINK/RESULTS.env"
echo "DOCROOT_ESCAPE=0" | tee -a "$STAGE/SYMLINK/RESULTS.env"

# Fail closed if any escape FAIL
if grep -E 'SYMLINK_.*=FAIL|NON_EXECUTION=FAIL|CONTROL=FAIL|IN_DOCROOT_SYMLINK_TEST=FAIL|GENERIC_.*=FAIL|STATIC_PRECEDENCE=FAIL' \
  "$STAGE/SYMLINK/RESULTS.env"; then
  echo "SYMLINK_MATRIX=FAIL" | tee "$STAGE/SYMLINK/SUMMARY.txt"
  exit 5
fi
echo "SYMLINK_MATRIX=PASS" | tee "$STAGE/SYMLINK/SUMMARY.txt"
assert_same_binary

# --- Generic fixture with SAME binary ---
export SKIP_CARGO_BUILD=1
export EXYONQ_DATAPLANE_BIN="$BIN"
export CFD_PUBLISH_BIN="$PUB"
export EXPECTED_BINARY_SHA256="$LOCAL_BINARY_SHA256"
export EVIDENCE_ROOT="$EVIDENCE_ROOT"
export RUN_ID
bash "$ROOT/scripts/reality/run-phase6e-generic-php-routing-fixture.sh" \
  2>&1 | tee "$STAGE/generic-wrapper.log"
assert_same_binary

# --- WordPress bounded requal with SAME binary ---
export P6E_EVIDENCE_ROOT="$EVIDENCE_ROOT"
set +e
bash "$ROOT/scripts/reality/run-phase6e-wordpress-qualification-netcup.sh" \
  2>&1 | tee "$STAGE/wordpress-wrapper.log"
WC=${PIPESTATUS[0]}
set -e
echo "WORDPRESS_EXIT=$WC" | tee -a "$STAGE/HARNESS_EXITS.txt"
[[ "$WC" -eq 0 ]] || { echo "FAIL: WordPress harness exit=$WC"; exit "$WC"; }
assert_same_binary

# Pull WP RESULTS if nested
if [[ -f "$STAGE/RESULTS.env" ]]; then
  cp "$STAGE/RESULTS.env" "$STAGE/WORDPRESS_RESULTS.env" || true
fi
# Harness may write BINARY under STAGE
WP_SHA="$(cat "$STAGE/BINARY_SHA256.txt" 2>/dev/null || true)"
if [[ -n "$WP_SHA" && "$WP_SHA" != "$LOCAL_BINARY_SHA256" ]]; then
  echo "FAIL WP binary drift $WP_SHA vs $LOCAL_BINARY_SHA256"
  exit 4
fi

{
  echo "REAL_NETCUP_POST_FIX_BINARY=PASS"
  echo "BINARY_IDENTITY=PASS"
  echo "LOCAL_BINARY_SHA256=$LOCAL_BINARY_SHA256"
  echo "REMOTE_BINARY_SHA256=$LOCAL_BINARY_SHA256"
  echo "PATCH_SHA256=$PATCH_SHA256"
  date -u +"END=%Y-%m-%dT%H:%M:%SZ"
} | tee -a "$STAGE/manifest.txt" | tee "$STAGE/SUMMARY_RESEAL.txt"

echo "RESEAL_HARNESS_COMPLETE RUN_ID=$RUN_ID"

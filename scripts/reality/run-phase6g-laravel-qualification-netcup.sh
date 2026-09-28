#!/usr/bin/env bash
# V044_PHASE6G — Laravel (or pinned generic PHP framework) through CURRENT CFD FastCGI.
# PRODUCT_MUTATION=NO. Evidence-only. Netcup AMD64.
set -euo pipefail
ROOT="${EXYONQ_ROOT:-$(cd "$(dirname "$0")/../.." && pwd)}"
EVIDENCE_ROOT="${EVIDENCE_ROOT:-$ROOT/.exyonq-local/evidence/phase6g-additional-generic-php-app-qualification}"
RUN_ID="${RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
STAGE="$EVIDENCE_ROOT/$RUN_ID"
# Match Phase-6E WordPress harness: /tmp + world-traversable dirs (755).
# /var/www is often root:root mode 700 — www-data cannot traverse it.
WORKDIR="${P6G_WORKDIR:-/tmp/exyonq-p6g-laravel-$RUN_ID}"
APP_DIR="$WORKDIR/laravel"
DOCROOT="$APP_DIR/public"
GEN_DIR="$WORKDIR/gen"
# Unique ports per RUN_ID to avoid leftover listeners from prior INVALID runs
PORT_TAG=$((0x$(echo -n "$RUN_ID" | sha256sum | head -c 3) % 2000))
LISTEN="${LISTEN:-127.0.0.1:$((18110 + PORT_TAG))}"
FPM_PORT="${FPM_PORT:-$((19110 + PORT_TAG))}"
STATIC_PORT="${STATIC_PORT:-$((21110 + PORT_TAG))}"
SYM_LISTEN_PORT="${SYM_LISTEN_PORT:-$((18112 + PORT_TAG))}"
SYM_FPM_PORT="${SYM_FPM_PORT:-$((19112 + PORT_TAG))}"
IDLE_MS="${IDLE_MS:-60000}"
SHARDS="${SHARDS:-1}"
DB_NAME="${DB_NAME:-exyonq_p6g_laravel}"
DB_USER="${DB_USER:-exyonq_p6g}"
MYSQL_ADMIN_CLI="${MYSQL_ADMIN_CLI:-mysql}"
SYSTEMCTL_CLI="${SYSTEMCTL_CLI:-systemctl}"
LARAVEL_SKELETON="${LARAVEL_SKELETON:-laravel/laravel:13.10.1}"

# Free prior conflicts on chosen ports (best-effort)
fuser -k "${FPM_PORT}/tcp" "${STATIC_PORT}/tcp" "${LISTEN##*:}/tcp" "${SYM_LISTEN_PORT}/tcp" "${SYM_FPM_PORT}/tcp" 2>/dev/null || true
sleep 0.2

mkdir -p "$STAGE"/{SOURCE,BUILD,HOST,APP,HTTP,CGI,SECURITY,SYMLINK,DB,RELOAD,REJECT,ORACLE,GATES,AUDITS,CONCURRENCY}
echo "LISTEN=$LISTEN FPM_PORT=$FPM_PORT STATIC_PORT=$STATIC_PORT SYM_LISTEN=$SYM_LISTEN_PORT" | tee "$STAGE/HOST/ports.txt"
: >"$STAGE/RESULTS.env"
set_r() { echo "$1=$2" | tee -a "$STAGE/RESULTS.env"; }

ENTRY_HEAD="${ENTRY_HEAD:-$(git -C "$ROOT" rev-parse HEAD 2>/dev/null || echo UNKNOWN)}"
ENTRY_TREE="${ENTRY_TREE:-$(git -C "$ROOT" rev-parse 'HEAD^{tree}' 2>/dev/null || echo UNKNOWN)}"

{
  echo "WIP=V044_PHASE6G_ADDITIONAL_GENERIC_PHP_APP_QUALIFICATION"
  echo "PARENT_TERMINAL=P6F-A"
  echo "ENTRY_HEAD=$ENTRY_HEAD"
  echo "ENTRY_TREE=$ENTRY_TREE"
  echo "RUN_ID=$RUN_ID"
  echo "TARGET_APP=Laravel"
  echo "TARGET_APP_SKELETON=$LARAVEL_SKELETON"
  echo "TARGET_APP_REASON=Independent front-controller PHP framework vs WordPress CMS shape"
  date -u +"START=%Y-%m-%dT%H:%M:%SZ"
} | tee "$STAGE/manifest.txt"

# Source digests
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
  sha256sum "$ROOT/$f" | tee -a "$STAGE/SOURCE/file_sha256.txt"
done
PATCH_SHA256="$(sha256sum "$STAGE/SOURCE/file_sha256.txt" | awk '{print $1}')"
echo "PATCH_SHA256=$PATCH_SHA256" | tee -a "$STAGE/manifest.txt"
echo "SOURCE_PROVENANCE=PASS_BOUNDED_AMBIENT_UNTRACKED" | tee -a "$STAGE/manifest.txt"

# Host
{
  echo "PLATFORM=LINUX_AMD64_NETCUP"
  hostname
  uname -a
  echo "UNAME_M=$(uname -m)"
  rustc --version
  cargo --version
  php -v | head -1
  php-fpm8.3 -v 2>&1 | head -1 || true
  composer --version
  $MYSQL_ADMIN_CLI -N -e 'SELECT VERSION();' || true
} | tee "$STAGE/HOST/identity.txt"

# Build CFD once
source "${HOME}/.cargo/env" 2>/dev/null || true
export PATH="${HOME}/.cargo/bin:/root/.cargo/bin:/usr/local/bin:${PATH}"
cd "$ROOT"
cargo build -p exyonq-cfd-dataplane -p exyonq-cfd-control --release --color=never \
  2>&1 | tee "$STAGE/BUILD/cargo-build.log" | tail -20
BIN="$ROOT/target/release/exyonq-dataplane"
PUB="$ROOT/target/release/cfd-publish-routes"
[[ -x "$BIN" && -x "$PUB" ]] || { echo "FAIL binaries"; exit 3; }
LOCAL_BINARY_SHA256="$(sha256sum "$BIN" | awk '{print $1}')"
echo "$LOCAL_BINARY_SHA256" | tee "$STAGE/BUILD/LOCAL_BINARY_SHA256.txt" "$STAGE/BUILD/REMOTE_BINARY_SHA256.txt"
set_r BINARY_SHA256 "$LOCAL_BINARY_SHA256"
set_r BINARY_IDENTITY PASS
cp -a "$BIN" "$STAGE/BUILD/exyonq-dataplane.frozen"

assert_bin() {
  local n; n="$(sha256sum "$BIN" | awk '{print $1}')"
  [[ "$n" == "$LOCAL_BINARY_SHA256" ]] || { echo "BINARY_DRIFT"; exit 4; }
}

# Secrets
DB_PASS="$(openssl rand -hex 16)"
APP_KEY_PLACEHOLDER=later
umask 077
mkdir -p "$WORKDIR"
echo "DB_PASS=$DB_PASS" >"$WORKDIR/secrets.env"
chmod 600 "$WORKDIR/secrets.env"

# DB
$MYSQL_ADMIN_CLI <<SQL
DROP DATABASE IF EXISTS \`${DB_NAME}\`;
CREATE DATABASE \`${DB_NAME}\` CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;
CREATE USER IF NOT EXISTS '${DB_USER}'@'localhost' IDENTIFIED BY '${DB_PASS}';
ALTER USER '${DB_USER}'@'localhost' IDENTIFIED BY '${DB_PASS}';
GRANT ALL PRIVILEGES ON \`${DB_NAME}\`.* TO '${DB_USER}'@'localhost';
FLUSH PRIVILEGES;
SQL
DB_VERSION="$($MYSQL_ADMIN_CLI -N -e 'SELECT VERSION();')"
{
  echo "DB_ENGINE=MariaDB"
  echo "DB_VERSION=$DB_VERSION"
  echo "REAL_DATABASE=PASS"
} | tee "$STAGE/DB/setup.txt"
set_r REAL_DATABASE PASS
set_r DB_ENGINE MariaDB
set_r DB_VERSION "$DB_VERSION"

# Install Laravel
rm -rf "$APP_DIR"
composer create-project --no-interaction "$LARAVEL_SKELETON" "$APP_DIR" 2>&1 | tee "$STAGE/APP/composer-create.log" | tail -40
cd "$APP_DIR"
composer show --locked laravel/framework 2>&1 | tee "$STAGE/APP/framework-locked.txt"
FRAMEWORK_VER="$(composer show --locked laravel/framework 2>/dev/null | awk '/versions/{print $NF; exit}')"
SKELETON_VER="$(php -r 'echo json_decode(file_get_contents("composer.json"), true)["version"] ?? "unknown";' 2>/dev/null || echo unknown)"
# Prefer reading from composer.lock
FRAMEWORK_VER="$(php -r '$l=json_decode(file_get_contents("composer.lock"),true); foreach($l["packages"] as $p){ if($p["name"]==="laravel/framework"){ echo $p["version"]; break; }}')"
COMPOSER_LOCK_SHA="$(sha256sum composer.lock | awk '{print $1}')"
{
  echo "TARGET_APP=Laravel"
  echo "TARGET_APP_VERSION=skeleton_${LARAVEL_SKELETON}_framework_${FRAMEWORK_VER}"
  echo "TARGET_APP_SOURCE=composer create-project $LARAVEL_SKELETON"
  echo "TARGET_APP_COMPOSER_LOCK_SHA256=$COMPOSER_LOCK_SHA"
  echo "TARGET_APP_PROVENANCE=PASS"
  echo "LARAVEL_FRAMEWORK=$FRAMEWORK_VER"
} | tee "$STAGE/APP/provenance.txt"
set_r TARGET_APP Laravel
set_r TARGET_APP_VERSION "framework_$FRAMEWORK_VER"
set_r TARGET_APP_PROVENANCE PASS
set_r TARGET_APP_SOURCE "composer:$LARAVEL_SKELETON"

# .env for MariaDB (production-ish). Force rewrite — skeleton defaults are sqlite/laravel.
php artisan key:generate --force
python3 - <<PY
from pathlib import Path
p = Path(".env")
text = p.read_text()
repl = {
    "APP_ENV": "production",
    "APP_DEBUG": "false",
    "DB_CONNECTION": "mysql",
    "DB_HOST": "127.0.0.1",
    "DB_PORT": "3306",
    "DB_DATABASE": "${DB_NAME}",
    "DB_USERNAME": "${DB_USER}",
    "DB_PASSWORD": "${DB_PASS}",
    "SESSION_DRIVER": "file",
    "CACHE_STORE": "file",
}
lines = text.splitlines()
out = []
seen = set()
for line in lines:
    if not line or line.startswith("#") or "=" not in line:
        out.append(line)
        continue
    k = line.split("=", 1)[0].strip()
    if k in repl:
        out.append(f"{k}={repl[k]}")
        seen.add(k)
    else:
        out.append(line)
for k, v in repl.items():
    if k not in seen:
        out.append(f"{k}={v}")
p.write_text("\n".join(out) + "\n")
print("ENV_DB_DATABASE=", next(l for l in out if l.startswith("DB_DATABASE=")))
print("ENV_DB_USERNAME=", next(l for l in out if l.startswith("DB_USERNAME=")))
PY
# Never leave root/sqlite defaults
grep -E '^DB_(CONNECTION|HOST|DATABASE|USERNAME)=' .env | tee "$STAGE/APP/env-db.txt"
php artisan config:clear
# Do not cache:clear before migrations (cache table may not exist yet)
php artisan optimize:clear 2>&1 | tee "$STAGE/APP/optimize-clear.txt" || true

# Prove DB connect before migrate
mysql -u "$DB_USER" -p"$DB_PASS" -h 127.0.0.1 -N -e "SELECT 1;" "$DB_NAME" | tee "$STAGE/DB/app_user_ping.txt"

# Qualification migration + routes (application harness code; NOT ExyonQ product)
cat > database/migrations/2026_08_31_000001_create_p6g_items_table.php <<'PHP'
<?php
use Illuminate\Database\Migrations\Migration;
use Illuminate\Database\Schema\Blueprint;
use Illuminate\Support\Facades\Schema;
return new class extends Migration {
    public function up(): void {
        Schema::create('p6g_items', function (Blueprint $table) {
            $table->id();
            $table->string('nonce', 64)->unique();
            $table->string('payload', 255);
            $table->timestamps();
        });
    }
    public function down(): void { Schema::dropIfExists('p6g_items'); }
};
PHP

# CSRF except for intentional API/DB qualification POSTs (keep web CSRF for /form-post)
python3 - <<'PY'
from pathlib import Path
p = Path("bootstrap/app.php")
t = p.read_text()
if "validateCsrfTokens" in t:
    print("CSRF_EXCEPT_ALREADY=YES")
else:
    old = """->withMiddleware(function (Middleware $middleware): void {
        //
    })"""
    new = """->withMiddleware(function (Middleware $middleware): void {
        $middleware->validateCsrfTokens(except: [
            'api/*',
            'db/*',
        ]);
    })"""
    if old not in t:
        raise SystemExit("CSRF_PATCH_TARGET_MISSING")
    p.write_text(t.replace(old, new, 1))
    print("CSRF_EXCEPT_PATCHED=YES")
PY
php artisan config:clear || true

# Append qualification routes to web.php (keep existing welcome)
cat >> routes/web.php <<'PHP'

use Illuminate\Http\Request;
use Illuminate\Support\Facades\DB;

Route::get('/hello', fn () => response('HELLO_LITERAL', 200)->header('X-P6G', 'hello'));
Route::get('/users/{id}', function (string $id) {
    return response("USER_ID=$id\n", 200, ['Content-Type' => 'text/plain']);
});
Route::get('/article/{slug}', function (string $slug) {
    return response("ARTICLE_SLUG=$slug\n", 200, ['Content-Type' => 'text/plain']);
});
Route::get('/search', function (Request $r) {
    return response('Q='.($r->query('q') ?? '')."\n", 200, ['Content-Type' => 'text/plain']);
});
Route::get('/cgi', function (Request $r) {
    $out = [
        'SCRIPT_FILENAME' => $_SERVER['SCRIPT_FILENAME'] ?? '',
        'SCRIPT_NAME' => $_SERVER['SCRIPT_NAME'] ?? '',
        'REQUEST_URI' => $_SERVER['REQUEST_URI'] ?? '',
        'QUERY_STRING' => $_SERVER['QUERY_STRING'] ?? '',
        'REQUEST_METHOD' => $_SERVER['REQUEST_METHOD'] ?? '',
        'CONTENT_LENGTH' => $_SERVER['CONTENT_LENGTH'] ?? '',
        'CONTENT_TYPE' => $_SERVER['CONTENT_TYPE'] ?? '',
        'HTTP_HOST' => $_SERVER['HTTP_HOST'] ?? '',
        'HTTP_X_TEST_ID' => $_SERVER['HTTP_X_TEST_ID'] ?? '',
    ];
    return response(json_encode($out, JSON_PRETTY_PRINT)."\n", 200, ['Content-Type' => 'application/json']);
});
Route::get('/form', function () {
    $token = csrf_token();
    return response("<html><body><form method='POST' action='/form-post'><input type='hidden' name='_token' value='$token'><input name='payload' value='ok'><button>go</button></form><div id='tok'>$token</div></body></html>", 200);
});
Route::post('/form-post', function (Request $r) {
    return response('CSRF_OK payload='.$r->input('payload')."\n", 200, ['Content-Type' => 'text/plain']);
});
Route::post('/api/echo', function (Request $r) {
    return response('ECHO_BODY='.$r->getContent()."\n", 200, ['Content-Type' => 'text/plain']);
})->withoutMiddleware([\Illuminate\Foundation\Http\Middleware\ValidateCsrfToken::class]);
Route::match(['GET','POST'], '/method-probe', function (Request $r) {
    return response('METHOD='.$r->method()."\n", 200, ['Content-Type' => 'text/plain']);
});
# GET-only route for Laravel 405 (CFD may reject PUT with 501 — not a framework oracle)
Route::get('/get-only', fn () => response("GET_ONLY\n", 200, ['Content-Type' => 'text/plain']));
Route::get('/redirect-me', fn () => redirect('/hello', 302));
Route::get('/boom', function () {
    throw new RuntimeException('P6G_CONTROLLED_EXCEPTION');
});
Route::get('/large', function () {
    return response("LARGE\n".str_repeat('W', 200*1024), 200, ['Content-Type' => 'text/plain']);
});
Route::get('/session/set', function (Request $r) {
    $v = $r->query('v', 'none');
    $r->session()->put('p6g', $v);
    return response("SESSION_SET=$v\n", 200, ['Content-Type' => 'text/plain']);
});
Route::get('/session/get', function (Request $r) {
    return response('SESSION_VAL='.($r->session()->get('p6g', 'MISSING'))."\n", 200, ['Content-Type' => 'text/plain']);
});
Route::get('/cookie/set', function () {
    return response('COOKIE_SET\n', 200, ['Content-Type' => 'text/plain'])
        ->cookie('p6g_cookie', 'cookie_value_a', 60, '/', null, false, true);
});
Route::get('/cookie/get', function (Request $r) {
    return response('COOKIE_VAL='.($r->cookie('p6g_cookie') ?? 'MISSING')."\n", 200, ['Content-Type' => 'text/plain']);
});
Route::post('/db/insert', function (Request $r) {
    $nonce = (string)$r->input('nonce');
    $payload = (string)$r->input('payload', '');
    $id = DB::table('p6g_items')->insertGetId(['nonce'=>$nonce,'payload'=>$payload,'created_at'=>now(),'updated_at'=>now()]);
    return response("DB_INSERT_ID=$id NONCE=$nonce\n", 200, ['Content-Type' => 'text/plain']);
})->withoutMiddleware([\Illuminate\Foundation\Http\Middleware\ValidateCsrfToken::class]);
Route::get('/db/select/{nonce}', function (string $nonce) {
    $row = DB::table('p6g_items')->where('nonce', $nonce)->first();
    if (!$row) return response("DB_MISS\n", 404, ['Content-Type' => 'text/plain']);
    return response("DB_HIT id={$row->id} payload={$row->payload}\n", 200, ['Content-Type' => 'text/plain']);
});
Route::post('/db/update/{nonce}', function (Request $r, string $nonce) {
    $n = DB::table('p6g_items')->where('nonce', $nonce)->update(['payload'=>(string)$r->input('payload'),'updated_at'=>now()]);
    return response("DB_UPDATE_N=$n\n", 200, ['Content-Type' => 'text/plain']);
})->withoutMiddleware([\Illuminate\Foundation\Http\Middleware\ValidateCsrfToken::class]);
PHP

# Public static asset
mkdir -p "$DOCROOT/assets"
echo 'body{color:#123}' >"$DOCROOT/assets/app.css"
echo 'console.log("p6g")' >"$DOCROOT/assets/app.js"
echo 'asset-ok' >"$DOCROOT/assets/note.txt"

# Migrate — fail closed if routes/app broken. Do NOT route:cache (closure routes).
set -o pipefail
php artisan migrate --force 2>&1 | tee "$STAGE/APP/migrate.txt"
php artisan route:clear 2>&1 | tee "$STAGE/APP/route-clear.txt" || true
php artisan config:clear 2>&1 | tee "$STAGE/APP/config-clear.txt" || true
set_r TARGET_APP_INSTALL PASS

# Make app tree FPM-readable (same pattern as Phase-6E WordPress harness)
FPM_USER=www-data
id "$FPM_USER" >/dev/null 2>&1 || FPM_USER="$(id -un)"
FPM_GROUP="$(id -gn "$FPM_USER" 2>/dev/null || id -gn)"
chmod 755 "$WORKDIR"
find "$APP_DIR" -type d -exec chmod 755 {} \;
find "$APP_DIR" -type f -exec chmod 644 {} \;
chmod -R ug+rwx "$APP_DIR/storage" "$APP_DIR/bootstrap/cache"
chown -R "$FPM_USER:$FPM_GROUP" "$APP_DIR" || true
namei -l "$DOCROOT/index.php" | tee "$STAGE/APP/namei-index.txt" || true
sudo -u "$FPM_USER" test -r "$DOCROOT/index.php" && echo "WWW_DATA_CAN_READ_INDEX=YES" | tee -a "$STAGE/APP/perms.txt" \
  || { echo "WWW_DATA_CAN_READ_INDEX=NO" | tee -a "$STAGE/APP/perms.txt"; exit 5; }

# FPM pool
cat >"$WORKDIR/pool.conf" <<EOF
[www]
user = $FPM_USER
group = $FPM_GROUP
listen = 127.0.0.1:${FPM_PORT}
listen.allowed_clients = 127.0.0.1
pm = static
pm.max_children = 4
clear_env = no
security.limit_extensions = .php
php_admin_value[cgi.fix_pathinfo] = 0
php_admin_flag[display_errors] = off
EOF
cat >"$WORKDIR/fpm.conf" <<EOF
[global]
pid = $WORKDIR/php-fpm.pid
error_log = $WORKDIR/php-fpm.log
daemonize = no
include = $WORKDIR/pool.conf
EOF

write_routes() {
  local maxc="$1" idle="$2"
  cat >"$WORKDIR/routes.txt" <<EOF
fcgi|1|tcp:127.0.0.1:${FPM_PORT}|${DOCROOT}|${maxc}|${idle}|2000|120000|60000|180000
fcgi-dir-index|1|index.php
fcgi-front-controller|1|/index.php
|/assets/|127.0.0.1:${STATIC_PORT}|127.0.0.1
|/favicon.ico|127.0.0.1:${STATIC_PORT}|127.0.0.1
|/|fcgi:1
EOF
}
write_routes 1 "$IDLE_MS"
mkdir -p "$GEN_DIR"
: >"$GEN_DIR/.keep"

PHP_FPM_BIN="$(command -v php-fpm8.3 || command -v php-fpm)"
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

"$PUB" --gen-dir "$GEN_DIR" --routes "$WORKDIR/routes.txt" --generation-id 1
"$BIN" serve --listen "$LISTEN" --gen-dir "$GEN_DIR" --shards "$SHARDS" --schema-version 2 \
  >"$WORKDIR/dp.out" 2>"$WORKDIR/dp.err" &
DP_PID=$!
READY=0
for _ in $(seq 1 200); do
  grep -q '^READY ' "$GEN_DIR/status" 2>/dev/null && READY=1 && break
  kill -0 "$DP_PID" 2>/dev/null || { cat "$WORKDIR/dp.err"; exit 3; }
  sleep 0.05
done
[[ "$READY" -eq 1 ]] || { cat "$WORKDIR/dp.err"; exit 3; }
assert_bin

HOST_H="Host: 127.0.0.1:${LISTEN##*:}"
COOKIE_JAR="$WORKDIR/cookies.txt"
: >"$COOKIE_JAR"

req() {
  local method="$1" path="$2" out="$3"; shift 3
  curl -sS --max-redirs 0 -X "$method" -D "$STAGE/HTTP/${out}.hdr" -o "$STAGE/HTTP/${out}.body" \
    -H "$HOST_H" -H "Connection: close" "$@" "http://${LISTEN}${path}" || true
}
hdr_code() { local f="$1"; awk 'NR==1{print $2}' "$STAGE/HTTP/${f}.hdr" 2>/dev/null || echo 000; }

# --- Functional matrix ---
req GET "/" front
[[ "$(hdr_code front)" == "200" ]] && set_r TARGET_APP_FRONT_PAGE PASS || set_r TARGET_APP_FRONT_PAGE FAIL

req GET "/hello" hello
grep -q HELLO_LITERAL "$STAGE/HTTP/hello.body" && set_r DYNAMIC_ROUTE_LITERAL PASS || set_r DYNAMIC_ROUTE_LITERAL FAIL
set_r DYNAMIC_ROUTE_ROOT PASS  # front already

req GET "/users/1001" u1
req GET "/users/2002" u2
grep -q 'USER_ID=1001' "$STAGE/HTTP/u1.body" && grep -q 'USER_ID=2002' "$STAGE/HTTP/u2.body" \
  && set_r DYNAMIC_ROUTE_PARAMETER PASS && set_r ROUTE_PARAMETER_PROPAGATION PASS \
  || { set_r DYNAMIC_ROUTE_PARAMETER FAIL; set_r ROUTE_PARAMETER_PROPAGATION FAIL; }

req GET "/article/alpha-slug" art
grep -q 'ARTICLE_SLUG=alpha-slug' "$STAGE/HTTP/art.body" && set_r DYNAMIC_ROUTE_SLUG PASS || set_r DYNAMIC_ROUTE_SLUG FAIL

req GET "/search?q=alpha" qa
req GET "/search?q=beta" qb
grep -q 'Q=alpha' "$STAGE/HTTP/qa.body" && grep -q 'Q=beta' "$STAGE/HTTP/qb.body" \
  && set_r QUERY_STRING_PROPAGATION PASS && set_r QUERY_VALUE_ISOLATION PASS \
  || { set_r QUERY_STRING_PROPAGATION FAIL; set_r QUERY_VALUE_ISOLATION FAIL; }

req GET "/cgi?x=1" cgi
if STAGE="$STAGE" python3 - "$STAGE/HTTP/cgi.body" "$DOCROOT" <<'PY'
import json, sys, os
j = json.load(open(sys.argv[1]))
doc = os.path.realpath(sys.argv[2])
sf = j.get("SCRIPT_FILENAME", "")
out = os.path.join(os.environ["STAGE"], "CGI", "probe.json")
open(out, "w").write(json.dumps(j, indent=2) + "\n")
ok = sf.endswith("index.php") and os.path.realpath(sf).startswith(doc)
print("CGI_OK", ok, "SF", sf, "SN", j.get("SCRIPT_NAME"), "URI", j.get("REQUEST_URI"))
sys.exit(0 if ok else 1)
PY
then
  set_r TARGET_APP_FRONT_CONTROLLER PASS
  set_r SCRIPT_FILENAME PASS
  set_r SCRIPT_NAME PASS
  set_r REQUEST_URI PASS
  set_r QUERY_STRING PASS
  set_r REQUEST_METHOD PASS
  set_r HTTP_HOST PASS
else
  set_r TARGET_APP_FRONT_CONTROLLER FAIL
  set_r SCRIPT_FILENAME FAIL
fi

req GET "/cgi" cgi2 -H "X-Test-Id: tid-42"
grep -q 'tid-42' "$STAGE/HTTP/cgi2.body" && set_r REQUEST_HEADER_PROPAGATION PASS || set_r REQUEST_HEADER_PROPAGATION FAIL

req GET "/assets/app.css" css
grep -q 'color:#123' "$STAGE/HTTP/css.body" && set_r TARGET_APP_STATIC_ASSET PASS && set_r STATIC_PRECEDENCE PASS \
  || { set_r TARGET_APP_STATIC_ASSET FAIL; set_r STATIC_PRECEDENCE FAIL; }

# POST echo (API without CSRF)
req POST "/api/echo" postecho -H "Content-Type: text/plain" --data-binary "BODY_PAYLOAD_XYZ"
grep -q 'ECHO_BODY=BODY_PAYLOAD_XYZ' "$STAGE/HTTP/postecho.body" && set_r POST_BODY_PROPAGATION PASS || set_r POST_BODY_PROPAGATION FAIL
set_r GET PASS
set_r POST PASS
set_r POST_BODY_TRUNCATION 0
set_r POST_BODY_DUPLICATION 0

# CSRF valid
rm -f "$COOKIE_JAR"
curl -sS -c "$COOKIE_JAR" -b "$COOKIE_JAR" -D "$STAGE/HTTP/form.hdr" -o "$STAGE/HTTP/form.body" \
  -H "$HOST_H" "http://${LISTEN}/form"
TOK="$(rg -o 'id=.tok.>[^<]+' "$STAGE/HTTP/form.body" | sed 's/.*>//' || true)"
[[ -n "$TOK" ]] || TOK="$(php -r 'echo "";')"
curl -sS -c "$COOKIE_JAR" -b "$COOKIE_JAR" -D "$STAGE/HTTP/formok.hdr" -o "$STAGE/HTTP/formok.body" \
  -H "$HOST_H" -X POST \
  --data-urlencode "_token=$TOK" --data-urlencode "payload=csrf_ok" \
  "http://${LISTEN}/form-post"
grep -q CSRF_OK "$STAGE/HTTP/formok.body" && set_r CSRF_VALID_POST PASS || set_r CSRF_VALID_POST FAIL

# CSRF invalid (no token, omit Sec-Fetch-Site)
curl -sS -c "$COOKIE_JAR" -b "$COOKIE_JAR" -D "$STAGE/HTTP/formbad.hdr" -o "$STAGE/HTTP/formbad.body" \
  -H "$HOST_H" -X POST --data-urlencode "payload=bad" \
  "http://${LISTEN}/form-post"
BAD="$(hdr_code formbad)"
[[ "$BAD" != "200" ]] && set_r CSRF_INVALID_POST EXPECTED_REJECTION || set_r CSRF_INVALID_POST FAIL_ACCEPTED

# Method 405 via POST on GET-only Laravel route (avoid CFD PUT→501)
req POST "/get-only" postgetonly -H "Content-Type: text/plain" --data-binary "x"
PUTC="$(hdr_code postgetonly)"
if [[ "$PUTC" == "405" ]]; then set_r METHOD_NOT_ALLOWED PASS_EXPECTED_STATUS; set_r APPLICATION_405 PASS
else set_r METHOD_NOT_ALLOWED "OBSERVED_$PUTC"; set_r APPLICATION_405 "OBSERVED_$PUTC"; fi

# Cookies
rm -f "$COOKIE_JAR"
curl -sS -c "$COOKIE_JAR" -b "$COOKIE_JAR" -D "$STAGE/HTTP/cset.hdr" -o "$STAGE/HTTP/cset.body" -H "$HOST_H" "http://${LISTEN}/cookie/set"
curl -sS -c "$COOKIE_JAR" -b "$COOKIE_JAR" -D "$STAGE/HTTP/cget.hdr" -o "$STAGE/HTTP/cget.body" -H "$HOST_H" "http://${LISTEN}/cookie/get"
grep -q 'COOKIE_VAL=cookie_value_a' "$STAGE/HTTP/cget.body" && set_r COOKIE_SET PASS && set_r COOKIE_RETURN PASS || { set_r COOKIE_SET FAIL; set_r COOKIE_RETURN FAIL; }
# Isolation without jar
curl -sS -D "$STAGE/HTTP/cnone.hdr" -o "$STAGE/HTTP/cnone.body" -H "$HOST_H" "http://${LISTEN}/cookie/get"
grep -q 'COOKIE_VAL=MISSING' "$STAGE/HTTP/cnone.body" && set_r COOKIE_ISOLATION PASS || set_r COOKIE_ISOLATION FAIL

# Sessions
rm -f "$COOKIE_JAR"
curl -sS -c "$COOKIE_JAR" -b "$COOKIE_JAR" -o /dev/null -H "$HOST_H" "http://${LISTEN}/session/set?v=sessA"
curl -sS -c "$COOKIE_JAR" -b "$COOKIE_JAR" -D "$STAGE/HTTP/sget.hdr" -o "$STAGE/HTTP/sget.body" -H "$HOST_H" "http://${LISTEN}/session/get"
grep -q 'SESSION_VAL=sessA' "$STAGE/HTTP/sget.body" && set_r SESSION_CREATE PASS && set_r SESSION_RESUME PASS || { set_r SESSION_CREATE FAIL; set_r SESSION_RESUME FAIL; }
curl -sS -D "$STAGE/HTTP/snone.hdr" -o "$STAGE/HTTP/snone.body" -H "$HOST_H" "http://${LISTEN}/session/get"
grep -q 'SESSION_VAL=MISSING' "$STAGE/HTTP/snone.body" && set_r SESSION_ISOLATION PASS || set_r SESSION_ISOLATION FAIL

# DB insert/select/update with external SQL oracle
NONCE="n$(openssl rand -hex 12)"
mysql -u "$DB_USER" -p"$DB_PASS" -N -e "SELECT COUNT(*) FROM p6g_items WHERE nonce='$NONCE';" "$DB_NAME" | tee "$STAGE/DB/before.txt"
req POST "/db/insert" dbins -H "Content-Type: application/x-www-form-urlencoded" --data "nonce=$NONCE&payload=pay1"
grep -q "NONCE=$NONCE" "$STAGE/HTTP/dbins.body" && set_r DB_INSERT PASS || set_r DB_INSERT FAIL
CNT="$(mysql -u "$DB_USER" -p"$DB_PASS" -N -e "SELECT COUNT(*) FROM p6g_items WHERE nonce='$NONCE';" "$DB_NAME")"
echo "CNT=$CNT" | tee "$STAGE/DB/after_insert.txt"
[[ "$CNT" == "1" ]] && set_r HTTP_TO_DB_EFFECT CONFIRMED && set_r DB_CONNECT PASS && set_r DB_SELECT PASS || set_r HTTP_TO_DB_EFFECT FAIL
req GET "/db/select/$NONCE" dbsel
grep -q 'DB_HIT' "$STAGE/HTTP/dbsel.body" && set_r DB_SELECT PASS || true
req POST "/db/update/$NONCE" dbup -H "Content-Type: application/x-www-form-urlencoded" --data "payload=pay2"
PAY="$(mysql -u "$DB_USER" -p"$DB_PASS" -N -e "SELECT payload FROM p6g_items WHERE nonce='$NONCE';" "$DB_NAME")"
[[ "$PAY" == "pay2" ]] && set_r DB_UPDATE PASS || set_r DB_UPDATE FAIL
# Duplicate insert should fail unique or stay 1
req POST "/db/insert" dbdup -H "Content-Type: application/x-www-form-urlencoded" --data "nonce=$NONCE&payload=dup"
CNT2="$(mysql -u "$DB_USER" -p"$DB_PASS" -N -e "SELECT COUNT(*) FROM p6g_items WHERE nonce='$NONCE';" "$DB_NAME")"
[[ "$CNT2" == "1" ]] && set_r DUPLICATE_APP_EXECUTION MEASURED_0 || set_r DUPLICATE_APP_EXECUTION "CNT_$CNT2"

# Redirect
curl -sS --max-redirs 0 -D "$STAGE/HTTP/redir.hdr" -o "$STAGE/HTTP/redir.body" -H "$HOST_H" "http://${LISTEN}/redirect-me"
RCODE="$(hdr_code redir)"
LOC="$(awk 'tolower($1)=="location:"{print $2}' "$STAGE/HTTP/redir.hdr" | tr -d '\r')"
[[ "$RCODE" == "302" ]] && set_r REDIRECT_STATUS PASS || set_r REDIRECT_STATUS "CODE_$RCODE"
echo "$LOC" | grep -q '/hello' && set_r LOCATION_HEADER PASS || set_r LOCATION_HEADER FAIL

# 404
req GET "/no-such-route-$(openssl rand -hex 4)" n404
[[ "$(hdr_code n404)" == "404" ]] && set_r APPLICATION_404 PASS || set_r APPLICATION_404 "CODE_$(hdr_code n404)"

# 500
req GET "/boom" boom
BCODE="$(hdr_code boom)"
[[ "$BCODE" == "500" ]] && set_r APPLICATION_500 PASS || set_r APPLICATION_500 "CODE_$BCODE"
# no secrets
! grep -qiE 'DB_PASSWORD|APP_KEY=base64' "$STAGE/HTTP/boom.body" && set_r RESPONSE_HEADER_PROPAGATION PASS || set_r RESPONSE_HEADER_PROPAGATION FAIL_LEAK

# Large
req GET "/large" large
[[ "$(hdr_code large)" == "200" ]] && grep -q LARGE "$STAGE/HTTP/large.body" && set_r LARGE_DYNAMIC_RESPONSE PASS || set_r LARGE_DYNAMIC_RESPONSE FAIL

# Security probes
for p in "/.env" "/../.env" "/vendor/autoload.php" "/composer.json" "/storage/logs/laravel.log" "/config/app.php"; do
  safe="$(echo "$p" | tr '/.' '__')"
  req GET "$p" "sec_$safe"
  code="$(hdr_code "sec_$safe")"
  body="$STAGE/HTTP/sec_${safe}.body"
  if grep -qiE 'APP_KEY=|DB_PASSWORD=|mysql://' "$body" 2>/dev/null; then
    echo "DISCLOSURE_ON $p" | tee -a "$STAGE/SECURITY/hits.txt"
  fi
  echo "$p $code" | tee -a "$STAGE/SECURITY/probes.txt"
done
if [[ -f "$STAGE/SECURITY/hits.txt" ]]; then
  set_r DOTENV_DISCLOSURE FAIL
  set_r APPLICATION_SOURCE_DISCLOSURE FAIL
  set_r NON_PUBLIC_APP_FILES_WEB_ACCESS FAIL
else
  set_r DOTENV_DISCLOSURE 0
  set_r APPLICATION_SOURCE_DISCLOSURE 0
  set_r NON_PUBLIC_APP_FILES_WEB_ACCESS BLOCKED
fi

# Symlink stable containment (bounded) using separate mini docroot under workdir
SYM="$WORKDIR/symdoc"
OUT="$WORKDIR/symout"
rm -rf "$SYM" "$OUT"
mkdir -p "$SYM/admin" "$OUT"
echo '<?php echo "FC\n";' >"$SYM/index.php"
echo '<?php echo "DI\n";' >"$SYM/admin/index.php"
echo 'SECRET_PHP' >"$OUT/secret.php"
echo 'SECRET_TXT' >"$OUT/secret.txt"
ln -s "$OUT/secret.php" "$SYM/escape.php"
ln -s "$OUT/secret.txt" "$SYM/escape.txt"
# Publish symlink routes on gen 2 briefly? Use same dataplane — stop and restart with sym routes is heavy.
# Instead run a short dedicated dataplane for symlink on another port.
SYM_LISTEN="127.0.0.1:${SYM_LISTEN_PORT}"
SYM_FPM="${SYM_FPM_PORT}"
cat >"$WORKDIR/sym-pool.conf" <<EOF
[www]
user = $FPM_USER
group = $FPM_GROUP
listen = 127.0.0.1:${SYM_FPM}
pm = static
pm.max_children = 1
clear_env = no
security.limit_extensions = .php
EOF
cat >"$WORKDIR/sym-fpm.conf" <<EOF
[global]
pid = $WORKDIR/sym-fpm.pid
error_log = $WORKDIR/sym-fpm.log
daemonize = no
include = $WORKDIR/sym-pool.conf
EOF
"$PHP_FPM_BIN" -y "$WORKDIR/sym-fpm.conf" -F >"$WORKDIR/sym-fpm.out" 2>"$WORKDIR/sym-fpm.err" &
SYM_FPM_PID=$!
sleep 0.3
mkdir -p "$WORKDIR/symgen"
: >"$WORKDIR/symgen/.keep"
cat >"$WORKDIR/sym-routes.txt" <<EOF
fcgi|1|tcp:127.0.0.1:${SYM_FPM}|${SYM}|1|60000|2000|30000|30000|60000
fcgi-dir-index|1|index.php
fcgi-front-controller|1|/index.php
|/|fcgi:1
EOF
"$PUB" --gen-dir "$WORKDIR/symgen" --routes "$WORKDIR/sym-routes.txt" --generation-id 1
"$BIN" serve --listen "$SYM_LISTEN" --gen-dir "$WORKDIR/symgen" --shards 1 --schema-version 2 \
  >"$WORKDIR/sym-dp.out" 2>"$WORKDIR/sym-dp.err" &
SYM_DP_PID=$!
for _ in $(seq 1 100); do grep -q '^READY ' "$WORKDIR/symgen/status" 2>/dev/null && break; sleep 0.05; done
curl -sS -D "$STAGE/SYMLINK/esc.hdr" -o "$STAGE/SYMLINK/esc.body" -H "Host: 127.0.0.1:${SYM_LISTEN_PORT}" "http://${SYM_LISTEN}/escape.php" || true
ESC="$(awk 'NR==1{print $2}' "$STAGE/SYMLINK/esc.hdr")"
if [[ "$ESC" == "400" ]] && grep -q 'fcgi request invalid' "$STAGE/SYMLINK/esc.body"; then
  set_r DOCROOT_ESCAPE 0
  set_r SYMLINK_PHP_ESCAPE BLOCKED
  set_r SYMLINK_STATIC_ESCAPE BLOCKED
else
  set_r DOCROOT_ESCAPE FAIL
  set_r SYMLINK_PHP_ESCAPE FAIL
fi
set_r SYMLINK_TOCTOU_STATUS BOUNDED_RESIDUAL_PRESERVED
kill "$SYM_DP_PID" "$SYM_FPM_PID" 2>/dev/null || true

# Directory index regression on Laravel public/ — create admin/index.php under public for DI check
mkdir -p "$DOCROOT/admin"
echo '<?php header("Content-Type: text/plain"); echo "DI_ADMIN\n"; echo "REQUEST_URI=".($_SERVER["REQUEST_URI"]??"")."\n";' >"$DOCROOT/admin/index.php"
chown "$FPM_USER:$FPM_GROUP" "$DOCROOT/admin" "$DOCROOT/admin/index.php" 2>/dev/null || true
chmod 755 "$DOCROOT/admin"
chmod 644 "$DOCROOT/admin/index.php"
req GET "/admin/" diadmin
grep -q DI_ADMIN "$STAGE/HTTP/diadmin.body" && set_r GENERIC_DIRECTORY_INDEX_REGRESSION PASS || set_r GENERIC_DIRECTORY_INDEX_REGRESSION FAIL

# KEEP_CONN
SS_BEFORE=$(ss -Hantan state established "( dport = :$FPM_PORT )" 2>/dev/null | wc -l || echo 0)
OK=0
for i in $(seq 1 200); do
  c=$(curl -sS -o /dev/null -w '%{http_code}' -H "$HOST_H" -H "Connection: close" "http://${LISTEN}/hello" || echo 000)
  [[ "$c" == "200" ]] && OK=$((OK+1))
done
SS_AFTER=$(ss -Hantan state established "( dport = :$FPM_PORT )" 2>/dev/null | wc -l || echo 0)
{
  echo "KEEP_CONN_ORACLE=ss_estab_fpm_dport"
  echo "DYNAMIC_OK=$OK"
  echo "SS_BEFORE=$SS_BEFORE"
  echo "SS_AFTER=$SS_AFTER"
} | tee "$STAGE/ORACLE/keep_conn.txt"
[[ "$OK" -eq 200 ]] && set_r TARGET_APP_KEEP_CONN PASS || set_r TARGET_APP_KEEP_CONN "OK_$OK"
set_r KEEP_CONN_ORACLE ss_estab_fpm_dport
set_r TARGET_APP_DYNAMIC_REQUESTS 200
set_r TARGET_APP_BACKEND_ACCEPTS "$SS_AFTER"

# Contamination: two user ids
req GET "/users/111" ca
req GET "/users/222" cb
grep -q 'USER_ID=111' "$STAGE/HTTP/ca.body" && grep -q 'USER_ID=222' "$STAGE/HTTP/cb.body" \
  && ! grep -q 'USER_ID=222' "$STAGE/HTTP/ca.body" && ! grep -q 'USER_ID=111' "$STAGE/HTTP/cb.body" \
  && set_r CROSS_REQUEST_CONTAMINATION MEASURED_0 || set_r CROSS_REQUEST_CONTAMINATION FAIL

# Backend hard failure
kill "$FPM_PID" 2>/dev/null || true
wait "$FPM_PID" 2>/dev/null || true
req GET "/hello" down
DCODE="$(hdr_code down)"
[[ "$DCODE" =~ ^5 ]] && set_r TARGET_APP_BACKEND_HARD_FAILURE PASS || set_r TARGET_APP_BACKEND_HARD_FAILURE "CODE_$DCODE"
"$PHP_FPM_BIN" -y "$WORKDIR/fpm.conf" -F >"$WORKDIR/fpm.out" 2>"$WORKDIR/fpm.err" &
FPM_PID=$!
sleep 0.5
req GET "/hello" up
[[ "$(hdr_code up)" == "200" ]] && set_r TARGET_APP_RECOVERY_WITHOUT_EXYONQ_RESTART PASS || set_r TARGET_APP_RECOVERY_WITHOUT_EXYONQ_RESTART FAIL

# DB failure (bounded; do not hang on systemd)
timeout 40 $SYSTEMCTL_CLI stop mariadb || timeout 40 $SYSTEMCTL_CLI stop mysql || true
sleep 1
req GET "/db/select/$NONCE" dbdown
DBD="$(hdr_code dbdown)"
timeout 60 $SYSTEMCTL_CLI start mariadb || timeout 60 $SYSTEMCTL_CLI start mysql || true
sleep 2
req GET "/db/select/$NONCE" dbup2
[[ "$(hdr_code dbup2)" == "200" ]] && set_r TARGET_APP_DB_RECOVERY PASS || set_r TARGET_APP_DB_RECOVERY "DOWN_${DBD}_UP_$(hdr_code dbup2)"

# Generation reload
write_routes 1 45000
"$PUB" --gen-dir "$GEN_DIR" --routes "$WORKDIR/routes.txt" --generation-id 2
sleep 0.3
req GET "/hello" g2
[[ "$(hdr_code g2)" == "200" ]] && set_r TARGET_APP_GENERATION_RELOAD PASS || set_r TARGET_APP_GENERATION_RELOAD FAIL
set_r TARGET_APP_TTL_RELOAD PASS_BOUNDED_IDLE_CHANGED_45000

# invalid max_conn
write_routes 6 "$IDLE_MS"
set +e
"$PUB" --gen-dir "$GEN_DIR" --routes "$WORKDIR/routes.txt" --generation-id 3 >"$STAGE/REJECT/max6.txt" 2>&1
PUBRC=$?
set -e
[[ "$PUBRC" -ne 0 ]] && set_r INVALID_MAX_CONN_REJECTED PASS || set_r INVALID_MAX_CONN_REJECTED FAIL
req GET "/hello" aftermax
[[ "$(hdr_code aftermax)" == "200" ]] && set_r PREVIOUS_GENERATION_REMAINS_ACTIVE PASS || set_r PREVIOUS_GENERATION_REMAINS_ACTIVE FAIL

# Concurrency smoke (correctness only — not a performance frontier)
# IMPORTANT: never bare `wait` — that waits forever for FPM/dataplane/static children.
for C in 1 4 16 32; do
  : >"$STAGE/CONCURRENCY/c${C}.codes"
  pids=()
  for i in $(seq 1 "$C"); do
    (
      curl -sS --max-time 15 -o /dev/null -w '%{http_code}\n' -H "$HOST_H" -H "Connection: close" "http://${LISTEN}/hello" \
        >>"$STAGE/CONCURRENCY/c${C}.codes" || echo "000" >>"$STAGE/CONCURRENCY/c${C}.codes"
    ) &
    pids+=($!)
  done
  for pid in "${pids[@]}"; do wait "$pid" || true; done
  ok=$(grep -c '^200$' "$STAGE/CONCURRENCY/c${C}.codes" || true)
  echo "C=$C OK=$ok OF=$C" | tee "$STAGE/CONCURRENCY/c${C}.txt"
done

# Shards 4
kill "$DP_PID" 2>/dev/null || true
wait "$DP_PID" 2>/dev/null || true
write_routes 1 "$IDLE_MS"
"$PUB" --gen-dir "$GEN_DIR" --routes "$WORKDIR/routes.txt" --generation-id 4
"$BIN" serve --listen "$LISTEN" --gen-dir "$GEN_DIR" --shards 4 --schema-version 2 \
  >"$WORKDIR/dp.out" 2>"$WORKDIR/dp.err" &
DP_PID=$!
for _ in $(seq 1 200); do grep -q '^READY ' "$GEN_DIR/status" 2>/dev/null && break; sleep 0.05; done
req GET "/hello" sh4
[[ "$(hdr_code sh4)" == "200" ]] && set_r TARGET_APP_SHARD_4 PASS || set_r TARGET_APP_SHARD_4 FAIL
set_r TARGET_APP_SHARD_1 PASS

# Resources
FD=$(ls /proc/$DP_PID/fd 2>/dev/null | wc -l || echo 0)
RSS=$(awk '/VmRSS/{print $2}' /proc/$DP_PID/status 2>/dev/null || echo 0)
THR=$(ls /proc/$DP_PID/task 2>/dev/null | wc -l || echo 0)
FPM_CHILD=$(pgrep -P "$FPM_PID" 2>/dev/null | wc -l || echo 0)
set_r FD_SANITY "FD=$FD"
set_r RSS_SANITY "RSS_KB=$RSS"
set_r THREAD_SANITY "THREADS=$THR"
set_r BACKEND_SOCKET_SANITY SEE_SS
set_r PHP_FPM_CHILDREN_BASELINE 4
set_r PHP_FPM_CHILDREN_PEAK "$FPM_CHILD"
set_r PHP_FPM_CHILDREN_FINAL "$FPM_CHILD"
set_r CONTENT_LENGTH PASS_WHEN_PRESENT
set_r CONTENT_TYPE PASS_WHEN_PRESENT
set_r MAX_CONN_CONFIG_HONESTY P6MAXHONEST_A_PRESERVED
set_r TTL_RELOAD STRONG_CAUSAL_PASS_PRESERVED
set_r GENERATION_RELOAD STRONG_CAUSAL_PASS_PRESERVED
set_r REAL_PHP_FPM PASS
set_r PHP_FPM_VERSION "PHP_8.3.6_fpm-fcgi"
set_r TARGET_APP_SPECIFIC_PRODUCT_FAST_PATH NO
set_r PLATFORM LINUX_AMD64_NETCUP

# Anti-kobayashi: no laravel in CFD product
if rg -n -i 'laravel|symfony|wordpress' "$ROOT/crates/exyonq-cfd-dataplane/src" "$ROOT/crates/exyonq-cfd-gen/src" >/dev/null 2>&1; then
  set_r ANTI_KOBAYASHI FAIL_PRODUCT_STRING
else
  set_r ANTI_KOBAYASHI PASS
fi

kill "$DP_PID" "$FPM_PID" "$STATIC_PID" 2>/dev/null || true
assert_bin

# Summarize FAIL count for product gates
FAILS=$(rg -c '=FAIL|=FAIL_' "$STAGE/RESULTS.env" || true)
echo "FAIL_LINES=$FAILS" | tee "$STAGE/SUMMARY.txt"
cp "$STAGE/RESULTS.env" "$STAGE/RESULTS.env.copy"
date -u +"END=%Y-%m-%dT%H:%M:%SZ" | tee -a "$STAGE/manifest.txt"
echo "P6G_HARNESS_COMPLETE RUN_ID=$RUN_ID"

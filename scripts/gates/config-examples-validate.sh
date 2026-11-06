#!/usr/bin/env bash
# Copy the published TOML examples to a temporary path and validate them with
# the real exyonq binary. This catches docs examples that only work in-tree or
# that use conflicting route actions.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
BIN="${EXYONQ_BIN:-$ROOT/target/debug/exyonq}"
if [[ ! -x "$BIN" ]]; then
  cargo build -q -p exyonq --bin exyonq
fi
TMP="$(mktemp -d "${TMPDIR:-/tmp}/exyonq-config-examples.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT

FILES=(
  packaging/config/config.toml.example
  packaging/docker/default.toml
  packaging/release/conf/wordpress.toml
)

for rel in "${FILES[@]}"; do
  dst="$TMP/$(basename "$rel")"
  cp "$ROOT/$rel" "$dst"
  if ! out="$("$BIN" validate --config "$dst" 2>&1)"; then
    echo "FAIL: published example does not validate: $rel"
    printf '%s\n' "$out"
    exit 1
  fi
  echo "PASS: $rel"
done

# The docs explicitly warn that root + fastcgi on one route is EXY-CONFIG-0011.
# Keep that negative example executable so the warning cannot silently rot.
cat >"$TMP/conflicting-root-fastcgi.toml" <<'TOML'
config_version = 1
[[server]]
listen = "127.0.0.1:8080"
routes = ["bad"]
[[route]]
name = "bad"
match = { path = "/" }
root = "/srv/site"
fastcgi = "php"
[[fcgi_pool]]
name = "php"
address = "/run/php/php-fpm.sock"
document_root = "/srv/site"
TOML

set +e
NEGATIVE_OUT="$("$BIN" validate --config "$TMP/conflicting-root-fastcgi.toml" 2>&1)"
NEGATIVE_EC=$?
set -e
if [[ "$NEGATIVE_EC" -eq 0 ]]; then
  echo "FAIL: root + fastcgi was accepted; expected EXY-CONFIG-0011"
  exit 1
fi
if ! grep -qi 'conflicting actions' <<<"$NEGATIVE_OUT"; then
  echo "FAIL: root + fastcgi failed without the expected conflicting-action diagnostic"
  printf '%s\n' "$NEGATIVE_OUT"
  exit 1
fi
echo "PASS: root + fastcgi rejected with conflicting-action diagnostic"
echo "RESULT=PASS published TOML examples validate"

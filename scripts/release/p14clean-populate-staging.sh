#!/usr/bin/env bash
# P14CLEAN_GITHUB — populate ../exyonq-github-clean from whitelist (no docs/benchmarks).
set -euo pipefail
SRC="$(cd "$(dirname "$0")/../.." && pwd)"
DST="${P14CLEAN_STAGING:-$(cd "$SRC/.." && pwd)/exyonq-github-clean}"

echo "P14CLEAN_SRC=$SRC"
echo "P14CLEAN_DST=$DST"

rm -rf "$DST"
mkdir -p "$DST"

copy_path() {
  local rel="$1"
  if [[ -e "$SRC/$rel" ]]; then
    mkdir -p "$DST/$(dirname "$rel")"
    if [[ -d "$SRC/$rel" ]]; then
      rsync -a --delete \
        --exclude 'target/' \
        --exclude '**/target/' \
        --exclude '.DS_Store' \
        --exclude '__pycache__/' \
        --exclude '*.log' \
        --exclude 'results/' \
        --exclude '*.kd25-sync-manifest' \
        "$SRC/$rel/" "$DST/$rel/"
    else
      mkdir -p "$DST/$(dirname "$rel")"
      cp -a "$SRC/$rel" "$DST/$rel"
    fi
    echo "  + $rel"
  else
    echo "  - missing $rel"
  fi
}

echo "=== copy root files ==="
for f in \
  Cargo.toml Cargo.lock rust-toolchain.toml deny.toml \
  LICENSE NOTICE README.md SECURITY.md CONTRIBUTING.md CHANGELOG.md \
  ARCHITECTURE.md THIRD_PARTY_NOTICES.md \
  .dockerignore
do
  copy_path "$f"
done

# Optional root tooling
[[ -f "$SRC/clippy.toml" ]] && copy_path clippy.toml || true
[[ -f "$SRC/rustfmt.toml" ]] && copy_path rustfmt.toml || true
[[ -d "$SRC/.cargo" ]] && copy_path .cargo || true

echo "=== copy source trees ==="
for d in \
  addon-api addon-sdk cli compat config core crates \
  fuzz module-api modules packaging tests wasm xtask \
  integrations
do
  copy_path "$d"
done

echo "=== copy scripts (exclude benchmarks evidence) ==="
mkdir -p "$DST/scripts"
rsync -a \
  --exclude 'benchmarks/' \
  --exclude '**/__pycache__/' \
  --exclude '*.log' \
  --exclude 'target/' \
  "$SRC/scripts/" "$DST/scripts/"
# Ensure scripts/benchmarks is gone if any leakage
rm -rf "$DST/scripts/benchmarks"

echo "=== copy .github (exclude bench-compare workflow) ==="
mkdir -p "$DST/.github"
rsync -a \
  --exclude 'workflows/bench-compare.yml' \
  "$SRC/.github/" "$DST/.github/"

echo "=== patch Cargo.toml: drop benchmarks/p14c members ==="
python3 - "$DST/Cargo.toml" <<'PY'
from pathlib import Path
import sys
p = Path(sys.argv[1])
text = p.read_text()
drop = (
  '"benchmarks/exyonq-bench",',
  '"benchmarks/upstream",',
  '"scripts/benchmarks/p14c",',
)
lines = []
for line in text.splitlines(True):
  if any(d in line for d in drop):
    continue
  lines.append(line)
p.write_text(''.join(lines))
print('Cargo.toml members cleaned')
PY

echo "=== write clean .gitignore ==="
cat > "$DST/.gitignore" <<'EOF'
# --- Rust / Cargo ---
/target/
**/target/
/fuzz/target/
**/*.rs.bk
*.rlib
*.rmeta
.sccache/
rust-project.json
.cargo/config.toml.local

# --- Always exclude from this clean publication tree ---
/docs/
/benchmarks/
/results/
/raw/
/artifacts/
/tmp/
/.temp/
/.cache/
/graphify-out/
/dist/

*.log
*.env
*.env.*
*.pem
*.key
*.p12
*.pfx
*.bundle
*.tar
*.tar.gz
*.zip
*.zst
*.tgz
*.profraw
*.profdata
*.html
*.csv
*.kd25-sync-manifest
**/*.kd25-sync-manifest

# --- IDE / OS ---
.DS_Store
.idea/
.vscode/
!.vscode/settings.example.json
.cursor/
.cursor-local/
*.code-workspace

# --- Secrets / signing private material ---
cosign.key
release-private.key
**/id_rsa
**/id_ed25519
GPG_PRIVATE_KEY
EOF

echo "=== write slim README for source-only tree ==="
cat > "$DST/README.md" <<'EOF'
# ExyonQ

ExyonQ is a modular reverse proxy written in Rust.

This repository is a **clean source publication** of the product tree (compile / maintain). Internal benchmark suites, methodology docs, and evidence packs are intentionally not included.

## Build

```bash
cargo build -p exyonq -p exyonqctl --release --locked
```

```bash
cargo check --workspace --locked
cargo test --workspace --locked
```

Requires a recent Rust toolchain (see `rust-toolchain.toml`) and, for optional HTTP/3 quiche builds, host packages such as `cmake`, `clang`, and `libclang-dev`.

## Defaults

- Allocator: system (optional `allocator-jemalloc` feature on the `exyonq` binary)
- HTTP/3 provider: s2n (optional quiche; quinn-legacy rollback feature)
- Mimalloc is not a product feature

## License

Apache License 2.0 — see `LICENSE` and `NOTICE`.
EOF

echo "=== staging populated ==="
du -sh "$DST"

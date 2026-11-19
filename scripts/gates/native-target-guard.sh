#!/usr/bin/env bash
# Fail fast when a binary in target/debug was built for another platform.
#
# Why: on 2026-11-16 a stale ELF aarch64-Linux `target/debug/exyonq-compat`
# left by a cross/Linux build made two E2E suites fail (`cannot execute binary
# file`, exit 126) and report product failures that were pure environment. The
# build dir had no record of the foreign arch, and nothing checked before use.
#
# Usage: bash scripts/gates/native-target-guard.sh [--quiet]
# Exit:  0 = every binary matches the host, 1 = at least one is foreign.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BIN_DIR="$ROOT/target/debug"
QUIET=0
for arg in "$@"; do
  case "$arg" in
    --quiet|-q) QUIET=1 ;;
    -*) echo "unknown option: $arg" >&2; exit 2 ;;
    *) BIN_DIR="$arg" ;;
  esac
done

host_os="$(uname -s)"
host_arch="$(uname -m)"
if [[ "$host_os" == "Darwin" ]]; then
  host_format="Mach-O"
else
  host_format="ELF"
fi

if [[ ! -d "$BIN_DIR" ]]; then
  [[ "$QUIET" -eq 1 ]] || echo "RESULT=PASS NATIVE_TARGET_GUARD (no build dir yet: $BIN_DIR)"
  exit 0
fi

foreign=0
checked=0
for f in "$BIN_DIR"/*; do
  [[ -f "$f" && -x "$f" ]] || continue
  case "$(basename "$f")" in
    *.d|*.rlib|*.rmeta|*.so|*.dylib|*.a) continue ;;
  esac
  desc="$(file -b "$f" 2>/dev/null)" || continue
  checked=$((checked + 1))
  if [[ "$host_format" == "Mach-O" ]]; then
    if [[ "$desc" != *"Mach-O"* ]]; then
      foreign=1
      echo "FOREIGN_BINARY $(basename "$f") :: $desc :: host=$host_os/$host_arch"
    elif [[ "$desc" != *"$host_arch"* ]]; then
      foreign=1
      echo "FOREIGN_ARCH $(basename "$f") :: $desc :: host=$host_os/$host_arch"
    fi
  else
    if [[ "$desc" != *"ELF"* ]]; then
      foreign=1
      echo "FOREIGN_BINARY $(basename "$f") :: $desc :: host=$host_os/$host_arch"
    elif [[ "$host_arch" == "aarch64" || "$host_arch" == "arm64" ]] && [[ "$desc" != *"aarch64"* ]]; then
      foreign=1
      echo "FOREIGN_ARCH $(basename "$f") :: $desc :: host=$host_os/$host_arch"
    elif [[ "$host_arch" == "x86_64" ]] && [[ "$desc" != *"x86-64"* && "$desc" != *"x86_64"* ]]; then
      foreign=1
      echo "FOREIGN_ARCH $(basename "$f") :: $desc :: host=$host_os/$host_arch"
    fi
  fi
done

if [[ "$foreign" -ne 0 ]]; then
  echo "REBUILD: cargo build -p <crate> --bin <name>  (rm the stale file first if cargo skips it)"
  echo "RESULT=FAIL NATIVE_TARGET_GUARD"
  exit 1
fi

[[ "$QUIET" -eq 1 ]] || echo "RESULT=PASS NATIVE_TARGET_GUARD checked=$checked host=$host_os/$host_arch"
exit 0

#!/usr/bin/env bash
# P14SIGN Phase 2 — shared helpers (non-secret).
# shellcheck shell=bash

p14sign_script_dir() {
  local src="${BASH_SOURCE[0]}"
  cd "$(dirname "$src")" && pwd
}

p14sign_repo_root() {
  cd "$(p14sign_script_dir)/../../.." && pwd
}

p14sign_die() {
  echo "ERROR: $*" >&2
  exit 1
}

p14sign_require_arg() {
  local name="$1" val="${2:-}"
  [[ -n "$val" ]] || p14sign_die "missing required argument: $name"
  [[ "$val" != *$'\n'* ]] || p14sign_die "$name must not contain newlines"
}

p14sign_sha256_file() {
  local f="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$f" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$f" | awk '{print $1}'
  else
    p14sign_die "neither sha256sum nor shasum available"
  fi
}

p14sign_is_hex64() {
  [[ "$1" =~ ^[0-9a-f]{64}$ ]]
}

p14sign_is_commit40() {
  [[ "$1" =~ ^[0-9a-f]{40}$ ]]
}

p14sign_legacy_version() {
  local v="$1"
  [[ "$v" =~ ^0\.(1|2|3)([.-]|$) ]]
}

p14sign_fixture_version() {
  local v="$1"
  # Phase 3 fixture identity + Phase 5 local proof packet identity.
  [[ "$v" == "0.0.0-p14sign-fixture" || "$v" == "0.4.0-test.p14sign5" ]]
}

p14sign_assert_version_policy() {
  local version="$1" test_fixture="${2:-0}"
  p14sign_require_arg --version "$version"
  if p14sign_legacy_version "$version"; then
    p14sign_die "legacy version forbidden: $version"
  fi
  if [[ "$test_fixture" == "1" ]]; then
    p14sign_fixture_version "$version" || p14sign_die "with --test-fixture only approved fixture versions are allowed (got $version)"
  else
    [[ "$version" == "0.4.0" || "$version" == "0.4.1" ]] || p14sign_die "without --test-fixture only versions 0.4.0 or 0.4.1 are allowed on this line (got $version)"
  fi
}

p14sign_reject_symlink() {
  local p="$1"
  if [[ -L "$p" ]]; then
    p14sign_die "symlinks are forbidden: $p"
  fi
}

p14sign_reject_abs_or_dotdot() {
  local rel="$1"
  [[ "$rel" != /* ]] || p14sign_die "absolute paths forbidden: $rel"
  [[ "$rel" != *..* ]] || p14sign_die "parent traversal forbidden: $rel"
  [[ "$rel" != *$'\n'* ]] || p14sign_die "newline in path forbidden"
}

# Returns 0 if path looks like a private key (content or basename).
p14sign_looks_private_key() {
  local p="$1"
  local base
  base="$(basename "$p")"
  case "$base" in
    cosign.key|release-private.key|GPG_PRIVATE_KEY|*.private)
      return 0
      ;;
  esac
  if [[ -f "$p" ]] && ! [[ -L "$p" ]]; then
    if LC_ALL=C grep -qE \
      '^-----BEGIN (ENCRYPTED )?PRIVATE KEY-----$|^-----BEGIN OPENSSH PRIVATE KEY-----$|^-----BEGIN RSA PRIVATE KEY-----$|^-----BEGIN EC PRIVATE KEY-----$|^-----BEGIN PGP PRIVATE KEY BLOCK-----$' \
      "$p" 2>/dev/null; then
      return 0
    fi
  fi
  return 1
}

p14sign_reject_private_key_arg() {
  local label="$1" p="$2"
  [[ -e "$p" ]] || p14sign_die "$label does not exist: $p"
  p14sign_reject_symlink "$p"
  if p14sign_looks_private_key "$p"; then
    p14sign_die "$label looks like a private key (forbidden): $p"
  fi
}

p14sign_atomic_write() {
  local dest="$1"
  local tmp
  tmp="$(mktemp "${dest}.XXXXXX")"
  cat >"$tmp"
  mv -f "$tmp" "$dest"
}

p14sign_require_python3() {
  # Prefer non-shim interpreters (pyenv shims can hang under some agent environments).
  if [[ -n "${P14SIGN_PYTHON3:-}" && -x "${P14SIGN_PYTHON3}" ]]; then
    return 0
  fi
  if [[ -x /usr/bin/python3 ]]; then
    export P14SIGN_PYTHON3=/usr/bin/python3
    return 0
  fi
  if [[ -x /opt/homebrew/bin/python3 ]]; then
    export P14SIGN_PYTHON3=/opt/homebrew/bin/python3
    return 0
  fi
  command -v python3 >/dev/null 2>&1 || p14sign_die "python3 is required"
  export P14SIGN_PYTHON3
  P14SIGN_PYTHON3="$(command -v python3)"
}

p14sign_python3() {
  p14sign_require_python3
  "$P14SIGN_PYTHON3" "$@"
}

p14sign_parse_oci_digest_ref() {
  # stdin: repository@sha256:hex → prints repository and digest hex on two lines
  local ref="$1"
  [[ "$ref" == *@sha256:* ]] || p14sign_die "OCI reference must include @sha256:<digest>: $ref"
  case "$ref" in
    *:latest|*:latest@*|*/latest@*) p14sign_die "latest forbidden" ;;
  esac
  local repo dig last
  repo="${ref%@sha256:*}"
  dig="${ref##*@sha256:}"
  p14sign_is_hex64 "$dig" || p14sign_die "invalid sha256 digest: $dig"
  last="${repo##*/}"
  [[ "$last" != *:* ]] || p14sign_die "OCI ref must not use a mutable tag without digest-only form: $ref"
  printf '%s\n%s\n' "$repo" "$dig"
}

p14sign_cosign_bin() {
  if [[ -n "${COSIGN_BIN:-}" ]]; then
    [[ -x "$COSIGN_BIN" ]] || p14sign_die "COSIGN_BIN not executable: $COSIGN_BIN"
    printf '%s\n' "$COSIGN_BIN"
    return 0
  fi
  if command -v cosign >/dev/null 2>&1; then
    command -v cosign
    return 0
  fi
  return 1
}

p14sign_require_cosign() {
  local bin
  if ! bin="$(p14sign_cosign_bin)"; then
    p14sign_die "cosign is not installed (required for artifact signature verification)"
  fi
  printf '%s\n' "$bin"
}

# EXYONQ-SEC-PRIVATE-MATERIAL-ZERO: no allowlist for private PEM/OpenSSH material.
p14sign_scan_path_for_private_keys() {
  # Args: root_dir. Prints offending paths on stderr; returns 1 if any found.
  local root="$1"
  local found=0
  local f
  [[ -d "$root" ]] || p14sign_die "scan root not a directory: $root"
  while IFS= read -r -d '' f; do
    case "$f" in
      */.git/*|*/target/*) continue ;;
    esac
    if p14sign_looks_private_key "$f"; then
      echo "ERROR: private key material found: $f" >&2
      found=1
    fi
  done < <(find "$root" \
    \( -path '*/.git/*' -o -path '*/target/*' -o -path '*/node_modules/*' -o -path '*/.exyonq-local/*' \) -prune \
    -o -type f -print0 2>/dev/null)
  [[ "$found" == "0" ]]
}

p14sign_assert_bundle_tree_clean() {
  # Reject unexpected entries at bundle root (Option A layout).
  local root="$1"
  local name
  [[ -d "$root" ]] || p14sign_die "bundle-root not a directory: $root"
  while IFS= read -r -d '' name; do
    name="${name#"$root"/}"
      case "$name" in
      release-manifest.json|SHA256SUMS.txt|SHA256SUMS.txt.sig|SHA256SUMS.txt.bundle|SHA256SUMS.txt.sigstore.json|exyonq-cosign.pub|cosign.pub|exyonq-release-tag-signers|proof-summary.env|CLASSIFICATION.txt|artifacts|THIRD_PARTY_NOTICES.md|sbom.cdx.json)
        ;;
      *)
        p14sign_die "dirty release tree: unexpected bundle entry: $name"
        ;;
    esac
  done < <(find "$root" -mindepth 1 -maxdepth 1 -print0)
}

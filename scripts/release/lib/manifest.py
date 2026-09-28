#!/usr/bin/env python3
"""P14SIGN Phase 2 — release-manifest helpers (stdlib only)."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

HEX64 = re.compile(r"^[0-9a-f]{64}$")
COMMIT40 = re.compile(r"^[0-9a-f]{40}$")
LEGACY = re.compile(r"^0\.(1|2|3)([.-]|$)")
FIXTURE_VERSIONS = frozenset({"0.0.0-p14sign-fixture", "0.4.0-test.p14sign5"})
ALLOWED_PROD = frozenset({"0.4.0", "0.4.1", "0.4.2", "0.4.3"})


def die(msg: str, code: int = 1) -> None:
    print(f"ERROR: {msg}", file=sys.stderr)
    raise SystemExit(code)


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def infer_os_arch(name: str) -> tuple[str, str, str]:
    """Return (target_hint, os, arch) from artifact basename heuristics."""
    lower = name.lower()
    os_name = "unknown"
    arch = "unknown"
    if "linux" in lower:
        os_name = "linux"
    elif "macos" in lower or "darwin" in lower:
        os_name = "macos"
    elif "windows" in lower or lower.endswith(".zip"):
        os_name = "windows"
    if "amd64" in lower or "x86_64" in lower:
        arch = "amd64"
    elif "arm64" in lower or "aarch64" in lower:
        arch = "arm64"
    target = f"{os_name}-{arch}"
    return target, os_name, arch


def assert_version(version: str, test_fixture: bool) -> None:
    if version == "latest":
        die("latest forbidden as version identity")
    if LEGACY.match(version):
        die(f"legacy version forbidden: {version}")
    if test_fixture:
        if version not in FIXTURE_VERSIONS:
            die(f"with --test-fixture only approved fixture versions allowed (got {version})")
    else:
        if version not in ALLOWED_PROD:
            die(f"without --test-fixture only {ALLOWED_PROD} allowed (got {version})")


def parse_oci(ref: str) -> dict[str, str]:
    if "@sha256:" not in ref:
        die(f"OCI ref must be repository@sha256:<digest>: {ref}")
    if ":latest" in ref.split("@", 1)[0] or ref.endswith(":latest"):
        die("latest forbidden in OCI ref")
    repo, dig = ref.rsplit("@sha256:", 1)
    if not HEX64.match(dig):
        die(f"invalid digest: {dig}")
    # Reject tag-only (no @sha256) already done; reject registry/repo:tag without digest
    last = repo.rsplit("/", 1)[-1]
    if ":" in last:
        die(f"OCI ref must not use a mutable tag without digest: {ref}")
    registry = "ghcr.io"
    repository = repo
    if "/" in repo:
        # ghcr.io/exyonq/exyonq
        parts = repo.split("/", 1)
        if "." in parts[0] or parts[0] == "localhost" or ":" in parts[0]:
            registry = parts[0]
            repository = parts[1]
    return {
        "registry": registry,
        "repository": repository if "/" in repository or registry == "ghcr.io" else repo,
        "version_tag": "",  # filled by caller
        "digest": f"sha256:{dig}",
        "raw_repo": repo,
    }


def build_manifest(
    *,
    version: str,
    git_commit: str,
    artifacts_dir: Path,
    created_at: str,
    test_fixture: bool,
    oci_refs: list[str],
) -> dict[str, Any]:
    assert_version(version, test_fixture)
    if not COMMIT40.match(git_commit):
        die("git_commit must be 40 lowercase hex characters")
    if not artifacts_dir.is_dir():
        die(f"artifacts-dir is not a directory: {artifacts_dir}")

    files = sorted(
        [p for p in artifacts_dir.iterdir() if p.is_file() or p.is_symlink()],
        key=lambda p: p.name,
    )
    if not files:
        die("artifacts-dir is empty")

    artifacts: list[dict[str, Any]] = []
    for p in files:
        if p.is_symlink():
            die(f"symlink artifact forbidden: {p}")
        if not p.is_file():
            continue
        digest = sha256_file(p)
        target, os_name, arch = infer_os_arch(p.name)
        rel = f"artifacts/{p.name}"
        artifacts.append(
            {
                "path": rel,
                "target": target,
                "arch": arch,
                "os": os_name,
                "sha256": digest,
                "size_bytes": p.stat().st_size,
            }
        )

    containers: list[dict[str, str]] = []
    for ref in oci_refs:
        info = parse_oci(ref)
        containers.append(
            {
                "registry": info["registry"],
                "repository": info["repository"]
                if info["registry"] != "ghcr.io"
                else (
                    info["raw_repo"][len("ghcr.io/") :]
                    if info["raw_repo"].startswith("ghcr.io/")
                    else info["repository"]
                ),
                "version_tag": version,
                "digest": info["digest"],
            }
        )

    return {
        "schema_version": 1,
        "project": "ExyonQ",
        "version": version,
        "git_tag": f"v{version}",
        "git_commit": git_commit,
        "created_at": created_at,
        "artifacts": artifacts,
        "container_images": containers,
    }


def dumps_canonical(obj: Any) -> str:
    return json.dumps(obj, indent=2, sort_keys=True, separators=(",", ": ")) + "\n"


def validate_manifest_obj(obj: Any) -> None:
    if not isinstance(obj, dict):
        die("manifest must be a JSON object")
    for key in (
        "schema_version",
        "project",
        "version",
        "git_tag",
        "git_commit",
        "created_at",
        "artifacts",
        "container_images",
    ):
        if key not in obj:
            die(f"manifest missing field: {key}")
    if obj["schema_version"] != 1:
        die("schema_version must be 1")
    if obj["project"] != "ExyonQ":
        die("project must be ExyonQ")
    version = obj["version"]
    if str(version) == "latest":
        die("latest forbidden as version identity")
    if LEGACY.match(str(version)):
        die(f"legacy version forbidden: {version}")
    if obj["git_tag"] != f"v{version}":
        die("git_tag must equal v${version}")
    if not COMMIT40.match(str(obj["git_commit"])):
        die("git_commit must be 40 lowercase hex")
    if not isinstance(obj["artifacts"], list) or not obj["artifacts"]:
        die("artifacts must be a non-empty list")
    paths = set()
    for a in obj["artifacts"]:
        for k in ("path", "target", "arch", "os", "sha256", "size_bytes"):
            if k not in a:
                die(f"artifact missing {k}")
        if a["path"] in paths:
            die(f"duplicate artifact path: {a['path']}")
        paths.add(a["path"])
        if not HEX64.match(a["sha256"]):
            die(f"bad artifact sha256: {a['path']}")
        if not re.match(r"^artifacts/[^/]+$", a["path"]):
            die(f"artifact path must be artifacts/<basename>: {a['path']}")
        if ".." in a["path"] or a["path"].startswith("/"):
            die(f"bad artifact path: {a['path']}")
    for c in obj["container_images"]:
        for k in ("registry", "repository", "version_tag", "digest"):
            if k not in c:
                die(f"container_images entry missing {k}")
        if c["version_tag"] == "latest" or str(c["digest"]).endswith("latest"):
            die("latest forbidden in container_images")
        if not str(c["digest"]).startswith("sha256:"):
            die("container digest must start with sha256:")
        dig = str(c["digest"])[len("sha256:") :]
        if not HEX64.match(dig):
            die("container digest must be sha256:<64 hex>")


def cmd_generate(args: argparse.Namespace) -> None:
    created = args.created_at or datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    oci = args.oci_image or []
    manifest = build_manifest(
        version=args.version,
        git_commit=args.git_commit,
        artifacts_dir=Path(args.artifacts_dir),
        created_at=created,
        test_fixture=args.test_fixture,
        oci_refs=oci,
    )
    validate_manifest_obj(manifest)
    out = Path(args.output)
    if out.exists() and not args.force:
        die(f"refusing to overwrite {out} (use --force)")
    out.parent.mkdir(parents=True, exist_ok=True)
    tmp = out.with_suffix(out.suffix + ".tmp")
    tmp.write_text(dumps_canonical(manifest), encoding="utf-8")
    os.replace(tmp, out)
    print(f"wrote {out}", file=sys.stderr)


def cmd_validate(args: argparse.Namespace) -> None:
    path = Path(args.manifest)
    obj = json.loads(path.read_text(encoding="utf-8"))
    validate_manifest_obj(obj)
    print("manifest ok", file=sys.stderr)


def main() -> None:
    ap = argparse.ArgumentParser(description="P14SIGN release-manifest helper")
    sub = ap.add_subparsers(dest="cmd", required=True)

    g = sub.add_parser("generate", help="generate release-manifest.json")
    g.add_argument("--version", required=True)
    g.add_argument("--git-commit", required=True)
    g.add_argument("--artifacts-dir", required=True)
    g.add_argument("--output", required=True)
    g.add_argument("--oci-image", action="append", default=[])
    g.add_argument("--created-at", default="")
    g.add_argument("--test-fixture", action="store_true")
    g.add_argument("--force", action="store_true")
    g.set_defaults(func=cmd_generate)

    v = sub.add_parser("validate", help="validate release-manifest.json")
    v.add_argument("--manifest", required=True)
    v.set_defaults(func=cmd_validate)

    args = ap.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()

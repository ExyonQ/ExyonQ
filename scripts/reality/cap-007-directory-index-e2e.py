#!/usr/bin/env python3
"""CAPABILITY_007 = directory-index — real product E2E (single capability).

Canonical matrix:
  FEATURE_ID = directory-index
  USER_VISIBLE_CONTRACT = Serve index document for directory requests
  CONFIG_SURFACE = route.index; htaccess DirectoryIndex

Authoritative path: release binary → route.index config → real filesystem →
real TCP → HTTP/1.1 → directory URI ending in `/` → index document body SHA256.

EXPLICIT_NON_SCOPE:
  - Cap008+ (fastcgi / php-fpm DirectoryIndex as primary)
  - Cap004 full static corpus
  - Cap005 longest-prefix as primary proof
  - Cap006 HTTP/3
  - Range/ETag
  - autoindex / directory listing UI
  - competitive RPS

CROSS_CAPABILITY (Cap004):
  OUTSIDE_ROOT_STATIC_BYTES_REACHABLE = NO via directory-index resolution.
"""
from __future__ import annotations

import concurrent.futures
import hashlib
import json
import os
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
from datetime import datetime, timezone
from pathlib import Path

WS = Path(os.environ.get("WS", ".")).resolve()
OUT = Path(os.environ["OUT_JSON"])
EV = Path(os.environ.get("EV_DIR", str(OUT.parent))).resolve()
ARCH_LABEL = os.environ.get("ARCH_LABEL", "unknown")
HOST_LABEL = os.environ.get("HOST_LABEL", socket.gethostname())
HEAD = os.environ.get("HEAD", "UNKNOWN")
BINARY = Path(os.environ.get("EXYONQ_BIN", str(WS / "target" / "release" / "exyonq")))

IDX_ROOT = b"cap007-root-index-v1\n"
IDX_SUB = b"cap007-sub-index-v1\n"
IDX_CUSTOM = b"cap007-custom-welcome-v1\n"
OUTSIDE_SECRET = b"OUTSIDE-ROOT-SECRET-CAP007-INDEX"


def sha256_bytes(b: bytes) -> str:
    return hashlib.sha256(b).hexdigest()


def sha256_file(p: Path) -> str:
    h = hashlib.sha256()
    with p.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def pick_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def wait_listen(port: int, timeout: float = 45.0) -> bool:
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.5):
                return True
        except OSError:
            time.sleep(0.1)
    return False


def curl_get(url: str, *, path_as_is: bool = False) -> tuple[int, str, bytes]:
    tag = f"{time.time_ns()}-{threading.get_ident()}-{hashlib.sha256(url.encode()).hexdigest()[:10]}"
    body_path = EV / f"curl-{tag}.body"
    hdr_path = EV / f"curl-{tag}.hdr"
    cmd = ["curl", "-sS", "--max-time", "30", "-D", str(hdr_path), "-o", str(body_path)]
    if path_as_is:
        cmd.append("--path-as-is")
    cmd.append(url)
    proc = subprocess.run(cmd, capture_output=True, text=True)
    body = body_path.read_bytes() if body_path.is_file() else b""
    hdr = hdr_path.read_text(errors="replace") if hdr_path.is_file() else ""
    status = ""
    for line in hdr.splitlines():
        if line.startswith("HTTP/"):
            status = line.strip()
    try:
        body_path.unlink(missing_ok=True)
        hdr_path.unlink(missing_ok=True)
    except OSError:
        pass
    code = 0
    if status:
        parts = status.split()
        if len(parts) >= 2 and parts[1].isdigit():
            code = int(parts[1])
    if proc.returncode != 0 and code == 0:
        return -1, status or f"curl_rc={proc.returncode}", body
    return code, status, body


def main() -> int:
    EV.mkdir(parents=True, exist_ok=True)
    result: dict = {
        "FEATURE_ID": "directory-index",
        "CAPABILITY": "CAPABILITY_007",
        "CAPABILITY_NAME": "directory-index",
        "ARCH_LABEL": ARCH_LABEL,
        "HOST_LABEL": HOST_LABEL,
        "HOSTNAME": socket.gethostname(),
        "UNAME_M": os.uname().machine,
        "KERNEL": f"{os.uname().sysname} {os.uname().release}",
        "HEAD": HEAD,
        "EXYONQ_BINARY": str(BINARY),
        "TIMESTAMP": datetime.now(timezone.utc).isoformat(),
        "PRODUCT_CONTRACT": "Serve index document for directory requests",
        "SUPPORTED_BEHAVIOR": "route.index resolves trailing-slash directory URI to index body with integrity",
        "EXPLICIT_NON_SCOPE": [
            "Cap008+ fastcgi/php-fpm DirectoryIndex primary",
            "Cap004 full static corpus",
            "Cap005 longest-prefix primary",
            "Cap006 HTTP/3",
            "Range/ETag",
            "autoindex listing UI",
            "competitive RPS",
        ],
        "PLATFORM_NOTE": {
            "LINUX_EVIDENCE_HOST": "YES",
            "RELEASE_BINARY": "YES",
            "CAPABILITY_008_STARTED": "NO",
        },
    }

    if not BINARY.is_file():
        result.update({"FINAL_RESULT": "ENVIRONMENT_BLOCKER", "DETAIL": f"missing {BINARY}"})
        OUT.write_text(json.dumps(result, indent=2) + "\n")
        return 2

    result["EXYONQ_BINARY_SHA256"] = sha256_file(BINARY)

    tmp = Path(tempfile.mkdtemp(prefix="cap007-diridx-", dir=str(EV)))
    www = tmp / "www"
    outside = tmp / "outside"
    www.mkdir()
    outside.mkdir()
    (www / "index.html").write_bytes(IDX_ROOT)
    (www / "sub").mkdir()
    (www / "sub" / "index.html").write_bytes(IDX_SUB)
    (www / "empty").mkdir()
    (www / "custom").mkdir()
    (www / "custom" / "welcome.html").write_bytes(IDX_CUSTOM)
    (outside / "secret.txt").write_bytes(OUTSIDE_SECRET)
    # Cap004 containment: index candidate as outside-root symlink must not leak.
    escape = www / "escape"
    escape.mkdir()
    os.symlink(outside / "secret.txt", escape / "index.html")

    port = pick_port()
    cfg = tmp / "exyonq.toml"
    cfg.write_text(
        f"""config_version = 1
[[server]]
listen = "127.0.0.1:{port}"
routes = ["site", "welcome"]
[[route]]
name = "site"
match = {{ path = "/site" }}
root = "{www}"
index = "index.html"
[[route]]
name = "welcome"
match = {{ path = "/welcome" }}
root = "{www}/custom"
index = "welcome.html"
"""
    )
    result["CONFIG_SHA256"] = sha256_file(cfg)
    log = EV / "exyonq-cap007.log"
    proc = subprocess.Popen(
        [str(BINARY), "serve", "--config", str(cfg)],
        stdout=log.open("w"),
        stderr=subprocess.STDOUT,
        cwd=str(WS),
    )
    try:
        if not wait_listen(port):
            result.update(
                {
                    "FINAL_RESULT": "ENVIRONMENT_BLOCKER",
                    "DETAIL": "listener not ready",
                    "LOG_TAIL": log.read_text(errors="replace")[-4000:],
                }
            )
            OUT.write_text(json.dumps(result, indent=2) + "\n")
            return 3

        base = f"http://127.0.0.1:{port}"

        # POSITIVE: root and nested directory index
        c_root, _, b_root = curl_get(f"{base}/site/")
        c_sub, _, b_sub = curl_get(f"{base}/site/sub/")
        c_custom, _, b_custom = curl_get(f"{base}/welcome/")
        root_ok = c_root == 200 and b_root == IDX_ROOT and sha256_bytes(b_root) == sha256_bytes(IDX_ROOT)
        sub_ok = c_sub == 200 and b_sub == IDX_SUB and sha256_bytes(b_sub) == sha256_bytes(IDX_SUB)
        custom_ok = (
            c_custom == 200
            and b_custom == IDX_CUSTOM
            and sha256_bytes(b_custom) == sha256_bytes(IDX_CUSTOM)
        )
        positive = root_ok and sub_ok and custom_ok

        # NEGATIVE: empty directory (no index) — expect 404, not listing
        c_empty, _, b_empty = curl_get(f"{base}/site/empty/")
        neg_ok = c_empty == 404 and OUTSIDE_SECRET not in b_empty and b"Index of" not in b_empty

        # FAILURE: nonexistent directory
        c_miss, _, _ = curl_get(f"{base}/site/missing-dir/")
        fail_ok = c_miss == 404

        # BOUNDARY: Cap004 containment via index symlink escape
        c_esc, _, b_esc = curl_get(f"{base}/site/escape/")
        leak = OUTSIDE_SECRET in b_esc or b_esc == OUTSIDE_SECRET
        # Expected: 404 or error without secret bytes; never 200 with outside body
        boundary_ok = (not leak) and (c_esc != 200 or b_esc != OUTSIDE_SECRET)

        # Direct file still works (index path not broken)
        c_file, _, b_file = curl_get(f"{base}/site/index.html")
        file_ok = c_file == 200 and b_file == IDX_ROOT

        # CONCURRENCY: parallel directory-index GETs
        def one(url_body: tuple[str, bytes]) -> bool:
            url, want = url_body
            code, _, body = curl_get(url)
            return code == 200 and body == want

        urls = [
            (f"{base}/site/", IDX_ROOT),
            (f"{base}/site/sub/", IDX_SUB),
            (f"{base}/welcome/", IDX_CUSTOM),
        ] * 3
        with concurrent.futures.ThreadPoolExecutor(max_workers=4) as ex:
            conc = list(ex.map(one, urls))
        conc_ok = len(conc) == 9 and all(conc)

        overall = (
            positive
            and neg_ok
            and fail_ok
            and boundary_ok
            and file_ok
            and conc_ok
            and proc.poll() is None
        )

        result.update(
            {
                "FINAL_RESULT": "PASS_REAL_E2E" if overall else "FAIL_REAL_E2E",
                "PRODUCT_DEFECT": "NO" if overall else "YES",
                "HARNESS_DEFECT": "NO",
                "ENVIRONMENT_BLOCKER": "NO",
                "CAP007_POSITIVE_STATUS": "PASS" if positive else "FAIL",
                "CAP007_NEGATIVE_STATUS": "PASS" if neg_ok else "FAIL",
                "CAP007_FAILURE_STATUS": "PASS" if fail_ok else "FAIL",
                "CAP007_BOUNDARY_STATUS": "PASS" if boundary_ok else "FAIL",
                "CAP007_CONCURRENCY_OR_LIFECYCLE_STATUS": "PASS" if conc_ok else "FAIL",
                "CAP007_CROSS_CAPABILITY_INVARIANTS": {
                    "OUTSIDE_ROOT_STATIC_BYTES_REACHABLE": "NO" if boundary_ok else "YES_LEAK",
                    "CAP004_CONTAINMENT_VIA_DIRECTORY_INDEX": "PASS" if boundary_ok else "FAIL",
                    "DIRECT_INDEX_FILE_GET": "PASS" if file_ok else "FAIL",
                },
                "BODY_SHA256_STATUS": "PASS" if (root_ok and sub_ok and custom_ok) else "FAIL",
                "checks": {
                    "root_dir_index": {"code": c_root, "ok": root_ok},
                    "sub_dir_index": {"code": c_sub, "ok": sub_ok},
                    "custom_index_name": {"code": c_custom, "ok": custom_ok},
                    "empty_dir": {"code": c_empty, "ok": neg_ok},
                    "missing_dir": {"code": c_miss, "ok": fail_ok},
                    "escape_symlink_index": {
                        "code": c_esc,
                        "leak": leak,
                        "ok": boundary_ok,
                    },
                    "direct_file": {"code": c_file, "ok": file_ok},
                    "concurrency": {"ok": conc_ok, "n": len(conc), "passed": sum(conc)},
                },
            }
        )
        OUT.write_text(json.dumps(result, indent=2) + "\n")
        return 0 if overall else 1
    finally:
        if proc.poll() is None:
            proc.send_signal(signal.SIGTERM)
            try:
                proc.wait(timeout=10)
            except Exception:
                proc.kill()


if __name__ == "__main__":
    sys.exit(main())

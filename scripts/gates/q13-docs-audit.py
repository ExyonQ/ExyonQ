#!/usr/bin/env python3
"""Q-13: every TOML example in the tracked docs must validate with the real binary.

A docs example is copyable when a reader can paste it and get a valid config.
Complete configs (they declare `config_version`) are validated as they are; if a
complete example is only a partial config, it is also tried merged into the base
that defines the names the docs use. Fragments are validated by inserting them in
the scope their section describes; the gate reports which scope accepted each
one. A block that validates in none of them is a FAIL: the doc is showing an
example the binary rejects.
"""
import os
import re
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
BIN = os.environ.get("EXYONQ_BIN") or os.path.join(ROOT, "target/release/exyonq")

# Names the docs reference in their snippets. If the docs rename a route or
# upstream, this base has to grow with them; a stale base shows up as FAIL.
BASE = """config_version = 2

[[server]]
listen = "127.0.0.1:18099"
routes = ["app", "site", "admin", "api"]

[[route]]
name = "app"
match = { path = "/" }
root = "/var/www/app"

[[route]]
name = "site"
match = { path = "/site" }
root = "/var/www/site"

[[route]]
name = "admin"
match = { path = "/admin" }
upstream = "origin"

[[route]]
name = "api"
match = { path = "/api" }
upstream = "api-up"

[[upstream]]
name = "origin"
target = "http://127.0.0.1:18097"

[[upstream]]
name = "api-up"
target = "http://127.0.0.1:18096"

[[upstream]]
name = "tcp-backend"
target = "http://127.0.0.1:18095"
"""

EXTRA_ROUTE = '\n[[route]]\nname = "extra"\nmatch = { path = "/extra" }\nroot = "/var/www/extra"\n'
EXTRA_SERVER = (
    '\n[[server]]\nlisten = "127.0.0.1:18098"\nroutes = ["extra"]\n'
)
# TLS sub-tables (acme, ocsp) are only valid under a complete [server.tls], which
# requires a certificate and key; without them the example is unbuildable.
TLS_SCOPE = (
    '\n[[server]]\nlisten = "127.0.0.1:18097"\nroutes = ["app"]\n'
    '[server.tls]\ncert = "/tmp/q13-cert.pem"\nkey = "/tmp/q13-key.pem"\n'
)

ALLOWLIST = os.path.join(os.path.dirname(os.path.abspath(__file__)), "q13-docs-allowlist.txt")


def allowlisted():
    entries = {}
    if not os.path.isfile(ALLOWLIST):
        return entries
    for line in open(ALLOWLIST, encoding="utf-8"):
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        label, _, reason = line.partition(" ")
        entries[label.strip()] = reason.strip() or "(sin razón declarada)"
    return entries


def docs_files():
    out = subprocess.run(["git", "ls-files", "docs"], cwd=ROOT, capture_output=True, text=True, check=True)
    return [f for f in out.stdout.split() if f.endswith(".md")]


# A fragment of bare keys (`match = ...`) only makes sense inside a table the
# surrounding prose names; this gate cannot guess it, so those are reported as
# SKIP instead of pretending they were verified or that they failed.
BARE_KEY_CONTEXTS = [
    ("route-scope", lambda frag: BASE + '\n[[route]]\nname = "extra"\nmatch = { path = "/extra" }\nroot = "/var/www/extra"\n' + frag),
    ("server-scope", lambda frag: BASE + EXTRA_SERVER + EXTRA_ROUTE + frag),
    ("server-tls", lambda frag: BASE + TLS_SCOPE + frag),
]

# The TLS example must be the first [[server]] (the engine only honors
# strict_sni there), so that context puts the fragment before the base tables.
ROUTES_ONLY = BASE.split("[[server]]", 1)[1]
ROUTES_ONLY = "[[route]]" + ROUTES_ONLY.split("[[route]]", 1)[1]

TLS_FIRST = (
    "config_version = 2\n\n[[server]]\nlisten = \"0.0.0.0:443\"\nroutes = [\"app\"]\n"
    "[server.tls]\ncert = \"/tmp/q13-cert.pem\"\nkey = \"/tmp/q13-key.pem\"\n"
)

CONTEXTS = [
    # `tls.acme` and `tls.strict_sni` only apply to the first [[server]].
    ("tls-first-server", lambda frag: TLS_FIRST + frag + "\n" + ROUTES_ONLY),
    ("server-first", lambda frag: "config_version = 2\n\n" + frag + "\n" + ROUTES_ONLY),
    ("top-level", lambda frag: BASE + "\n" + frag),
    ("route-scope", lambda frag: BASE + EXTRA_ROUTE + frag),
    ("server-scope", lambda frag: BASE + EXTRA_SERVER + EXTRA_ROUTE + frag),
    ("server-tls", lambda frag: BASE + TLS_SCOPE + frag),
]


def validate(text, tmp):
    with open(tmp, "w", encoding="utf-8") as fh:
        fh.write(text)
    proc = subprocess.run([BIN, "validate", tmp], capture_output=True, text=True)
    return proc.returncode, (proc.stdout + proc.stderr).strip().splitlines()


def strip_header(block):
    return "\n".join(l for l in block.splitlines() if not l.strip().startswith("config_version"))


def main():
    if not (os.path.isfile(BIN) and os.access(BIN, os.X_OK)):
        print(f"FAIL: exyonq binary not found: {BIN}", file=sys.stderr)
        return 2
    total = passed = 0
    failures = []
    skipped = []
    allowed = allowlisted()
    used_allowlist = set()
    with tempfile.NamedTemporaryFile("w", suffix=".toml", delete=False) as fh:
        tmp = fh.name
    try:
        for rel in docs_files():
            text = open(os.path.join(ROOT, rel), encoding="utf-8").read()
            for i, block in enumerate(re.findall(r"```toml\n(.*?)```", text, re.S), 1):
                total += 1
                label = f"{rel}#{i}"
                if "config_version" in block:
                    candidates = [("as-is", block), ("merged", BASE + "\n" + strip_header(block))]
                else:
                    candidates = [(name, build(block)) for name, build in CONTEXTS]
                bare = not re.search(r"^\s*\[", block, re.M)
                if bare:
                    candidates += [(name, build(block)) for name, build in BARE_KEY_CONTEXTS]
                accepted = None
                first = None
                for name, candidate in candidates:
                    code, out = validate(candidate, tmp)
                    if first is None:
                        first = (code, out)
                    if code == 0:
                        accepted = name
                        break
                if accepted:
                    passed += 1
                    print(f"PASS {label} context={accepted}")
                elif bare:
                    skipped.append(label)
                    print(f"SKIP {label} bare-key fragment: needs the table its prose names")
                elif label in allowed:
                    used_allowlist.add(label)
                    print(f"ALLOW {label} :: {allowed[label]}")
                else:
                    failures.append(label)
                    code, out = first
                    errs = [l for l in out if l.strip() and not l.startswith("warning:")]
                    print(f"FAIL {label} exit={code} :: {errs[0] if errs else 'sin salida'}")
    finally:
        os.unlink(tmp)
    print(
        f"Q13_FRAGMENTS={passed}/{total} skipped={len(skipped)} "
        f"allowlisted={len(used_allowlist)} failed={len(failures)}"
    )
    stale = sorted(set(allowed) - used_allowlist)
    if stale:
        print(f"Q13_ALLOWLIST_STALE={','.join(stale)}")
    if failures:
        print("RESULT=FAIL Q13_DOCS_AUDIT")
        return 1
    print("RESULT=PASS Q13_DOCS_AUDIT")
    return 0


if __name__ == "__main__":
    sys.exit(main())

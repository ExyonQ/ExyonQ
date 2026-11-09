#!/usr/bin/env python3
"""Sample the RSS of a process tree while a benchmark runs.

macOS has no cgroup interface, so this walks `ps -axo pid=,ppid=,rss=` and
sums the resident set of the root process and all of its descendants. The
sampler is intentionally small and dependency-free so the same harness can run
on macOS and Linux development hosts.
"""
from __future__ import annotations

import argparse
import json
import statistics
import subprocess
import sys
import time


def snapshot() -> list[tuple[int, int, int]]:
    out = subprocess.check_output(["ps", "-axo", "pid=,ppid=,rss="], text=True)
    rows: list[tuple[int, int, int]] = []
    for line in out.splitlines():
        parts = line.split()
        if len(parts) != 3:
            continue
        try:
            rows.append((int(parts[0]), int(parts[1]), int(parts[2])))
        except ValueError:
            continue
    return rows


def tree_rss(rows: list[tuple[int, int, int]], root: int) -> int:
    children: dict[int, list[int]] = {}
    rss: dict[int, int] = {}
    for pid, ppid, kib in rows:
        children.setdefault(ppid, []).append(pid)
        rss[pid] = kib
    total = 0
    seen: set[int] = set()
    stack = [root]
    while stack:
        pid = stack.pop()
        if pid in seen:
            continue
        seen.add(pid)
        total += rss.get(pid, 0)
        stack.extend(children.get(pid, []))
    return total


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--pid", type=int, required=True)
    parser.add_argument("--duration", type=float, required=True)
    parser.add_argument("--interval", type=float, default=0.1)
    parser.add_argument("--out", required=True)
    args = parser.parse_args()

    samples: list[int] = []
    deadline = time.monotonic() + args.duration
    while time.monotonic() < deadline:
        try:
            rows = snapshot()
        except (OSError, subprocess.SubprocessError):
            break
        value = tree_rss(rows, args.pid)
        if value <= 0:
            break
        samples.append(value)
        time.sleep(args.interval)

    if not samples:
        print("RSS_SAMPLER_FAIL=no samples", file=sys.stderr)
        return 1

    result = {
        "samples": len(samples),
        "rss_kib_min": min(samples),
        "rss_kib_median": int(statistics.median(samples)),
        "rss_kib_peak": max(samples),
    }
    with open(args.out, "w", encoding="utf-8") as handle:
        json.dump(result, handle, indent=2, sort_keys=True)
        handle.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

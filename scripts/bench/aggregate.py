#!/usr/bin/env python3
"""Aggregate Q-12 raw oha/RSS repetitions into medians with 95% intervals.

Every raw artifact is validated first: one JSON object from oha with a 1.0
success rate, only 200 responses and no transport errors, plus one RSS sample
file for the same repetition. A missing, duplicated or invalid repetition is a
hard failure; it is never reported as zero.
"""
from __future__ import annotations

import argparse
import json
import math
import random
import statistics
import sys
from pathlib import Path
from typing import Any

BOOTSTRAP_ROUNDS = 10_000
BOOTSTRAP_SEED = 20261108


class AggregateError(ValueError):
    pass


def load_object(path: Path) -> dict[str, Any]:
    if path.is_symlink() or not path.is_file():
        raise AggregateError(f"{path.name}: missing or non-regular artifact")
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise AggregateError(f"{path.name}: malformed JSON ({error})") from error
    if not isinstance(value, dict) or not value:
        raise AggregateError(f"{path.name}: empty/non-object JSON")
    return value


def positive(value: Any, label: str) -> float:
    try:
        number = float(value)
    except (TypeError, ValueError) as error:
        raise AggregateError(f"{label}: missing or malformed number") from error
    if not math.isfinite(number) or number <= 0:
        raise AggregateError(f"{label}: value must be finite and positive")
    return number


def validate_oha(data: dict[str, Any], label: str) -> tuple[float, float, float, float]:
    summary = data.get("summary")
    latency = data.get("latencyPercentiles")
    status = data.get("statusCodeDistribution")
    errors = data.get("errorDistribution")
    if not isinstance(summary, dict) or not isinstance(latency, dict):
        raise AggregateError(f"{label}: summary/percentiles missing")
    if summary.get("successRate") != 1.0:
        raise AggregateError(f"{label}: successRate is not 1.0")
    if not isinstance(status, dict) or not status or any(key != "200" for key in status):
        raise AggregateError(f"{label}: non-200 response in status distribution")
    if isinstance(errors, dict):
        for count in errors.values():
            try:
                if int(count) != 0:
                    raise AggregateError(f"{label}: transport/application errors present")
            except (TypeError, ValueError) as error:
                raise AggregateError(f"{label}: malformed error counter") from error
    elif errors not in (None, {}):
        raise AggregateError(f"{label}: malformed error distribution")
    return (
        positive(summary.get("requestsPerSec"), f"{label} requestsPerSec"),
        positive(latency.get("p50"), f"{label} p50") * 1000.0,
        positive(latency.get("p95"), f"{label} p95") * 1000.0,
        positive(latency.get("p99"), f"{label} p99") * 1000.0,
    )


def interval(values: list[float], rng: random.Random) -> dict[str, Any]:
    if len(values) < 2:
        raise AggregateError("at least two repetitions are required for an interval")
    ordered = sorted(values)
    medians: list[float] = []
    for _ in range(BOOTSTRAP_ROUNDS):
        sample = [ordered[rng.randrange(len(ordered))] for _ in ordered]
        medians.append(statistics.median(sample))
    medians.sort()
    low = medians[int(0.025 * len(medians))]
    high = medians[int(0.975 * len(medians)) - 1]
    return {
        "median": statistics.median(values),
        "ci95": [low, high],
        "values": values,
    }


def discover(raw: Path, payload: str, server: str, repetitions: int) -> list[tuple[Path, Path]]:
    cells: list[tuple[Path, Path]] = []
    for index in range(1, repetitions + 1):
        metric = raw / f"{payload}__{server}__r{index}.json"
        resource = raw / f"{payload}__{server}__r{index}.rss.json"
        if not metric.is_file():
            raise AggregateError(f"missing repetition {metric.name}")
        if not resource.is_file():
            raise AggregateError(f"missing repetition {resource.name}")
        cells.append((metric, resource))
    expected_names = {metric.name for metric, _ in cells}
    extras = [
        path
        for path in raw.glob(f"{payload}__{server}__r*.json")
        if not path.name.endswith(".rss.json") and path.name not in expected_names
    ]
    if extras:
        raise AggregateError(f"stale repetition artifact: {extras[0].name}")
    return cells


def format_ci(cell: dict[str, Any], digits: int) -> str:
    low, high = cell["ci95"]
    return f"{cell['median']:.{digits}f} [{low:.{digits}f}, {high:.{digits}f}]"


def build_report(raw: Path, out: Path, meta: dict[str, Any]) -> dict[str, Any]:
    repetitions = int(meta["repetitions"])
    if repetitions < 2:
        raise AggregateError("repetitions must be >= 2")
    rng = random.Random(BOOTSTRAP_SEED)
    results: dict[str, Any] = {}
    for payload in meta["payloads"]:
        results[payload] = {}
        for server in meta["servers"]:
            cells = discover(raw, payload, server, repetitions)
            rps: list[float] = []
            p50: list[float] = []
            p95: list[float] = []
            p99: list[float] = []
            rss: list[float] = []
            for metric_path, resource_path in cells:
                label = f"{payload}/{server}/{metric_path.stem}"
                metric = load_object(metric_path)
                throughput, l50, l95, l99 = validate_oha(metric, label)
                resource = load_object(resource_path)
                rps.append(throughput)
                p50.append(l50)
                p95.append(l95)
                p99.append(l99)
                rss.append(positive(resource.get("rss_kib_peak"), f"{label} rss") / 1024.0)
            results[payload][server] = {
                "repetitions": repetitions,
                "rps": interval(rps, rng),
                "p50_ms": interval(p50, rng),
                "p95_ms": interval(p95, rng),
                "p99_ms": interval(p99, rng),
                "rss_peak_mib": interval(rss, rng),
            }
    return {
        "schema": 1,
        "measurement_class": meta.get("measurement_class", "local_native_non_canonical"),
        "meta": meta,
        "results": results,
    }


def write_markdown(report: dict[str, Any], path: Path) -> None:
    meta = report["meta"]
    hardware = meta.get("hardware", {})
    software = meta.get("software", {})
    servers = meta.get("servers", [])
    measurement = report.get(
        "measurement_class", meta.get("measurement_class", "local_native_non_canonical")
    )
    lines = [
        "# Q-12 HTTP/1.1 benchmark — native run",
        "",
        f"> `measurement_class = {measurement}`. These numbers are real executions",
        "> on the declared host, but they are **not** a competitive claim and their",
        "> absolute values are not portable to another host.",
        "",
        "## Environment",
        "",
        f"- Host: {hardware.get('cpu', 'unknown')}, "
        f"{hardware.get('logical_cpus', 0)} logical CPUs, "
        f"{hardware.get('memory_bytes', 0) // (1024**3)} GiB RAM",
        f"- OS: {hardware.get('os', 'unknown')} ({hardware.get('arch', 'unknown')})",
    ]
    if software.get("exyonq"):
        lines.append(
            f"- ExyonQ: `{software['exyonq']}` (SHA-256 `{meta.get('exyonq_sha256', '')}`)"
        )
    if software.get("nginx"):
        lines.append(f"- nginx: `{software['nginx']}`")
    if software.get("httpd"):
        lines.append(f"- Apache: `{software['httpd']}`")
    if software.get("ols"):
        lines.append(f"- OpenLiteSpeed: `{software['ols']}`")
    if software.get("oha"):
        lines.append(f"- Load generator: `{software['oha']}`")
    lines += [
        f"- Repetitions: {meta['repetitions']} × {meta['duration']}, "
        f"{meta['connections']} connections",
        f"- Source: `{meta.get('git_head', '')}` (working tree dirty: "
        f"{'yes' if meta.get('git_status_porcelain') else 'no'})",
        "",
        "## Results",
        "",
        "Values are median over repetitions with a 95% bootstrap interval of the",
        "median (10,000 resamples). RSS is the peak resident set of the server",
        "process tree sampled every 100 ms.",
        "",
    ]
    for payload, servers_for_payload in report["results"].items():
        lines += [
            f"### `{payload}`",
            "",
            "| Server | Requests/s | p50 ms | p95 ms | p99 ms | RSS peak MiB |",
            "| --- | ---: | ---: | ---: | ---: | ---: |",
        ]
        for server, cell in servers_for_payload.items():
            lines.append(
                f"| {server} | {format_ci(cell['rps'], 1)} | {format_ci(cell['p50_ms'], 2)} | "
                f"{format_ci(cell['p95_ms'], 2)} | {format_ci(cell['p99_ms'], 2)} | "
                f"{format_ci(cell['rss_peak_mib'], 1)} |"
            )
        lines.append("")
    lines += [
        "## Method",
        "",
        "- All servers run natively on the same host and kernel, read the same",
        "  payload files from one directory, use HTTP/1.1 keep-alive and disable",
        "  access logging.",
        "- Warm-up: 2 s per server/payload before the measured repetitions.",
        f"- Each repetition starts a fresh {meta['duration']} closed-loop `oha` run; server order",
        "  rotates per repetition to spread drift. `oha` 1.14 has no thread",
        "  selector: concurrency is the declared connection count.",
        "- Every response is validated by `oha` (status 200, success rate 1.0, no",
        "  errors) and, before measurement, by an exact SHA-256 body oracle per",
        "  server/payload.",
        "",
        "## Limitations",
        "",
        f"- Native {hardware.get('os', 'unknown')} ({hardware.get('arch', 'unknown')}); "
        "absolute numbers are not portable.",
    ]
    if "ols" in servers:
        lines += [
            "- OpenLiteSpeed is an already-running native process on a loopback",
            "  URL. The harness rejects non-loopback URLs, but the operator must",
            "  ensure it is a native process: a container, VM or remote endpoint",
            "  is not comparable.",
        ]
    else:
        lines += [
            "- OpenLiteSpeed was not selected for this run. Set",
            "  `Q12_SERVERS=...,ols` with a native OLS process if it is installed.",
        ]
    if meta.get("ci"):
        lines += [
            "- This report was produced inside CI on a shared runner; runner",
            "  noise is part of the result and the tolerances must reflect it.",
        ]
    else:
        lines += [
            "- This report was produced outside CI; it does not establish CI",
            "  execution or a CI baseline.",
        ]
    lines += [
        "- Host noise is real: the 95% intervals of different servers can",
        "  overlap. The report is an engineering baseline, not a ranking.",
        "",
    ]
    path.write_text("\n".join(lines), encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--raw", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--meta", required=True, type=Path)
    args = parser.parse_args()
    try:
        meta = load_object(args.meta)
        report = build_report(args.raw, args.out, meta)
        (args.out / "report.json").write_text(
            json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )
        write_markdown(report, args.out / "report.md")
    except (AggregateError, OSError) as error:
        print(f"AGGREGATE_FAIL={error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

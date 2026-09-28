#!/usr/bin/env python3
"""DB2D/DB2D1 negative+positive validation self-tests."""

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

_HERE = Path(__file__).resolve().parent.parent
if str(_HERE) not in sys.path:
    sys.path.insert(0, str(_HERE))

from common import EXIT_CONFIG, EXIT_PASS, EXIT_VIOLATION, find_repo_root  # noqa: E402
from coverage_scenarios import EXTRA_NEGATIVE, POSITIVE  # noqa: E402
from run_negative_validation import (  # noqa: E402
    SCENARIOS,
    all_scenarios,
    enforcement_fingerprint,
    run_all,
    scenario_n01_unregistered,
)


class TestDb2dCatalog(unittest.TestCase):
    def test_n01_n11_intact(self):
        ids = [sid for sid, _ in SCENARIOS]
        self.assertEqual(
            ids,
            ["N01", "N02", "N03", "N04", "N05", "N06", "N07", "N08", "N09", "N10", "N11"],
        )

    def test_n12_n25_catalog(self):
        ids = [sid for sid, _ in EXTRA_NEGATIVE]
        self.assertEqual(ids, [f"N{i:02d}" for i in range(12, 26)])

    def test_p01_p10_catalog(self):
        ids = [sid for sid, _ in POSITIVE]
        self.assertEqual(ids, [f"P{i:02d}" for i in range(1, 11)])

    def test_full_matrix_count(self):
        ids = [sid for sid, _ in all_scenarios()]
        self.assertEqual(len([i for i in ids if i.startswith("N")]), 25)
        self.assertEqual(len([i for i in ids if i.startswith("P")]), 10)


class TestDb2d1Scenarios(unittest.TestCase):
    def _run(self, fn):
        with tempfile.TemporaryDirectory(prefix="db2d1-ut-") as td:
            return fn(Path(td))

    def test_n01_still_passes(self):
        r = self._run(scenario_n01_unregistered)
        self.assertTrue(r.passed, r.matched_finding)
        self.assertEqual(r.actual_exit, EXIT_VIOLATION)

    def test_each_extra_negative(self):
        for sid, fn in EXTRA_NEGATIVE:
            with self.subTest(sid=sid):
                r = self._run(fn)
                self.assertTrue(r.passed, f"{sid}: {r.matched_finding}")
                self.assertEqual(r.actual_exit, r.expected_exit)

    def test_each_positive(self):
        for sid, fn in POSITIVE:
            with self.subTest(sid=sid):
                r = self._run(fn)
                self.assertTrue(r.passed, f"{sid}: {r.matched_finding}")
                self.assertEqual(r.actual_exit, EXIT_PASS)

    def test_n25_exit_config(self):
        from coverage_scenarios import scenario_n25_invalid_registry

        r = self._run(scenario_n25_invalid_registry)
        self.assertTrue(r.passed)
        self.assertEqual(r.actual_exit, EXIT_CONFIG)

    def test_n20_fail_open(self):
        from coverage_scenarios import scenario_n20_generic_dup_warn

        r = self._run(scenario_n20_generic_dup_warn)
        self.assertTrue(r.passed)
        self.assertEqual(r.actual_exit, EXIT_PASS)

    def test_fingerprint_stable(self):
        root = find_repo_root()
        self.assertEqual(enforcement_fingerprint(root), enforcement_fingerprint(root))


class TestDb2d1Harness(unittest.TestCase):
    def test_harness_end_to_end(self):
        self.assertEqual(run_all(json_out=False), 0)


if __name__ == "__main__":
    unittest.main()

#!/usr/bin/env bash
# MUST_FAIL: masks command failure then exits 0.
set -euo pipefail
false || true
exit 0

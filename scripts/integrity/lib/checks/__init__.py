"""Integrity check modules."""
from __future__ import annotations

from .boundary import run as check_boundary
from .placeholder import run as check_placeholder
from .dead_path import run as check_dead_path
from .config_effect import run as check_config_effect
from .test_integrity import run as check_test_integrity
from .fake_success import run as check_fake_success
from .error_propagation import run as check_error_propagation
from .claims import run as check_claims
from .docs_vs_code import run as check_docs_vs_code
from .security_bypass import run as check_security_bypass
from .bench_path import run as check_bench_path
from .dependency_reality import run as check_dependency_reality
from .zero_fake import run as check_zero_fake

QUICK_CHECKS = (
    check_zero_fake,
    check_boundary,
    check_placeholder,
    check_fake_success,
    check_security_bypass,
    check_bench_path,
    check_test_integrity,
)

FULL_CHECKS = (
    check_zero_fake,
    check_boundary,
    check_placeholder,
    check_dead_path,
    check_config_effect,
    check_test_integrity,
    check_fake_success,
    check_error_propagation,
    check_claims,
    check_docs_vs_code,
    check_security_bypass,
    check_bench_path,
    check_dependency_reality,
)

__all__ = ["QUICK_CHECKS", "FULL_CHECKS"]

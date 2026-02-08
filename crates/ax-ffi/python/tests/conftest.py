"""Pytest configuration for AX tests."""

import pytest
import sys


def pytest_configure(config):
    """Configure pytest."""
    # Add markers for test categorization
    config.addinivalue_line("markers", "slow: mark test as slow running")
    config.addinivalue_line("markers", "integration: mark as integration test")


def pytest_collection_modifyitems(config, items):
    """Skip tests if ax module is not available."""
    try:
        import ax  # noqa: F401
    except ImportError:
        skip_marker = pytest.mark.skip(reason="ax module not built")
        for item in items:
            item.add_marker(skip_marker)

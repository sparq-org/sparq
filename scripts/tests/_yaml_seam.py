"""Shared PyYAML import for the workflow-seam suites (#5820).

The workflow-wiring tests parse `.github/workflows/*.yml` with PyYAML. Without
it, a module-scope `import yaml` made the WHOLE file error on a bare local
checkout, so an author could not tell a missing dependency from a regression in
their own diff.

The skip is LOCAL-ONLY: under CI (`CI` or `GITHUB_ACTIONS` set, as GitHub
Actions always does) a missing PyYAML is still a hard ImportError, so dropping
the workflow's `Install PyYAML` step can never make these gates silently vanish.
"""
from __future__ import annotations

import os
import sys
import unittest

_MSG = (
    "PyYAML is not installed — the workflow-seam tests in this file were SKIPPED "
    "(local run only; CI treats this as a hard error). Install it with "
    "`python3 -m pip install pyyaml` to run them."
)


def running_in_ci(env: dict[str, str] | None = None) -> bool:
    env = os.environ if env is None else env
    return bool(env.get("CI") or env.get("GITHUB_ACTIONS"))


def yaml_or_local_skip():
    """Return the `yaml` module, or skip the calling test module (locally only)."""
    try:
        import yaml  # noqa: PLC0415
    except ImportError:
        if running_in_ci():
            raise
        caller = sys._getframe(1).f_globals.get("__name__")
        if caller == "__main__":
            # Run directly (`python3 scripts/tests/test_x.py`): report and exit 0.
            print(f"SKIPPED: {_MSG}", file=sys.stderr)
            sys.exit(0)
        # Under unittest discovery or pytest, SkipTest raised at import time is
        # reported as a skipped module rather than an error.
        raise unittest.SkipTest(_MSG) from None
    return yaml

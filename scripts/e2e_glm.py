#!/usr/bin/env python3
"""opencoder e2e entry point: deep business-contract verification vs real glm5.2.

Replaces the former bash-only scripts/e2e-glm.sh (kept as a thin wrapper).
Run:  scripts/e2e-glm.sh [binary]        # or: python3 scripts/e2e_glm.py [binary]

Each scenario asserts an actual business contract (fork copy integrity, bundle
roundtrip, resume context-load, compaction content-awareness, subagent DB
tracking, plan read-only, web steer+queue delivery) rather than a surface
marker. See scripts/e2e/{lib,cli_scenarios,web_scenarios}.py for per-contract
rationale. Stdlib only; no third-party Python dependency.
"""

from __future__ import annotations

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from e2e import cli_scenarios, web_scenarios  # noqa: E402
from e2e.lib import Counter, ensure_auth, resolve_bin  # noqa: E402


def main() -> int:
    import argparse

    ap = argparse.ArgumentParser(description="opencoder deep e2e vs real glm5.2")
    ap.add_argument("binary", nargs="?", default=None, help="path to the opencoder binary")
    ap.add_argument("--skip-web", action="store_true", help="skip the E11 serve/HTTP scenario")
    ap.add_argument("--only", choices=("cli", "web"), help="run only one suite")
    args = ap.parse_args()

    bin_path = resolve_bin(args.binary)
    if not os.path.isfile(bin_path):
        print(f"FAIL: binary not found: {bin_path} (build first or pass a path)", file=sys.stderr)
        return 2
    api_key = ensure_auth()

    total = Counter()
    if args.only != "web":
        total += cli_scenarios.run_all(bin_path, api_key)
    if args.only != "cli" and not args.skip_web:
        total += web_scenarios.run_all(bin_path, api_key)

    print("\n" + "=" * 60)
    print(f"e2e result: {total.passed} passed, {total.failed} failed, {total.skipped} skipped")
    print("=" * 60)
    return 1 if total.failed else 0


if __name__ == "__main__":
    sys.exit(main())

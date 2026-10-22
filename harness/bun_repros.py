#!/usr/bin/env python3
"""Run the small nightly fixtures for Bun's whole-program blockers.

Each fixture is built by the pinned nightly, then linked again with ra-jit. The
fixture's output must match cargo's, and its report must contain the refusal the
fixture was added to measure. Change ``expect`` below to ``compiled`` when the
corresponding frontend or MIR support lands.
"""
import argparse
import json
import os
import re
import shutil
import sys
import tempfile


HERE = os.path.dirname(os.path.abspath(__file__))
NIGHTLY = "nightly-2026-09-15"
sys.path.insert(0, HERE)

# difftest.py reads RUSTUP_TOOLCHAIN when it is imported. Keep this assignment
# before importing it so cargo, rustc and ra-jit inspect the same nightly tree.
os.environ["RUSTUP_TOOLCHAIN"] = NIGHTLY
os.environ["RA_JIT_OTHER_TOOLCHAIN"] = "1"
from difftest import Fixture, RA_JIT  # noqa: E402


CASES = {
    "adt_const_params": {
        "package": "runner",
        "crate": "adt_const_params",
        "expected": "compiled",
        "reasons": (),
    },
    "type_info": {
        "package": "runner",
        "crate": "type_info",
        "expected": "compiled",
        "reasons": (),
    },
    "inherent_associated_types": {
        "package": "runner",
        "crate": "inherent_associated_types",
        "expected": "compiled",
        "reasons": (),
    },
}


def check_toolchain():
    """Fail with the installation command instead of silently using stable rustc."""
    rustup = shutil.which("rustup")
    if rustup is None:
        raise SystemExit(f"bun_repros.py needs rustup with {NIGHTLY} installed")
    probe = os.spawnvp(os.P_WAIT, rustup, [rustup, "run", NIGHTLY, "rustc", "-V"])
    if probe:
        raise SystemExit(f"bun_repros.py needs {NIGHTLY}; install it with `rustup toolchain install {NIGHTLY}`")


def rows_for(fx):
    return [
        row
        for rows in fx.reports.values()
        for row in rows
        if row[0] == "compiled" or (row[0] == "skipped" and "no rustc symbol for function" not in row[2])
    ]


def run_case(name, root, json_path=None):
    case = CASES[name]
    workspace = os.path.join(root, name)
    target = tempfile.mkdtemp(prefix=f"ra-jit-bun-{name}-")
    os.environ["CARGO_TARGET_DIR"] = target
    fx = Fixture(workspace, [case["crate"]], ["-p", case["package"]])
    fx.rustc_build()
    expected = fx.behaviour(fx.reference)
    fx.ra_jit_build()
    actual = fx.behaviour(fx.binary)
    rows = rows_for(fx)
    skipped = [row for row in rows if row[0] == "skipped"]
    compiled = len(rows) - len(skipped)
    reasons = [row[2] for row in skipped]
    matched_reason = next((reason for reason in reasons if any(pattern.search(reason) for pattern in case["reasons"])), None)
    state = "compiled" if not skipped else "refused"
    result = {
        "case": name,
        "state": state,
        "compiled": compiled,
        "refused": len(skipped),
        "reasons": reasons,
        "match": actual == expected,
        "work": fx.work,
    }
    print(f"{name}: {compiled} compiled, {len(skipped)} refused; output {'matches' if actual == expected else 'DIFFERS'}")
    for reason in sorted(set(reasons)):
        print(f"  {reason}")
    if actual != expected:
        raise SystemExit(f"{name}: ra-jit output differs from cargo's")
    if case["expected"] == "compiled" and skipped:
        raise SystemExit(f"{name}: expected every function to compile, {len(skipped)} remain refused")
    if case["expected"] == "refused" and matched_reason is None:
        raise SystemExit(f"{name}: no refusal matched the expected blocker")
    if json_path:
        with open(json_path, "w") as out:
            json.dump(result, out, indent=1)
    return result


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("cases", nargs="*", choices=sorted(CASES))
    parser.add_argument("--json-dir", help="write one JSON report per case")
    args = parser.parse_args()
    check_toolchain()
    root = os.path.join(HERE, "bun-repros")
    results = []
    for name in args.cases or sorted(CASES):
        path = os.path.join(args.json_dir, f"{name}.json") if args.json_dir else None
        results.append(run_case(name, root, path))
    print(json.dumps({"toolchain": NIGHTLY, "ra_jit": RA_JIT, "cases": results}, indent=1))


if __name__ == "__main__":
    main()

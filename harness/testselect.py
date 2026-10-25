#!/usr/bin/env python3
"""testselect.py: `cargo ra-jit test` with `RA_JIT_TESTS=affected` runs the tests an edit
reached since they last passed, and no others.

usage: testselect.py    (RA_JIT_BIN names the ra-jit built; cargo-ra-jit is found next to it)

A package has a library with unit tests and an integration test of it: tests that call one
function or another of the library, one that downcasts a `dyn Any` of what one returns (a
`TypeId`, which code generation takes from rustc), one that keeps it in a `HashMap` (seeded at
random), one that prints a backtrace (which reads the environment and the working directory),
one that reads a file, one that calls nothing. It is tested until the daemon serves the build,
then edited a step at a time, and after each the tests that ran have to be the ones the step
names: those whose code or what it calls changed, those added or given other attributes, the
one that reads a file every time, the one that failed until it passes (and not the others that
ran and passed beside it), and with
`RA_JIT_TESTS=all` every test. A test that ran has to end as its source says. With `-v` the
output says why the one that reads a file runs, by the calls from it to the file system, and
how many tests run every time.

Python 3, stdlib only.
"""
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
os.environ.setdefault("RUSTUP_TOOLCHAIN", open(os.path.join(HERE, "toolchain")).read().strip())
TIMEOUT = 900
LIB = """pub fn helper(x: u32) -> u32 {
    x + 1
}

pub fn other(x: u32) -> u32 {
    x * 2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_helper() {
        assert_eq!(helper(1), 2);
    }

    #[test]
    fn uses_other() {
        assert_eq!(other(2), 4);
    }

    #[test]
    fn uses_any() {
        let value: Box<dyn std::any::Any> = Box::new(helper(1));
        assert_eq!(value.downcast_ref::<u32>(), Some(&2));
    }

    #[test]
    fn uses_a_map() {
        let mut map = std::collections::HashMap::new();
        map.insert(helper(1), "two");
        assert_eq!(map.get(&2), Some(&"two"));
    }

    #[test]
    fn shows_a_backtrace() {
        let shown = std::backtrace::Backtrace::capture().to_string();
        assert_eq!(helper(shown.len().min(0) as u32), 1);
    }

    #[test]
    fn reads_a_file() {
        assert!(std::fs::read_to_string("Cargo.toml").unwrap().contains("tsel"));
    }

    #[test]
    fn plain() {
        assert_eq!(1 + 1, 2);
    }
}
"""
FILES = {
    "Cargo.toml": '[package]\nname = "tsel"\nversion = "0.1.0"\nedition = "2021"\n',
    "src/lib.rs": LIB,
    "tests/it.rs": """#[test]
fn it_helper() {
    assert_eq!(tsel::helper(0), 1);
}

#[test]
fn it_other() {
    assert_eq!(tsel::other(0), 0);
}
""",
}
CALL_HELPER = {"tests::uses_helper", "tests::uses_any", "tests::uses_a_map", "tests::shows_a_backtrace", "it_helper"}
ALL = CALL_HELPER | {"tests::uses_other", "tests::reads_a_file", "tests::plain", "it_helper", "it_other"}
FILE = "tests::reads_a_file"
# (what the step is, [(file, old, new)], RA_JIT_TESTS, the tests that run, whether they pass)
STEPS = [
    ("nothing edited", [], "affected", {FILE}, True),
    ("a function one test calls", [("src/lib.rs", "x + 1", "x + 2 - 1")], "affected",
     CALL_HELPER | {FILE}, True),
    ("a function another test calls", [("src/lib.rs", "x * 2", "x + x")], "affected",
     {"tests::uses_other", "it_other", FILE}, True),
    ("a test added", [("src/lib.rs", "    #[test]\n    fn plain()",
                       "    #[test]\n    fn added() {\n        assert_eq!(helper(2), 3);\n    }\n\n"
                       "    #[test]\n    fn plain()")], "affected",
     {"tests::added", FILE}, True),
    ("a test ignored", [("src/lib.rs", "    #[test]\n    fn plain()", "    #[test]\n    #[ignore]\n    fn plain()")],
     "affected", {"tests::plain", FILE}, True),
    ("a test that fails", [("src/lib.rs", "other(2), 4", "other(2), 5")], "affected",
     {"tests::uses_other", FILE}, False),
    ("nothing edited after it failed", [], "affected", {"tests::uses_other", FILE}, False),
    ("the test fixed", [("src/lib.rs", "other(2), 5", "other(2), 4")], "affected",
     {"tests::uses_other", FILE}, True),
    # The unit tests fail, so cargo runs no integration test after them.
    ("one of several reached failing", [("src/lib.rs", "x + 2 - 1", "x + 3 - 2"),
                                         ("src/lib.rs", "assert_eq!(helper(1), 2);", "assert_eq!(helper(1), 3);")],
     "affected", CALL_HELPER - {"it_helper"} | {"tests::added", FILE}, False),
    ("nothing edited after one failed", [], "affected", {"tests::uses_helper", FILE}, False),
    ("that one fixed", [("src/lib.rs", "assert_eq!(helper(1), 3);", "assert_eq!(helper(1), 2);")],
     "affected", {"tests::uses_helper", "it_helper", FILE}, True),
    ("every test asked for", [], "all", ALL | {"tests::added"}, True),
    ("nothing edited again", [], "affected", {FILE}, True),
    ("why a test runs every time", [], "affected -v", {FILE}, True),
]
# What a step's output has to say, beside the tests it runs.
SAYS = {
    "why a test runs every time": [
        r"Always tests::reads_a_file: it reaches `\w+`, which reads or writes files, by "
        r"tsel::tests::reads_a_file -> std::fs::read_to_string::<…> -> .* -> \w+$",
        r"Skipped \d+ tests, unchanged since the last pass; 1 test run every time \(said above\);",
    ],
}
RAN = re.compile(r"^test (\S+) \.\.\. (ok|ignored|FAILED)", re.M)


def test(ws, env, mode):
    mode, *args = mode.split()
    ran = subprocess.run(["cargo", "ra-jit", "test", *args], cwd=ws, env=dict(env, RA_JIT_TESTS=mode),
                         capture_output=True, text=True, timeout=TIMEOUT)
    return ran.returncode, ran.stdout + ran.stderr


def main():
    bin_dir = os.path.dirname(os.path.abspath(os.environ["RA_JIT_BIN"]))
    if not os.path.exists(os.path.join(bin_dir, "cargo-ra-jit")):
        raise SystemExit(f"no cargo-ra-jit beside {os.environ['RA_JIT_BIN']}")
    root = tempfile.mkdtemp(prefix="ra-jit-testselect-")
    # The daemon's sockets are under its home, and a socket's path is short (104 bytes on macOS).
    ws, home = os.path.join(root, "tsel"), tempfile.mkdtemp(prefix="rj-ts-", dir="/tmp")
    for path, text in FILES.items():
        os.makedirs(os.path.dirname(os.path.join(ws, path)), exist_ok=True)
        open(os.path.join(ws, path), "w").write(text)
    env = dict(os.environ, PATH=f"{bin_dir}{os.pathsep}{os.environ['PATH']}", RA_JIT_HOME=home,
               RUST_BACKTRACE="0")
    for name in ("CARGO_TARGET_DIR", "CARGO_INCREMENTAL", "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "RA_JIT_TESTS"):
        env.pop(name, None)
    wrong = 0
    try:
        deadline = time.monotonic() + 300
        while time.monotonic() < deadline:
            code, said = test(ws, env, "affected")
            if code != 0:
                raise SystemExit(f"the tests failed before any edit:\n{said[-3000:]}")
            if "[ra-jit]" in said:
                break
            time.sleep(0.5)
        else:
            raise SystemExit(f"no build said `[ra-jit]`; the daemon's log is in {home}")
        # The daemon's first build of the tests: each is new to it, and runs.
        test(ws, env, "affected")
        for what, edits, mode, expected, passes in STEPS:
            for path, old, new in edits:
                text = open(os.path.join(ws, path)).read()
                assert old in text, (what, old)
                open(os.path.join(ws, path), "w").write(text.replace(old, new, 1))
            code, said = test(ws, env, mode)
            ran = {name for name, _ in RAN.findall(said)}
            failed = {name for name, end in RAN.findall(said) if end == "FAILED"}
            verdict = "ok"
            if "[ra-jit]" not in said:
                verdict = "NOT SERVED BY THE DAEMON"
            elif ran != expected:
                extra, missing = sorted(ran - expected), sorted(expected - ran)
                verdict = f"RAN {', '.join(extra) or 'nothing more'}; DID NOT RUN {', '.join(missing) or '-'}"
            elif (code == 0) != passes or (failed == set()) != passes:
                verdict = f"ENDED {code} WITH {', '.join(sorted(failed)) or 'no test'} FAILED"
            elif unsaid := [it for it in SAYS.get(what, []) if not re.search(it, said, re.M)]:
                verdict = f"DID NOT SAY {' NOR '.join(unsaid)}"
            wrong += verdict != "ok"
            print(f"{what:34} {verdict}", flush=True)
            if verdict != "ok" and os.environ.get("TESTSELECT_VERBOSE"):
                print(said[-4000:])
    finally:
        subprocess.run(["cargo", "ra-jit", "stop"], cwd=ws, env=env, capture_output=True, timeout=120)
    print(f"{len(STEPS) - wrong} of {len(STEPS)} right; under {root}, the daemon's home {home}")
    if wrong:
        sys.exit(1)
    shutil.rmtree(root, ignore_errors=True)
    shutil.rmtree(home, ignore_errors=True)


if __name__ == "__main__":
    main()

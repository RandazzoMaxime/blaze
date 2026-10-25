#!/usr/bin/env python3
"""storetracked.py: a daemon that starts on a store holding the checkout's code takes that
code as it is and patches the first edit to it, with nothing of rustc's (#105). Linux and
macOS: on Windows `cargo ra-jit` hands builds to cargo.

usage: storetracked.py    (RA_JIT_BIN names the ra-jit built; cargo-ra-jit and ra-jit-rustc
                           are found next to it)

A patch is cut from what changed since the build it patches, and it knew that by marks
computed in its own process: a mark of code the store had came from the process that wrote
it, so the daemon built everything of the checkout the store gave it again before the first
edit could be patched ("compiling what the store had of the checkout"), all of nushell's
73,586 functions for a start with nothing changed. Each function of the checkout now goes
into the store with a mark of its code and of what it names by stable names, the same in any
process, checkout or build of ra-jit, so the base takes what it found as it is and an edit
recompiles only what it touched.

A daemon builds the workspace into an empty store and stops. A second one on the same
checkout, and a third on a copy of it elsewhere with a home of its own, start on that store:
each has to take the checkout's functions from it, compile none of them again, and splice
every edit after: a generic function's body, whose instance is the checkout's and was found
in the store, a closure's, a static's value, the program's own text, and all of it taken
back. The program has to print what the source says.

Python 3, stdlib only.
"""
import glob
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
LIB, APP = "lib/src/lib.rs", "app/src/main.rs"
FILES = {
    "Cargo.toml": '[workspace]\nmembers = ["app", "lib"]\nresolver = "2"\n',
    "lib/Cargo.toml": '[package]\nname = "tlib"\nversion = "0.1.0"\nedition = "2021"\n',
    LIB: """pub static BASE: u64 = 10;

pub fn scale<T: Into<u64>>(x: T) -> u64 {
    x.into() * 3
}

pub fn each<T: Copy + Into<u64>>(v: &[T]) -> u64 {
    v.iter().map(|&x| x.into() + 1).sum()
}

pub fn total(v: &[u64]) -> u64 {
    walk(v, &|x| x * 2)
}

fn walk<F: Fn(u64) -> u64>(v: &[u64], f: &F) -> u64 {
    match v {
        [] => 0,
        [x, rest @ ..] => f(*x) + walk(rest, f),
    }
}
""",
    "app/Cargo.toml": '[package]\nname = "tapp"\nversion = "0.1.0"\nedition = "2021"\n\n'
                      '[dependencies]\ntlib = { path = "../lib" }\n',
    APP: """fn main() {
    println!("{} {} {} {} {}", "a", tlib::scale(2u8), tlib::each(&[1u32, 2]), tlib::BASE, tlib::total(&[1, 2]));
}
""",
}
FIRST = "a 6 5 10 6"
# (what the step is, [(file, old, new)], what the program prints after it)
STEPS = [
    ("a generic function's body", [(LIB, "x.into() * 3", "x.into() * 4")], "a 8 5 10 6"),
    # A closure is named by where in its body it is written: this one, and the instances
    # generic over it, get new names, and the ones from the store are called no more.
    ("a line above a closure", [(LIB, "    v.iter()", "    let _ = 0u64;\n    v.iter()")], "a 8 5 10 6"),
    # The same of a recursive function's instance, which calls itself.
    ("a line above a closure a recursive function takes", [(LIB, "    walk(v", "    let _ = 1u64;\n    walk(v")],
     "a 8 5 10 6"),
    ("a closure's body", [(LIB, "x.into() + 1", "x.into() + 2")], "a 8 7 10 6"),
    ("a static's value", [(LIB, "BASE: u64 = 10", "BASE: u64 = 12")], "a 8 7 12 6"),
    ("the program's own text", [(APP, '"a"', '"b"')], "b 8 7 12 6"),
    ("all of it taken back", [(LIB, "x.into() * 4", "x.into() * 3"), (LIB, "    let _ = 0u64;\n", ""),
                              (LIB, "    let _ = 1u64;\n", ""), (LIB, "x.into() + 2", "x.into() + 1"),
                              (LIB, "BASE: u64 = 12", "BASE: u64 = 10"), (APP, '"b"', '"a"')], FIRST),
]
REBUILT = "compiling what the store had of the checkout"


def build(ws, env, step):
    ran = subprocess.run(["cargo", "ra-jit", "build"], cwd=ws, env=env, capture_output=True, text=True,
                         timeout=TIMEOUT)
    said = ran.stdout + ran.stderr
    if ran.returncode != 0:
        raise SystemExit(f"{step}: the build failed:\n{said[-3000:]}")
    return said


class Logs:
    """What the workers of a daemon about to start log. A worker that was a spare keeps the
    spare's log, and a worker of the same build as one before it writes the same file anew,
    so what the daemons before logged is moved aside."""

    def __init__(self, home):
        self.home = home
        for path in self.paths():
            os.replace(path, f"{path}.before")

    def paths(self):
        return sorted(glob.glob(os.path.join(self.home, "*", "*", "log")))

    def text(self):
        said = ""
        for path in self.paths():
            with open(path, errors="replace") as log:
                said += log.read()
        return said

    def links(self):
        return re.findall(r"^link: \S+ \((.*)\)$", self.text(), re.M)


def program(ws):
    return subprocess.run([os.path.join(ws, "target", "debug", "tapp")], capture_output=True, text=True,
                          timeout=60).stdout.strip()


def serve(ws, env, label):
    """Builds until the daemon serves the build, then waits until it has nothing left to do
    unasked."""
    build(ws, env, f"{label}: the first build")
    for _ in range(240):
        if "[ra-jit]" in build(ws, env, f"{label}: the wait for the daemon"):
            break
        time.sleep(0.5)
    else:
        raise SystemExit(f"{label}: no build said `[ra-jit]`; the daemon's log is in {env['RA_JIT_HOME']}")
    for _ in range(40):
        status = subprocess.run(["cargo", "ra-jit", "status"], cwd=ws, env=env, capture_output=True,
                                text=True, timeout=120).stdout
        if "loading" not in status and "building" not in status:
            break
        time.sleep(0.25)
    # What the daemon would do unasked after its build, it starts at once.
    time.sleep(2)


def restart(ws, env, label, steps):
    """A daemon that starts on the store: the result is how many of its checks failed."""
    logs = Logs(env["RA_JIT_HOME"])
    serve(ws, env, label)
    found = [(int(f), int(a)) for f, a in
             re.findall(r"^store: (\d+) functions found, (\d+) compiled and added$", logs.text(), re.M)]
    wrong = 0
    took = sum(f for f, _ in found)
    verdict = "ok" if took and program(ws) == FIRST else f"FOUND {took}, PRINTED {program(ws)!r}"
    wrong += verdict != "ok"
    print(f"{label + ': the start':48} {verdict:12} store {found}", flush=True)
    for what, edits, expected in steps:
        for file, old, new in edits:
            path = os.path.join(ws, file)
            text = open(path).read()
            assert old in text, (what, old)
            open(path, "w").write(text.replace(old, new, 1))
        seen = len(logs.links())
        said = build(ws, env, what)
        printed = program(ws)
        made = logs.links()[seen:]
        how = made[-1] if made else "no link"
        verdict = "ok"
        if printed != expected:
            verdict = f"PRINTED {printed!r}, NOT {expected!r}"
        elif "[ra-jit]" not in said:
            verdict = "NOT SERVED BY THE DAEMON"
        elif not how.startswith("spliced"):
            verdict = "NOT SPLICED"
        wrong += verdict != "ok"
        print(f"{label + ': ' + what:48} {verdict:12} {how[:120]}", flush=True)
    if REBUILT in logs.text():
        print(f"{label}: NOT the store's code taken as it is: the daemon said `{REBUILT}`", flush=True)
        wrong += 1
    return wrong


def main():
    bin_dir = os.path.dirname(os.path.abspath(os.environ["RA_JIT_BIN"]))
    if not glob.glob(os.path.join(bin_dir, "cargo-ra-jit*")):
        raise SystemExit(f"no cargo-ra-jit beside {os.environ['RA_JIT_BIN']}")
    # The daemon's sockets are under its home, and a socket's path has to be short: macOS's
    # temporary directory is too long a start.
    root = os.path.realpath(tempfile.mkdtemp(prefix="ra-jit-tracked-", dir="/tmp"))
    ws, copy, store = os.path.join(root, "ws"), os.path.join(root, "copy"), os.path.join(root, "store")
    for path, text in FILES.items():
        os.makedirs(os.path.dirname(os.path.join(ws, path)), exist_ok=True)
        open(os.path.join(ws, path), "w").write(text)

    def env_of(home):
        env = dict(os.environ, PATH=f"{bin_dir}{os.pathsep}{os.environ['PATH']}", RA_JIT_HOME=home,
                   RA_JIT_CACHE=store, RA_JIT_RUSTC_BASE="0", RA_JIT_STATS="1")
        for name in ("CARGO_TARGET_DIR", "CARGO_INCREMENTAL", "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS"):
            env.pop(name, None)
        return env

    here, there = env_of(os.path.join(root, "h")), env_of(os.path.join(root, "h2"))
    wrong = 0
    try:
        serve(ws, here, "filling the store")
        subprocess.run(["cargo", "ra-jit", "stop"], cwd=ws, env=here, capture_output=True, timeout=120)
        shutil.copytree(ws, copy, ignore=shutil.ignore_patterns("target"))
        wrong += restart(ws, here, "restart", STEPS)
        subprocess.run(["cargo", "ra-jit", "stop"], cwd=ws, env=here, capture_output=True, timeout=120)
        wrong += restart(copy, there, "another checkout", STEPS[:1])
    finally:
        for at, env in ((ws, here), (copy, there)):
            if os.path.isdir(at):
                subprocess.run(["cargo", "ra-jit", "stop"], cwd=at, env=env, capture_output=True, timeout=120)
    print(f"{wrong} wrong; under {root}")
    if wrong:
        sys.exit(1)
    shutil.rmtree(root, ignore_errors=True)


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""latelisted.py: what a file an `include!` names declares is listed in its crate, though the
daemon listed the crate beside cargo's build before the file was loaded.

usage: latelisted.py    (RA_JIT_BIN names the ra-jit built; cargo-ra-jit is found next to it)

bun's crates `include!` the code its generators write, from a directory a build script names in
a variable (`BUN_CODEGEN_DIR`). The VFS has none of those files when the daemon, told of the
build ahead of cargo, lists a crate cargo has built while cargo still compiles the rest: the
files are read once the names are resolved. The listing was kept, so the functions they declare
were missing from the program's entries until an edit had the crate listed again, and that
edit's patch then compiled every one of them as a function added. Here a library includes
`exports.in` (not a `.rs` file, so nothing loads it but the macro) with EXPORTS functions of the
C ABI, and the binary's build script keeps cargo busy after it. Once the daemon serves the build,
a function is added to the library: its patch has to be of a few functions, not of the
EXPORTS, and the program has to print what cargo's does.

Python 3, stdlib only.
"""
import glob, os, re, shutil, subprocess, sys, tempfile, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import cargo_ra_jit
from cargo_ra_jit import Side, wait_warm
from spare import FIRST_MAIN, FIRST_TOML, settled

EXPORTS = 64
# How long the binary's build script keeps cargo busy once the library is built.
WAIT_SECS = 10
FILES = {
    "Cargo.toml": '[workspace]\nmembers = ["app", "gen"]\nresolver = "2"\n',
    "gen/Cargo.toml": '[package]\nname = "gen"\nversion = "0.1.0"\nedition = "2021"\nbuild = "build.rs"\n',
    "gen/build.rs": 'fn main() {\n'
                    '    let dir = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("generated");\n'
                    '    println!("cargo:rerun-if-changed=generated/exports.in");\n'
                    '    println!("cargo:rustc-env=GEN_DIR={}", dir.display());\n'
                    '}\n',
    "gen/generated/exports.in": "".join(f'#[no_mangle]\npub extern "C" fn late_export_{i}() -> u32 {{\n    {i}\n}}\n\n'
                                        for i in range(EXPORTS)),
    "gen/src/lib.rs": 'include!(concat!(env!("GEN_DIR"), "/exports.in"));\n\n'
                      'pub fn value() -> u32 {\n    1\n}\n',
    "app/Cargo.toml": '[package]\nname = "app"\nversion = "0.1.0"\nedition = "2021"\nbuild = "build.rs"\n\n'
                      '[dependencies]\ngen = { path = "../gen" }\n',
    "app/build.rs": f'fn main() {{\n    println!("cargo:rerun-if-changed=build.rs");\n'
                    f'    std::thread::sleep(std::time::Duration::from_secs({WAIT_SECS}));\n}}\n',
    "app/src/main.rs": 'fn main() {\n    println!("{} {}", gen::value(), gen::late_export_3());\n}\n',
}


def patches(home):
    """The `patch: N functions` lines of the workers' logs, in order."""
    found = []
    for log in sorted(glob.glob(os.path.join(home, "*", "*", "log"))):
        found += [int(n) for n in re.findall(r"^patch: (\d+) functions$", open(log, errors="replace").read(), re.M)]
    return found


def worker_log(home):
    """The log of the worker that loaded the workspace beside cargo's build, if one did."""
    for log in glob.glob(os.path.join(home, "*", "*", "log")):
        if re.search(r"^compiled ahead: ", open(log, errors="replace").read(), re.M):
            return log
    return None


def compiled_for_edits(log):
    """Waits until the worker has compiled the program it patches edits of, which it does
    while no build asks: what it lists then is what an edit's patch is told apart from."""
    began = time.time()
    while time.time() - began < 300:
        text = open(log, errors="replace").read()
        at = text.find("compiling the program for edits to patch")
        if at >= 0 and "\nedit-to-binary: " in text[at:]:
            return True
        time.sleep(0.25)
    return False


def build(side, step):
    _, ran = side.cargo("build")
    if ran.returncode != 0:
        raise SystemExit(f"{step}: the build failed:\n{(ran.stdout + ran.stderr)[-3000:]}")
    return ran.stdout + ran.stderr


def main():
    bin_dir = os.path.dirname(os.path.abspath(os.environ["RA_JIT_BIN"]))
    if not os.path.exists(os.path.join(bin_dir, "cargo-ra-jit")):
        raise SystemExit(f"no cargo-ra-jit beside {os.environ['RA_JIT_BIN']}")
    # Short, for the daemon's sockets under its home.
    root = os.path.realpath(tempfile.mkdtemp(prefix="ra-jit-ll-", dir="/tmp"))
    ws, first_ws, home = (os.path.join(root, n) for n in ("ws", "first", "home"))
    for path, text in FILES.items():
        os.makedirs(os.path.dirname(os.path.join(ws, path)), exist_ok=True)
        open(os.path.join(ws, path), "w").write(text)
    os.makedirs(os.path.join(first_ws, "src"))
    open(os.path.join(first_ws, "Cargo.toml"), "w").write(FIRST_TOML)
    open(os.path.join(first_ws, "src/main.rs"), "w").write(FIRST_MAIN)
    env = cargo_ra_jit.base_env(home)
    env["PATH"] = bin_dir + os.pathsep + env["PATH"]
    for name in ("RUSTC_WRAPPER", "CARGO_INCREMENTAL", "RA_JIT_SPARE"):
        env.pop(name, None)
    first, side = Side(first_ws, env, True), Side(ws, env, True)
    reference = Side(ws, env, False)
    program = os.path.join(ws, "target", "debug", "app")
    wrong = []
    try:
        # A daemon with a spare already up is told of the next build ahead of cargo.
        build(first, "first")
        wait_warm(first, first_ws)
        settled(home)
        build(side, "cold")
        _, states = wait_warm(side, ws)
        if states != ["ready"]:
            raise SystemExit(f"the daemon did not get ready: {states}; its logs are under {home}")
        for _ in range(240):
            if "[ra-jit]" in build(side, "wait for the daemon"):
                break
            time.sleep(0.5)
        else:
            raise SystemExit(f"no build said `[ra-jit]`; the daemon's logs are under {home}")
        log = worker_log(home)
        if log is None:
            raise SystemExit(f"the daemon listed nothing beside cargo's build, which is what this tests; its logs are under {home}")
        if not compiled_for_edits(log):
            raise SystemExit(f"the daemon did not compile the program for edits to patch; its log is {log}")
        seen = len(patches(home))
        lib = os.path.join(ws, "gen/src/lib.rs")
        text = open(lib).read()
        open(lib, "w").write(text.replace("pub fn value() -> u32 {\n    1\n}",
                                          "pub fn value() -> u32 {\n    1 + added()\n}\n\nfn added() -> u32 {\n    40\n}"))
        said = build(side, "a function added")
        made = patches(home)[seen:]
        ran = subprocess.run([program], capture_output=True, text=True, timeout=60)
        _, theirs = reference.cargo("run", "-q", "-p", "app")
        if "[ra-jit]" not in said:
            wrong.append("the edit was not served by the daemon")
        elif not made:
            wrong.append("the edit was not patched")
        elif made[-1] >= EXPORTS:
            wrong.append(f"the patch is of {made[-1]} functions: the {EXPORTS} the included file declares were not listed")
        if ran.stdout != theirs.stdout:
            wrong.append(f"the program printed {ran.stdout.strip()!r}, cargo's {theirs.stdout.strip()!r}")
        print(f"{'a function added':30} {'ok' if not wrong else 'WRONG':8} patch of {made[-1] if made else '-'} functions")
    finally:
        first.cargo("stop")
    for why in wrong:
        print(f"  {why}")
    if wrong:
        print(f"under {root}")
        sys.exit(1)
    shutil.rmtree(root, ignore_errors=True)


if __name__ == "__main__":
    main()

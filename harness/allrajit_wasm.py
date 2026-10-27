#!/usr/bin/env python3
"""allrajit_wasm.py: wasm programs with nothing rustc built in them (docs/no-rustc.md, step G).

allrajit.py's programs that a wasm32-wasip1 program can be (a hello, the rich one without its
threads and its caught panics, as wasm's panics abort, and the one built with
`panic = "abort"`) are built by ra-jit for wasm32-wasip1 with every crate its own, the
standard library included, and linked by ra-jit (`--exe`) with the toolchain's `rust-lld`
and the wasi-libc the toolchain ships (C, as glibc is on Linux). A wasm32-unknown-unknown
`cdylib` is linked too (`--shared`), each export called with `wasmtime run --invoke`. Nothing
ra-jit could not compile may be left in a module. What each prints and how it exits, under
`wasmtime run`, has to be what cargo's build of the same source does; the sizes of both are
printed.

The standard library of each target is compiled once into a store that starts empty, as a
unit, and taken from it by the programs after the first.

    python3 allrajit_wasm.py           # needs wasmtime, and rustup's wasm32-wasip1 and
                                       # wasm32-unknown-unknown targets
"""
import os, re, shutil, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from difftest import RA_JIT
from storelib import scratch

WASI = "wasm32-wasip1"
UNKNOWN = "wasm32-unknown-unknown"
WASMTIME = os.environ.get("RA_JIT_WASMTIME", "wasmtime")

HELLO = """fn main() {
    let v: Vec<u32> = (1..=10).collect();
    println!("hello {} {:?}", v.iter().sum::<u32>(), std::env::args().count());
}
"""
RICH = r"""use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::io::Write as _;

#[derive(Debug, Clone, PartialEq)]
enum Shape { Circle(f64), Rect { w: f64, h: f64 } }

trait Area { fn area(&self) -> f64; }
impl Area for Shape {
    fn area(&self) -> f64 {
        match self { Shape::Circle(r) => std::f64::consts::PI * r * r, Shape::Rect { w, h } => w * h }
    }
}

fn main() {
    let mut words = HashMap::new();
    for w in "the quick brown fox jumps over the lazy dog the end".split(' ') {
        *words.entry(w).or_insert(0usize) += 1;
    }
    let sorted: BTreeMap<_, _> = words.iter().collect();
    println!("words {:?}", sorted);
    let shapes: Vec<Box<dyn Area>> = vec![Box::new(Shape::Circle(1.5)), Box::new(Shape::Rect { w: 2.0, h: 3.25 })];
    let total: f64 = shapes.iter().map(|s| s.area()).sum();
    println!("area {total:.6} {:e} {}", total, 1.0f64 / 3.0);
    println!("shape {:?}", Shape::Rect { w: 1.0, h: 2.5 });
    let mut s = String::new();
    write!(s, "{:>8}|{:<6}|{:^7}|{:#x}|{:08.3}", "right", "left", "mid", 255, -3.14159).unwrap();
    println!("{s}");
    let text = std::fs::read_to_string("Cargo.toml").unwrap();
    println!("lines {}", text.lines().count());
    println!("parsed {:?} {:?}", "42".parse::<i64>(), "x".parse::<u8>().is_err());
    std::io::stdout().flush().unwrap();
    if std::env::args().count() > 1 { let none: Option<u8> = None; none.expect("asked to fail"); }
}
"""
ABORT = r"""fn main() {
    let caught = std::panic::catch_unwind(|| 1);
    println!("caught {:?}", caught.ok());
    if std::env::args().count() > 1 { panic!("aborting {}", 3); }
}
"""
# (program, its source, the runs to compare: the arguments of each, its panic strategy)
PROGRAMS = [("hello", HELLO, [["a", "b"]], "unwind"), ("rich", RICH, [[], ["fail"]], "unwind"),
            ("abort", ABORT, [[], ["fail"]], "abort")]
# A library of a page or a host's guest: no system, its functions called by name.
# Its own allocator over a static, until ra-jit compiles `llvm.wasm.memory.grow` (sub-lane
# `simd`): dlmalloc, the target's, grows the memory with it.
LIBRARY = r"""use std::alloc::{GlobalAlloc, Layout};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

struct Bump { heap: Cell<[u8; 1 << 20]>, used: Cell<usize> }
unsafe impl Sync for Bump {}
unsafe impl GlobalAlloc for Bump {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let start = (self.used.get() + layout.align() - 1) & !(layout.align() - 1);
        if start + layout.size() > 1 << 20 { return std::ptr::null_mut(); }
        self.used.set(start + layout.size());
        unsafe { self.heap.as_ptr().cast::<u8>().add(start) }
    }
    unsafe fn dealloc(&self, _: *mut u8, _: Layout) {}
}
#[global_allocator]
static HEAP: Bump = Bump { heap: Cell::new([0; 1 << 20]), used: Cell::new(0) };

thread_local! {
    static SEEN: RefCell<BTreeMap<u64, String>> = RefCell::new(BTreeMap::new());
}

#[unsafe(no_mangle)]
pub extern "C" fn allrajit_count(n: u64) -> u64 {
    let text: String = (0..n).map(|i| format!("{i},")).collect();
    SEEN.with(|seen| seen.borrow_mut().insert(n, text.clone()));
    let seen = SEEN.with(|seen| seen.borrow().len() as u64);
    seen * 1000 + text.len() as u64 * 10
}

#[unsafe(no_mangle)]
pub extern "C" fn allrajit_mix(a: u32, b: u32) -> u32 {
    a.rotate_left(b % 32) ^ b.wrapping_mul(0x9e37_79b9)
}
"""
CALLS = [("allrajit_count", ["1"]), ("allrajit_count", ["10"]), ("allrajit_mix", ["7", "35"])]


def run(cmd, cwd, what, env=None):
    start = time.monotonic()
    # cargo's build is read from the package's own target directory.
    env = {**os.environ, "RUST_BACKTRACE": "0", **(env or {})}
    for name in ("CARGO_TARGET_DIR", "CARGO_BUILD_TARGET", "RUSTFLAGS"):
        env.pop(name, None)
    ran = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, env=env)
    took = time.monotonic() - start
    if ran.returncode != 0:
        raise SystemExit(f"FAIL: {what} exited {ran.returncode}:\n{ran.stdout}{ran.stderr}")
    return ran, took


def outcome(module, args, cwd):
    """How the module runs under wasmtime: its status, what it printed, and what it said up to
    the trap of an abort, whose backtrace names functions as each compiler named them."""
    ran = subprocess.run([WASMTIME, "run", "--dir", ".", module, *args], cwd=cwd,
                         capture_output=True, text=True, env={**os.environ, "RUST_BACKTRACE": "0"})
    said = ran.stderr.split("Error: failed to run main module")[0]
    return ran.returncode, ran.stdout, said


def invoked(module, cwd):
    """What each of CALLS answers, each in a process of its own."""
    answers = []
    for name, args in CALLS:
        ran = subprocess.run([WASMTIME, "run", "--invoke", name, module, *args], cwd=cwd,
                             capture_output=True, text=True)
        answers.append((ran.returncode, ran.stdout.strip()))
    return answers


def package(work, name, manifest, path, source):
    root = os.path.join(work, name)
    os.makedirs(os.path.join(root, "src"))
    with open(os.path.join(root, "Cargo.toml"), "w") as f:
        f.write(f'[package]\nname = "{name}"\nversion = "0.1.0"\nedition = "2021"\n{manifest}[workspace]\n')
    with open(os.path.join(root, "src", path), "w") as f:
        f.write(source)
    subprocess.run(["cargo", "generate-lockfile", "-q"], cwd=root, check=True)
    return root


def ra_jit(root, crate, out, exe, target, store, *extra):
    env = {"RA_JIT_CACHE": store, "RA_JIT_STD_FROM": "off"}
    return run([RA_JIT, root, "--crate", crate, "--out", out, "--exe", exe, "--target", target,
                *extra], root, f"ra-jit {crate} ({target})", env)


def linked(ran, what):
    """How long ra-jit's link took, failing when it left something it could not compile."""
    said = re.search(r"^linked .* in (\S+), (\d+) functions in it ra-jit could not compile$",
                     ran.stderr, re.M)
    if not said:
        raise SystemExit(f"FAIL: {what}: ra-jit did not say it linked:\n{ran.stderr}")
    if said.group(2) != "0":
        raise SystemExit(f"FAIL: {what}: the module can reach {said.group(2)} functions ra-jit "
                         f"could not compile:\n{ran.stderr}")
    return said.group(1)


def nothing_of_rustc(ran, what):
    """That the standard library was ra-jit's own: compiled now or taken from the store."""
    said = re.findall(r"^standard library: (.*)$", ran.stderr, re.M)
    if not said:
        raise SystemExit(f"FAIL: {what}: ra-jit built no standard library of its own:\n{ran.stderr}")
    return "from the store" if said[0].startswith("from the store") else "compiled"


def kib(path):
    return f"{os.path.getsize(path) / 1024:.0f} KiB"


def main():
    if shutil.which(WASMTIME) is None:
        raise SystemExit(f"FAIL: no {WASMTIME} to run the modules with")
    work = scratch("ra-jit-allrajit-wasm-")
    store = os.path.join(work, "store")
    passed = False
    lines = []
    try:
        for name, source, runs, panic in PROGRAMS:
            manifest = '[profile.dev]\npanic = "abort"\n' if panic == "abort" else ""
            root = package(work, name, manifest, "main.rs", source)
            run(["cargo", "build", "-q", "--target", WASI], root, f"cargo build of {name}")
            theirs = os.path.join(root, "target", WASI, "debug", f"{name}.wasm")
            expected = [outcome(theirs, args, root) for args in runs]
            ours = os.path.join(work, f"{name}.wasm")
            ran, took = ra_jit(root, name, os.path.join(work, f"{name}.o"), ours, WASI, store)
            std = nothing_of_rustc(ran, name)
            link_took = linked(ran, name)
            for args, want in zip(runs, expected):
                got = outcome(ours, args, root)
                if got != want:
                    raise SystemExit(f"FAIL: {name} {' '.join(args)} differs from cargo's:\n"
                                     f"cargo:  {want}\nra-jit: {got}")
            lines.append(f"{name} ({WASI}): built and linked in {took:.2f} s, the standard library "
                         f"{std}, the link {link_took}, {len(runs)} runs as cargo's; "
                         f"{kib(ours)}, cargo's {kib(theirs)}")
        root = package(work, "shared", '[lib]\ncrate-type = ["cdylib"]\n', "lib.rs", LIBRARY)
        run(["cargo", "build", "-q", "--target", UNKNOWN], root, "cargo build of the cdylib")
        theirs = os.path.join(root, "target", UNKNOWN, "debug", "shared.wasm")
        expected = invoked(theirs, root)
        if any(code != 0 for code, _ in expected):
            raise SystemExit(f"FAIL: cargo's cdylib did not answer: {expected}")
        ours = os.path.join(work, "shared.wasm")
        ran, took = ra_jit(root, "shared", os.path.join(work, "shared.o"), ours, UNKNOWN, store,
                           "--shared")
        std = nothing_of_rustc(ran, "the cdylib")
        link_took = linked(ran, "the cdylib")
        got = invoked(ours, root)
        if got != expected:
            raise SystemExit(f"FAIL: the cdylib differs from cargo's:\ncargo:  {expected}\nra-jit: {got}")
        lines.append(f"the cdylib ({UNKNOWN}): built and linked in {took:.2f} s, the standard "
                     f"library {std}, the link {link_took}, {len(CALLS)} calls as cargo's; "
                     f"{kib(ours)}, cargo's {kib(theirs)}")
        print("\n".join(lines))
        print(f"PASS: {len(PROGRAMS)} {WASI} programs and a {UNKNOWN} cdylib with nothing of rustc's")
        passed = True
    finally:
        if passed:
            shutil.rmtree(work, ignore_errors=True)
        elif lines:
            print("\n".join(lines))


if __name__ == "__main__":
    main()

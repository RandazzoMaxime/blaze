#!/usr/bin/env python3
"""allrajit.py: programs with nothing rustc built in them (docs/no-rustc.md, steps F and G).

Two programs, a hello and one that uses collections, `catch_unwind`, threads, channels,
formatting, a file and an uncaught panic, are built by ra-jit with every crate its own,
the standard library included, and linked by `cc`. Each is built two ways:

  - whole: the program and the standard library in one build, one object;
  - over the unit: the standard library built once, alone, into an object of its own
    (`std.o`), and each program built over it with `--prebuilt`.

What each prints, and how it exits, has to be what cargo's build of the same source does,
thread ids aside.

    python3 allrajit.py            # both ways
    python3 allrajit.py --unit     # over the unit only
"""
import os, re, shutil, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from difftest import RA_JIT
from storelib import scratch

# The crates of the standard library a program on Linux links, by the names cargo gives them.
STD = ["std", "core", "alloc", "compiler_builtins", "panic_unwind", "unwind", "libc", "cfg_if",
       "hashbrown", "rustc_demangle", "std_detect", "addr2line", "gimli", "object", "miniz_oxide",
       "adler2", "memchr"]
LIBS = ["-lgcc_s", "-lc", "-lm", "-lpthread", "-ldl", "-lutil", "-lrt"]

HELLO = """fn main() {
    let v: Vec<u32> = (1..=10).collect();
    println!("hello {} {:?}", v.iter().sum::<u32>(), std::env::args().count());
}
"""
RICH = r"""use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::io::Write as _;
use std::sync::{Arc, Mutex, mpsc};

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
    let caught = std::panic::catch_unwind(|| {
        let v: Vec<i32> = Vec::new();
        v[3]
    });
    println!("caught {}", caught.is_err());
    let r = std::panic::catch_unwind(|| -> i32 { panic!("boom {}", 7) });
    let payload = r.unwrap_err();
    let msg = payload.downcast_ref::<String>().cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default();
    println!("msg {msg}");
    let counter = Arc::new(Mutex::new(0u64));
    let (tx, rx) = mpsc::channel();
    let handles: Vec<_> = (0..4u64).map(|i| {
        let counter = Arc::clone(&counter);
        let tx = tx.clone();
        std::thread::spawn(move || {
            for j in 0..1000 { *counter.lock().unwrap() += i * j; }
            tx.send(i).unwrap();
        })
    }).collect();
    drop(tx);
    for h in handles { h.join().unwrap(); }
    let mut got: Vec<u64> = rx.iter().collect();
    got.sort();
    println!("threads {} {:?}", *counter.lock().unwrap(), got);
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
# (program, its source, the runs to compare: the arguments of each)
PROGRAMS = [("hello", HELLO, [["a", "b"]]), ("rich", RICH, [[], ["fail"]])]


def run(cmd, cwd, what):
    start = time.monotonic()
    ran = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True,
                         env={**os.environ, "RUST_BACKTRACE": "0"})
    took = time.monotonic() - start
    if ran.returncode != 0:
        raise SystemExit(f"FAIL: {what} exited {ran.returncode}:\n{ran.stdout}{ran.stderr}")
    return ran, took


def outcome(binary, args, cwd):
    ran = subprocess.run([binary, *args], cwd=cwd, capture_output=True, text=True,
                         env={**os.environ, "RUST_BACKTRACE": "0"})
    unthreaded = lambda text: re.sub(r"thread '([^']*)' \(\d+\)", r"thread '\1' (N)", text)
    return ran.returncode, unthreaded(ran.stdout), unthreaded(ran.stderr)


def package(work, name, source):
    root = os.path.join(work, name)
    os.makedirs(os.path.join(root, "src"))
    with open(os.path.join(root, "Cargo.toml"), "w") as f:
        f.write(f'[package]\nname = "{name}"\nversion = "0.1.0"\nedition = "2021"\n[workspace]\n')
    with open(os.path.join(root, "src/main.rs"), "w") as f:
        f.write(source)
    subprocess.run(["cargo", "generate-lockfile", "-q"], cwd=root, check=True)
    return root


def ra_jit(root, crates, out, *extra):
    args = sum((["--crate", c] for c in crates), [])
    return run([RA_JIT, root, *args, "--out", out, *extra], root, f"ra-jit {' '.join(crates[:1])}")


def main():
    if not sys.platform.startswith("linux"):
        print("not run: the link line here is Linux's (step G gives ra-jit its own)")
        return
    ways = ["unit"] if "--unit" in sys.argv else ["whole", "unit"]
    work = scratch("ra-jit-allrajit-")
    passed = False
    try:
        roots = {name: package(work, name, source) for name, source, _ in PROGRAMS}
        expected = {}
        for name, _, runs in PROGRAMS:
            root = roots[name]
            run(["cargo", "build", "-q"], root, f"cargo build of {name}")
            binary = os.path.join(root, "target/debug", name)
            expected[name] = [outcome(binary, args, root) for args in runs]
        lines = []
        unit = None
        if "unit" in ways:
            unit = os.path.join(work, "std.o")
            _, took = ra_jit(roots["hello"], STD, unit)
            lines.append(f"the standard library alone: {took:.2f} s, {os.path.getsize(unit) >> 20} MB")
        for way in ways:
            for name, _, runs in PROGRAMS:
                root = roots[name]
                obj = os.path.join(work, f"{name}-{way}.o")
                binary = os.path.join(work, f"{name}-{way}")
                if way == "whole":
                    _, took = ra_jit(root, [name, *STD], obj)
                    objects = [obj]
                else:
                    _, took = ra_jit(root, [name], obj, "--prebuilt", unit)
                    objects = [obj, unit]
                _, linked = run(["cc", *objects, "-o", binary, *LIBS], root, f"the link of {name} ({way})")
                for args, want in zip(runs, expected[name]):
                    got = outcome(binary, args, root)
                    if got != want:
                        raise SystemExit(f"FAIL: {name} {' '.join(args)} ({way}) differs from cargo's:\n"
                                         f"cargo:  {want}\nra-jit: {got}")
                lines.append(f"{name} ({way}): built in {took:.2f} s, linked in {linked:.2f} s, "
                             f"{len(runs)} runs as cargo's")
        print("\n".join(lines))
        print(f"PASS: {len(PROGRAMS)} programs with nothing of rustc's, {' and '.join(ways)}")
        passed = True
    finally:
        if passed:
            shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    main()

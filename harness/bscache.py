#!/usr/bin/env python3
"""bscache.py: what the build scripts of the bench corpus reach, as the sandbox records it, and
what a hermetic sandbox would refuse them.

usage: bscache.py record [PROJECT...] --ra-jit BIN [--work DIR] [--out DIR] [--jobs N]
       bscache.py report --out DIR

`record` fetches each project of harness/bench.py at its pinned revision (all of them when none
is named) and runs the step of a workspace load that builds and runs its build scripts and proc
macros, `cargo check --compile-time-deps --workspace --all-targets`, through ra-jit's wrapper
(RUSTC_WRAPPER=ra-jit-rustc beside BIN, RA_JIT_ONLY_ASKS), with an empty store (RA_JIT_CACHE of its own), so every
script runs, and runs in the sandbox (cargo/script_sandbox.rs) through `ra-jit build-script`.
Proc macros are rustc's here (RA_JIT_PROC_MACRO_SERVER=0): what is measured is the scripts'
runs. The sandbox leaves beside each script's OUT_DIR the record of what it reached
(`ra-jit-record`: R read, W write refused, N network refused, X record not whole) and how cargo
ran it (`ra-jit-ran.json`). Each project's are collected into `<out>/<project>.json`.

`report` classifies every line of every record and writes `<out>/README.md` and, every
would-be denial by package, `<out>/denied.json`:

  network     every N line: refused now, and by a hermetic sandbox
  writes      every W line: a write outside OUT_DIR, refused now
  executables an R line of a regular file with the executable bit and an ELF or `#!` start (the
              record does not tell a run from a look, so a script that looks for a program on
              PATH without running it counts too), outside the toolchain (its sysroot, ra-jit),
              the Nix closure of the C compiler (`cc`, `c++`, `ar`) and pkg-config, and the
              package (its own configure and scripts are inputs)
  reads       every other R line outside the package, its OUT_DIR, the toolchain and that
              closure; a dependency's OUT_DIR (what `links` gives a script by DEP_*) is listed
              apart, as a read a hermetic sandbox allows
"""
import collections, concurrent.futures, json, os, re, shutil, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from resolve import BENCH_PROJECT, fetch_git, option  # noqa: E402

RECORD = "ra-jit-record"
RAN = "ra-jit-ran.json"


def projects():
    source = open(os.path.join(HERE, "bench.py")).read()
    return {name: (repo, rev) for name, repo, rev in BENCH_PROJECT.findall(source)}


def run_dirs(target):
    """Each directory a script ran in below `target`: the parent of its OUT_DIR."""
    for root, dirs, files in os.walk(target):
        if RAN in files:
            yield root
            dirs[:] = [d for d in dirs if d == "out"]


def record(name, repo, rev, work, ra_jit, jobs):
    source = os.path.join(work, "src", name)
    fetch_git(repo, rev, source)
    target = os.path.join(work, "target", name)
    store = os.path.join(work, "store", name)
    for dir in (target, store):
        shutil.rmtree(dir, ignore_errors=True)
    env = dict(os.environ)
    for key in list(env):
        if key.startswith(("CARGO_", "RA_JIT_", "RUSTC_")) and key != "CARGO_HOME":
            del env[key]
    env.update(
        RUSTC_WRAPPER=os.path.join(os.path.dirname(ra_jit), "ra-jit-rustc"),
        RA_JIT_ONLY_ASKS="1",
        RA_JIT_PROC_MACRO_SERVER="0",
        RA_JIT_CACHE=store,
        CARGO_TARGET_DIR=target,
        CARGO_BUILD_JOBS=str(jobs),
        CARGO_UNSTABLE_CHECKSUM_FRESHNESS="false",
        CARGO_HOME=os.path.join(work, "cargo-home"),
        RUST_BACKTRACE="0",
    )
    command = ["cargo", "check", "-Zunstable-options", "--compile-time-deps", "--workspace", "--all-targets"]
    proc = subprocess.run(command, cwd=source, env=env, capture_output=True, text=True, errors="replace")
    failed = sorted(set(re.findall(r"failed to run custom build command for `([^`]+)`", proc.stderr)))
    scripts = []
    for dir in run_dirs(target):
        ran = json.load(open(os.path.join(dir, RAN)))
        var = dict(ran["env"])
        lines = None
        if os.path.exists(os.path.join(dir, RECORD)):
            lines = [line.split("\t", 2) for line in open(os.path.join(dir, RECORD)).read().splitlines()]
        scripts.append(
            dict(
                package=var.get("CARGO_PKG_NAME"),
                version=var.get("CARGO_PKG_VERSION"),
                manifest_dir=var.get("CARGO_MANIFEST_DIR"),
                out_dir=var.get("OUT_DIR"),
                real=ran["real"],
                rustc=var.get("RUSTC"),
                cargo_home=var.get("CARGO_HOME"),
                home=var.get("HOME"),
                env={k: var[k] for k in ("NIX_CFLAGS_COMPILE", "NIX_LDFLAGS", "CFLAGS", "LDFLAGS", "PKG_CONFIG_PATH") if k in var},
                record=[dict(reached=r, what=w) for r, _, w in lines] if lines is not None else None,
                executables=executables(lines or []),
            )
        )
    return dict(
        project=name,
        rev=rev,
        status=proc.returncode,
        failed=failed,
        stderr_tail=proc.stderr[-4000:] if proc.returncode else "",
        source=source,
        target=target,
        scripts=sorted(scripts, key=lambda it: (it["package"] or "", it["out_dir"] or "")),
    )


def executables(lines):
    """The paths of R lines that are programs: a regular file, executable, starting as one."""
    found = []
    for reached, _, what in lines:
        if reached != "R" or what.endswith(".so") or ".so." in os.path.basename(what):
            continue
        try:
            if not (os.path.isfile(what) and os.access(what, os.X_OK)):
                continue
            with open(what, "rb") as file:
                start = file.read(4)
        except OSError:
            continue
        if start == b"\x7fELF" or start[:2] == b"#!":
            found.append(what)
    return found


def closure(programs):
    """The Nix store paths `programs` run from, and everything they need."""
    roots = []
    for program in programs:
        found = shutil.which(program)
        if found:
            real = os.path.realpath(found)
            if real.startswith("/nix/store/"):
                roots.append("/".join(real.split("/")[:4]))
    if not roots:
        return []
    out = subprocess.run(["nix-store", "-qR", *roots], capture_output=True, text=True)
    return sorted(set(out.stdout.split()))


def under(path, dir):
    return bool(dir) and (path == dir or path.startswith(dir.rstrip("/") + "/"))


def flag_dirs(*values):
    """The directories `-I`, `-isystem`, `-idirafter`, `-L` and `-B` name in compiler flags."""
    dirs = []
    for value in values:
        words = (value or "").split()
        for at, word in enumerate(words):
            for flag in ("-isystem", "-idirafter", "-I", "-L", "-B"):
                if word == flag and at + 1 < len(words):
                    dirs.append(words[at + 1])
                elif word.startswith(flag) and len(word) > len(flag) and word[len(flag)] == "/":
                    dirs.append(word[len(flag):])
    return [os.path.normpath(it) for it in dirs]


def classify(project, allowed):
    """Each script's lines in classes: what a hermetic sandbox refuses and what it allows."""
    target = project["target"]
    source = project["source"]
    out = []
    for script in project["scripts"]:
        record = script["record"]
        if record is None:
            out.append(dict(script, classes=None))
            continue
        run_dir = os.path.dirname(script["out_dir"] or "")
        env = script.get("env")
        if env is None and os.path.exists(os.path.join(run_dir, RAN)):
            env = dict(json.load(open(os.path.join(run_dir, RAN)))["env"])
        env = env or {}
        toolchain = [allowed["sysroot"], allowed["ra_jit_dir"], os.path.dirname(script["real"]), run_dir]
        toolchain += allowed["closure"]
        # What the dev shell has the C compiler and pkg-config search.
        searched = flag_dirs(env.get("NIX_CFLAGS_COMPILE"), env.get("NIX_LDFLAGS"), env.get("CFLAGS"), env.get("LDFLAGS"))
        searched += [it for it in (env.get("PKG_CONFIG_PATH") or "").split(":") if it]
        roots = toolchain + searched + [script["out_dir"], script["manifest_dir"], target, source, script["cargo_home"]]
        exes = set(script["executables"])
        classes = collections.defaultdict(list)
        for line in record:
            reached, what = line["reached"], line["what"]
            name = os.path.basename(what)
            if reached == "N":
                classes["network"].append(what)
            elif reached == "W" and any(under(it, what) for it in (script["out_dir"],) if it):
                classes["allowed: making a directory above OUT_DIR, which is there"].append(what)
            elif reached == "W":
                classes["writes"].append(what)
            elif reached == "X":
                classes["incomplete"].append(what)
            elif any(under(what, it) for it in toolchain):
                classes["allowed: toolchain and C compiler"].append(what)
            elif what in exes and under(what, script["manifest_dir"]):
                classes["allowed: the package's own programs (configure, its scripts)"].append(what)
            elif what in exes and not under(what, source):
                classes["executables"].append(what)
            elif under(what, script["out_dir"]):
                classes["allowed: OUT_DIR"].append(what)
            elif under(what, script["manifest_dir"]):
                classes["allowed: package"].append(what)
            elif under(what, target) and re.search(r"/build/[^/]+(/[0-9a-f]+)?/out(/|$)", what):
                classes["allowed: a dependency's OUT_DIR"].append(what)
            elif any(under(what, it) for it in searched):
                classes["allowed: the C compiler's and pkg-config's search paths"].append(what)
            elif any(under(it, what) for it in roots if it):
                classes["allowed: a directory above an allowed one"].append(what)
            elif not os.path.lexists(what):
                classes["allowed: looked up, not there"].append(what)
            elif name.endswith(".so") or ".so." in name or what in ("/etc/ld-nix.so.preload", "/etc/ld.so.cache", "/etc/ld.so.preload"):
                classes["allowed: shared libraries and the loader's files"].append(what)
            elif name == ".cargo" or name in ("config", "config.toml") and "/.cargo/" in what or what == "/etc/_rjem_malloc.conf":
                classes["allowed: cargo's configuration, read by ra-jit as rustc"].append(what)
            elif under(what, target):
                classes["reads: target directory"].append(what)
            elif under(what, source):
                classes["reads: workspace outside the package"].append(what)
            elif under(what, script["cargo_home"]):
                classes["reads: cargo home (other packages)"].append(what)
            elif what.startswith("/nix/store/"):
                classes["reads: Nix store outside the C compiler"].append(what)
            elif what.startswith("/etc/"):
                classes["reads: /etc"].append(what)
            elif under(what, script["home"]):
                classes["reads: home"].append(what)
            elif what.startswith(("/proc/", "/sys/", "/dev/")):
                classes["allowed: kernel views"].append(what)
            else:
                classes["reads: elsewhere"].append(what)
        out.append(dict(script, classes=dict(classes)))
    return out


REFUSED = ["network", "writes", "executables"]


def refused(classes):
    return {k: v for k, v in classes.items() if k in REFUSED or k.startswith("reads:") or k == "incomplete"}


# Every would-be denial, by package: what `report` writes beside README.md.
DENIED = "denied.json"


def report(out_dir):
    rows, details, failures = [], [], []
    totals = collections.Counter()
    for file in sorted(os.listdir(out_dir)):
        if not file.endswith(".json") or file == DENIED:
            continue
        project = json.load(open(os.path.join(out_dir, file)))
        scripts = classify(project, project["allowed"])
        count = collections.Counter()
        for script in scripts:
            count["scripts"] += 1
            if script["classes"] is None:
                count["no record"] += 1
                continue
            denied = refused(script["classes"])
            count["clean"] += not denied
            for kind in REFUSED:
                count[kind] += kind in denied
            count["reads"] += any(k.startswith("reads:") for k in denied)
            if denied:
                details.append((project["project"], script, denied))
        count["failed"] = len(project["failed"])
        for package in project["failed"]:
            said = [line.strip() for line in project["stderr_tail"].splitlines()]
            said = [line for line in said if line.startswith(("note:", "Error", "thread", "error:")) or "panicked" in line]
            failures.append((project["project"], dict(package=package, said=said[-6:])))
        totals.update(count)
        rows.append((project["project"], project["status"], count))
    lines = [
        "# Build scripts of the bench corpus under the sandbox's record",
        "",
        "Made by `harness/bscache.py record` then `report` (see its docstring for how each line is",
        "classed). One row per project; a script counts once per class it has a line in.",
        "",
        "| project | cargo | scripts | failed | no record | clean | network | writes | executables | reads |",
        "|---|---|---|---|---|---|---|---|---|---|",
    ]
    for name, status, c in rows + [("total", "", totals)]:
        cells = [c[k] for k in ("scripts", "failed", "no record", "clean", "network", "writes", "executables", "reads")]
        lines.append(f"| {name} | {status} | " + " | ".join(map(str, cells)) + " |")
    lines += ["", "## Scripts that failed", ""]
    for name, file in sorted(failures):
        lines.append(f"### {name}: `{file['package']}`")
        lines += ["", "```", *file["said"], "```", ""]
    lines += ["", "## What each script would be refused", ""]
    by_package = collections.OrderedDict()
    for project, script, denied in details:
        key = f"{script['package']} {script['version']}"
        entry = by_package.setdefault(key, dict(projects=[], denied=collections.defaultdict(set)))
        entry["projects"].append(project)
        for kind, whats in denied.items():
            entry["denied"][kind].update(tidy(what, script) for what in whats)
    for key, entry in sorted(by_package.items()):
        lines.append(f"### `{key}` ({', '.join(sorted(set(entry['projects'])))})")
        lines.append("")
        for kind, whats in sorted(entry["denied"].items()):
            shown = sorted(whats)
            more = f" and {len(shown) - 12} more" if len(shown) > 12 else ""
            lines.append(f"- {kind}: " + ", ".join(f"`{it}`" for it in shown[:12]) + more)
        lines.append("")
    open(os.path.join(out_dir, "README.md"), "w").write("\n".join(lines))
    denied = {key: dict(projects=sorted(set(entry["projects"])),
                        denied={kind: dict(count=len(whats), first=sorted(whats)[:50])
                                for kind, whats in sorted(entry["denied"].items())})
              for key, entry in sorted(by_package.items())}
    json.dump(denied, open(os.path.join(out_dir, DENIED), "w"), indent=1, sort_keys=True)
    print("\n".join(lines[:8 + len(rows) + 1]))


def tidy(what, script):
    """`what` with the places of one machine named by what they are."""
    for place, name in ((script["manifest_dir"], "$PACKAGE"), (script["cargo_home"], "$CARGO_HOME"), (script["home"], "$HOME")):
        if place and what.startswith(place.rstrip("/") + "/"):
            return name + what[len(place.rstrip("/")):]
    return re.sub(r"^/nix/store/[0-9a-z]{32}-", "/nix/store/*-", what)


def main():
    argv = sys.argv[1:]
    if not argv or argv[0] not in ("record", "report"):
        sys.exit(__doc__)
    mode = argv.pop(0)
    out_dir = option(argv, "--out", os.path.join(HERE, "..", "bench", "results", "bscache"))
    os.makedirs(out_dir, exist_ok=True)
    if mode == "report":
        return report(out_dir)
    ra_jit = os.path.abspath(option(argv, "--ra-jit") or sys.exit("--ra-jit BIN"))
    work = option(argv, "--work", os.path.join(os.environ.get("TMPDIR", "/tmp"), "blaze-bscache"))
    jobs = int(option(argv, "--jobs", "8"))
    corpus = projects()
    names = argv or list(corpus)
    sysroot = subprocess.run(["rustc", "--print", "sysroot"], capture_output=True, text=True).stdout.strip()
    allowed = dict(sysroot=sysroot, ra_jit_dir=os.path.dirname(ra_jit), closure=closure(["cc", "c++", "ar", "pkg-config"]))
    for name in names:
        repo, rev = corpus[name]
        result = record(name, repo, rev, work, ra_jit, jobs)
        result["allowed"] = allowed
        json.dump(result, open(os.path.join(out_dir, f"{name}.json"), "w"), indent=1)
        print(f"{name}: cargo {result['status']}, {len(result['scripts'])} scripts, failed {result['failed']}", flush=True)


if __name__ == "__main__":
    main()

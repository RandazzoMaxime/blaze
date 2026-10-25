#!/usr/bin/env python3
"""resolve_graph.py: the resolve blaze-resolve makes of a workspace's packages against the one
cargo makes, over a corpus. cargo is the oracle; blaze never runs it outside this harness.

usage: resolve_graph.py CORPUS [PART...] [--work DIR] [--jobs N] [--out FILE] [--limit N] [--keep]

CORPUS is harness/resolve/corpus.json (its parts `bench`, `crates` and `workspaces`, as
harness/resolve.py reads them). Each case is fetched under `--work` (default
$TMPDIR/blaze-resolve-graph) with a CARGO_HOME of its own there, and `cargo metadata
--format-version 1` is run on it twice: `locked`, with the Cargo.lock it ships (cases without
one are skipped), and `fresh`, with none. The `metadata_resolve` example of blaze-resolve
reads each output, resolves the members from the packages listed, and prints its `resolve`,
which is compared with cargo's, key order included. So this checks the resolver's graph, its
feature unification and the JSON; not which versions are chosen, as the registry holds only
the versions cargo chose. Each checkout is deleted once compared, unless `--keep`. A case is
equal, different, blaze-failed or cargo-failed; the first difference or error of each that is
not equal is printed, and `--out` writes them all as JSON.
"""
import concurrent.futures, json, os, shutil, subprocess, sys, tarfile, tempfile, urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
CARGO = os.environ.get("CARGO_ORACLE", "cargo +1.99.0").split()
USER_AGENT = "blaze resolve_graph.py (https://github.com/indexable-inc/blaze)"
PARTS = ["bench", "crates", "workspaces"]
CLASSES = ["equal", "different", "blaze-failed", "cargo-failed"]


def option(argv, name, default=None):
    if name in argv:
        i = argv.index(name)
        value = argv[i + 1]
        del argv[i : i + 2]
        return value
    return default


def fetch_git(repo, rev, dest):
    os.makedirs(dest, exist_ok=True)
    git = lambda *a: subprocess.run(["git", "-C", dest, *a], check=True, capture_output=True)
    git("init", "-q")
    git("fetch", "-q", "--depth", "1", f"https://github.com/{repo}", rev)
    git("-c", "advice.detachedHead=false", "checkout", "-q", "FETCH_HEAD")


def fetch_crate(name, version, dest):
    url = f"https://static.crates.io/crates/{name}/{name}-{version}.crate"
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    with urllib.request.urlopen(request) as response, tempfile.TemporaryFile() as file:
        shutil.copyfileobj(response, file)
        file.seek(0)
        with tarfile.open(fileobj=file, mode="r:gz") as archive:
            archive.extractall(os.path.dirname(dest), filter="data")
    os.rename(os.path.join(os.path.dirname(dest), f"{name}-{version}"), dest)


def bench_projects():
    """The projects of harness/bench.py at their pinned revisions."""
    import re
    text = open(os.path.join(HERE, "bench.py")).read()
    pattern = r'^    "([\w.-]+)": dict\(\s*repo="([^"]+)",[^\n]*?rev="([0-9a-f]{40})"'
    return [{"name": n, "repo": r, "rev": v} for n, r, v in re.findall(pattern, text, re.M)]


def cases_of(part, corpus, work, limit):
    entries = bench_projects() if part == "bench" else corpus[part]
    for entry in entries[:limit]:
        if part == "crates":
            name, version = entry.split("@")
            dest = os.path.join(work, part, f"{name}-{version}")
            fetch = lambda d=dest, n=name, v=version: fetch_crate(n, v, d)
            yield {"part": part, "name": f"{name}-{version}", "dir": dest, "fetch": fetch}
        else:
            dest = os.path.join(work, part, entry["name"])
            fetch = lambda d=dest, e=entry: fetch_git(e["repo"], e["rev"], d)
            yield {"part": part, "name": entry["name"], "dir": dest, "fetch": fetch}


def build_blaze():
    target = os.environ.get("CARGO_TARGET_DIR", os.path.join(ROOT, "target"))
    subprocess.run([*CARGO, "build", "--release", "-q", "-p", "blaze-resolve", "--example",
                    "metadata_resolve"], cwd=ROOT, check=True)
    return os.path.join(target, "release", "examples", "metadata_resolve")


def first_difference(a, b, at="$"):
    if type(a) is not type(b):
        return f"{at}: {json.dumps(a)[:300]} != {json.dumps(b)[:300]}"
    if isinstance(a, dict):
        if list(a) != list(b):
            return f"{at}: keys {list(a)}, cargo's {list(b)}"
        for k in a:
            if (d := first_difference(a[k], b[k], f"{at}.{k}")) is not None:
                return d
        return None
    if isinstance(a, list):
        if isinstance(a, list) and a and isinstance(a[0], dict) and "id" in a[0]:
            ids_a, ids_b = [n.get("id") for n in a], [n.get("id") for n in b]
            if ids_a != ids_b:
                only_b = [i for i in ids_b if i not in ids_a][:3]
                only_a = [i for i in ids_a if i not in ids_b][:3]
                return f"{at}: nodes only cargo's {only_b}, only blaze's {only_a}"
        for i, (x, y) in enumerate(zip(a, b)):
            if (d := first_difference(x, y, f"{at}[{i}]")) is not None:
                return d
        if len(a) != len(b):
            return f"{at}: {len(a)} items, cargo's {len(b)}"
        return None
    return None if a == b else f"{at}: {json.dumps(a)[:300]} != {json.dumps(b)[:300]}"


def one(case, mode, blaze, cargo_home):
    """Runs cargo and blaze on the checkout in place; `fresh` moves its Cargo.lock aside for the
    run and puts it back."""
    lock = os.path.join(case["dir"], "Cargo.lock")
    if mode == "fresh" and os.path.exists(lock):
        os.rename(lock, lock + ".shipped")
    env = {k: v for k, v in os.environ.items() if not k.startswith("CARGO_") or k == "CARGO_ORACLE"}
    env["CARGO_HOME"] = cargo_home
    try:
        cargo = subprocess.run([*CARGO, "metadata", "--format-version", "1"], cwd=case["dir"],
                               env=env, capture_output=True, text=True, errors="replace")
    finally:
        if os.path.exists(lock + ".shipped"):
            os.replace(lock + ".shipped", lock)
    out = {"part": case["part"], "name": case["name"], "mode": mode}
    if cargo.returncode:
        lines = [l for l in cargo.stderr.splitlines() if l.startswith("error")]
        return {**out, "class": "cargo-failed", "why": (lines or [cargo.stderr.strip()])[0][:300]}
    ours = subprocess.run([blaze], input=cargo.stdout, capture_output=True, text=True)
    if ours.returncode:
        return {**out, "class": "blaze-failed", "why": ours.stderr.strip().splitlines()[0][:300]}
    theirs = json.loads(cargo.stdout)["resolve"]
    diff = first_difference(json.loads(ours.stdout), theirs)
    return {**out, "class": "different" if diff else "equal", "why": diff}


def main():
    argv = sys.argv[1:]
    work = option(argv, "--work", os.path.join(tempfile.gettempdir(), "blaze-resolve-graph"))
    jobs = int(option(argv, "--jobs", "8"))
    out_file = option(argv, "--out")
    limit = option(argv, "--limit")
    limit = int(limit) if limit else None
    keep = "--keep" in argv and not argv.remove("--keep")
    corpus = json.load(open(argv.pop(0)))
    parts = argv or PARTS
    blaze = build_blaze()
    cargo_home = os.path.join(work, "cargo-home")
    os.makedirs(cargo_home, exist_ok=True)
    cases = [c for part in parts for c in cases_of(part, corpus, work, limit)]

    def run(case):
        try:
            if not os.path.exists(case["dir"]):
                case["fetch"]()
        except Exception as e:
            return [{"part": case["part"], "name": case["name"], "mode": "fetch",
                     "class": "cargo-failed", "why": f"fetch: {e}"[:300]}]
        shipped = os.path.exists(os.path.join(case["dir"], "Cargo.lock"))
        results = [one(case, mode, blaze, cargo_home) for mode in ["locked", "fresh"][not shipped:]]
        if not keep:
            shutil.rmtree(case["dir"], ignore_errors=True)
        return results

    results = []
    with concurrent.futures.ThreadPoolExecutor(jobs) as pool:
        for rs in pool.map(run, cases):
            for r in rs:
                results.append(r)
                if r["class"] != "equal":
                    print(f"{r['class']:13} {r['mode']:6} {r['part']}/{r['name']}: {r['why']}", flush=True)
    for mode in ["locked", "fresh"]:
        counts = {c: sum(1 for r in results if r["mode"] == mode and r["class"] == c) for c in CLASSES}
        print(f"{mode}: " + ", ".join(f"{n} {c}" for c, n in counts.items()))
    if out_file:
        json.dump(results, open(out_file, "w"), indent=1)


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""fetchdiff.py: blaze-registry's fetching against cargo's, package by package.

usage: fetchdiff.py [--work DIR] [--popular N] [--projects NAME,...] [--blaze FETCH]

The lock files are the bench projects' (bench.py's PROJECTS, each at its pinned revision: its
Cargo.lock, or the one cargo makes when it has none; not bun's, whose tree bunprep.py makes), one for each of the N most downloaded
crates of crates.io (default 300: a project depending on its newest version, locked by cargo),
and one of a few crates from a second sparse registry, which this script serves on 127.0.0.1
from crates.io's index and files under a `dl` with no markers. cargo 1.99 fetches every lock
file (`cargo fetch --locked`) into a CARGO_HOME of its own, and blaze-registry's `fetch` example
(FETCH, or built here) into another. A package is equal when its directory in registry/src has
the same name in both and the same tree (every path, its kind and mode, a file's size, sha256
and mtime, a link's target; the mtime of `.cargo-ok` and of directories is when they were
made), and its .crate file in registry/cache the same bytes. The names of the directories in
registry/index, registry/cache and registry/src have to be the same too.

Prints equal/total and writes every difference to WORK/fetchdiff.json. Git packages are counted
apart: blaze-registry does not fetch them yet. Network-heavy: run it on a dev host.
"""
import argparse, ast, hashlib, http.server, json, os, shutil, stat, subprocess, sys, threading, time
import urllib.error, urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
TOOLCHAIN = "1.99.0"
AGENT = "blaze harness/fetchdiff.py (https://github.com/indexable-inc/blaze)"
MIRROR_PORT = 48273
MIRROR = f"sparse+http://127.0.0.1:{MIRROR_PORT}/index/"
MIRROR_CRATES = {"serde": "1", "regex": "1", "tokio": "1", "clap": "4", "rand": "0.9"}


def projects():
    """bench.py's PROJECTS as name -> (repo, rev, prepare), read from its source: importing
    bench.py asks for the toolchain ra-jit builds with."""
    tree = ast.parse(open(os.path.join(HERE, "bench.py")).read())
    for node in tree.body:
        if isinstance(node, ast.Assign) and any(getattr(it, "id", None) == "PROJECTS" for it in node.targets):
            found = {}
            for key, value in zip(node.value.keys, node.value.values):
                fields = {kw.arg: kw.value.value for kw in value.keywords if isinstance(kw.value, ast.Constant)}
                found[key.value] = (fields["repo"], fields["rev"], fields.get("prepare"))
            return found
    sys.exit("fetchdiff: no PROJECTS in bench.py")


def tool(name):
    return subprocess.run(["rustup", "which", name, "--toolchain", TOOLCHAIN], capture_output=True, text=True,
                          check=True).stdout.strip()


def get(url, headers=()):
    request = urllib.request.Request(url, headers={"user-agent": AGENT, **dict(headers)})
    with urllib.request.urlopen(request, timeout=60) as response:
        return response.read()


class Mirror(http.server.BaseHTTPRequestHandler):
    """A sparse registry of its own URL with crates.io's packages: its index files are
    crates.io's (whose dependencies name no registry, so this one), its `dl` has no markers."""

    def do_GET(self):
        try:
            if self.path == "/index/config.json":
                body = json.dumps({"dl": f"http://127.0.0.1:{MIRROR_PORT}/dl"}).encode()
            elif self.path.startswith("/index/"):
                body = get("https://index.crates.io/" + self.path[len("/index/"):])
            elif self.path.startswith("/dl/") and self.path.endswith("/download"):
                name, version = self.path.split("/")[2:4]
                body = get(f"https://static.crates.io/crates/{name}/{name}-{version}.crate")
            else:
                return self.send_error(404)
        except urllib.error.HTTPError as err:
            return self.send_error(err.code)
        etag = '"' + hashlib.sha256(body).hexdigest() + '"'
        if self.headers.get("if-none-match") == etag:
            self.send_response(304)
            self.end_headers()
            return
        self.send_response(200)
        self.send_header("content-length", str(len(body)))
        self.send_header("etag", etag)
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args):
        pass


class Cargo:
    def __init__(self, home):
        self.home = home
        self.env = {**os.environ, "CARGO_HOME": home, "RUSTC": tool("rustc"), "CARGO_TERM_COLOR": "never"}
        self.cargo = tool("cargo")

    def run(self, args, cwd):
        return subprocess.run([self.cargo, *args], cwd=cwd, env=self.env, capture_output=True, text=True)


def write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(text)


def project(dir, deps, cargo):
    """A package depending on `deps` (TOML lines), locked by cargo. Returns cargo's error, if any."""
    write(os.path.join(dir, "Cargo.toml"),
          '[package]\nname = "fetchdiff"\nversion = "0.0.0"\nedition = "2021"\n\n[dependencies]\n' + deps)
    write(os.path.join(dir, "src", "lib.rs"), "")
    if not os.path.exists(os.path.join(dir, "Cargo.lock")):
        made = cargo.run(["generate-lockfile"], dir)
        if made.returncode:
            return made.stderr.strip()[-2000:]
    return None


def popular(count):
    """The `count` most downloaded crates of crates.io, with their newest stable versions."""
    found = []
    page = 1
    while len(found) < count:
        listed = json.loads(get(f"https://crates.io/api/v1/crates?sort=downloads&per_page=100&page={page}"))
        if not listed["crates"]:
            break
        found += [(it["name"], it["max_stable_version"]) for it in listed["crates"] if it.get("max_stable_version")]
        page += 1
        time.sleep(1)
    return found[:count]


def clone(dir, repo, rev):
    if not os.path.exists(os.path.join(dir, ".git")):
        os.makedirs(dir, exist_ok=True)
        for args in (["init", "-q"], ["fetch", "-q", "--depth", "1", f"https://github.com/{repo}", rev],
                     ["checkout", "-q", "FETCH_HEAD"]):
            subprocess.run(["git", *args], cwd=dir, check=True)


def packages(lock):
    """A lock file's packages as (source, name, version), with the git ones apart."""
    import tomllib
    with open(lock, "rb") as f:
        listed = tomllib.load(f).get("package", [])
    registry, git = set(), set()
    for it in listed:
        source = it.get("source")
        if source and source.startswith(("registry+", "sparse+")):
            registry.add((source, it["name"], it["version"]))
        elif source and source.startswith("git+"):
            git.add((source, it["name"], it["version"]))
    return registry, git


def tree(root):
    """Every path under `root` with what has to be equal of it."""
    found = {}
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames.sort()
        for name in dirnames + filenames:
            path = os.path.join(dirpath, name)
            rel = os.path.relpath(path, root)
            st = os.lstat(path)
            mode = stat.S_IMODE(st.st_mode)
            if stat.S_ISLNK(st.st_mode):
                found[rel] = ("link", os.readlink(path))
            elif stat.S_ISDIR(st.st_mode):
                found[rel] = ("dir", oct(mode))
            elif name == ".cargo-ok" and dirpath == root:
                found[rel] = ("file", oct(mode), open(path, "rb").read().decode(errors="replace"))
            else:
                found[rel] = ("file", oct(mode), st.st_size, sha256(path), int(st.st_mtime))
    return found


def sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as f:
        while chunk := f.read(1 << 20):
            digest.update(chunk)
    return digest.hexdigest()


def listing(home, part):
    dir = os.path.join(home, "registry", part)
    return sorted(os.listdir(dir)) if os.path.isdir(dir) else []


def source_dir(home, source):
    """The directory cargo named for `source` in registry/src: the one of its host."""
    url = source.split("+", 1)[1]
    host = url.split("://", 1)[1].split("/", 1)[0].split(":")[0]
    if url == "https://github.com/rust-lang/crates.io-index":
        host = "index.crates.io"
    named = [it for it in listing(home, "src") if it.rsplit("-", 1)[0] == host]
    return named[0] if len(named) == 1 else None


def differences(cargo_home, blaze_home, source, name, version):
    dir = source_dir(cargo_home, source)
    if dir is None:
        return ["cargo has no single directory for its source"]
    package = f"{name}-{version}"
    left = os.path.join(cargo_home, "registry", "src", dir, package)
    right = os.path.join(blaze_home, "registry", "src", dir, package)
    if not os.path.isdir(right):
        return [f"blaze has no {dir}/{package}"]
    if not os.path.isdir(left):
        return [f"cargo has no {dir}/{package}"]
    found = []
    a, b = tree(left), tree(right)
    for rel in sorted(a.keys() | b.keys()):
        if a.get(rel) != b.get(rel):
            found.append(f"{rel}: cargo {a.get(rel)}, blaze {b.get(rel)}")
    crate = os.path.join("registry", "cache", dir, f"{package}.crate")
    have = [sha256(os.path.join(home, crate)) if os.path.exists(os.path.join(home, crate)) else None
            for home in (cargo_home, blaze_home)]
    if have[0] != have[1]:
        found.append(f"{crate}: cargo {have[0]}, blaze {have[1]}")
    return found


def blaze_fetch(given):
    if given:
        return given
    target = os.environ.get("CARGO_TARGET_DIR") or os.path.join(ROOT, "target")
    subprocess.run(["cargo", f"+{TOOLCHAIN}", "build", "--release", "-q", "-p", "blaze-registry", "--example", "fetch"],
                   cwd=ROOT, check=True)
    return os.path.join(target, "release", "examples", "fetch")


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--work", default=os.path.abspath("fetchdiff-work"))
    parser.add_argument("--popular", type=int, default=300)
    parser.add_argument("--projects", help="names of bench.py's PROJECTS, all of them without")
    parser.add_argument("--blaze", help="blaze-registry's `fetch` example, built here without")
    args = parser.parse_args()
    work = os.path.abspath(args.work)
    fetch = blaze_fetch(args.blaze)
    cargo_home, blaze_home = os.path.join(work, "cargo-home"), os.path.join(work, "blaze-home")
    for home in (cargo_home, blaze_home):
        shutil.rmtree(home, ignore_errors=True)
        os.makedirs(home)
    write(os.path.join(cargo_home, "config.toml"), f'[registries.mirror]\nindex = "{MIRROR}"\n')
    cargo = Cargo(cargo_home)
    server = http.server.ThreadingHTTPServer(("127.0.0.1", MIRROR_PORT), Mirror)
    threading.Thread(target=server.serve_forever, daemon=True).start()

    locks, failed = {}, {}
    names = args.projects.split(",") if args.projects is not None else None
    for name, (repo, rev, prepare) in projects().items():
        if names is not None and name not in names:
            continue
        if prepare:
            failed[name] = f"fetches from the tree {prepare}.py makes"
            continue
        dir = os.path.join(work, "projects", name)
        clone(dir, repo, rev)
        if not os.path.exists(os.path.join(dir, "Cargo.toml")):
            failed[name] = "no Cargo.toml at the root"
            continue
        if not os.path.exists(os.path.join(dir, "Cargo.lock")):
            made = cargo.run(["generate-lockfile"], dir)
            if made.returncode:
                failed[name] = made.stderr.strip()[-2000:]
                continue
        locks[name] = dir
    for name, version in popular(args.popular) if args.popular else ():
        dir = os.path.join(work, "popular", name)
        if error := project(dir, f'{name} = "={version}"\n', cargo):
            failed[f"{name} {version}"] = error
        else:
            locks[f"{name} {version}"] = dir
    dir = os.path.join(work, "mirror")
    deps = "".join(f'{name} = {{ version = "{req}", registry = "mirror" }}\n' for name, req in MIRROR_CRATES.items())
    if error := project(dir, deps, cargo):
        failed["mirror"] = error
    else:
        locks["mirror"] = dir

    seconds = {"cargo": 0.0, "blaze": 0.0}
    registry, git, fetched = set(), set(), {}
    for name, dir in locks.items():
        start = time.monotonic()
        ran = cargo.run(["fetch", "--locked"], dir)
        seconds["cargo"] += time.monotonic() - start
        if ran.returncode:
            failed[name] = "cargo fetch: " + ran.stderr.strip()[-2000:]
            continue
        start = time.monotonic()
        ran = subprocess.run([fetch, os.path.join(dir, "Cargo.lock"), blaze_home], capture_output=True, text=True)
        seconds["blaze"] += time.monotonic() - start
        if ran.returncode:
            failed[name] = "blaze fetch: " + ran.stderr.strip()[-2000:]
            continue
        for kind, count in json.loads(ran.stdout.strip().splitlines()[-1]).items():
            fetched[kind] = fetched.get(kind, 0) + count
        found, from_git = packages(os.path.join(dir, "Cargo.lock"))
        registry |= found
        git |= from_git
    server.shutdown()

    unequal = {}
    for source, name, version in sorted(registry):
        if found := differences(cargo_home, blaze_home, source, name, version):
            unequal[f"{name} {version} ({source})"] = found
    dirs = {part: (listing(cargo_home, part), listing(blaze_home, part)) for part in ("index", "cache", "src")}
    dirs_equal = all(a == b for a, b in dirs.values())
    result = {
        "locks": len(locks), "packages": len(registry), "equal": len(registry) - len(unequal), "git": len(git),
        "dirs": dirs, "dirs_equal": dirs_equal, "blaze": fetched, "seconds": seconds,
        "unequal": unequal, "failed": failed,
    }
    with open(os.path.join(work, "fetchdiff.json"), "w") as f:
        json.dump(result, f, indent=1)
    print(f"{len(locks)} lock files, {len(failed)} not fetched (fetchdiff.json says why)")
    print(f"{result['equal']}/{len(registry)} registry packages equal; {len(git)} git packages not compared")
    for part, (a, b) in dirs.items():
        print(f"registry/{part}: {'equal' if a == b else 'cargo ' + ', '.join(a) + '; blaze ' + ', '.join(b)}")
    print(f"cargo fetch {seconds['cargo']:.1f} s, blaze {seconds['blaze']:.1f} s")
    for package, found in list(unequal.items())[:20]:
        print(f"  {package}: {found[0]}" + (f" (and {len(found) - 1} more)" if len(found) > 1 else ""))
    sys.exit(1 if unequal or not dirs_equal else 0)


if __name__ == "__main__":
    main()

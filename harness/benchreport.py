#!/usr/bin/env python3
"""benchreport.py: the README's build table and docs/benchmarks.md, from a result of bench.py.

    benchreport.py RESULT [--stored FILE] [--noise PERCENT] [--max-age HOURS] [--force] [--check]

RESULT is what `bench.py --out` wrote. It is compared with the stored result (`--stored`,
default docs/benchmarks/linux-x86_64.json). When the two differ by more than noise, the stored
one is replaced and the parts of README.md and docs/benchmarks.md between
`<!-- bench:start -->` and `<!-- bench:end -->` are written again from it; otherwise nothing is
written. More than noise is any of:
  - a median time of either side, in any phase, moved by more than `--noise` percent (default
    10) and by more than 50 ms
  - a phase the daemon built is now built by cargo, or the other way round
  - a project or backend is new or gone
  - the stored result is older than `--max-age` hours (default 24), so that the commit the
    README names is never more than a day behind main
  - `--force`
A result of `bench.py --runtime` (harness/runbench.py's numbers) writes the README's second
table instead, between `<!-- runbench:start -->` and `<!-- runbench:end -->`: how fast each
benchmark's program runs built by ra-jit's LLVM against `cargo build --release`, and the
geometric mean. It is stored in docs/benchmarks/runtime-<os>-<arch>.json (`--stored` to say
otherwise), and differs when a median of any build moved by more than noise, a benchmark is new
or gone, or the stored run is older than `--max-age`. A build that failed, or whose program did
not do what the release build's did, has the whole result refused.
A result in which a program said something else than cargo's (`wrong`) is refused: exit 2,
nothing written. A test suite that ended otherwise under `cargo ra-jit test` is not refused: the
tests table says how, and the workflow fails after writing. `--check` writes nothing and exits 1
when it would have written.

Memory and disk (bench.py's `memory`, `resident` and `disk_mb`, docs/benchmarks.md "Memory and
disk") are written beside the times: a table per backend in docs/benchmarks.md, and a small
table in the README between `<!-- benchmem:start -->` and `<!-- benchmem:end -->`. A peak of
either side moving by more than noise and 100 MB counts as a change. `--memory` writes only
those two (README's block, and docs/benchmarks.md's block of the same markers) from RESULT and
stores it in docs/benchmarks/memory-<os>-<arch>.json: the way to publish a run measured by hand
on another machine without touching the build table, which stays the benchmark machine's.

Every table of the README comes with its chart (benchcharts.py), written to
docs/benchmarks/charts/ by the same call that writes the table: the edit speed-ups with the
build table, memory with the memory table, the runtime ratios with the runtime table. `--charts`
writes all three again from the stored results, and nothing else.

The last line says `changed` or `unchanged`, and so does `changed=true|false` in
$GITHUB_OUTPUT when that is set.
"""
import datetime, glob, json, math, os, re, shutil, sys

import benchcharts

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
STORED = os.path.join(ROOT, "docs/benchmarks/linux-x86_64.json")
README = os.path.join(ROOT, "README.md")
DOC = os.path.join(ROOT, "docs/benchmarks.md")
START, END = "<!-- bench:start -->", "<!-- bench:end -->"
RUN_START, RUN_END = "<!-- runbench:start -->", "<!-- runbench:end -->"
MEM_START, MEM_END = "<!-- benchmem:start -->", "<!-- benchmem:end -->"
FLOOR_MB = 100
REPO = "https://github.com/indexable-inc/ra-jit"
FLOOR_S = 0.05

# The order of the projects and phases in the tables, and what the README calls each edit.
PROJECTS = ["ripgrep", "fd", "bat", "hyperfine", "just", "axum", "nushell", "helix", "bevy", "zed", "cargo"]
EDITS = {
    "body": "Edit a function",
    "leaf": "Edit a widely used function",
    "signature": "Change a signature",
    "generic": "Edit a generic function",
    "newfn": "Add a function",
    "struct": "Add a struct field",
    "no-op": "Nothing changed",
}
DOC_PHASES = ["body", "leaf", "signature", "generic", "newfn", "struct", "no-op"]
# What docs/benchmarks.md calls each setup of bench.py, in the order of its table.
SETUPS = {
    "check": "`cargo check`",
    "mold": "mold",
    "wild": "wild",
    "threads8": "`-Zthreads=8`",
    "nightly": "nightly",
    "cranelift": "nightly, Cranelift",
    "fastest": "nightly, Cranelift, wild, `-Zthreads=8`",
}
README_PHASES = ["body", "leaf", "signature", "no-op"]
BACKEND_NAMES = {"cranelift": "Cranelift", "llvm": "LLVM"}


def seconds(value):
    if value is None:
        return "-"
    return f"{value * 1000:.0f} ms" if value < 1 else f"{value:.2f} s"


def speedup(cargo, ours):
    if not cargo or not ours:
        return ""
    ratio = cargo / ours
    return f"{ratio:.0f}x" if ratio >= 10 else f"{ratio:.1f}x"


def spread(samples):
    if not samples:
        return ""
    return f"{seconds(min(samples))} to {seconds(max(samples))}"


def ordered(result, backend):
    keys = [k for k in result["results"] if k.endswith("/" + backend)]
    rank = lambda k: (PROJECTS.index(k.split("/")[0]) if k.split("/")[0] in PROJECTS else len(PROJECTS), k)
    return sorted(keys, key=rank)


def project_name(key, measured):
    tag = measured.get("tag", "")
    return f"{key.split('/')[0]} {tag.lstrip('v').removeprefix('axum-v')}".strip()


def failure(key, measured):
    """What failed and where, in the project's own paths: a line for under the tables."""
    said = [line for line in measured["failed"].splitlines()[1:] if line.strip()] or measured["failed"].splitlines()
    said = re.sub(r"/\S*?/[\w.-]+-(cranelift|llvm)/(target/\S*?: )?", "", said[-1].removeprefix("error: "))
    said = said.removeprefix("ra-jit did not build ")
    if len(said) > 140:
        said = said[:140].rsplit(" ", 1)[0]
        said += "`" * (said.count("`") % 2) + " …"
    return f"{project_name(key, measured)}: only the steps shown are measured, then the build failed: {said}"


def failures(result, keys):
    failed = [failure(k, result["results"][k]) for k in keys if result["results"][k].get("failed")]
    return ["", *(f"- {line}" for line in failed)] if failed else []


def cargo_built(phase):
    return phase is not None and phase.get("built") != "daemon"


def short(commit):
    return (commit or "unknown")[:10]


def machine_line(result):
    host = result.get("host") or {}
    cores, of = host.get("cores") or result.get("cores"), host.get("machine_cores")
    cpu = re.sub(r"( \d+-Core)? Processor$", "", host.get("cpu") or "") or None
    details = [p for p in (cpu, f"{cores} of its {of} cores" if of and of != cores else f"{cores} cores",
                           f"{host['memory_gib']} GiB" if host.get("memory_gib") else None) if p]
    return f"{host.get('name') or 'one machine'} ({', '.join(details)})"


def provenance(result, which=True):
    """Who ran it: the CI run, or a person with the same harness; with `which`, which build of
    ra-jit it timed."""
    built = (f" The ra-jit measured is `cargo build --release`; the binaries a release ships are the `dist` "
             f"profile with PGO, which start and edit faster ([how]({REPO}/blob/main/docs/reports/RELEASE-BUILD.md)).") if which else ""
    if result.get("run_url"):
        return f"Measured by [CI]({result['run_url']}) on the dedicated benchmark machine, which re-runs it after pushes to main." + built
    return "Measured by hand with `harness/bench.py`; the `bench-readme` workflow re-runs it on the dedicated benchmark machine." + built


def megabytes(mb):
    if mb is None:
        return "-"
    return f"{mb / 1000:.1f} GB" if mb >= 1000 else f"{mb:,.0f} MB"


def has_memory(result):
    return any(phase.get("memory") for measured in result["results"].values() for phase in measured["phases"].values())


def where(result):
    """The machine, commit and date of a result, for a chart's caption."""
    host = result.get("host") or {}
    name = host.get("name") or re.sub(r"( \d+-Core)? Processor$", "", host.get("cpu") or "") or "one machine"
    return f"{platform(result)}, {name}, ra-jit {short(result.get('commit'))}, {result['date'][:10]}."


def edits_chart(result, backend="cranelift"):
    rows = []
    for key in ordered(result, backend):
        measured = result["results"][key]
        ratios = [p["cargo_s"] / p["wall_s"] for label, p in measured["phases"].items()
                  if label in EDITS and label != "no-op" and p.get("cargo_s") and p.get("wall_s")]
        rows.append((project_name(key, measured), ratios))
    return benchcharts.edits(rows, where(result))


def memory_chart(result, backend):
    rows = []
    for key in ordered(result, backend):
        measured = result["results"][key]
        phase = next((measured["phases"][l] for l in DOC_PHASES if l != "no-op" and (measured["phases"].get(l) or {}).get("memory")), None)
        if phase:
            rows.append((project_name(key, measured), phase["memory"]["cargo"]["peak_mb"], phase["memory"]["ra-jit"]["peak_mb"]))
    return benchcharts.memory(rows, where(result))


def memory_backend(result):
    with_memory = [b for b in BACKEND_NAMES if any(k.endswith("/" + b) and any(p.get("memory") for p in m["phases"].values())
                                                   for k, m in result["results"].items())]
    return with_memory[0] if with_memory else None


def memory_readme(result):
    """The README's memory table: what an edit and a cold build held at their peak, what the
    daemon holds between edits, and what each side keeps on disk."""
    backend = memory_backend(result)
    keys = [k for k in ordered(result, backend) if any(p.get("memory") for p in result["results"][k]["phases"].values())]
    summary = memory_chart(result, backend)
    commit = result.get("commit")
    lines = [MEM_START, benchcharts.picture("memory", f"Peak memory while an edit to one function builds, cargo and ra-jit: {summary}"), "",
             f"Memory and disk, {BACKEND_NAMES[backend]}. Peak: the most every process doing the work held at once, cargo → "
             f"ra-jit. For ra-jit that is the daemon, its workers and what they run, and `cargo ra-jit build` with any cargo it "
             f"ran; for plain cargo, cargo itself, rustc, build scripts and the linker. On disk: cargo's target directory → ra-jit's target "
             f"directory, store and home. {platform(result)}, {machine_line(result)}, ra-jit [`{short(commit)}`]({REPO}/commit/{commit}) "
             f"on {result['date'][:10]}"
             + (f", load average {result['host']['load'].split(', ')[0]} when it started" if (result.get("host") or {}).get("load") else "")
             + "; how it is measured: [docs/benchmarks.md](docs/benchmarks.md#memory-and-disk).", "",
             "| Project | Edit a function, peak | Daemon between edits | Cold until ready, peak | On disk |",
             "|---|---:|---:|---:|---:|"]
    for key in keys:
        measured = result["results"][key]
        phases = measured["phases"]
        edit = next((phases[l]["memory"] for l in DOC_PHASES if l != "no-op" and (phases.get(l) or {}).get("memory")), None)
        cold = (phases.get("cold") or {}).get("memory")
        pair = lambda m: f"{megabytes(m['cargo']['peak_mb'])} → **{megabytes(m['ra-jit']['peak_mb'])}**" if m else ""
        resident = measured.get("resident") or {}
        held = (f"{megabytes(resident['ready_mb'])} ready, {megabytes(resident['end_mb'])} after {resident['builds']} builds"
                if resident.get("end_mb") is not None else megabytes(resident.get("ready_mb")) if resident else "")
        disk = measured.get("disk_mb") or {}
        ours = sum(disk.get(k) or 0 for k in ("target", "store", "home"))
        on_disk = f"{megabytes(disk.get('cargo target'))} → {megabytes(ours)}" if disk else ""
        lines.append(f"| {project_name(key, measured)} | {pair(edit)} | {held} | {pair(cold)} | {on_disk} |")
    lines += failures(result, keys)
    lines.append(MEM_END)
    return "\n".join(lines)


def memory_tables(result, keys, backend):
    """docs/benchmarks.md's memory tables of one backend: every step's peaks, the daemon between
    builds, and the disk."""
    keys = [k for k in keys if any(p.get("memory") for p in result["results"][k]["phases"].values())]
    if not keys:
        return []
    lines = ["", f"#### Memory and disk, {BACKEND_NAMES[backend]}", "",
             "Peak: the most every process doing the work held at once, sampled every 10 ms and never below the largest "
             "process, median of the runs. Largest process: the peak of the one process that held the most, as the kernel "
             "kept it: `wait4`'s for the command's tree, and each process's peak sampled (on macOS the kernel's peak footprint "
             "of each, counting for the daemon only a peak it passed during the step). ra-jit is the daemon, its workers and "
             "what they run, and the `cargo ra-jit build` with any cargo it ran; cargo is cargo itself, rustc, build scripts "
             "and the linker. Daemon after: what the daemon and its workers hold once the step is over ([how](#memory-and-disk)).", "",
             "| Project | Step | cargo, peak | ra-jit, peak | cargo, largest process | ra-jit, largest process | Daemon after |",
             "|---|---|---:|---:|---:|---:|---:|"]
    for key in keys:
        measured = result["results"][key]
        for label in ["cold", *DOC_PHASES]:
            m = (measured["phases"].get(label) or {}).get("memory")
            if not m:
                continue
            step = "Cold, until the daemon is ready" if label == "cold" else EDITS[label]
            lines.append(f"| {project_name(key, measured)} | {step} | {megabytes(m['cargo']['peak_mb'])} | {megabytes(m['ra-jit']['peak_mb'])} | "
                         f"{megabytes(m['cargo']['largest_mb'])} | {megabytes(m['ra-jit']['largest_mb'])} | {megabytes(m['ra-jit'].get('resident_mb'))} |")
    lines += ["", "The daemon between builds, and what each side keeps on disk once the edits are done (as `du` counts it). "
              "After the first build: once the step after the cold build (nothing changed) is done; growth is from then to the end, "
              "over every edit made and taken back. Warm store: the cold start of another checkout with the store the first one filled.", "",
              "| Project | Daemon ready | After the first build | At the end | Growth | Warm store: peak, ready | cargo target | ra-jit target | Store | Home |",
              "|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|"]
    for key in keys:
        measured = result["results"][key]
        resident, disk = measured.get("resident") or {}, measured.get("disk_mb") or {}
        first = (resident.get("after") or [{}])[0]
        end = f"{megabytes(resident['end_mb'])} after {resident.get('builds')} builds" if resident.get("end_mb") is not None else "-"
        growth = (resident["end_mb"] - first["mb"]) if resident.get("end_mb") is not None and first.get("mb") is not None else None
        warm = (measured["phases"].get("cold") or {}).get("warm_store_memory")
        lines.append(f"| {project_name(key, measured)} | {megabytes(resident.get('ready_mb'))} | {megabytes(first.get('mb'))} | {end} | "
                     f"{'-' if growth is None else ('+' if growth >= 0 else '−') + megabytes(abs(growth))} | "
                     f"{megabytes(warm['peak_mb']) + ', ' + megabytes(warm['resident_mb']) if warm else '-'} | "
                     f"{megabytes(disk.get('cargo target'))} | {megabytes(disk.get('target'))} | {megabytes(disk.get('store'))} | {megabytes(disk.get('home'))} |")
    return lines


def readme_section(result):
    backend = "cranelift"
    keys = ordered(result, backend)
    commit = result.get("commit")
    summary = edits_chart(result, backend)
    lines = [
        START,
        *([benchcharts.picture("edits", f"How many times faster an edit builds under ra-jit than under cargo: {summary}"), ""] if summary else []),
        f"Edit, then build, on real projects at pinned commits. Each cell is incremental `cargo build` → "
        f"`cargo ra-jit build`, both warm, same linker, median of {result['runs']} edits. Linux x86-64, "
        f"{machine_line(result)}, ra-jit [`{short(commit)}`]({REPO}/commit/{commit}) on {result['date'][:10]}.",
        "",
        "| Project | " + " | ".join(EDITS[p] for p in README_PHASES) + " |",
        "|---|" + "---:|" * len(README_PHASES),
    ]
    fell_back = False
    for key in keys:
        measured = result["results"][key]
        cells = []
        for label in README_PHASES:
            phase = measured["phases"].get(label)
            if phase is None:
                cells.append("")
                continue
            mark = ""
            if cargo_built(phase):
                mark, fell_back = " (c)", True
            cells.append(f"{seconds(phase['cargo_s'])} → **{seconds(phase['wall_s'])}**{mark} ({speedup(phase['cargo_s'], phase['wall_s'])})")
        lines.append(f"| {project_name(key, measured)} | " + " | ".join(cells) + " |")
    lines += failures(result, keys)
    lines.append("")
    if fell_back:
        lines.append("(c): the daemon handed this edit to cargo (a fallback, so about cargo's time).")
        lines.append("")
    lines.append(provenance(result) + " Cold builds, the spread of every number, the LLVM tier and how fast the "
                 "built programs run: [docs/benchmarks.md](docs/benchmarks.md) ([raw JSON](docs/benchmarks/linux-x86_64.json)).")
    lines.append(END)
    return "\n".join(lines)


def doc_section(result):
    commit = result.get("commit")
    host = result.get("host") or {}
    lines = [START, f"### Latest run", "",
             f"ra-jit [`{short(commit)}`]({REPO}/commit/{commit}), {result['date']}, Rust {result['toolchain']}, "
             f"{result['runs']} runs of each edit. Raw: [benchmarks/linux-x86_64.json](benchmarks/linux-x86_64.json). "
             f"{provenance(result)}", "",
             "| Machine | |", "|---|---|"]
    for label, field in (("Name", "name"), ("CPU", "cpu"), ("Cores", "cores"), ("Memory (GiB)", "memory_gib"), ("Kernel", "kernel"),
                         ("rustc", "rustc"), ("C compiler", "cc"), ("Linker", "ld"), ("LLVM", "llvm"),
                         ("Incremental", "incremental"), ("Compiler cache", "wrapper")):
        value = host.get(field)
        lines.append(f"| {label} | {value if value is not None else 'none' if field == 'wrapper' else '-'} |")
    lines.append("")
    for backend in [b for b in BACKEND_NAMES if any(k.endswith("/" + b) for k in result["results"])]:
        keys = ordered(result, backend)
        lines += [f"#### Edits, {BACKEND_NAMES[backend]}", "",
                  "Median, then the fastest and slowest run. Built: who built the edit (daemon, or cargo and why). "
                  "Link: the change was spliced into the program, or it was linked again.", "",
                  "| Project | Edit | cargo | ra-jit | | cargo runs | ra-jit runs | Built | Link |",
                  "|---|---|---:|---:|---:|---|---|---|---|"]
        for key in keys:
            measured = result["results"][key]
            for label in DOC_PHASES:
                phase = measured["phases"].get(label)
                if phase is None:
                    continue
                lines.append(f"| {project_name(key, measured)} | {EDITS[label]} | {seconds(phase['cargo_s'])} | {seconds(phase['wall_s'])} | "
                             f"{speedup(phase['cargo_s'], phase['wall_s'])} | {spread(phase.get('cargo_samples_s'))} | {spread(phase.get('samples_s'))} | "
                             f"{phase['built'][:80]} | {phase.get('link') or ''} |")
        lines += failures(result, keys)
        cold_runs = result.get("cold_runs", 1)
        lines += ["", f"#### Cold, {BACKEND_NAMES[backend]}", "",
                  "From an empty target directory, crates already downloaded, " + ("one run." if cold_runs == 1 else
                  f"median of {cold_runs} runs, each from nothing.") + " `cargo ra-jit build` is cargo's own build (the daemon is "
                  "not loaded yet); the daemon then loads the workspace and builds it once, and edits are fast from then on. "
                  "Warm store: the same in another checkout of the same commit, with the store the first one filled.", "",
                  "| Project | `cargo build` | `cargo ra-jit build` | Daemon ready, empty store | Daemon ready, warm store | "
                  "Functions compiled | Daemon memory | Store |",
                  "|---|---:|---:|---:|---:|---:|---:|---:|"]
        for key in keys:
            measured = result["results"][key]
            cold = measured["phases"].get("cold")
            if cold:
                ready = lambda total, samples: seconds(total) + (f" ({spread(samples)})" if samples and len(samples) > 1 else "")
                store = f"{measured['store_mb']:,} MB" if measured.get("store_mb") is not None else "-"
                runs = measured.get("cold_runs", cold_runs)
                name = project_name(key, measured) + ("" if runs == cold_runs else " (one run)" if runs == 1 else f" ({runs} runs)")
                lines.append(f"| {name} | {ready(cold['cargo_s'], cold.get('cargo_samples_s'))} | "
                             f"{ready(cold.get('build_s'), cold.get('build_samples_s'))} | {ready(cold.get('wall_s'), cold.get('samples_s'))} | "
                             f"{ready(cold.get('warm_store_s'), cold.get('warm_store_samples_s'))} | "
                             f"{cold.get('compiled', 0):,} | {cold.get('rss_mb', 0):,} MB | {store} |")
        lines += memory_tables(result, keys, backend) + setups_table(result, keys, backend) + tests_table(result, keys, backend)
        timed = [k for k in keys if result["results"][k].get("runtime")]
        if timed:
            lines += ["", f"#### How fast the program runs, {BACKEND_NAMES[backend]}", "",
                      "The same command run by the program ra-jit linked and by cargo's debug and release builds, in turn. "
                      "Median, then fastest and slowest; where measured, then the program's peak resident memory while it ran "
                      "(median of the runs, as the kernel kept it) and the size of the program.", "",
                      "| Project | Command | ra-jit | cargo debug | cargo release |", "|---|---|---:|---:|---:|"]
            for key in timed:
                run = result["results"][key]["runtime"]
                cell = lambda r: f"{seconds(r['median_s'])} ({seconds(r['min_s'])} to {seconds(r['max_s'])})" + (
                    f"; {megabytes(r['peak_mb'])}, {r['size_mb']} MB" if r.get("peak_mb") is not None else "")
                command = " ".join(run["command"]).replace("{library}", "<the toolchain's library source>").replace("|", "\\|")
                lines.append(f"| {project_name(key, result['results'][key])} | `{command}` | {cell(run['ra-jit'])} | "
                             f"{cell(run['cargo debug'])} | {cell(run['cargo release'])} |")
        lines.append("")
    lines.append(END)
    return "\n".join(lines)


def setups_table(result, keys, backend):
    """The same steps built the fastest ways there are besides ra-jit, for the projects that had them."""
    keys = [k for k in keys if any(p.get("setups") for p in result["results"][k]["phases"].values())]
    if not keys:
        return []
    names = [s for s in SETUPS if any(s in (p.get("setups") or {}) for k in keys for p in result["results"][k]["phases"].values())]
    nightly = (result.get("host") or {}).get("nightly")
    lines = ["", f"#### Against the fastest setups, {BACKEND_NAMES[backend]}", "",
             "The same steps, each built after plain cargo by the fastest ways to build Rust there already are, each into a "
             "target directory of its own, median of the same runs. Plain cargo is incremental, with the default linker. "
             f"mold and wild link instead of GNU ld; `-Zthreads=8` is the parallel front end on the same compiler; the nightly ({nightly or 'see the JSON'}) "
             "is there to tell its own difference from Cranelift's (rustc_codegen_cranelift comes only with nightlies); `cargo check` "
             "builds no code, so nothing can be run after it. The fastest of each row is in bold; \"failed\" is a setup that could "
             "not build the project, and is not timed after that.", "",
             "| Project | Step | cargo | " + " | ".join(SETUPS[n] for n in names) + " | ra-jit |",
             "|---|---|---:|" + "---:|" * (len(names) + 1)]
    for key in keys:
        measured = result["results"][key]
        for label in ["cold", *DOC_PHASES]:
            phase = measured["phases"].get(label)
            if not phase or not phase.get("setups"):
                continue
            ours = phase["wall_s"] if label != "cold" else phase.get("wall_s")
            times = {"cargo": phase["cargo_s"], "ra-jit": ours}
            times.update({n: phase["setups"].get(n, {}).get("median_s") for n in names})
            # `cargo check` builds nothing to run, so it is not what the fastest is chosen from.
            fastest = min((v, n) for n, v in times.items() if v is not None and n != "check")[1]
            cell = lambda n: ("failed" if (phase["setups"].get(n) or {}).get("failed") else "-") if times[n] is None else (
                f"**{seconds(times[n])}**" if n == fastest else seconds(times[n]))
            step = "Cold, until the daemon is ready" if label == "cold" else EDITS[label]
            lines.append(f"| {project_name(key, measured)} | {step} | {cell('cargo')} | " + " | ".join(cell(n) for n in names) + f" | {cell('ra-jit')} |")
    failures = sorted({f"{project_name(k, result['results'][k])}, {SETUPS[n]}: {s['failed']}" for k in keys
                       for p in result["results"][k]["phases"].values() for n, s in (p.get("setups") or {}).items() if s.get("failed")})
    if failures:
        lines += [""] + [f"- {f}" for f in failures]
    return lines


def said_otherwise(difference):
    """How a test suite ended otherwise under ra-jit, for a table cell."""
    if found := re.fullmatch(r"(-?\d+) for (-?\d+)", difference or ""):
        return f"`cargo ra-jit test` exited {found[1]}, `cargo test` {found[2]}"
    return (difference or "").replace("|", "\\|")[:120]


def tests_table(result, keys, backend):
    keys = [k for k in keys if result["results"][k]["phases"].get("test")]
    if not keys:
        return []
    lines = ["", f"#### Test suites, {BACKEND_NAMES[backend]}", "",
             "`cargo ra-jit test` and `cargo test` of the same tests, median of three: after the edit of a widely used function "
             "(the file named), and again with nothing changed. Each includes running the tests. First runs: the three untimed "
             "runs of `cargo ra-jit test` before them (the first is handed to cargo, the daemon builds the test targets at the "
             "second). Every test has to end the same way under both, every time.", "",
             "| Project | Command | Tests | Same outcome | Edited | Edit, then test: cargo | ra-jit | | Built | Nothing changed: cargo | ra-jit | First runs of ra-jit |",
             "|---|---|---:|---|---|---:|---:|---:|---|---:|---:|---|"]
    for key in keys:
        test = result["results"][key]["phases"]["test"]
        if "edit_s" not in test:
            continue
        lines.append(f"| {project_name(key, result['results'][key])} | `cargo {' '.join(test['command'])}` | {test['tests']:,} | "
                     f"{'yes' if test['same'] else 'no: ' + said_otherwise(test.get('difference'))} | `{test['edited']}` | "
                     f"{seconds(test['edit_cargo_s'])} | {seconds(test['edit_s'])} | {speedup(test['edit_cargo_s'], test['edit_s'])} | {test['built'][:60]} | "
                     f"{seconds(test['cargo_s'])} | {seconds(test['wall_s'])} | {', '.join(seconds(s) for s in test['first_s'])} |")
    return lines


def platform(result):
    host = result.get("host") or {}
    system = {"Linux": "Linux", "Darwin": "macOS"}.get(host.get("os"), host.get("os") or "unknown OS")
    arch = {"x86_64": "x86-64", "AMD64": "x86-64", "arm64": "arm64", "aarch64": "arm64"}.get(host.get("arch"), host.get("arch") or "")
    return f"{system} {arch}".strip()


def runtime_wrong(result):
    """Why a runtime result may not be published: a build that failed or whose program
    behaved otherwise than cargo's release build."""
    wrong = []
    for name, program in (result.get("runbench") or {}).get("programs", {}).items():
        for build, info in program["builds"].items():
            if info.get("error"):
                wrong.append(f"{name} {build}: the build failed")
            for bench in info.get("differs", []):
                wrong.append(f"{bench} {build}: the program did not do what cargo's release build did")
        for bench, timed in program.get("benches", {}).items():
            for build in ("release", "ra-jit"):
                if build not in timed:
                    wrong.append(f"{bench} {build}: not timed")
    return wrong


def times(ratio):
    return f"{ratio:.2f}x" if ratio < 10 else f"{ratio:.0f}x"


def runtime_chart(measured, where_text):
    """The runtime chart of a runbench.py result: of its ra-jit builds, the one whose geometric
    mean is lowest, against the release build, per benchmark it ran."""
    ours = [b for b in measured["builds"] if b not in ("release", "checked")]
    ratios = lambda b: [(bench, row[b]["ratio"]) for p in measured["programs"].values() for bench, row in p["benches"].items() if b in row]
    mean = lambda rows: math.exp(sum(math.log(v) for _, v in rows) / len(rows)) if rows else None
    scored = [(mean(ratios(b)), b) for b in ours if ratios(b)]
    if not scored:
        return None
    _, build = min(scored)
    variant = (measured.get("variants") or {}).get(build)
    env = {"RA_JIT_OPT": "3", **(variant or {})}
    named = ", ".join([f"-O{env.pop('RA_JIT_OPT')}", *(f"{k}={v}" for k, v in env.items())])
    return benchcharts.runtime(ratios(build), mean(ratios(build)), named, where_text)


def runtime_section(result):
    """The README's table of how fast what ra-jit's LLVM builds runs, against cargo's release build."""
    measured, commit = result["runbench"], result.get("commit")
    builds = measured["builds"]
    checked = "checked" in builds
    summary = runtime_chart(measured, where(result))
    rows, rounds, sized = [], None, []
    mem = lambda row, b: megabytes(row[b]["peak_mb"]) if row.get(b, {}).get("peak_mb") is not None else "-"
    for name, program in measured["programs"].items():
        for bench, row in program["benches"].items():
            rounds = len(row["release"]["runs"])
            rows.append(f"| {bench} | {seconds(row['release']['median_s'])} | **{seconds(row['ra-jit']['median_s'])}** | "
                        f"{times(row['ra-jit']['ratio'])} |" + (f" {times(row['checked']['ratio'])} |" if checked else "") +
                        f" {mem(row, 'release')} → {mem(row, 'ra-jit')} |")
        info = program["builds"]
        if info.get("release", {}).get("size_mb") is not None and info.get("ra-jit", {}).get("size_mb") is not None:
            sized.append(f"| {name} | {info['release']['size_mb']} MB → {info['ra-jit']['size_mb']} MB | "
                         f"{megabytes(info['release'].get('build_peak_mb'))} → {megabytes(info['ra-jit'].get('build_peak_mb'))} | "
                         f"{megabytes(info['release'].get('build_largest_mb'))} → {megabytes(info['ra-jit'].get('build_largest_mb'))} |")
    geomean = measured.get("geomean") or {}
    lines = [
        RUN_START,
        *([benchcharts.picture("runtime", f"Time of each benchmark, ra-jit's build ÷ cargo build --release: {summary}"), ""] if summary else []),
        f"How fast the programs run. Each program is built by `cargo build --release` and by ra-jit with LLVM at "
        f"`-O3`, every crate of it compiled by ra-jit, with the release profile; each benchmark is then run "
        f"{'once' if rounds == 1 else f'{rounds} times, the builds taking turns, median'}. A run is published only "
        f"when every program did what the release build's did. {platform(result)}, {machine_line(result)}, ra-jit "
        f"[`{short(commit)}`]({REPO}/commit/{commit}) on {result['date'][:10]}.",
        "",
        "| Benchmark | `cargo build --release` | ra-jit, LLVM | ra-jit ÷ release |" + (" release with dev checks ÷ release |" if checked else "")
        + " Peak memory, release → ra-jit |",
        "|---|---:|---:|---:|" + ("---:|" if checked else "") + "---:|",
        *rows,
    ]
    if geomean.get("ra-jit"):
        lines.append(f"| **Geometric mean** | | | **{times(geomean['ra-jit'])}** |" +
                     (f" {times(geomean['checked'])} |" if checked and geomean.get("checked") else (" |" if checked else "")) + " |")
    if sized:
        lines += ["", "What each program takes, release → ra-jit: its size, and its build's memory (the most the build's processes "
                  "held at once, and the largest one process: rustc or ra-jit, or the linker).", "",
                  "| Program | Size | Build, peak | Build, largest process |", "|---|---:|---:|---:|", *sized]
    lines += ["", "Release with dev checks: `cargo build --release` with `-C overflow-checks=on -C debug-assertions=on`. "
              + (provenance(result, which=False).replace("`harness/bench.py`", "`harness/bench.py --runtime`") if result.get("run_url") else
                 "Measured by hand with `harness/bench.py --runtime` (harness/runbench.py); the `bench-readme` workflow "
                 "re-runs it on the dedicated benchmark machine."), RUN_END]
    return "\n".join(lines)


def runtime_differences(stored, result, noise, max_age_h):
    if stored is None or not stored.get("runbench"):
        return ["nothing stored"]
    why = []
    when = lambda r: datetime.datetime.strptime(r["date"], "%Y-%m-%dT%H:%M:%SZ").replace(tzinfo=datetime.timezone.utc)
    if (when(result) - when(stored)).total_seconds() / 3600 > max_age_h:
        why.append("the stored result is old")
    rows = lambda r: {(p, b): row for p, prog in r["runbench"]["programs"].items() for b, row in prog["benches"].items()}
    was, now = rows(stored), rows(result)
    if set(was) != set(now):
        why.append(f"benchmarks: {sorted(was)} -> {sorted(now)}")
    for key, row in now.items():
        for build, cell in row.items():
            old = was.get(key, {}).get(build)
            if old and abs(cell["median_s"] - old["median_s"]) / old["median_s"] * 100 > noise:
                why.append(f"{key[1]} {build}: {old['median_s']} -> {cell['median_s']}")
    return why


def splice(path, section, start=START, end=END):
    text = open(path).read()
    pattern = re.compile(re.escape(start) + ".*?" + re.escape(end), re.S)
    if not pattern.search(text):
        raise SystemExit(f"{path} has no {start} ... {end} section")
    open(path, "w").write(pattern.sub(lambda _: section, text, count=1))


def differences(stored, result, noise, max_age_h):
    """Why `result` is worth writing over `stored`; empty when it is within noise."""
    if stored is None:
        return ["nothing stored"]
    why = []
    if set(stored["results"]) != set(result["results"]):
        why.append(f"projects: {sorted(stored['results'])} -> {sorted(result['results'])}")
    when = lambda r: datetime.datetime.strptime(r["date"], "%Y-%m-%dT%H:%M:%SZ").replace(tzinfo=datetime.timezone.utc)
    age = (when(result) - when(stored)).total_seconds() / 3600
    if age > max_age_h:
        why.append(f"the stored result is {age:.0f} h old")
    for key, now in result["results"].items():
        was = stored["results"].get(key)
        if was is None:
            continue
        for label, phase in now["phases"].items():
            old = was["phases"].get(label)
            if old is None:
                why.append(f"{key} {label}: new")
                continue
            for field in ("wall_s", "cargo_s"):
                before, after = old.get(field), phase.get(field)
                if before and after and abs(after - before) > FLOOR_S and abs(after - before) / before * 100 > noise:
                    why.append(f"{key} {label} {field}: {before} -> {after}")
            if (old.get("built") == "daemon") != (phase.get("built") == "daemon"):
                why.append(f"{key} {label}: built by {old.get('built')} -> {phase.get('built')}")
            for side in ("ra-jit", "cargo"):
                before, after = ((p.get("memory") or {}).get(side, {}).get("peak_mb") for p in (old, phase))
                if (before is None) != (after is None):
                    why.append(f"{key} {label}: {side} memory {'measured' if after is not None else 'no longer measured'}")
                elif before and abs(after - before) > FLOOR_MB and abs(after - before) / before * 100 > noise:
                    why.append(f"{key} {label} {side} peak: {before} MB -> {after} MB")
    return why


def memory_stored(result):
    return os.path.join(ROOT, f"docs/benchmarks/memory-{platform(result).replace(' ', '-').lower()}.json")


def memory_doc(result):
    """docs/benchmarks.md's section of a run published with `--memory`: its machine and its tables."""
    host = result.get("host") or {}
    commit = result.get("commit")
    lines = [MEM_START, f"### Memory, {platform(result)}, {host.get('name') or 'measured by hand'}", "",
             f"ra-jit [`{short(commit)}`]({REPO}/commit/{commit}), {result['date']}, Rust {result['toolchain']}, {result['runs']} runs of "
             f"each edit, {result.get('cold_runs', 1)} cold. Measured by hand with `harness/bench.py`, published with `benchreport.py --memory`; "
             f"raw: [benchmarks/{os.path.basename(memory_stored(result))}](benchmarks/{os.path.basename(memory_stored(result))}). "
             "Its times are in the raw file too; the build table above is the benchmark machine's.", "",
             "| Machine | |", "|---|---|"]
    shown = dict(host, os="macOS" if host.get("os") == "Darwin" else host.get("os"),
                 ld=None if "unknown option" in (host.get("ld") or "") else host.get("ld"))
    for label, field in (("Name", "name"), ("CPU", "cpu"), ("Cores", "cores"), ("Memory (GiB)", "memory_gib"), ("OS", "os"),
                         ("Kernel", "kernel"), ("rustc", "rustc"), ("Linker", "ld")):
        lines.append(f"| {label} | {shown.get(field) if shown.get(field) is not None else '-'} |")
    if host.get("load"):
        lines.append(f"| Load average at the start | {host['load']} |")
    for backend in [b for b in BACKEND_NAMES if any(k.endswith("/" + b) for k in result["results"])]:
        lines += memory_tables(result, ordered(result, backend), backend)
        lines += failures(result, ordered(result, backend))
    lines += ["", MEM_END]
    return "\n".join(lines)


def charts():
    """Every chart again from the stored results: the build table's, the memory table's (the
    benchmark machine's when it measured memory, else the newest published with `--memory`)
    and the runtime table's (the newest runtime result)."""
    if os.path.exists(STORED):
        result = json.load(open(STORED))
        print("edits:", edits_chart(result))
        memory_from = result if has_memory(result) else None
    else:
        memory_from = None
    if memory_from is None:
        stored = sorted(glob.glob(os.path.join(ROOT, "docs/benchmarks/memory-*.json")), key=os.path.getmtime)
        memory_from = json.load(open(stored[-1])) if stored else None
    if memory_from:
        print("memory:", memory_chart(memory_from, memory_backend(memory_from)))
    runs = sorted(glob.glob(os.path.join(ROOT, "docs/benchmarks/runtime-*.json")), key=os.path.getmtime)
    if runs:
        result = json.load(open(runs[-1]))
        measured = result.get("runbench", result)
        print("runtime:", runtime_chart(measured, where(result) if "runbench" in result else measured.get("note", "").split(", ", 1)[-1] + "."))


def options(argv, name, default=None):
    if name in argv:
        at = argv.index(name)
        value = argv[at + 1]
        del argv[at:at + 2]
        return value
    return default


def main():
    argv = sys.argv[1:]
    stored_arg = options(argv, "--stored")
    noise = float(options(argv, "--noise", "10"))
    max_age = float(options(argv, "--max-age", "24"))
    force, check, memory_only = "--force" in argv, "--check" in argv, "--memory" in argv
    if "--charts" in argv:
        charts()
        return
    paths = [a for a in argv if not a.startswith("--")]
    if len(paths) != 1:
        raise SystemExit(__doc__)
    result = json.load(open(paths[0]))
    runtime = "runbench" in result
    if memory_only and (runtime or not has_memory(result)):
        raise SystemExit(f"{paths[0]}: --memory takes a result of bench.py that measured memory")
    if runtime:
        stored_path = stored_arg or os.path.join(ROOT, f"docs/benchmarks/runtime-{platform(result).replace(' ', '-').lower()}.json")
        wrong = runtime_wrong(result) if result["runbench"] else ["runbench.py wrote nothing"]
    elif memory_only:
        stored_path = stored_arg or memory_stored(result)
        wrong = [f"{key}: {line}" for key, measured in result["results"].items() for line in measured.get("wrong", [])]
    else:
        stored_path = stored_arg or STORED
        wrong = [f"{key}: {line}" for key, measured in result["results"].items() for line in measured.get("wrong", [])]
    tests = [line for line in wrong if not runtime and line.split(": ", 1)[1].startswith("tests (")]
    if programs := [line for line in wrong if line not in tests]:
        print("refused: a program did not say what cargo's said\n" + "\n".join(programs))
        sys.exit(2)
    if tests:
        print("written with tests that ended otherwise than under cargo (shown in the table):\n" + "\n".join(tests))
    stored = json.load(open(stored_path)) if os.path.exists(stored_path) else None
    why = (["--force"] if force else []) + (runtime_differences if runtime else differences)(stored, result, noise, max_age)
    changed = bool(why)
    print("\n".join(why[:40]) or f"every number within {noise:.0f}% (or 50 ms) of the stored run")
    if changed and not check:
        os.makedirs(os.path.dirname(stored_path), exist_ok=True)
        if os.path.abspath(paths[0]) != os.path.abspath(stored_path):
            shutil.copyfile(paths[0], stored_path)
        if runtime:
            splice(README, runtime_section(result), RUN_START, RUN_END)
        elif memory_only:
            splice(README, memory_readme(result), MEM_START, MEM_END)
            splice(DOC, memory_doc(result), MEM_START, MEM_END)
        else:
            splice(README, readme_section(result))
            splice(DOC, doc_section(result))
            if has_memory(result):
                splice(README, memory_readme(result), MEM_START, MEM_END)
    if os.environ.get("GITHUB_OUTPUT"):
        open(os.environ["GITHUB_OUTPUT"], "a").write(f"changed={'true' if changed else 'false'}\n")
    print("changed" if changed else "unchanged")
    sys.exit(1 if check and changed else 0)


if __name__ == "__main__":
    main()

# Benchmarks

The README's benchmark tables come from here. Each is a matrix over three platforms, macOS
arm64, Linux aarch64 and Linux x86-64, each measured on a dedicated machine of its own, on real
projects at pinned commits, against plain cargo on the same machine, and written back to the
README by a bot commit. A platform's numbers compare with each other and with its own history,
not with another platform's: the machines differ in more than their architecture. This page
says how, so that anyone can check it or run it again.

## What is measured

`harness/bench.py` copies each project's pinned checkout and builds it two ways side by side:
`cargo ra-jit build` (the daemon) and plain `cargo build` into a target directory of its own.
Both build the same source after every edit, one after the other, so they see the same machine
state.

| Phase | What happens | ra-jit | cargo |
|---|---|---|---|
| Cold | build from an empty target directory and an empty ra-jit store; crates are already downloaded | the build returns (cargo builds the first time) and then the daemon loads the workspace in the background; both are reported | `cargo build` |
| Cold, warm store | after everything else, `--cold-runs` times: a second copy of the project with an empty target directory and a daemon of its own, but the store the measured copy filled with its cold build and edits (as a second checkout of the same project, or a teammate's store, would have) | build and daemon ready, as cold | |
| Nothing changed | build again with no edit | | |
| Edit a function | one statement is added to a function in the main crate | | incremental |
| Edit a widely used function | one statement in a function many places call: in a library crate of the workspace when there is one (ripgrep's `grep-matcher`, axum's `axum-core`, nushell's `nu-path`, bat's library), otherwise in a module the rest of the crate uses (fd, hyperfine, just) | | incremental |
| Change a signature | a method gets another parameter and every caller passes it | | incremental |
| Edit a generic function | a statement in a generic function used at many types (ripgrep and nushell) | | incremental |
| Add a function | a new function in the main crate, called from the function of "Edit a function" | | incremental |
| Add a struct field | a struct gets one more field (`_bench_edit: ()`) and every place that builds it sets it; in a library crate where there is one (nushell's `nu-protocol::Alias`, built in `nu-parser`) | | incremental |
| Tests | (`--tests`) the project's test suite, or the crates of it named in `PROJECTS`, after the edits: `cargo ra-jit test` and `cargo test`, run twice each, the second timed; every test has to end the same way under both | | |
| Runtime | (ripgrep) the program ra-jit linked, cargo's debug build and cargo's release build run the same search over the toolchain's own library source | | |

The edits do not change what the program does. After the first edit of each kind the program
ra-jit linked is run and has to print what cargo's prints (exit status, stdout and stderr); in
the runtime phase all three programs have to print the same. A run in which any program differs
is not published (`benchreport.py` refuses it and the workflow fails).

Each edit is made and taken back `--runs` times (5 in CI). The README shows the median of the
edits; this page also shows the fastest and slowest run of each side, and the raw JSON keeps
every sample, the time of taking each edit back, who built each step (the daemon, or cargo and
why), whether the daemon spliced the change into the program or linked it again, how many
functions it compiled, the memory of both sides (below) and the daemon's own timeline of each
build.
Cold is `--cold-runs` runs of each (5 in CI), each with a new target directory, home and store;
the store size after it is kept too.

An edit the daemon hands to cargo (a fallback: ra-jit always builds what cargo would) is measured
once, marked `(c)` in the README, and shows about cargo's time.

## Memory and disk

Every build in the table is also measured for memory, on both sides, and so is the disk each
side leaves behind. The README shows a summary under the time table; this page has all of it.

| Number | ra-jit | cargo |
|---|---|---|
| Peak, each phase and edit | the most `cargo ra-jit build`, the daemon and every process under either held at once: cargo when the client hands it the build, its rustc, build scripts and linker, the daemon's workers and what they run | the most `cargo build`, rustc, build scripts and the linker held at once |
| Largest process | the peak of the largest of those processes on its own | the same |
| Daemon between edits | what the daemon and its workers hold once it is ready, after nothing changed, after each kind of edit and at the end of the run, with the number of builds so far: memory that grows with every edit shows here | |
| On disk | the target directory, the store and ra-jit's home | the target directory |
| Cold, warm store | the peak, and what the daemon holds once ready | |
| Runtime | the peak of the program ra-jit linked while it runs | the peak of cargo's debug and release programs |

`harness/runbench.py` (runtime parity) adds the peak of each program while it runs, the size of
each binary, and the peak of each build: `cargo build --release` against ra-jit's whole-program
LLVM build.

**How.** One helper, `harness/memory.py`, measures all of it, and every harness that reports
memory uses it. It combines two readings:

- **The exact peak of one process.** When the harness waits for a command (`wait4`), the
  kernel reports the largest resident size any single process of what it waited for reached
  (`ru_maxrss`; KiB on Linux, bytes on macOS). This is exact, but it is one process: cargo's 16
  rustc at once count as the largest of them.
- **The sum over a tree, sampled.** A thread reads what every process under the command, and
  under the daemon, holds every 10 ms (`RA_JIT_MEMORY_INTERVAL`) and keeps the largest sum. On
  Linux that is the resident size, from `/proc/PID/statm`, with the children of each thread in
  `/proc/PID/task/*/children`. On macOS it is the physical footprint, from
  `proc_pid_rusage`, with `proc_listchildpids`: what Activity Monitor shows, the memory a
  process owns, still counted when macOS has compressed it or swapped it out. Resident size
  would not do on a Mac: when memory is short macOS compresses the pages of a process that is
  idle, and the daemon between two edits read 119 MB on nushell on a Mac whose memory was full,
  where it held 7 GB a minute before. The kernel also keeps each process's peak footprint,
  which the sampler reads as the largest of that process. Each process it saw stays watched,
  so the daemon, which the first build starts and which then outlives it, is followed with its
  workers.

The peak reported is the sum, and never less than the peak of the largest process. What the
daemon holds between edits is one reading of its tree when it is idle.

**Error.** The sum can miss a rise shorter than 10 ms and a process that lives less than
10 ms (a short `cc` or build script): it reads low by at most what such a process held. On
Linux it can also read high: a page of a shared library that several processes have mapped
counts once in each (resident size, not proportional size). That is mostly libLLVM and
`librustc_driver`, so it inflates cargo's peak with many rustc at once more than ra-jit's,
whose daemon is one process with a few workers; proportional size costs a walk of every
mapping per reading. The footprint macOS counts leaves those pages out. A Linux number and a
macOS number are therefore not the same count; within one machine both sides are.

**Cost.** The sampling thread wakes every 10 ms and reads a few dozen small files or calls;
while it runs, Python switches threads every 0.1 ms instead of 5 ms, so the end of a timed
command is seen within that. On the MacBook of the macOS results, a no-op `cargo build` of ra-jit itself took a
median 322 ms without it and 319 ms with it (30 runs each, taking turns): no cost the noise
shows. Two runs of the Mac's measurement gave the same peaks within 1%.

**Disk** is counted as `du` counts it: the blocks each file uses, a file with several hard
links once.

The history (`rss_mb`) still has the daemon's workers' high-water marks summed, as before; the
history's memory alerts check the new numbers too (below).

Each platform's memory tables are with the rest of its numbers, under [Results](#results).

## Projects

Pinned by tag and commit in `harness/bench.py` (`PROJECTS`); the clone has to be at that commit
or the run stops.

| Project | Tag | Commit | Run to check the output |
|---|---|---|---|
| ripgrep | 15.1.0 | `af60c2de9d85` | `rg -n --no-heading --sort path "fn main" ...` |
| fd | v10.3.0 | `d38148f0aabd` | `fd --extension rs main src` |
| bat | v0.26.0 | `c734087e1d15` | `bat --plain --color=never rustfmt.toml` |
| hyperfine | v1.19.0 | `12fec4209864` | `hyperfine --help` |
| just | 1.43.0 | `b7e7c46495b3` | `just --list --unsorted` |
| axum | axum-v0.8.6 | `c1bb9c3e9062` | built (`-p example-hello-world`), not run: it is a server |
| nushell | 0.108.0 | `da141be11b3a` | `nu -n -c "[3 1 2] \| sort \| str join ','"` |
| helix | 25.07.1 | `a05c151bb6e8` | `hx --version` (grammars are not built: `HELIX_DISABLE_AUTO_GRAMMAR_BUILD=1`) |
| bevy | v0.20.0 | `b3bdd75becaf` | built (`--example breakout`), not run: it opens a window |
| zed | v1.24.4 | `aed37547d854` | `zed --help`; not in the CI run, see below |

zed (1.7 million lines in 249 crates, 1,291 crates with its dependencies) is in `PROJECTS` and
every edit of it builds, but it is not measured yet: ra-jit's daemon does not fit it in 123 GB.
On a c8i.16xlarge (2026-10-21, ra-jit `2b2f2dcd5b`) the daemon loaded the workspace (20 s),
its 1,756,350 symbols (26 s) and resolved the crates (25 s), then went from 20 GB to more
than 111 GB in 40 seconds and was stopped at a 112 GB limit before it was ready; cargo alone
builds it in a few minutes on the same machine. Its build needs `nix develop .#bench-zed`.

`cargo` 0.92.0 is in `PROJECTS` too but not in the CI run (it doubles the run's length).

## Fairness

- Same machine, same source, same moment: the two builds alternate after every edit.
- Same toolchain on both sides: Rust 1.99.0 (`harness/toolchain`), from the flake's
  `devShells.bench`, which also pins LLVM 20 for the LLVM tier.
- Cargo is warm and incremental: the dev profile's default (`CARGO_INCREMENTAL` unset).
- Same linker: the toolchain's default (`cc`, which runs GNU ld on the benchmark machine). No mold,
  no lld, no sccache or other `RUSTC_WRAPPER` on either side (bench.py removes it).
- Crates are downloaded before the cold build on both sides; downloading is not timed.
- ra-jit's store starts empty for each project and backend and for every cold run. "Cold, warm
  store" is the one phase that starts with a store, and it is reported apart.
- The machine runs nothing else: it is a dedicated instance used only by this workflow.

## Tiers

Each project is measured with each code producer ra-jit has: Cranelift (the default, in the
README) and LLVM (`RA_JIT_BACKEND=llvm RA_JIT_OPT=2 RA_JIT_EMIT=fast`). Cranelift is measured
with ra-jit as it is installed and released, without the `llvm` feature (`RA_JIT_BIN`), and LLVM
with a build that has it (`RA_JIT_LLVM_BIN`): a binary that links libLLVM loads it each time it
starts, about 6 ms on the benchmark machine, and every `cargo ra-jit` starts it once. A new tier
is one more entry in `BACKENDS` in `harness/bench.py`; the report makes a table for every
backend in the result.

## The fastest setups

With `--setups` (all of them in the nightly run, `--setups-on ripgrep,nushell`), every step plain
cargo builds is built again by each of the fastest ways to build Rust there already are, each
into a target directory of its own, right after plain cargo, on the same source:

| Setup | What it is |
|---|---|
| mold | cargo with mold 3.0 as the linker (`-C link-arg=-fuse-ld=mold`) |
| wild | cargo with wild 0.10 as the linker (`-C link-arg=-fuse-ld=wild`) |
| `-Zthreads=8` | rustc's parallel front end, on the same 1.99.0 (`RUSTC_BOOTSTRAP=1`) |
| nightly | plain cargo on the nightly below, to tell the nightly's own difference from Cranelift's |
| cg_clif | rustc_codegen_cranelift (`-Zcodegen-backend=cranelift`), on the nightly: it ships with nightlies only |
| cg_clif + wild + `-Zthreads=8` | all three at once, the fastest cargo build there is |
| `cargo check` | builds no code, so there is nothing to run after it: the floor of what rustc does |

The flags go in through `--config target.'cfg(all())'.rustflags`, which cargo adds to each
project's own (helix sets `--cfg tokio_unstable` there; `RUSTFLAGS` would replace it).
`-Zshare-generics` is not a column: it is already on in every debug build. The nightly is
`rust-bin.nightly."2026-10-09"` with `rustc-codegen-cranelift-preview`, from the flake
(`RA_JIT_NIGHTLY`). A setup that cannot build a project is reported as failed and not timed
after that.

Hot-patching a running program is compared on its own (`harness/hotpatch.py`): a small program
that calls one function every millisecond through `subsecond::call`, edited 30 times by each of
Dioxus's `dx serve --hotpatch` (subsecond, which patches the running program on Linux),
`cargo ra-jit build` and incremental `cargo build`, the last two followed by starting the program
again (ra-jit patches a running program on macOS only). Each edit is timed from writing the file
to the program running the new code. It is a small program on purpose: subsecond needs the
program to call through `subsecond::call` and to keep running, which a command-line tool like
ripgrep does not. The restart flows stop the program before building, as `cargo watch` does.

On Linux (dev-aws-7, 16 cores of a c7i.16xlarge, the bench shell), 30 edits each, milliseconds
from the save to the new code running ([raw JSON](benchmarks/hotpatch/linux-x86_64.json)):

| Tool | median | p90 | min | max | of which the build |
|---|---:|---:|---:|---:|---:|
| subsecond (`dx serve --hotpatch`), patched, 0 restarts | 289 | 463 | 283 | 535 | |
| `cargo ra-jit build`, then a restart (all 30 built by the daemon and spliced) | **40** | 72 | 39 | 78 | 39 |
| `cargo build`, then a restart | 361 | 413 | 353 | 427 | 360 |

On a program this small, starting it again is nearly free, so ra-jit's build plus a restart
beats subsecond's patch of the running program; a program with state to rebuild after a
restart would lose that. On a MacBook Pro (M5 Max, 18 cores, on AC, other work running: a load
of 7 to 28), the same 30 edits each ([raw JSON](benchmarks/hotpatch/macbook.json)):

| Tool | median | p90 | min | max | of which the build |
|---|---:|---:|---:|---:|---:|
| subsecond (`dx serve --hotpatch`), patched, 0 restarts | 346 | 417 | 320 | 421 | |
| `cargo ra-jit build`, then a restart (all 30 built by the daemon) | 469 | 527 | 410 | 841 | 299 |
| `cargo build`, then a restart | 520 | 593 | 486 | 651 | 365 |

On macOS ra-jit also patches a running program without a restart: a program that links
`ra-jit-live` (`harness/live`, as small as this one) takes an edit in a median of 12.5 ms (p90
28.0, p99 48.9) when it goes through `cargo ra-jit build`, and 4.0 ms (p90 11.6, p99 13.8) when an
editor sends it over the daemon's socket (`harness/live_latency.py --program probe cargo socket`,
30 edits each, same MacBook, [raw JSON](benchmarks/live/macbook-probe-cargo.json)). That is the
flow to set against subsecond's; the restart flow above is what ra-jit does where it cannot
patch, today Linux.

## The machines

Every result records its machine and platform: the name it was given (`RA_JIT_BENCH_MACHINE`),
the CPU, the cores the run could use (it may be pinned with `taskset`) and the machine's total,
memory, OS, kernel, rustc, C compiler, linker, LLVM version and the load average when it
started (below and in the JSON). The platforms, their runners' labels and how a result is
filed are `harness/benchplatforms.py`.

Each platform is meant to run on a dedicated machine that runs nothing else, registered as a
self-hosted GitHub Actions runner. A shared runner is no use: GitHub's hosted macOS arm64 runner
(`macos-15`) is a 3-core VM with 7 GB of memory, less than nushell's daemon holds, with
neighbours that move the numbers. The three:

| Platform | Runner labels | Machine | Why |
|---|---|---|---|
| Linux x86-64 | `self-hosted, linux, x64, bench-x86` | `dev-aws-bench-x86` (i-01666717f0de39201, us-west-2a), resized to m8a.4xlarge: 16 AMD EPYC 9R45 (Zen 5) cores, one thread per core, 64 GiB | already launched for it, on demand, ttl a year, tagged `ix-dev-purpose=ra-jit-bench`; launched as a c8a.4xlarge (32 GiB), too little for nushell's daemon (15 GB) beside cargo |
| Linux aarch64 | `self-hosted, linux, ARM64, bench-arm64` | a new `dev-aws-bench-arm64`, m8g.4xlarge: 16 Graviton4 (Neoverse V2) cores, 64 GiB, on demand | the same core count and memory as the x86 machine; a cloud VM is not bare metal, but its cores are not shared with other tenants |
| macOS arm64 | `self-hosted, macOS, ARM64, bench-macos` | a Mac mini M4 Pro (14 cores, 64 GB) kept for it, plugged in, not sleeping (`sudo pmset -a sleep 0`), or an EC2 mac-m4pro.metal host if it has to be in AWS | bare metal, and the only way to measure macOS: a MacBook in use is busy with other work |

`harness/benchrunner.sh PLATFORM MACHINE` sets a runner up: it checks the platform is the
machine's, installs the runner in `~/actions-runner`, registers it with the platform's labels and
runs it at boot (on Linux as user `andrew` from a user systemd unit,
`ra-jit-bench-runner.service`, lingering; on macOS as the runner's launchd service), and its
`.env` names the machine for the results. It needs Nix with flakes on the machine:

```
scp harness/benchrunner.sh HOST:
gh api -X POST repos/indexable-inc/ra-jit/actions/runners/registration-token --jq .token \
  | ssh HOST bash benchrunner.sh linux-aarch64 "'AWS m8g.4xlarge (dev-aws-bench-arm64)'"
```

Then name the platform in the repository variable `BENCH_RUNNERS`, comma-separated, which turns
its jobs on: `gh variable set BENCH_RUNNERS -R indexable-inc/ra-jit -b linux-x86-64,linux-aarch64,macos-arm64`.
A platform it does not name is skipped by every run rather than left waiting for a runner that
is not there, and the README keeps its last numbers, under their machine, date and commit.
(`BENCH_RUNNER=true`, from before there were platforms, still means Linux x86-64.)

Until the runners exist the numbers are measured by hand with the same harness, pinned to 16
cores (`taskset -c 0-15`) so that cargo and the daemon have the 16 cores of the machines above:
on 2026-10-22, Linux x86-64 on `dev-aws-8` (a c7a.48xlarge, AMD EPYC 9R14, idle and claimed for
the run) and Linux aarch64 on a c8g.16xlarge (Graviton4, Neoverse-V2) launched for it. macOS
arm64's edits are the MacBook Pro (M5 Max) run of 2026-10-21, made while it was busy with other
work (its load average is in the README), so its times are noisier than the others'; its runtime
is one run on GitHub's `macos-15` runner, a shared virtual M1 with 3 cores and 7 GiB, whose runs
of a benchmark varied by up to 17× (eza, which reads the disk), so the README marks every
benchmark whose runs varied by more than 1.5× with ≈. The first published run
(2026-10-20) was on `dev-aws-11`, a bare-metal c8a.metal-24xl pinned the same way.

## When it runs and what it commits

`.github/workflows/bench-readme.yaml` runs on every push to main that touches code (not docs,
the site or the README), every night at 09:17 UTC, and by hand (`gh workflow run bench-readme
--repo indexable-inc/ra-jit`, with `-f force=true` to write the README even when nothing moved).
One run at a time; a push during a run waits, and a newer push replaces the waiting one. A run
has a job per registered platform, on its runner, at the same time, and then one job that
publishes them all.

That job hands each platform's result to `harness/benchreport.py`, which compares it with the
stored one of the same platform (`docs/benchmarks/<platform>.json`, e.g. `linux-x86-64.json`;
`runtime-<platform>.json` for the runtime). It replaces the stored result, and writes the
README's tables and this page's results from the newest stored result of every platform, only
when:

- a median of either side, in any phase, moved by more than 10% and more than 50 ms,
- a step the daemon built is now built by cargo, or the other way round,
- a peak of either side moved by more than 10% and 100 MB,
- a project or backend was added or removed, or
- the stored run is more than 24 hours old (so the commit the README names stays within a day
  of main).

It then commits those files to main as `github-actions[bot]` with `[skip ci]` in the message (so
the commit starts no other workflow), once for every platform, rebasing onto main if main moved
during the run. It uses the job's `GITHUB_TOKEN` with `contents: write`; main has no branch
protection. `bench-bun.yaml` does the same for bun, nightly, on the same runners.

## History and regressions

Every run of the workflow appends one line per platform to
`docs/benchmarks/history/<platform>.jsonl` (`harness/benchhistory.py build`), whether or not the
README changed: the medians, spreads, who built each step, memory and store size of each project
and phase, the platform, the machine and the run's link. The site's
[/bench](https://ra-jit.ix.dev/bench) page charts those files, a platform and machine at a time,
and the workflow starts the site's deploy after it commits.

`benchhistory.py check` then compares the new line with the median of the last seven from the
same machine with the same cores. A number regressed when it is above that median by more than
its noise band and by more than a floor: the band is 10% or three times the spread the history
shows (1.4826 times the median absolute deviation), whichever is larger, and the floors are
50 ms for times, 100 MB for memory and 50 MB for the store and disk. Memory checked is
ra-jit's: its peak in each phase, what the daemon holds after each phase, once ready and at the
end of the run, and its disk; cargo's memory is charted, not checked. A step the daemon built that cargo
builds now is a regression whatever its time. On a regression the workflow opens an issue
labelled `bench-regression` with a table of what moved (or comments on the one already open),
writes the table to the run's summary and fails.

Live edits are checked the same way. `.github/workflows/live-latency.yaml` runs every night on
main on a GitHub macOS arm64 runner (`macos-15`), times socket, burst and replay edits of the
four live programs (`harness/live_latency.py`, [live.md](live.md#latency-under-typing)), appends
one line for all of them to `docs/benchmarks/history/macos-arm64-live.jsonl` and checks the p50
and p90 of each program and mode (floor 1 ms). A shared runner is noisier than the benchmark
machine; the band widens with the spread its own history shows.

## Run it yourself

On any of the platforms with Nix (or with Rust 1.99.0, rust-src, LLVM 20 and Python 3 installed):

```
nix develop .#bench
cargo build -p ra-jit --release
CARGO_TARGET_DIR=target-llvm cargo build -p ra-jit --release --features llvm
python3 harness/bench.py ripgrep fd --runs 5 --out /tmp/bench.json   # any of the projects
python3 harness/benchreport.py /tmp/bench.json --check               # what would change
```

`--backend cranelift` measures one tier. `--cold-runs 5` repeats the cold builds, `--setups
all --setups-on ripgrep,nushell` adds the setups above, `--tests` the test suites, and
`--reference DIR` puts cargo's target directories outside the work directory.
`python3 harness/hotpatch.py --edits 30` times hot-patching (needs `dx`: the bench shell has it). Projects are cloned once into `RA_JIT_PROJECTS`
(default `~/ra-jit-projects`). The whole CI set (seven projects, both tiers, five runs) took 38
minutes on 16 Zen 5 cores, plus the build of ra-jit. It needs about 20 GB of memory (nushell)
and 30 GB of disk.
`python3 harness/bench.py --compare OLD.json NEW.json` lists what got slower or holds more
memory. On macOS give `--work` a short directory (`--work /tmp/bench`): the daemon's socket
path has to fit in 104 bytes, which a work directory under `$TMPDIR` does not leave room for.
`python3 harness/benchreport.py RESULT` publishes a run as its platform's (the machine it ran
on): `RA_JIT_BENCH_MACHINE` names that machine in the tables, and `taskset -c 0-15` before
`python3` gives it the 16 cores of the benchmark machines. `benchreport.py --render` writes
every table and chart again from the stored results.

## Caveats

- Live edits (the patch applied to a running program) work on macOS on Apple silicon only and are
  measured there by `harness/live_latency.py`, per keystroke under typing as well as one edit at
  a time: [live.md](live.md#latency-under-typing) has the numbers from a MacBook, and
  `history/macos-arm64-live.jsonl` the runs.
- One machine, one OS. The numbers on a laptop will differ; the ratios are what carries over.
- Cold is the least precise number here: five runs at most, each minutes long on the large
  projects.
- The setups compared are measured on ripgrep and nushell only (`--setups-on`): seven more
  builds of every step on every project would make the nightly run too long for one machine.
- An edit is a small, fixed change. A larger change (many functions, a new type) costs more;
  `harness/agents.py` makes random edits of the kinds coding agents make.
- axum's output is not checked (it is a server); its build is.
- ripgrep's runtime is one short workload (tens of milliseconds, over 16 threads), so it says more
  about start-up and directory walking than about generated code; the program ra-jit linked has
  had the edits spliced into it and taken back, as a program in use would.
- Cold for `cargo ra-jit build` is cargo's own build (the daemon is not loaded yet), so it matches
  plain cargo; the daemon then loads the workspace in the background ("Daemon ready after").
- The C compiler comes from Nix (gcc 16) with the wrapper's hardening turned off, as a
  distribution's gcc behaves; it is the same for both sides. Nix's linker wrapper runs
  `realpath` for each library directory of the shell on every link cargo makes (ra-jit's links
  do not go through it), so the bench shell holds only what the measured projects need, 72
  directories (a link through `cc` costs 0.23 s); zed's libraries are in `nix develop
  .#bench-zed`. With them in the bench shell (282 directories) every cargo link cost 0.85 s, which
  made cargo and subsecond look 0.6 s slower. A result records the count (`library_dirs`).

## What the 2026-10-21 run found

The results below come from two runs on dev-aws-7 (16 cores of a c7i.16xlarge, ra-jit
`2b2f2dcd5b`): ripgrep to nushell on both tiers in one, bevy and helix on Cranelift with one
cold run each in the second, since helix with its test targets needs more memory than the
machine has. ra-jit loses or fails on these:

- **Doctests.** Without `-q`, `cargo ra-jit test` builds the doctests of every library itself, in
  a process that loads the workspace each time (about 2 s per library), on every run: ripgrep's
  suite takes 27.8 s with nothing changed against 4.3 s under `cargo test`, nushell's 12.4 s
  against 2.5 s. With `-q` cargo hands the doctests to rustdoc and ripgrep's run takes 4.9 s
  against 4.1 s. fd, hyperfine and just, whose libraries have few or no doctests, are faster than
  cargo after an edit and the same with nothing changed. In ripgrep, nushell and axum the edited
  library is one whose unit tests the daemon leaves to rustc, so that edit is cargo's build.
- **axum's `example-testing`.** The test binary the daemon builds crashes (SIGSEGV) in
  `the_real_deal`, which serves axum on a TCP socket and calls it with hyper's client: tokio's
  `Registration::readiness` reads a null `Arc<ScheduledIo>` from `TcpListener::accept`. Five of
  the six tests pass; cargo's binary passes all six. Seen on Linux and on macOS.
- **bevy's breakout.** The daemon cannot type `(apply_velocity, move_paddle,
  check_for_collisions).chain()` (the method comes from impls bevy generates for tuples of
  systems), and `cargo ra-jit build` fails at the first edit instead of handing the build to
  cargo. Cold and no-op are measured; the edits are not.
- **helix.** After a 31 s cold build (the same as cargo's) the daemon takes 638 s to be ready
  (636 to 641 s over five cold runs) and holds 98 GB; with a warm store it is no faster (681 s).
  With the test targets loaded too it was killed for memory on a 123 GB machine. Its edits, once
  ready, are 10x to 48x faster than cargo's.
  nushell, a larger workspace, is ready 33 s after its build, at 16 GB.
- **zed** (1.7M lines, 1,291 crates) does not fit: the daemon needs more than 111 GB.
- **Cold.** `cargo ra-jit build` from nothing is cargo's build plus the start of the daemon:
  6.29 s against 5.56 s on ripgrep, and the daemon is ready 6.2 s later.
- **No-op** on ripgrep: 33 ms against cargo's 30 ms. This run's Cranelift column used the
  binary built with the `llvm` feature, which spends about 6 ms loading libLLVM at every start;
  the bench now measures that column with ra-jit as released (19 ms on ripgrep in
  [results.md](results.md#warm-edits-on-linux-x86-64-where-the-time-goes-and-build-scripts-2026-10-20)).

Two commits after `2b2f2dcd5b` (`20708cd2da`, `c8f58ac7d9`) stop the daemon from running build
scripts again that cargo just ran, which roughly halves the edits of fd and hyperfine (fd's
function edit 88 to 43 ms on another machine, same page). The next run of the workflow
measures them here.

## Earlier one-off measurements

Edit, then build, on a workspace of 200 generated crates (`harness/genws.py`), Linux x86-64,
8 CPUs, both warm and incremental, same linker, median of three. Measured once by hand before
this page existed; not updated by CI.

| Edit | cargo | ra-jit | |
|---|---:|---:|---:|
| A function in the main crate | 1.76 s | 0.42 s | 4.2x |
| A function in a crate everything depends on | 6.33 s | 0.60 s | 10.6x |
| Functions in 30 crates at once | 6.26 s | 0.62 s | 10.1x |
| A new function | 2.09 s | 0.56 s | 3.7x |
| A signature used in every crate | 13.87 s | 10.87 s | 1.3x |

## Results

<!-- bench:start -->
Every number of the newest run of each platform. Each platform is a machine of its own: its numbers compare with each other and with its own history, not with another platform's.

### macOS arm64

ra-jit [`84760c3f51`](https://github.com/indexable-inc/ra-jit/commit/84760c3f517a76f6e8a3aa41ccd81fc3f26b87c7), 2026-10-21T21:03:27Z, Rust 1.99.0, 3 runs of each edit, measured by hand. Raw: [benchmarks/macos-arm64.json](benchmarks/macos-arm64.json).

| Machine | |
|---|---|
| Name | MacBook Pro (Mac17,6), macOS 27.0.1, with other work running |
| CPU | Apple M5 Max |
| Cores | 18 |
| Memory (GiB) | 128 |
| OS | macOS |
| Kernel | 27.0.0 |
| rustc | rustc 1.99.0 (b940084d7 2026-09-28) |
| C compiler | Apple clang version 21.0.0 (clang-2100.3.34.2) |
| Linker | - |
| LLVM | - |
| Incremental | profile default (on for dev) |
| Compiler cache | none |
| Load average at the start | 48.0, 46.1, 48.4 |

#### Edits, Cranelift, macOS arm64

Median, then the fastest and slowest run. Built: who built the edit (daemon, or cargo and why). Link: the change was spliced into the program, or it was linked again.

| Project | Edit | cargo | ra-jit | | cargo runs | ra-jit runs | Built | Link |
|---|---|---:|---:|---:|---|---|---|---|
| ripgrep 15.1.0 | Edit a function | 993 ms | 126 ms | 7.9x | 646 ms to 1.11 s | 125 ms to 191 ms | daemon | spliced |
| ripgrep 15.1.0 | Edit a widely used function | 1.31 s | 159 ms | 8.2x | 1.02 s to 4.28 s | 104 ms to 160 ms | daemon | spliced |
| ripgrep 15.1.0 | Change a signature | 828 ms | 175 ms | 4.7x | 785 ms to 1.36 s | 102 ms to 305 ms | daemon | spliced |
| ripgrep 15.1.0 | Edit a generic function | 1.93 s | 100 ms | 19x | 1.27 s to 1.95 s | 92 ms to 157 ms | daemon | spliced |
| ripgrep 15.1.0 | Add a function | 1.81 s | 248 ms | 7.3x | 706 ms to 2.86 s | 104 ms to 271 ms | daemon | spliced |
| ripgrep 15.1.0 | Add a struct field | 1.88 s | 191 ms | 9.8x | 1.35 s to 1.96 s | 94 ms to 298 ms | daemon | spliced |
| ripgrep 15.1.0 | Nothing changed | 67 ms | 54 ms | 1.2x | 63 ms to 72 ms | 50 ms to 61 ms | daemon |  |
| fd 10.3.0 | Edit a function | 630 ms | 109 ms | 5.8x | 611 ms to 1.29 s | 99 ms to 140 ms | daemon | spliced |
| fd 10.3.0 | Edit a widely used function | 734 ms | 102 ms | 7.2x | 691 ms to 1.01 s | 87 ms to 105 ms | daemon | spliced |
| fd 10.3.0 | Change a signature | 681 ms | 113 ms | 6.0x | 563 ms to 835 ms | 100 ms to 205 ms | daemon | spliced |
| fd 10.3.0 | Add a function | 712 ms | 126 ms | 5.7x | 593 ms to 745 ms | 106 ms to 228 ms | daemon | spliced |
| fd 10.3.0 | Add a struct field | 1.09 s | 127 ms | 8.6x | 894 ms to 1.31 s | 113 ms to 143 ms | daemon | spliced |
| fd 10.3.0 | Nothing changed | 144 ms | 49 ms | 2.9x | 85 ms to 369 ms | 48 ms to 84 ms | daemon |  |
| nushell 0.108.0 | Edit a function | 1.81 s | 454 ms | 4.0x | 1.66 s to 1.89 s | 383 ms to 713 ms | daemon | spliced |
| nushell 0.108.0 | Edit a widely used function | 14.39 s | 850 ms | 17x | 12.49 s to 15.24 s | 840 ms to 861 ms | daemon | spliced |
| nushell 0.108.0 | Change a signature | 10.28 s | 716 ms | 14x | 9.54 s to 12.34 s | 694 ms to 867 ms | daemon | spliced |
| nushell 0.108.0 | Edit a generic function | 20.61 s | 2.12 s | 9.7x | 9.59 s to 26.96 s | 986 ms to 2.50 s | daemon | full link: a program too large for frames in front of its own: one is 0x1000020 bytes in |
| nushell 0.108.0 | Add a function | 2.47 s | 444 ms | 5.6x | 1.74 s to 2.86 s | 353 ms to 480 ms | daemon | spliced |
| nushell 0.108.0 | Add a struct field | 25.84 s | 2.51 s | 10x | 23.58 s to 30.94 s | 2.21 s to 3.54 s | daemon | full link: a program too large for frames in front of its own: one is 0x1000008 bytes in |
| nushell 0.108.0 | Nothing changed | 407 ms | 45 ms | 9.0x | 347 ms to 433 ms | 41 ms to 56 ms | daemon |  |

#### Cold, Cranelift, macOS arm64

From an empty target directory, crates already downloaded, one run. `cargo ra-jit build` is cargo's own build (the daemon is not loaded yet); the daemon then loads the workspace and builds it once, and edits are fast from then on. Warm store: the same in another checkout of the same commit, with the store the first one filled.

| Project | `cargo build` | `cargo ra-jit build` | Daemon ready, empty store | Daemon ready, warm store | Functions compiled | Daemon memory | Store |
|---|---:|---:|---:|---:|---:|---:|---:|
| ripgrep 15.1.0 | 9.83 s | 12.08 s | 14.68 s | 17.62 s | 18,437 | 2,360 MB | 72 MB |
| fd 10.3.0 | 9.17 s | 11.57 s | 12.62 s | 13.16 s | 8,478 | 1,946 MB | 32 MB |
| nushell 0.108.0 | 49.69 s | 77.23 s | 92.67 s | 139.83 s | 167,893 | 16,137 MB | 1,121 MB |

#### Memory and disk, Cranelift, macOS arm64

Peak: the most every process doing the work held at once, sampled every 10 ms and never below the largest process, median of the runs. Largest process: the peak of the one process that held the most, as the kernel kept it: `wait4`'s for the command's tree, and each process's peak sampled (on macOS the kernel's peak footprint of each, counting for the daemon only a peak it passed during the step). ra-jit is the daemon, its workers and what they run, and the `cargo ra-jit build` with any cargo it ran; cargo is cargo itself, rustc, build scripts and the linker. Daemon after: what the daemon and its workers hold once the step is over ([how](#memory-and-disk)).

| Project | Step | cargo, peak | ra-jit, peak | cargo, largest process | ra-jit, largest process | Daemon after |
|---|---|---:|---:|---:|---:|---:|
| ripgrep 15.1.0 | Cold, until the daemon is ready | 541 MB | 2.4 GB | 356 MB | 2.2 GB | 2.2 GB |
| ripgrep 15.1.0 | Edit a function | 235 MB | 2.9 GB | 235 MB | 1.9 GB | 2.9 GB |
| ripgrep 15.1.0 | Edit a widely used function | 243 MB | 2.9 GB | 243 MB | 1.9 GB | 2.8 GB |
| ripgrep 15.1.0 | Change a signature | 235 MB | 2.9 GB | 235 MB | 1.9 GB | 2.8 GB |
| ripgrep 15.1.0 | Edit a generic function | 237 MB | 2.9 GB | 237 MB | 1.9 GB | 2.8 GB |
| ripgrep 15.1.0 | Add a function | 234 MB | 2.9 GB | 234 MB | 1.9 GB | 2.8 GB |
| ripgrep 15.1.0 | Add a struct field | 286 MB | 2.9 GB | 286 MB | 1.9 GB | 2.8 GB |
| ripgrep 15.1.0 | Nothing changed | 40 MB | 2.8 GB | 40 MB | 1.9 GB | 2.8 GB |
| fd 10.3.0 | Cold, until the daemon is ready | 897 MB | 1.9 GB | 389 MB | 1.8 GB | 1.8 GB |
| fd 10.3.0 | Edit a function | 212 MB | 2.6 GB | 212 MB | 1.6 GB | 2.6 GB |
| fd 10.3.0 | Edit a widely used function | 199 MB | 2.6 GB | 192 MB | 1.6 GB | 2.6 GB |
| fd 10.3.0 | Change a signature | 226 MB | 2.6 GB | 226 MB | 1.6 GB | 2.6 GB |
| fd 10.3.0 | Add a function | 211 MB | 2.6 GB | 211 MB | 1.6 GB | 2.6 GB |
| fd 10.3.0 | Add a struct field | 258 MB | 2.6 GB | 258 MB | 1.6 GB | 2.6 GB |
| fd 10.3.0 | Nothing changed | 48 MB | 2.5 GB | 48 MB | 1.6 GB | 2.5 GB |
| nushell 0.108.0 | Cold, until the daemon is ready | 2.4 GB | 16.1 GB | 1.1 GB | 15.3 GB | 11.5 GB |
| nushell 0.108.0 | Edit a function | 1.1 GB | 12.5 GB | 1.1 GB | 11.5 GB | 12.5 GB |
| nushell 0.108.0 | Edit a widely used function | 1.1 GB | 12.5 GB | 1.1 GB | 11.5 GB | 12.5 GB |
| nushell 0.108.0 | Change a signature | 1.1 GB | 12.5 GB | 1.1 GB | 11.5 GB | 12.5 GB |
| nushell 0.108.0 | Edit a generic function | 1.1 GB | 13.2 GB | 1.1 GB | 11.3 GB | 12.2 GB |
| nushell 0.108.0 | Add a function | 1.1 GB | 12.5 GB | 1.1 GB | 11.5 GB | 12.5 GB |
| nushell 0.108.0 | Add a struct field | 1.1 GB | 13.3 GB | 1.1 GB | 11.4 GB | 12.2 GB |
| nushell 0.108.0 | Nothing changed | 156 MB | 12.1 GB | 156 MB | 11.2 GB | 12.1 GB |

The daemon between builds, and what each side keeps on disk once the edits are done (as `du` counts it). After the first build: once the step after the cold build (nothing changed) is done; growth is from then to the end, over every edit made and taken back. Warm store: the cold start of another checkout with the store the first one filled.

| Project | Daemon ready | After the first build | At the end | Growth | Warm store: peak, ready | cargo target | ra-jit target | Store | Home |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| ripgrep 15.1.0 | 2.2 GB | 2.8 GB | 2.8 GB after 39 builds | +23 MB | 2.3 GB, 2.0 GB | 416 MB | 526 MB | 72 MB | 0 MB |
| fd 10.3.0 | 1.8 GB | 2.5 GB | 2.6 GB after 33 builds | +50 MB | 1.9 GB, 1.6 GB | 355 MB | 427 MB | 33 MB | 0 MB |
| nushell 0.108.0 | 11.5 GB | 12.1 GB | 12.5 GB after 39 builds | +367 MB | 16.5 GB, 11.7 GB | 5.3 GB | 5.8 GB | 1.1 GB | 1 MB |

#### How fast the program runs, Cranelift, macOS arm64

The same command run by the program ra-jit linked and by cargo's debug and release builds, in turn. Median, then fastest and slowest; where measured, then the program's peak resident memory while it ran (median of the runs, as the kernel kept it) and the size of the program.

| Project | Command | ra-jit | cargo debug | cargo release |
|---|---|---:|---:|---:|
| ripgrep 15.1.0 | `rg -uuu --sort path -c \w+_unchecked\(\|unsafe \{ <the toolchain's library source>` | 1.01 s (676 ms to 1.43 s); 12 MB, 28.5 MB | 753 ms (679 ms to 1.07 s); 8 MB, 15.6 MB | 167 ms (129 ms to 308 ms); 6 MB, 6.6 MB |

### Linux x86-64

ra-jit [`2b2f2dcd5b`](https://github.com/indexable-inc/ra-jit/commit/2b2f2dcd5bc4d7132fe62bf36669a34c657dafe7), 2026-10-20T23:52:46Z, Rust 1.99.0, 5 runs of each edit, measured by hand. Raw: [benchmarks/linux-x86-64.json](benchmarks/linux-x86-64.json).

| Machine | |
|---|---|
| Name | AWS c7i.16xlarge (dev-aws-7), pinned to 16 cores with taskset |
| CPU | Intel(R) Xeon(R) Platinum 8488C |
| Cores | 16 |
| Memory (GiB) | 124 |
| OS | Linux |
| Kernel | 6.18.53 |
| rustc | rustc 1.99.0 (b940084d7 2026-09-28) |
| C compiler | gcc (GCC) 16.2.0 |
| Linker | GNU ld (GNU Binutils) 2.46 |
| LLVM | 20.1.8 |
| Incremental | profile default (on for dev) |
| Compiler cache | none |

#### Edits, Cranelift, Linux x86-64

Median, then the fastest and slowest run. Built: who built the edit (daemon, or cargo and why). Link: the change was spliced into the program, or it was linked again.

| Project | Edit | cargo | ra-jit | | cargo runs | ra-jit runs | Built | Link |
|---|---|---:|---:|---:|---|---|---|---|
| ripgrep 15.1.0 | Edit a function | 655 ms | 104 ms | 6.3x | 643 ms to 658 ms | 102 ms to 165 ms | daemon | spliced |
| ripgrep 15.1.0 | Edit a widely used function | 1.00 s | 62 ms | 16x | 990 ms to 1.03 s | 58 ms to 86 ms | daemon | spliced |
| ripgrep 15.1.0 | Change a signature | 809 ms | 90 ms | 9.0x | 806 ms to 812 ms | 89 ms to 100 ms | daemon | spliced |
| ripgrep 15.1.0 | Edit a generic function | 985 ms | 62 ms | 16x | 959 ms to 997 ms | 60 ms to 64 ms | daemon | spliced |
| ripgrep 15.1.0 | Add a function | 663 ms | 119 ms | 5.6x | 657 ms to 665 ms | 118 ms to 121 ms | daemon | spliced |
| ripgrep 15.1.0 | Add a struct field | 1.20 s | 82 ms | 15x | 1.16 s to 1.25 s | 80 ms to 87 ms | daemon | spliced |
| ripgrep 15.1.0 | Nothing changed | 30 ms | 33 ms | 0.9x | 27 ms to 31 ms | 32 ms to 36 ms | daemon |  |
| fd 10.3.0 | Edit a function | 649 ms | 115 ms | 5.6x | 634 ms to 662 ms | 114 ms to 152 ms | daemon | spliced |
| fd 10.3.0 | Edit a widely used function | 600 ms | 103 ms | 5.8x | 599 ms to 615 ms | 103 ms to 116 ms | daemon | spliced |
| fd 10.3.0 | Change a signature | 652 ms | 129 ms | 5.1x | 646 ms to 699 ms | 128 ms to 191 ms | daemon | spliced |
| fd 10.3.0 | Add a function | 640 ms | 121 ms | 5.3x | 634 ms to 656 ms | 119 ms to 134 ms | daemon | spliced |
| fd 10.3.0 | Add a struct field | 925 ms | 118 ms | 7.8x | 896 ms to 964 ms | 117 ms to 134 ms | daemon | spliced |
| fd 10.3.0 | Nothing changed | 43 ms | 35 ms | 1.2x | 42 ms to 44 ms | 33 ms to 37 ms | daemon |  |
| bat 0.26.0 | Edit a function | 650 ms | 69 ms | 9.4x | 632 ms to 661 ms | 68 ms to 412 ms | daemon | spliced |
| bat 0.26.0 | Edit a widely used function | 936 ms | 58 ms | 16x | 924 ms to 955 ms | 56 ms to 82 ms | daemon | spliced |
| bat 0.26.0 | Change a signature | 900 ms | 64 ms | 14x | 882 ms to 919 ms | 63 ms to 78 ms | daemon | spliced |
| bat 0.26.0 | Add a function | 668 ms | 75 ms | 8.9x | 645 ms to 703 ms | 73 ms to 86 ms | daemon | spliced |
| bat 0.26.0 | Add a struct field | 1.24 s | 73 ms | 17x | 1.23 s to 1.38 s | 72 ms to 95 ms | daemon | spliced |
| bat 0.26.0 | Nothing changed | 81 ms | 32 ms | 2.5x | 79 ms to 82 ms | 31 ms to 32 ms | daemon |  |
| hyperfine 1.19.0 | Edit a function | 528 ms | 117 ms | 4.5x | 520 ms to 535 ms | 115 ms to 159 ms | daemon | spliced |
| hyperfine 1.19.0 | Edit a widely used function | 569 ms | 117 ms | 4.9x | 563 ms to 609 ms | 117 ms to 121 ms | daemon | spliced |
| hyperfine 1.19.0 | Change a signature | 583 ms | 162 ms | 3.6x | 570 ms to 623 ms | 160 ms to 168 ms | daemon | spliced |
| hyperfine 1.19.0 | Add a function | 525 ms | 121 ms | 4.3x | 523 ms to 527 ms | 118 ms to 129 ms | daemon | spliced |
| hyperfine 1.19.0 | Add a struct field | 659 ms | 150 ms | 4.4x | 655 ms to 701 ms | 149 ms to 160 ms | daemon | spliced |
| hyperfine 1.19.0 | Nothing changed | 56 ms | 32 ms | 1.8x | 56 ms to 56 ms | 31 ms to 32 ms | daemon |  |
| just 1.43.0 | Edit a function | 899 ms | 58 ms | 16x | 896 ms to 965 ms | 57 ms to 200 ms | daemon | spliced |
| just 1.43.0 | Edit a widely used function | 926 ms | 58 ms | 16x | 923 ms to 945 ms | 56 ms to 68 ms | daemon | spliced |
| just 1.43.0 | Change a signature | 900 ms | 93 ms | 9.7x | 895 ms to 904 ms | 83 ms to 95 ms | daemon | spliced |
| just 1.43.0 | Add a function | 1.05 s | 80 ms | 13x | 1.02 s to 1.10 s | 79 ms to 86 ms | daemon | spliced |
| just 1.43.0 | Add a struct field | 2.36 s | 149 ms | 16x | 2.35 s to 2.49 s | 146 ms to 155 ms | daemon | spliced |
| just 1.43.0 | Nothing changed | 54 ms | 30 ms | 1.8x | 53 ms to 56 ms | 30 ms to 31 ms | daemon |  |
| axum 0.8.6 | Edit a function | 559 ms | 69 ms | 8.1x | 550 ms to 570 ms | 68 ms to 112 ms | daemon | spliced |
| axum 0.8.6 | Edit a widely used function | 1.11 s | 63 ms | 18x | 1.10 s to 1.15 s | 62 ms to 65 ms | daemon | spliced |
| axum 0.8.6 | Add a function | 561 ms | 71 ms | 7.9x | 551 ms to 563 ms | 70 ms to 81 ms | daemon | spliced |
| axum 0.8.6 | Nothing changed | 102 ms | 34 ms | 3.0x | 100 ms to 104 ms | 32 ms to 35 ms | daemon |  |
| nushell 0.108.0 | Edit a function | 1.62 s | 279 ms | 5.8x | 1.60 s to 1.65 s | 275 ms to 819 ms | daemon | spliced |
| nushell 0.108.0 | Edit a widely used function | 5.78 s | 271 ms | 21x | 5.72 s to 5.86 s | 266 ms to 277 ms | daemon | spliced |
| nushell 0.108.0 | Change a signature | 5.82 s | 336 ms | 17x | 5.75 s to 5.98 s | 323 ms to 406 ms | daemon | spliced |
| nushell 0.108.0 | Edit a generic function | 7.86 s | 240 ms | 33x | 7.77 s to 8.04 s | 229 ms to 372 ms | daemon | spliced |
| nushell 0.108.0 | Add a function | 1.65 s | 282 ms | 5.9x | 1.60 s to 1.67 s | 276 ms to 287 ms | daemon | spliced |
| nushell 0.108.0 | Add a struct field | 9.71 s | 400 ms | 24x | 9.67 s to 9.94 s | 392 ms to 472 ms | daemon | spliced |
| nushell 0.108.0 | Nothing changed | 240 ms | 38 ms | 6.3x | 238 ms to 241 ms | 37 ms to 39 ms | daemon |  |
| helix 25.07.1 | Edit a function | 2.80 s | 231 ms | 12x | 2.78 s to 2.83 s | 224 ms to 610 ms | daemon | spliced |
| helix 25.07.1 | Edit a widely used function | 4.16 s | 86 ms | 48x | 4.13 s to 4.16 s | 84 ms to 106 ms | daemon | spliced |
| helix 25.07.1 | Change a signature | 3.56 s | 354 ms | 10x | 3.53 s to 3.60 s | 346 ms to 365 ms | daemon | spliced |
| helix 25.07.1 | Add a function | 2.81 s | 289 ms | 9.7x | 2.79 s to 2.83 s | 282 ms to 464 ms | daemon | spliced |
| helix 25.07.1 | Add a struct field | 10.44 s | 404 ms | 26x | 10.36 s to 10.98 s | 390 ms to 414 ms | daemon | spliced |
| helix 25.07.1 | Nothing changed | 114 ms | 34 ms | 3.4x | 108 ms to 114 ms | 32 ms to 45 ms | daemon |  |
| bevy 0.20.0 | Nothing changed | 264 ms | 41 ms | 6.4x | 260 ms to 271 ms | 40 ms to 42 ms | daemon |  |

- bevy 0.20.0: only the steps shown are measured, then the build failed: examples/showcase/breakout.rs:67:18: no method `chain` on type `(fn apply_velocity(Query<'_, '_, (&mut Transform, &Velocity), ()>, Res<'_,` …

#### Cold, Cranelift, Linux x86-64

From an empty target directory, crates already downloaded, median of 5 runs, each from nothing. `cargo ra-jit build` is cargo's own build (the daemon is not loaded yet); the daemon then loads the workspace and builds it once, and edits are fast from then on. Warm store: the same in another checkout of the same commit, with the store the first one filled.

| Project | `cargo build` | `cargo ra-jit build` | Daemon ready, empty store | Daemon ready, warm store | Functions compiled | Daemon memory | Store |
|---|---:|---:|---:|---:|---:|---:|---:|
| ripgrep 15.1.0 | 5.56 s (5.53 s to 5.57 s) | 6.29 s (6.25 s to 6.32 s) | 12.51 s (12.48 s to 12.55 s) | 11.74 s (11.70 s to 11.79 s) | 17,103 | 2,554 MB | 184 MB |
| fd 10.3.0 | 33.16 s (32.92 s to 33.39 s) | 33.76 s (33.72 s to 33.89 s) | 40.27 s (40.21 s to 40.39 s) | 39.73 s (39.63 s to 39.78 s) | 8,157 | 2,162 MB | 45 MB |
| bat 0.26.0 | 10.91 s (10.79 s to 10.96 s) | 11.11 s (11.11 s to 11.19 s) | 22.70 s (22.68 s to 22.78 s) | 22.11 s (21.79 s to 22.27 s) | 11,353 | 2,880 MB | 41 MB |
| hyperfine 1.19.0 | 5.57 s (5.55 s to 5.59 s) | 6.11 s (6.09 s to 6.15 s) | 14.29 s (14.06 s to 14.30 s) | 13.76 s (13.76 s to 13.76 s) | 5,762 | 2,163 MB | 24 MB |
| just 1.43.0 | 7.62 s (7.53 s to 7.73 s) | 7.70 s (7.69 s to 7.80 s) | 16.16 s (16.16 s to 16.27 s) | 14.91 s (14.82 s to 15.09 s) | 21,033 | 2,930 MB | 226 MB |
| axum 0.8.6 | 8.58 s (8.55 s to 8.59 s) | 9.15 s (9.11 s to 9.22 s) | 16.22 s (16.17 s to 16.29 s) | 16.30 s (16.24 s to 16.35 s) | 5,515 | 2,472 MB | 137 MB |
| nushell 0.108.0 | 31.99 s (31.87 s to 32.55 s) | 33.12 s (32.81 s to 33.26 s) | 66.50 s (65.63 s to 67.25 s) | 63.53 s (62.47 s to 63.61 s) | 160,179 | 15,887 MB | 1,429 MB |
| helix 25.07.1 (one run) | 31.30 s | 32.11 s | 670.71 s | 712.74 s | 149,754 | 98,182 MB | 14,638 MB |
| bevy 0.20.0 (one run) | 75.10 s | 76.64 s | 115.78 s | - | 9,917 | 8,468 MB | - |

#### Against the fastest setups, Cranelift, Linux x86-64

The same steps, each built after plain cargo by the fastest ways to build Rust there already are, each into a target directory of its own, median of the same runs. Plain cargo is incremental, with the default linker. mold and wild link instead of GNU ld; `-Zthreads=8` is the parallel front end on the same compiler; the nightly (rustc 1.101.0-nightly (a30aa9064 2026-10-08)) is there to tell its own difference from Cranelift's (rustc_codegen_cranelift comes only with nightlies); `cargo check` builds no code, so nothing can be run after it. The fastest of each row is in bold; "failed" is a setup that could not build the project, and is not timed after that.

| Project | Step | cargo | `cargo check` | mold | wild | `-Zthreads=8` | nightly | nightly, Cranelift | nightly, Cranelift, wild, `-Zthreads=8` | ra-jit |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| ripgrep 15.1.0 | Cold, until the daemon is ready | 5.56 s | 2.87 s | 5.45 s | 5.44 s | 3.97 s | 5.78 s | 5.14 s | **3.25 s** | 12.51 s |
| ripgrep 15.1.0 | Edit a function | 655 ms | 194 ms | 609 ms | 590 ms | 671 ms | 666 ms | 619 ms | 585 ms | **104 ms** |
| ripgrep 15.1.0 | Edit a widely used function | 1.00 s | 468 ms | 951 ms | 941 ms | 986 ms | 1.07 s | 979 ms | 859 ms | **62 ms** |
| ripgrep 15.1.0 | Change a signature | 809 ms | 292 ms | 752 ms | 735 ms | 819 ms | 813 ms | 742 ms | 686 ms | **90 ms** |
| ripgrep 15.1.0 | Edit a generic function | 985 ms | 463 ms | 917 ms | 903 ms | 966 ms | 1.01 s | 951 ms | 853 ms | **62 ms** |
| ripgrep 15.1.0 | Add a function | 663 ms | 197 ms | 613 ms | 595 ms | 659 ms | 674 ms | 626 ms | 563 ms | **119 ms** |
| ripgrep 15.1.0 | Add a struct field | 1.20 s | 297 ms | 1.16 s | 1.16 s | 1.16 s | 1.24 s | 1.00 s | 833 ms | **82 ms** |
| ripgrep 15.1.0 | Nothing changed | 30 ms | 29 ms | **28 ms** | 29 ms | 30 ms | 30 ms | 30 ms | 30 ms | 33 ms |
| nushell 0.108.0 | Cold, until the daemon is ready | 31.99 s | 17.41 s | 31.68 s | 31.45 s | 29.21 s | 33.21 s | 28.64 s | **23.12 s** | 66.50 s |
| nushell 0.108.0 | Edit a function | 1.62 s | 421 ms | 1.11 s | 1.25 s | 1.66 s | 1.65 s | 1.41 s | 1.11 s | **279 ms** |
| nushell 0.108.0 | Edit a widely used function | 5.78 s | 3.14 s | 5.25 s | 5.41 s | 5.82 s | 5.81 s | 5.53 s | 5.05 s | **271 ms** |
| nushell 0.108.0 | Change a signature | 5.82 s | 3.10 s | 5.32 s | 5.47 s | 5.88 s | 5.94 s | 5.58 s | 5.22 s | **336 ms** |
| nushell 0.108.0 | Edit a generic function | 7.86 s | 3.76 s | 7.36 s | 7.50 s | 6.90 s | 7.92 s | 7.06 s | 5.97 s | **240 ms** |
| nushell 0.108.0 | Add a function | 1.65 s | 418 ms | 1.11 s | 1.27 s | 1.66 s | 1.65 s | 1.42 s | 1.10 s | **282 ms** |
| nushell 0.108.0 | Add a struct field | 9.71 s | 3.31 s | 9.26 s | 9.32 s | 9.61 s | 9.99 s | 7.83 s | 6.93 s | **400 ms** |
| nushell 0.108.0 | Nothing changed | 240 ms | 241 ms | 244 ms | 242 ms | 243 ms | 246 ms | 248 ms | 245 ms | **38 ms** |

#### Test suites, Cranelift, Linux x86-64

`cargo ra-jit test` and `cargo test` of the same tests, median of three: after the edit of a widely used function (the file named), and again with nothing changed. Each includes running the tests. First runs: the three untimed runs of `cargo ra-jit test` before them (the first is handed to cargo, the daemon builds the test targets at the second). Every test has to end the same way under both, every time.

| Project | Command | Tests | Same outcome | Edited | Edit, then test: cargo | ra-jit | | Built | Nothing changed: cargo | ra-jit | First runs of ra-jit |
|---|---|---:|---|---|---:|---:|---:|---|---:|---:|---|
| ripgrep 15.1.0 | `cargo test --workspace` | 1,168 | yes | `crates/matcher/src/lib.rs` | 5.34 s | 29.69 s | 0.2x | cargo: a library its unit tests are linked with as rustc bui | 4.33 s | 27.77 s | 33.55 s, 27.78 s, 27.91 s |
| fd 10.3.0 | `cargo test` | 237 | yes | `src/dir_entry.rs` | 941 ms | 515 ms | 1.8x | daemon | 388 ms | 390 ms | 5.41 s, 1.44 s, 402 ms |
| hyperfine 1.19.0 | `cargo test` | 105 | yes | `src/command.rs` | 1.10 s | 716 ms | 1.5x | daemon | 572 ms | 548 ms | 5.00 s, 555 ms, 564 ms |
| just 1.43.0 | `cargo test` | 1,443 | yes | `src/lexer.rs` | 2.03 s | 1.23 s | 1.6x | daemon | 765 ms | 808 ms | 6.55 s, 790 ms, 800 ms |
| axum 0.8.6 | `cargo test -p example-testing` | 6 | no: `cargo ra-jit test` exited 101, `cargo test` 0 | `axum-core/src/body.rs` | 1.29 s | 1.56 s | 0.8x | cargo: a library its unit tests are linked with as rustc bui | 109 ms | 7.06 s | 10.52 s, 261 ms, 283 ms |
| nushell 0.108.0 | `cargo test -p nu-path -p nu-protocol` | 349 | yes | `crates/nu-path/src/components.rs` | 4.92 s | 12.61 s | 0.4x | cargo: a library its unit tests are linked with as rustc bui | 2.46 s | 12.37 s | 29.66 s, 8.59 s, 8.51 s |

#### How fast the program runs, Cranelift, Linux x86-64

The same command run by the program ra-jit linked and by cargo's debug and release builds, in turn. Median, then fastest and slowest; where measured, then the program's peak resident memory while it ran (median of the runs, as the kernel kept it) and the size of the program.

| Project | Command | ra-jit | cargo debug | cargo release |
|---|---|---:|---:|---:|
| ripgrep 15.1.0 | `rg -uuu --sort path -c \w+_unchecked\(\|unsafe \{ <the toolchain's library source>` | 45 ms (43 ms to 46 ms) | 44 ms (42 ms to 45 ms) | 16 ms (15 ms to 17 ms) |

#### Edits, LLVM, Linux x86-64

Median, then the fastest and slowest run. Built: who built the edit (daemon, or cargo and why). Link: the change was spliced into the program, or it was linked again.

| Project | Edit | cargo | ra-jit | | cargo runs | ra-jit runs | Built | Link |
|---|---|---:|---:|---:|---|---|---|---|
| ripgrep 15.1.0 | Edit a function | 656 ms | 128 ms | 5.1x | 652 ms to 729 ms | 126 ms to 202 ms | daemon | spliced |
| ripgrep 15.1.0 | Edit a widely used function | 1.01 s | 62 ms | 16x | 1.00 s to 1.01 s | 59 ms to 90 ms | daemon | spliced |
| ripgrep 15.1.0 | Change a signature | 800 ms | 115 ms | 7.0x | 799 ms to 808 ms | 114 ms to 119 ms | daemon | spliced |
| ripgrep 15.1.0 | Edit a generic function | 967 ms | 70 ms | 14x | 962 ms to 982 ms | 70 ms to 102 ms | daemon | spliced |
| ripgrep 15.1.0 | Add a function | 655 ms | 137 ms | 4.8x | 649 ms to 668 ms | 134 ms to 141 ms | daemon | spliced |
| ripgrep 15.1.0 | Add a struct field | 1.24 s | 81 ms | 15x | 1.21 s to 1.27 s | 80 ms to 83 ms | daemon | spliced |
| ripgrep 15.1.0 | Nothing changed | 29 ms | 32 ms | 0.9x | 29 ms to 30 ms | 32 ms to 37 ms | daemon |  |
| fd 10.3.0 | Edit a function | 653 ms | 132 ms | 4.9x | 632 ms to 656 ms | 129 ms to 163 ms | daemon | spliced |
| fd 10.3.0 | Edit a widely used function | 603 ms | 107 ms | 5.6x | 599 ms to 644 ms | 107 ms to 119 ms | daemon | spliced |
| fd 10.3.0 | Change a signature | 661 ms | 152 ms | 4.3x | 653 ms to 716 ms | 149 ms to 189 ms | daemon | spliced |
| fd 10.3.0 | Add a function | 636 ms | 136 ms | 4.7x | 631 ms to 646 ms | 133 ms to 149 ms | daemon | spliced |
| fd 10.3.0 | Add a struct field | 960 ms | 121 ms | 7.9x | 939 ms to 966 ms | 118 ms to 126 ms | daemon | spliced |
| fd 10.3.0 | Nothing changed | 43 ms | 32 ms | 1.3x | 42 ms to 44 ms | 30 ms to 36 ms | daemon |  |
| bat 0.26.0 | Edit a function | 641 ms | 106 ms | 6.0x | 637 ms to 642 ms | 105 ms to 448 ms | daemon | spliced |
| bat 0.26.0 | Edit a widely used function | 931 ms | 59 ms | 16x | 921 ms to 994 ms | 57 ms to 84 ms | daemon | spliced |
| bat 0.26.0 | Change a signature | 887 ms | 67 ms | 13x | 882 ms to 890 ms | 66 ms to 93 ms | daemon | spliced |
| bat 0.26.0 | Add a function | 646 ms | 114 ms | 5.7x | 631 ms to 653 ms | 111 ms to 124 ms | daemon | spliced |
| bat 0.26.0 | Add a struct field | 1.24 s | 73 ms | 17x | 1.22 s to 1.27 s | 72 ms to 96 ms | daemon | spliced |
| bat 0.26.0 | Nothing changed | 82 ms | 32 ms | 2.6x | 81 ms to 82 ms | 31 ms to 41 ms | daemon |  |
| hyperfine 1.19.0 | Edit a function | 504 ms | 124 ms | 4.1x | 501 ms to 514 ms | 122 ms to 163 ms | daemon | spliced |
| hyperfine 1.19.0 | Edit a widely used function | 542 ms | 117 ms | 4.6x | 535 ms to 546 ms | 116 ms to 120 ms | daemon | spliced |
| hyperfine 1.19.0 | Change a signature | 565 ms | 240 ms | 2.4x | 553 ms to 577 ms | 236 ms to 246 ms | daemon | spliced |
| hyperfine 1.19.0 | Add a function | 501 ms | 128 ms | 3.9x | 499 ms to 503 ms | 126 ms to 131 ms | daemon | spliced |
| hyperfine 1.19.0 | Add a struct field | 639 ms | 148 ms | 4.3x | 636 ms to 674 ms | 146 ms to 161 ms | daemon | spliced |
| hyperfine 1.19.0 | Nothing changed | 55 ms | 32 ms | 1.7x | 54 ms to 55 ms | 30 ms to 33 ms | daemon |  |
| just 1.43.0 | Edit a function | 899 ms | 64 ms | 14x | 897 ms to 966 ms | 62 ms to 200 ms | daemon | spliced |
| just 1.43.0 | Edit a widely used function | 932 ms | 58 ms | 16x | 919 ms to 963 ms | 56 ms to 61 ms | daemon | spliced |
| just 1.43.0 | Change a signature | 907 ms | 106 ms | 8.6x | 901 ms to 970 ms | 104 ms to 112 ms | daemon | spliced |
| just 1.43.0 | Add a function | 1.07 s | 83 ms | 13x | 1.05 s to 1.09 s | 82 ms to 88 ms | daemon | spliced |
| just 1.43.0 | Add a struct field | 2.40 s | 156 ms | 15x | 2.34 s to 2.47 s | 150 ms to 161 ms | daemon | spliced |
| just 1.43.0 | Nothing changed | 54 ms | 30 ms | 1.8x | 54 ms to 55 ms | 29 ms to 30 ms | daemon |  |
| axum 0.8.6 | Edit a function | 550 ms | 71 ms | 7.7x | 549 ms to 585 ms | 70 ms to 112 ms | daemon | spliced |
| axum 0.8.6 | Edit a widely used function | 1.11 s | 64 ms | 17x | 1.10 s to 1.17 s | 64 ms to 69 ms | daemon | spliced |
| axum 0.8.6 | Add a function | 557 ms | 71 ms | 7.8x | 553 ms to 569 ms | 69 ms to 82 ms | daemon | spliced |
| axum 0.8.6 | Nothing changed | 103 ms | 33 ms | 3.1x | 100 ms to 105 ms | 32 ms to 35 ms | daemon |  |
| nushell 0.108.0 | Edit a function | 1.61 s | 590 ms | 2.7x | 1.60 s to 1.68 s | 587 ms to 1.89 s | daemon | spliced |
| nushell 0.108.0 | Edit a widely used function | 5.71 s | 247 ms | 23x | 5.67 s to 5.74 s | 244 ms to 260 ms | daemon | spliced |
| nushell 0.108.0 | Change a signature | 5.79 s | 294 ms | 20x | 5.73 s to 5.86 s | 287 ms to 446 ms | daemon | spliced |
| nushell 0.108.0 | Edit a generic function | 7.77 s | 492 ms | 16x | 7.71 s to 7.96 s | 486 ms to 616 ms | daemon | spliced |
| nushell 0.108.0 | Add a function | 1.61 s | 587 ms | 2.7x | 1.59 s to 1.62 s | 587 ms to 591 ms | daemon | spliced |
| nushell 0.108.0 | Add a struct field | 9.69 s | 326 ms | 30x | 9.63 s to 9.90 s | 322 ms to 464 ms | daemon | spliced |
| nushell 0.108.0 | Nothing changed | 240 ms | 38 ms | 6.3x | 239 ms to 240 ms | 38 ms to 38 ms | daemon |  |

#### Cold, LLVM, Linux x86-64

From an empty target directory, crates already downloaded, median of 5 runs, each from nothing. `cargo ra-jit build` is cargo's own build (the daemon is not loaded yet); the daemon then loads the workspace and builds it once, and edits are fast from then on. Warm store: the same in another checkout of the same commit, with the store the first one filled.

| Project | `cargo build` | `cargo ra-jit build` | Daemon ready, empty store | Daemon ready, warm store | Functions compiled | Daemon memory | Store |
|---|---:|---:|---:|---:|---:|---:|---:|
| ripgrep 15.1.0 | 5.61 s (5.57 s to 5.63 s) | 6.27 s (6.26 s to 6.28 s) | 14.29 s (14.27 s to 14.32 s) | 11.66 s (11.64 s to 11.69 s) | 17,103 | 2,580 MB | 68 MB |
| fd 10.3.0 | 32.89 s (32.76 s to 33.40 s) | 33.61 s (33.53 s to 34.14 s) | 40.71 s (40.61 s to 41.24 s) | 39.99 s (39.95 s to 40.22 s) | 8,157 | 2,209 MB | 30 MB |
| bat 0.26.0 | 10.90 s (10.89 s to 10.99 s) | 11.08 s (11.03 s to 11.15 s) | 23.82 s (23.66 s to 24.13 s) | 21.82 s (21.79 s to 21.88 s) | 11,353 | 2,922 MB | 39 MB |
| hyperfine 1.19.0 | 5.54 s (5.53 s to 5.69 s) | 6.11 s (6.09 s to 6.13 s) | 14.60 s (14.58 s to 14.62 s) | 13.74 s (13.73 s to 13.76 s) | 5,762 | 2,229 MB | 16 MB |
| just 1.43.0 | 7.56 s (7.54 s to 7.69 s) | 7.75 s (7.73 s to 7.80 s) | 18.04 s (17.98 s to 18.31 s) | 15.17 s (14.79 s to 15.23 s) | 21,033 | 3,125 MB | 110 MB |
| axum 0.8.6 | 8.57 s (8.55 s to 8.59 s) | 9.19 s (9.15 s to 9.27 s) | 16.56 s (16.51 s to 16.64 s) | 16.33 s (16.30 s to 16.42 s) | 5,515 | 2,468 MB | 43 MB |
| nushell 0.108.0 | 32.03 s (31.78 s to 32.04 s) | 32.84 s (32.72 s to 33.42 s) | 84.12 s (83.52 s to 84.22 s) | 63.39 s (62.09 s to 63.75 s) | 160,179 | 15,404 MB | 1,093 MB |

#### How fast the program runs, LLVM, Linux x86-64

The same command run by the program ra-jit linked and by cargo's debug and release builds, in turn. Median, then fastest and slowest; where measured, then the program's peak resident memory while it ran (median of the runs, as the kernel kept it) and the size of the program.

| Project | Command | ra-jit | cargo debug | cargo release |
|---|---|---:|---:|---:|
| ripgrep 15.1.0 | `rg -uuu --sort path -c \w+_unchecked\(\|unsafe \{ <the toolchain's library source>` | 43 ms (43 ms to 44 ms) | 42 ms (42 ms to 44 ms) | 15 ms (15 ms to 16 ms) |

<!-- bench:end -->

# The harnesses

ra-jit is tested by comparison with rustc. Almost every script under `harness/` builds
something with cargo and rustc, builds it again with ra-jit's code in place of rustc's for
some crates, runs both, and compares standard output, standard error and exit status. A
difference is a failure. So is a function ra-jit refused, in the harnesses that count them.

There are also Rust unit tests in `crates/ra-jit` and in the code generator; the harnesses
are what carries the weight.

## Running them

You need Python 3, Rust 1.99.0 with `rust-src`, a C compiler and linker, and a release build
of ra-jit. Some harnesses need more: `debuginfo.py` needs gdb and `llvm-dwarfdump`,
`buildmodel.py` needs the `aarch64-unknown-linux-gnu` and `x86_64-unknown-linux-musl`
targets with their cross tools and qemu, `remote.py` needs a MinIO or SeaweedFS binary,
`storefull.py` needs passwordless `sudo` to mount a tmpfs.

    cargo +1.99.0 build -p ra-jit --release
    cd harness
    export RA_JIT_BIN=$PWD/../target/release/ra-jit

`RA_JIT_BIN` names the ra-jit under test and defaults to `../target/release/ra-jit`. The
scripts pick the toolchain from `harness/toolchain` unless `RUSTUP_TOOLCHAIN` is set. They
also default `RUSTFLAGS` to `-C link-dead-code`, which `cargo ra-jit` does not use, and
`CARGO_INCREMENTAL` to 0. Scratch directories go under `TMPDIR`; several of the harnesses
need a few gigabytes there.

### The gate

The gate is what CI runs: its fast tier on every pull request and push to main, all of it
nightly ([CI](#ci)). A change is not done until all of it exits 0 under both backends.
`harness/ci.py` is the one list of it: which harness runs in which part, in which tier, on
which hosts and under which backend.

    python3 ci.py --list            # every harness, its part, its quick part, its environment
    python3 ci.py --parts           # the parts that have harnesses on this machine
    python3 ci.py --parts fast      # the fast tier's parts here (quick-1, quick-2)
    python3 ci.py core              # run one part here, as CI does

A part runs all of its harnesses even when one fails, then names the ones that failed and how
long each took. Alongside the harnesses, CI runs `cargo fmt --all -- --check` and
`python3 harness/unittests.py`, which runs the Rust unit tests and holds them to its `OPEN`
list (below).

### Under Cranelift

Cranelift is the default. Either leave `RA_JIT_BACKEND` unset or set it:

    export RA_JIT_BACKEND=cranelift

### Under LLVM

Build with the `llvm` feature against LLVM 20, then select the backend. The gate is run at
`-O2` with fast instruction selection, and that is the only LLVM configuration it covers.

    LLVM_SYS_201_PREFIX=/usr/lib/llvm-20 cargo +1.99.0 build -p ra-jit --release --features llvm
    export RA_JIT_BACKEND=llvm RA_JIT_OPT=2 RA_JIT_EMIT=fast

A binary built with `--features llvm` still uses Cranelift unless `RA_JIT_BACKEND=llvm` is
set, so one build serves both runs. Under LLVM also run

    python3 backends.py corpus runner cases --bin runner --levels 2f

which reports how many functions LLVM compiled itself and how many it left to Cranelift,
with reasons. The daemon's log says `backend: llvm -O2 fast emit` when LLVM is in use.

## What each one checks

### Correctness of generated code

| Script | What it checks |
|---|---|
| `difftest.py <workspace> --bin <pkg> --crate <name>` | The base comparison. Builds the binary with cargo, compiles the named crates with ra-jit, links ra-jit's object ahead of rustc's rlibs, runs both. `--layouts` also compares the size and alignment of every type ra-jit laid out with rustc's `-Zprint-type-sizes`. `--bisect` links each ra-jit function in on its own to name the ones that change behaviour. `--mixed N` runs N builds with a random half of the functions left to rustc. `--memcheck` runs under valgrind |
| `harness/corpus/` | The workspace `difftest.py corpus` runs: a library with one module per area of the code generator and a runner that calls every function and prints the result |
| `breakers.py` | Every reduction under `harness/breakers/`, each run on its own so a crash in one does not hide the others. These are minimal cases of bugs found in real crates, rustc's tests and the fuzzer |
| `unwinds.py` | Edits to functions a panic unwinds through |
| `growth.py` | That the code of one function grows in a straight line with the arms of its match: the `serialize` `#[derive(Serialize)]` gives an internally tagged enum, a `?` in every arm, at 160 and 320 arms (issue #5) |
| `asmedits.py` | Edits to `asm!` blocks and the code around them |
| `fuzz/fuzz.py` | Generated programs, see below |
| `uitest.py <rust checkout>` | rustc's own `tests/ui` run-pass tests. Every file is either run or left out with a counted reason |
| `libsuite.py <rust checkout> coretests\|alloctests\|stdtests` | The standard library's test suites, cut down to what builds on stable |
| `cratetest.py <crate-dir>` | A crate's own integration tests: each test binary is relinked with ra-jit's objects and every test must report the same result |
| `cratesweep.sh`, `vendor.sh`, `sweepreport.py` | `cratetest.py` over a list of crates.io releases, and a summary |
| `bintest.py <name>` | A real program from `breakers/bins.json`: the same command lines given to cargo's binary and ra-jit's |
| `alone.py` | A program whose workspace crates rustc compiles nothing of, so no refused function can hide behind rustc's copy |
| `wholeprogram.py` | Programs ra-jit builds whole, with its own `main` (as `runbench.py` builds them): one on the default allocator, one with a `#[global_allocator]` of its own, one with a library's, one whose allocator is under a `#[cfg]` that is false. Each has to print and exit as cargo's build does; `--release` builds both in the release profile |
| `backends.py` | One program compiled by each code producer: counts, time per function, and what LLVM left to Cranelift |
| `borrowck_ui.py <rust checkout>` | The borrow checker ([borrowck.md](borrowck.md)) against rustc's borrow-check UI tests: each unit passes when ra-jit reports the errors (code and line) the pinned nightly rustc reports, under Polonius and NLL. `--min-pass N` fails below N Polonius passes |
| `scoreboard.py` | Runs the targets in `targets.json` and fails when behaviour differs or coverage drops below the committed `scoreboard.json` |
| `mirparity.py <workspace> --crate <name> [--level N] [--rustc-level 1\|2]` | ra-jit's MIR (`ra-jit mir`, at `RA_JIT_MIR_OPT` level N) against rustc's `-Zunpretty=mir`, function by function: the share that match in shape, statement counts, statements and types, and the most frequent differences. Not a gate: the numbers are in [research/mir-gap.md](research/mir-gap.md) |

### The daemon following edits

| Script | What it checks |
|---|---|
| `stress.py` | 100 edits of the kinds people make (bodies, signatures, new items, moved lines, broken code, reverts, an editor's edit while another client's build runs) applied in order to a three-crate workspace under one warm daemon, each compared with `cargo build` of the same source. The first edit cargo builds after broken code has to be a patch of the build before it ([#31](https://github.com/indexable-inc/ra-jit/issues/31)), and the edit sent during a build has to be taken ([#32](https://github.com/indexable-inc/ra-jit/issues/32)); it exits 1 when a step is wrong |
| `programs.py` | Two programs of one workspace built by one daemon. With `RA_JIT_ALONE=1` rustc builds stand-ins for the workspace's crates and the daemon compiles every function of both programs, `main` too: under LLVM an edit of what both use has to reach each ([#63](https://github.com/indexable-inc/ra-jit/issues/63)) |
| `worktrees.py` | Several checkouts of one workspace built by one daemon, edited independently |
| `includes.py` | Edits to a file only `include!` names |
| `splices.py` | Edits that are hard on the splice: thread-locals, statics, vtables, function pointers, closures, panics, foreign calls. Counts how each step was linked |
| `farsplices.py` | Splices into a binary whose code is further from its end than an AArch64 branch reaches |
| `unreached.py` | Edits under `cargo ra-jit` that call instances of another crate's generic function the executable did not reach when it was linked: each is spliced, not linked in full. The daemon's own links, with `RA_JIT_RUSTC_BASE=0`. Linux and macOS; on Windows `cargo ra-jit` hands builds to cargo |
| `loadedits.py` | A file written while a worker loads, after the worker read it and before it took its timestamp (held there with `RA_JIT_LOAD_PAUSE_MS`), is built as written: the next build, and the one after an edit of another crate, print the new text ([#53](https://github.com/indexable-inc/ra-jit/issues/53)). Linux and macOS |
| `agents.py` | Random edits of the kind coding agents make, on a real workspace |
| `invalidation.py` | How much the database redoes after different kinds of edit |
| `movedlines.py` | A panic reports its new line after lines are added above its function (seven ways to panic, each against cargo), and moving a function compiles nothing |
| `buildmodel.py` | One small cargo project per feature of cargo's build model, under both backends, with steps that change one input at a time |
| `cargo_ra_jit.py` | `cargo ra-jit` against plain cargo on seven real projects, 22 flag combinations, and 11 daemon lifecycle checks. Needs the network and about 20 minutes; not in CI |
| `debuginfo.py` | Backtraces, `llvm-dwarfdump --verify`, gdb stopping and stepping, locals |
| `live_e2e.py` | Edits of a running program that links `ra-jit-live`, each applied as a patch. Linux x86-64 and macOS AArch64 |
| `live_latency.py` | Edit-to-running time of live edits on `harness/live`, `demo/life`, `harness/live-bevy` and `harness/live-egui`: isolated edits, bursts of valid keystrokes, real typing replayed key by key (most texts in between do not compile; a program told to restart is started again), and a session's restarts against `cargo run`. Writes p50 to p99 and every sample, with the machine and its battery, to `LATENCY_JSON`. Measures; checks nothing; macOS on AArch64 ([live.md](live.md#latency-under-typing)) |
| `fncache.py` | `#[ra_jit::cache]` ([fn-cache.md](fn-cache.md)): a cold run computes and a warm one finds the result on disk; comment, format, moved-line and unrelated edits keep it, an edit of a function it reaches through another drops it; generic instances are kept apart; impure functions are refused saying where; a restarted daemon finds the same results; cargo's build caches nothing. `--live`: a live edit computes again and drops the old results, macOS on AArch64 |
| `spaces.py` | A two-crate workspace under a directory whose name has spaces, `$$`, `#` and quotes, built and edited with `cargo ra-jit build`; every edit has to be applied by the daemon against rustc's symbols ([#14](https://github.com/indexable-inc/ra-jit/issues/14)) |
| `rustcbase.py` | The first edits after a cold `cargo ra-jit build`, spliced into the executable rustc linked ([#11](https://github.com/indexable-inc/ra-jit/issues/11)), against plain cargo: bodies, closures, a dependency crate, function-local statics, two edits in a row, and a `#[ra_jit::cache]` function and what it reaches have to be patches; generic, `#[inline]` and default trait functions, statics, new items, `async`, `impl Trait` and `include_str!` edits have to be built in full and say why; and the daemon's own base, built while nothing waits, takes the edits after it. Every edit's output and exit status against cargo's, macOS and Linux |
| `headerpad.py` | A program whose unwind table does not fit the room its link leaves after the load commands: the first link is done again with room, the edits after it are spliced, and a restarted daemon starts from the room kept beside the executable ([#13](https://github.com/indexable-inc/ra-jit/issues/13)). It is built for macOS 14, as nix's shell builds, which Apple's linker gives chained fixups unless told not to ([#30](https://github.com/indexable-inc/ra-jit/issues/30)). The daemon's own links, with `RA_JIT_RUSTC_BASE=0`. macOS on AArch64 |
| `tiers.py` | Tiers ([tiers.md](tiers.md)): `demo/lifebench` running under the daemon reaches levels 2 and 3 with output identical to cargo's, an edit of a tiered function demotes it and is applied at once, and `RA_JIT_TIER=off` changes nothing. macOS on AArch64 with `--features llvm`; it exits 0 elsewhere |
| `tierbench.py` | The benchmark table of [tiers.md](tiers.md): `demo/lifebench` at level 1, 2 and 3 under a daemon and as cargo builds it (release, and `-O3` with the dev profile's checks), in interleaved rounds, the steady rate of each; `--together` runs a round's builds at once, for a loaded machine. Measures; checks nothing; not in CI |

### The store and the remote

| Script | What it checks |
|---|---|
| `keys.py` | Whether the store ever hands a build code compiled from something else. One case per thing code depends on besides its file's text |
| `storekills.py` | The daemon killed at random moments while writing its store |
| `storecorrupts.py` | Packs damaged on disk, and stores of other formats |
| `storefull.py` | A store on a disk with no room |
| `storeshared.py` | Several daemons and `ra-jit cache` commands on one store at once |
| `deepnames.py` | Closures nested 20 deep keep the store under 64 MB, and a second daemon finds their entries again: a type spelled out longer than 1 KB is named by its hash (issue #29) |
| `remote.py`, `s3fake.py` | A store shared by two machines through an S3 service, with the transfer broken, objects spoiled and manifests forged |
| `deps.py` | cargo's builds of registry crates pushed and pulled. Builds offline from cargo's registry cache, so it does not run on a clean CI machine |

### Timing

`bench.py`, `editloop.py`, `edittimes.py`, `scaling.py`, `dxedits.py`, `linkbench.py`,
`shapes.py`, `heap.py`, `heapfold.py`, `fnincr.py`, `realbench.py` and `live_latency.py` measure and do not gate.
`heap.py` gives a heap profile's bytes to the salsa query whose value or record holds them, and to
ra-jit's own structures; `heapfold.py` makes of a profile on macOS what `jeprof --collapsed` makes
of it on Linux, for `heap.py` to read ([research/daemon-memory.md](research/daemon-memory.md)). `fnincr.py` counts
the queries an edit inside one function of a big file runs (the file comes from `genbig.py`),
against the same functions in many small files ([reports/INCREMENTAL.md](reports/INCREMENTAL.md)). `realbench.py` times
`cargo ra-jit build` against `cargo build` on ripgrep, just and nushell (cold, no-op and
three kinds of edit); the numbers are in [results.md](results.md#11-real-projects). `coldprof.py`
times a cold `cargo ra-jit build` of a real project until the daemon is ready and its first
edit, from an empty or a filled store, with every line the daemon logged and the cores busy
along the way: the readiness check of [#11](https://github.com/indexable-inc/ra-jit/issues/11),
whose numbers are in [cargo.md](cargo.md#what-a-build-does). With `--max-wait S` it fails when
the daemon was ready more than S seconds after the command returned. Their docstrings say how to run them.
`bench.py` on real projects, with `benchreport.py`, is also what writes the README's build
table: [benchmarks.md](benchmarks.md). Besides the edits `bench.py` times repeated cold builds
against fresh cargo with an empty and with a warm store (`--cold-runs`), the same steps under
the fastest cargo setups (`--setups`: mold, wild, `-Zthreads=8`, cg_clif and `cargo check`) and
the test suites under both (`--tests`). `bench.py --runtime` runs `runbench.py`, which times
programs ra-jit builds whole with LLVM against `cargo build --release`, and `benchreport.py`
writes that result to the README's runtime table, refusing it when any program's output differs
from cargo's. `abbench.py` runs two or more builds of ra-jit against each other on `bench.py`'s
edits, in interleaved rounds: an edit's wall time, the daemon's own time and (macOS) its
workers' cycles, the cold start, and the workers' peak memory. It is how the release build's
profile was chosen ([reports/RELEASE-BUILD.md](reports/RELEASE-BUILD.md)), and with `--train` it
is the training of its PGO. `hotpatch.py` times an edit of a running program under Dioxus's
subsecond against `cargo ra-jit build` and `cargo build` with a restart; its program is
`harness/hotpatch`. `benchhistory.py` appends a run to `docs/benchmarks/history/` (what the
site's /bench page charts) and, with `check`, fails when a median moved past its noise band.
`memory.py` is how every harness measures memory: `memory.run` runs a command and returns its
time with the exact peak of its largest process (`wait4`) and the peak of its whole tree
(sampled), `memory.Sampler` follows a long-lived process such as the daemon with its workers,
and `memory.in_child` measures a function that runs commands itself
([benchmarks.md](benchmarks.md#memory-and-disk)); a harness that reports memory uses it rather
than a way of its own. `benchcharts.py` draws the README's charts (SVG, a light and a dark
version) from the same results `benchreport.py` writes the tables from; `benchreport.py`
regenerates them whenever it writes a table, and `benchreport.py --charts` on its own.
`live_latency.py` times live edits on macOS on AArch64, per keystroke under typing (`replay`,
`burst`) as well as one edit at a time (`socket`, or `cargo` through `cargo ra-jit build`), and
the restarts of a session against `cargo run` (`restart`); its JSON goes to `benchhistory.py
live`, and the numbers measured on a MacBook are in [live.md](live.md#latency-under-typing).
`nix develop .#bench` has everything `bench.py` and `hotpatch.py` need (the nightly, mold, wild,
`dx`, bevy's libraries; zed's are in `.#bench-zed`, since every library directory slows each
link cargo makes), and `live_latency.py` runs under it as under rustup's 1.99.0. Under another
toolchain the daemon builds nothing, and the harness stops at once with what `cargo ra-jit`
says of it.

## The fuzzer

`fuzz/fuzz.py` builds generated programs with cargo and with ra-jit under each backend and
compares what they print. `fuzz/fuzz.py smoke` is part of the gate. How to run longer
runs, what a finding is and what the fuzzer does not cover are in
[fuzzing.md](fuzzing.md).

## Adding a corpus case

The corpus is for things the code generator must handle that are small enough to state as
a function. Add a `pub fn` to the module for its area in
`harness/corpus/cases/src/lib.rs`, or a new `pub mod`. Call it from
`harness/corpus/runner/src/main.rs` inside `case!(...)`, which prints the expression and its
value. The result has to be deterministic and `Debug`. Then run

    python3 difftest.py corpus --bin runner --crate cases --layouts

It must print `MATCH`, and the count of functions from ra-jit must go up by what you added,
with none left to rustc.

## Adding a breaker

A breaker is the reduction of a bug found somewhere else. Add a file
`harness/breakers/cases/src/<name>.rs` with a `//!` comment at the top that says where the
bug was found and what the cause is, and a `pub fn run()` returning something `Debug`. Add
`<name>` to the `cases![...]` list in `harness/breakers/cases/src/lib.rs`. Then

    python3 breakers.py <name>

A breaker should fail before the fix and pass after. If you are recording a difference you
are not fixing, add the name to `OPEN` in `breakers.py`, or to `LEFT` if ra-jit refuses the
function and says so. Both lists are the project's list of known wrong or missing
behaviour, so keep the comment in the case honest about why it is open.

A bug that only shows in a crate's own tests goes in `CRATES` in `breakers.py` as a small
crate under `harness/breakers/`.

## Reproducing a failure

Start from the script's last lines. Each daemon harness prints the path of the daemon's log
and of the scratch copy of the source it was editing, for example

    99 of 99 right, 0 restarts; log /tmp/ra-jit-stress-out-XXXX/daemon.log; source /tmp/ra-jit-stress-XXXX

and the stepwise ones take `--only TEXT`, `--from N` and `--keep` (see each docstring).

For a difference in generated code:

1. Run `difftest.py` on the workspace with `--bisect`. It names the functions whose ra-jit
   version changes the output, with the first differing line.
2. `ra-jit --report FILE.tsv` writes the outcome of every function (compiled, skipped and
   why). `--deny FILE` leaves the functions listed in a file to rustc, and `--isolate DIR`
   writes one object per function.
3. `RA_JIT_DUMP_MIR=<function name>` prints the MIR of functions with exactly that name
   (`{closure}` for closures). `RA_JIT_DUMP_LLVM=1` prints the LLVM module of every function
   compiled, and `RA_JIT_DUMP_CLIF=1` the Cranelift IR of drop glue.
4. Reduce it to a function, add it as a breaker, and fix it there.

For a fuzzer finding, `fuzz.py repro SEED-ROUND` rebuilds the program and
`fuzz.py check found/FILE.rs` runs a stored reduction.

For a CI failure, the last line of the harness step lists which scripts failed.

## CI

`.github/workflows/ra-jit.yaml` runs in two tiers. It calls `ra-jit-platform.yaml` once for
each platform, each on a machine of its own kind.

The **fast tier** runs on every pull request, every push to main and every entry of the merge
queue, and takes about ten minutes. On Linux x86-64, Linux AArch64 and macOS arm64 it:

- builds ra-jit once, with the `llvm` feature, and hands the binary to its other jobs;
- runs `harness/unittests.py`;
- runs the parts of `ci.py --parts fast` (`quick-1` and `quick-2`, three to six minutes each)
  under each backend, as a job each;
- on Linux x86-64, runs rustc's borrow-check tests with a floor under the passes, and the
  run-pass tests of `harness/ui-pinned.txt`, against that build.

`fork alone` builds and tests `vendor/rust-analyzer` outside this repository.

The **full tier** runs nightly and by hand (`gh workflow run ra-jit.yaml --ref <branch> -f
tier=full`, or `-f tier=fast`), and takes about an hour. It adds Windows x86-64, every part
of `ci.py --parts` (`keys.py`, `buildmodel.py`, the store and the daemon harnesses,
`debuginfo.py`), `cargo_ra_jit.py` on its checks, on ripgrep and hyperfine and on fd (Linux
x86-64 and macOS), and on ripgrep, fd, bat, hyperfine, just, axum and nushell with each
project's tests (Linux AArch64), and the ripgrep bench. A failed nightly opens an issue named
"Nightly ra-jit failed", or comments on it while it is open
(`.github/actions/nightly-issue`).

A change to nothing but documentation or the site runs `fmt` and nothing else. On a pull
request a newer push cancels the run before it. Every push to main and every merge-queue entry
runs to the end in a concurrency group of its own: with one group for main, a merge every few
minutes replaced each waiting run with the next, and 7 of 10 runs on main were cancelled.

The build and the unit tests compile through sccache, which keeps rustc's output in the
Actions cache; rust-cache keeps the dependencies from crates.io and leaves out the crates of
this repository and `vendor/`, which are most of the build. The harnesses run without the
wrapper, since `cargo_ra_jit.py` tests a user's own.

`ci` is the job that says whether everything that ran passed, and it is the one check branch
rules and the merge queue require. A job that is renamed or split does not change it.
Measured on 2026-10-22 ([#67](https://github.com/indexable-inc/ra-jit/pull/67)): the fast
tier took 15.4 minutes with the sccache cache empty and 10.1 with it filled, where the macOS
build (4 minutes) and then its harnesses (6) are the longest path. One tier that ran everything
took 44 to 67 minutes before.

The repository ruleset `main` (Settings, Rules) holds the default branch to three rules. A
push to it needs `ci` to have passed on the commit pushed, the branch can't be deleted, and
it can't be force-pushed. Repository admins bypass all three, so a merge pushed straight to
main and a scrub of history still work for them. A ruleset can't exempt the token a workflow
runs with, so the `bench-readme` commit to main is refused while the ruleset is active.
`gh api repos/indexable-inc/ra-jit/rulesets` shows its state: `evaluate` only records what
it would have refused (Insights, Rule insights), and `active` refuses it.

What is known to be wrong is written down, says why, and does not fail the run. Once it comes
out right, it fails the run until it is taken off the list:

- `OPEN` in `unittests.py`: Rust unit tests that fail;
- `OPEN` and `LEFT` in `breakers.py`: breakers that differ from rustc or that ra-jit refuses;
- `OPEN` in `debuginfo.py`: checks whose frames or variables are wrong;
- `"open"` in a `buildmodel/*/case.json`: a case that comes out WRONG, on every host or by
  host (`{"linux": "why"}`).

The other workflows: `bench-readme.yaml` measures real projects on the dedicated benchmark
machine and commits the README's numbers to main. It runs only while the repository variable
`BENCH_RUNNER` is `true` ([benchmarks.md](benchmarks.md)). `bench.yaml` (the store's
microbenchmarks, two minutes) runs on pull requests and pushes too. `claims.yaml` (all of
rustc's `tests/ui`, both backends, and the scoreboard) and `nix-live.yaml` (the flake, the demo
and live edits, 40 minutes; also on a change to the flake, `nix/` or the dependencies) run
nightly and open an issue when they fail, like ra-jit.yaml's full tier. `fork-sync.yaml` runs
on pushes to main that touch `vendor/`, `fuzz.yaml` nightly, and `live-*.yaml` on their own
branches. None of them is part of `ci`; the few `tests/ui` programs that once differed from
rustc (`harness/ui-pinned.txt`, `uitest.py --tests`) run in ra-jit.yaml's `borrowck` job,
which is. `site.yaml` deploys the
site, and `release.yaml` builds releases from tags, each target through
`.github/scripts/build_release.py` (the `dist` profile with PGO, [reports/RELEASE-BUILD.md](reports/RELEASE-BUILD.md)),
the recipe the flake's `dist` runs too. `pgo.yaml` trains the profiles it uses, by hand and on
pushes to main that change Cargo.lock, a manifest or the toolchain, and pins them in
`pgo/profiles.json`.

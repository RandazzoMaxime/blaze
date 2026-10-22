# Changelog

All notable changes to ra-jit are written here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `#[ra_jit::cache]` ([#17](https://github.com/indexable-inc/ra-jit/issues/17)): the
  `ra-jit-cache` crate's attribute keeps what a pure function returns, in memory and under
  `target/ra-jit/fn-cache`, by its arguments and a hash of the code ra-jit compiled for it and
  for everything it reaches. Comments, formatting and edits elsewhere keep the results; an
  edit to anything it reaches drops them, live edits too. Functions that read statics that
  change, thread-locals or the system (I/O, time, randomness) are refused with where that is
  written. Under plain cargo it does nothing. `cargo ra-jit cache clear` forgets what was kept
  ([docs/fn-cache.md](docs/fn-cache.md), `harness/fncache.py`).
- `harness/abbench.py` runs builds of ra-jit against each other on `bench.py`'s edits, in
  interleaved rounds: edit and cold-start times, the workers' cycles on macOS and their peak
  memory. With `--train` it is the training of the release build's PGO.
- rustc's MIR optimizations, optionally: `RA_JIT_MIR_OPT=1` runs what rustc runs on a debug
  build's MIR (copy propagation, removing unused locals, storage markers and drops of what
  needs none, folding constant branches, simplifying the control flow and instructions),
  `RA_JIT_MIR_OPT=2` what it runs above `-Copt-level=0` that is ported (also dead stores and
  unreachable blocks). Off by default. The interpreter's tests run every program at all three
  levels and require the same result ([docs/research/mir-gap.md](docs/research/mir-gap.md)).
- `ra-jit mir <dir> --crate <name> [--level N]` prints a crate's MIR as ra-jit compiles it,
  in rustc's `-Zunpretty=mir` format, and `harness/mirparity.py` compares it with rustc's
  function by function: at level 1, 21.5% of functions are identical to rustc's, against 4.0%
  without the passes.
- The README has a table of how fast programs ra-jit builds with LLVM run against
  `cargo build --release`, written by `bench.py --runtime` and `benchreport.py` and kept current
  by the `bench-readme` workflow on the Linux benchmark machine. A run in which any program's
  output differs from cargo's is never published. The first, on Linux x86-64, has every
  program's output the same as cargo's; on macOS arm64, ripgrep, fd, bat and jsonrt do not
  build whole yet (NEON intrinsics ra-jit does not have).
- [docs/research/](docs/research/README.md): what 2026's Rust compiler work means for ra-jit,
  and the MIR gap with rustc with the production gaps ranked; rustc's pass sources in
  [docs/upstream/rustc-mir-transform/](docs/upstream/rustc-mir-transform/SOURCE.md).

- Tiers ([#6](https://github.com/indexable-inc/ra-jit/issues/6)): a program that takes
  edits with `ra-jit-live` is sampled while it runs, and the functions it spends its time in
  are compiled again with LLVM, at `-O3` with what they call built in (level 2) and then with
  the branch counts level 2 took as `!prof` (level 3), and swapped in as patches without a
  restart. An edit sends an edited function back to Cranelift in the same build.
  `RA_JIT_TIER=off` turns it off. The Game of Life window shows each hot function's level
  and the promotions as they happen; `demo/lifebench` and `harness/tiers.py` measure and
  check it ([docs/tiers.md](docs/tiers.md)).
- `ra_jit_live::tiers()`, and the sampler that the daemon turns on in a running program.
  It stops once the worker that reads it is gone.
- Benchmarks on real projects that keep the README current: `harness/bench.py` measures
  ripgrep, fd, bat, hyperfine, just, axum and nushell at pinned commits (cold build, no-op,
  four kinds of edit, and how fast ripgrep's program runs) against incremental cargo, on both
  tiers, keeping every sample; `harness/benchreport.py` writes the README table and
  [docs/benchmarks.md](docs/benchmarks.md) from the result; the `bench-readme` workflow runs
  them on a dedicated Linux x86-64 machine after pushes to main and nightly, and commits the
  numbers to main when they moved beyond noise.
- `harness/bench.py` measures what [#7](https://github.com/indexable-inc/ra-jit/issues/7)
  asks of real projects: helix, a bevy example and zed (1.7M lines) join the seven, every project gets two more
  edits (a new function, a struct field set wherever the struct is built), the cold build is
  run `--cold-runs` times against fresh cargo, each with an empty store, and as many times
  in a second checkout with the store the first filled, the store's size is kept, and `--tests`
  runs each project's test suite under `cargo ra-jit test` and `cargo test` and fails on any
  test that ends differently. A project whose build fails keeps what it measured, with the
  error, and the run goes on to the next. A run with anything wrong or failed still writes its
  result and ends with exit code 3; `benchreport.py` publishes a test suite that ended differently in the tests
  table (a program that prints something else is still refused), and the workflow commits the
  numbers, then fails.
- `bench.py --setups` builds every step again with the fastest ways to build Rust there
  already are ([#8](https://github.com/indexable-inc/ra-jit/issues/8)): mold, wild,
  `-Zthreads=8`, rustc_codegen_cranelift on a pinned nightly, all three at once, and
  `cargo check`, each into its own target directory; `harness/hotpatch.py` times an edit of
  a running program under Dioxus's subsecond (`dx serve --hotpatch`) against `cargo ra-jit
  build` and `cargo build` with a restart, stopping the program before the build as `cargo
  watch` does. The bench shell has mold, wild, `dx` and the nightly, on Linux and on macOS.
  Measured on a MacBook (`docs/benchmarks/hotpatch/macbook.json`): subsecond 346 ms
  median, `cargo ra-jit build` with a restart 469 ms, `cargo build` with a restart 520 ms; a
  live patch through `cargo ra-jit build` 12.5 ms and through the editor's socket 4.0 ms
  (`docs/benchmarks/live/macbook-probe-cargo.json`). On Linux, 16 cores of a c7i.16xlarge
  (`docs/benchmarks/hotpatch/linux-x86_64.json`): subsecond 289 ms, `cargo ra-jit build`
  with a restart 40 ms, `cargo build` with a restart 361 ms. On ripgrep's cold build mold,
  wild, `-Zthreads=8` and cg_clif together are fastest (3.25 s against cargo's 5.56 s), and
  every edit there and on nushell builds faster under ra-jit than under any of them, `cargo
  check` included; with nothing changed, on ripgrep, cargo with mold is faster (28 ms against 33).
- Benchmark history and regression alerts
  ([#10](https://github.com/indexable-inc/ra-jit/issues/10)): `harness/benchhistory.py`
  appends each run's medians to `docs/benchmarks/history/*.jsonl` and checks the last against
  the runs before it on the same machine; the `bench-readme` workflow commits the history on
  every run, starts the site's deploy, and on a regression past the noise band opens (or
  comments on) an issue labelled `bench-regression` and fails. The live-edit latency workflow
  runs every night on main on a macOS arm64 runner and does the same with its history.
- [ra-jit.ix.dev/bench](https://ra-jit.ix.dev/bench): the benchmark history over commits, from
  `docs/benchmarks/history`: the latest run's table per project, and charts of each edit,
  the cold build, the daemon's peak memory, the store size and live edit-to-running latency
  ([#10](https://github.com/indexable-inc/ra-jit/issues/10)).
- LLVM is given what rustc gives it: reference parameters marked `noalias`, `readonly`,
  `dereferenceable`, `align`, `nonnull` and `noundef` as rustc marks them, addresses as
  pointers rather than integers, branches threaded before `-O3`, unsigned overflow checked as
  rustc checks it, a second round of cleanup after `-O3`, and, at a level above 1, the bodies
  of the callees it keeps calling and of what those call. Level 3 of `demo/lifebench` runs
  at 88% of a release build and 94 to 96% of one that checks overflow as the dev profile
  does, from 71 to 74% and 77 to 81%, measured side by side on a loaded machine
  ([docs/tiers.md](docs/tiers.md)).
- `harness/tierbench.py`, the benchmark table of docs/tiers.md; `--together` runs the builds
  of a round at once, so that a loaded machine slows them alike.
- `RA_JIT_DUMP_LLVM=after` prints the LLVM IR as the passes left it.
- Live-edit latency under real typing ([#9](https://github.com/indexable-inc/ra-jit/issues/9)):
  `harness/live_latency.py replay` types scripted edits one key at a time at 30 to 100 ms a
  key, with pauses at word ends, typos and backspaces, sends the whole file on each key as an
  editor does, and times every key whose text compiles to the program running it, by patch or
  by a restart as a launcher would do one; it counts the keys that do not compile, the
  superseded edits and the restarts. `restart` times a session's struct changes against
  `cargo run`. Every table has p95 and p99, and `LATENCY_JSON` carries the machine, its
  battery, the commit and the run, which `benchhistory.py live` reads. `harness/live-bevy`
  (bevy 0.20, generic systems) and `harness/live-egui` (egui 0.37, generic widgets) are real
  programs that take edits. Measured on a MacBook Pro in
  [docs/live.md](docs/live.md#latency-under-typing).
- A borrow checker, the crate `hir-ty-borrowck` in the rust-analyzer subtree, written to give
  rustc's verdict: Polonius alpha by default as nightly rustc does, NLL with
  `RA_JIT_BORROWCK=nll`. It gives rustc's errors on
  1055 of 1578 of rustc's own borrow-check tests that it can run, and none on any of the 245
  rustc accepts, scored by `harness/borrowck_ui.py`, and finds nothing in ripgrep, fd, bat,
  hyperfine, just, axum and nushell. Its errors are rust-analyzer diagnostics and
  `ra-jit borrowck` prints them ([docs/borrowck.md](docs/borrowck.md)).
- The daemon refuses to build an edit with a borrow-check error, as it does one with a type
  error; `RA_JIT_BORROWCK_REFUSE=0` turns that off. The checker still misses about a third
  of rustc's borrow-check errors, so `cargo check` stays the final word.
- `ra-jit borrowck`: the borrow-check errors of single files, or of a cargo workspace with
  `--project`.
- Inlining across crates, on by default in a program built whole with LLVM at `-O2` or
  `-O3`: it has what it calls in every crate built into it or shown to LLVM, the standard
  library's generic and `#[inline]` functions as rustc does across crates.
  `RA_JIT_INLINE=crate` keeps that to the function's own crate, `RA_JIT_INLINE=all` does it
  in other builds too. On Linux x86-64 the geometric mean of the 14 runtime benchmarks is
  1.15x the time of `cargo build --release` at `-O3` and 1.16x at `-O2`, against
  3.89x with each crate alone ([README](README.md#how-fast)); the programs other than
  the kernels take 2.3 to 3.7 times as long to build (ripgrep 30.1 s against 8.5 s). Commit 99cbb902ad's message
  says the levels alone turned it on; until b460bbe327 they did not, and it took
  `RA_JIT_INLINE=all`.

### Changed

- An edit in a package whose build script cargo runs again for it (one that names no
  `rerun-if-changed`, as ripgrep's, fd's and hyperfine's) no longer waits for a
  `cargo check` that runs every build script of the workspace: the daemon runs that one
  script again as cargo ran it and compares what it prints and writes with what it loaded,
  as `cargo ra-jit` already did, and asks cargo only when the two differ, the script is older
  than its source, or it was compiled reading a variable cargo did not set. A script
  `cargo ra-jit` ran again for the same build, with nothing it reads written since, is not
  run a second time. On Linux x86-64 an edit of ripgrep's `search` goes from 72 to 48 ms,
  fd's `run` from 78 to 42 ms and hyperfine's `run` from 80 to 35 ms.
- ra-jit is built one way, whether downloaded or built by nix: the new `dist` profile (thin
  LTO), optimized with a profile of ra-jit's own work (PGO) trained on edits of fd, bat, just
  and hyperfine. `.github/scripts/build_release.py` is the recipe; the release workflow runs it
  for every target and the flake's `daemon` (so `nix run`, `nix profile install` and the demo)
  runs it inside its derivation. The profiles are trained by the new `pgo` workflow, which
  runs again whenever Cargo.lock, a manifest or the toolchain changes, and pinned by sha256 in
  `pgo/profiles.json`, so every build uses the same file; the build says how much of the code
  changed since its profile was trained, and a release refuses a profile of other crates.
  `cargo install --locked --profile dist ra-jit` builds the same profile without PGO, which
  starts about 10% and edits about 3% slower than a release's binaries: cargo takes no compiler
  flags from a crate, and crates.io's crates are not the ones a profile can be trained on.
  Against the plain `release` build, on Linux: the
  cold start 14% shorter on nushell and on ripgrep, an edit of a function 8% shorter on
  nushell, the same peak memory. mimalloc v3 stays the allocator: jemalloc, mimalloc v2 and
  snmalloc start colder and edit no faster (an edit spends 1.8% of its time allocating, 73%
  revalidating salsa's memos). `release` itself is unchanged and stays the build to work with,
  so rebuilding ra-jit after a change takes as long as before
  ([docs/reports/RELEASE-BUILD.md](docs/reports/RELEASE-BUILD.md)).
- Under `RA_JIT_BACKEND=llvm`, a body of more than 10 000 Cranelift instructions is compiled
  by Cranelift. Since LLVM lowers a handler that takes more than the exception (tiers), it
  compiled every function, and an edit of nushell's `main` (35 778 instructions) waited
  229 ms for LLVM: on Linux x86-64 the edit went from 343 ms to 472 ms, and is 280 ms now.
  65 of nushell's 160 177 functions are over the limit, none of ripgrep's or of
  `demo/lifebench`'s. A tier still compiles such a body with LLVM.
- `vendor/rust-analyzer` and `vendor/rustc_abi` are git subtrees instead of submodules: a
  plain `git clone` builds, and the flake builds from its own tree with no second pin of the
  forks.
- The first build of a workspace through cargo has the daemon load it and compile ahead
  while cargo builds ([#11](https://github.com/indexable-inc/ra-jit/issues/11)): a spare
  worker reads the workspace and compiles what it can while cargo runs, hears what the
  build scripts said and where the proc macros are, and is the worker the build is handed
  to. On nushell, Linux (32 cores): ready for edits 58 s after a cold build from an empty
  store instead of 71 s; cargo's own build takes about 3 s longer, 40 s instead of 37 s,
  while the worker loads beside it.
- A build through cargo asks the toolchain's `rustc` beside `cargo` for its version and
  sysroot, not the rustup proxy: 16.6 ms to 7.3 ms each of its three calls on Linux.
- An edit to function bodies only lists no functions again and finds the program's tests
  as they were: 5.5 ms to 0.5 ms of an edit of nushell's `main`.
- `RA_JIT_STATS` shows where a splice and the refusals before a link spend their time
  (`splice-us`, `link-us`, `refuse-us`).
- `harness/realbench.py --pause S` waits once the daemon is ready, as a person would before
  the first edit.
- `vendor/subtree.py` replaces `vendor/subtree.sh`. `push` sends what main changed under
  `vendor/` to the fork, one fork commit per commit of main, and only fast-forwards the fork;
  the `fork sync` workflow runs it on every push to main, so the forks no longer fall behind
  (rust-analyzer's had: "warm edits" never reached it). `upstream` merges an upstream
  rust-analyzer release into `vendor/rust-analyzer` in one command, with conflicts left in
  the tree as `git merge` leaves them; before, that took a clone of the fork and a pull.

- Code ra-jit gives LLVM is faster: SIMD intrinsics on 128-bit lanes of integers (add,
  subtract, bitwise, saturating, compare, `simd_bitmask`) are vector instructions instead of a
  loop over lanes, and LLVM's own; a slot whose address is rounded up to an alignment above 16
  (`__m256i`) is an alloca of that alignment, which LLVM can take apart; `memcpy`, `memmove`
  and `memset` are LLVM's intrinsics rather than calls it knows nothing of, so a copy of a
  known size is a few loads and stores.
- A program built whole runs faster with LLVM: a landing pad whose cleanup only resumes is
  left out, so LLVM sees a call that cannot unwind into anything; what a check that is off
  (`ub_checks()` and its kind) guards is not compiled, so its panics do not use up what may be
  built in; a `match` is a chain of
  comparisons of up to 256 that LLVM makes a switch of; bodies are built in and shown up to
  16 calls deep (an iterator's adapters reach `Vec::push` that deep); `offset` is a
  `getelementptr inbounds`, also once Cranelift has inlined it; and a body built to be
  measured and then not used gives back what its build counted of its call's share, so that
  `vec![false; n]`'s `from_elem` is shown to LLVM, which then knows the vector's length and
  drops the sieve kernel's bounds checks. A body shown to LLVM carries its `#[inline]`,
  `#[inline(always)]` or `#[inline(never)]` as rustc emits it. An address the code computed
  as an integer (from `ptrtoint`s, through adds, phis and selects) is rebuilt after the passes
  as pointers and `getelementptr`s on the pointers it came from, so that LLVM can follow what
  is stored and loaded through it: nbody's array of deltas leaves the stack. On Linux x86-64 the geometric mean at `-O3` goes
  from 1.20x (ra-jit f3eeb5295e) to 1.15x of release's time ([README](README.md#how-fast)).
- On macOS arm64, ripgrep, fd, bat and jsonrt build whole: NEON's pairwise maximum, minimum
  and add, its table lookups (`tbl1`, `tbx1`), `sqdmulh`, `uaddlp` and `saddlp`, the CRC-32
  instructions, and the generic `simd_reduce_max`, `simd_reduce_min` and `simd_ctpop` are
  compiled, lane by lane or as one instruction. On a MacBook Pro (Apple M5 Max, macOS 27.0.1),
  median of three runs: ripgrep 1.15x, fd 1.01x, bat 1.16x and jsonrt 1.26x of release's
  time, every output the same as release's.

### Fixed

- bat builds whole: an `impl` inside a function body of a dependency (bincode's `EnumAccess`
  for `&mut Deserializer`, inside `deserialize_enum`) was not found, as rust-analyzer only
  looked into the bodies of workspace crates; a dependency's file is now lexed for an `impl`
  inside a function's braces, and only the functions holding one are lowered.
- A whole program's build no longer builds a body it found too large again at each call
  that allows no more: with what such a body counted given back, ripgrep's build across
  crates took 416 s on Linux x86-64 and numeric's 151 s; 30.1 s and 8.7 s now.
- LLVM's `instcombine`, as ra-jit's pipelines name it, runs without verifying that it reached
  its fixpoint, as the one in `default<O3>` does: an LLVM built with that check ended the
  process on a function of jsonrt, and of nushell at `-O3` across crates.
- fd built with inlining across crates crashed the LLVM backend on Linux and macOS: an address
  rebuilt as a `getelementptr` may be a sum that is a constant (a global's address plus an
  offset), which is in no block, and LLVM read freed memory when the builder was put before it.

- `harness/apps.py`'s perf stage ends with its timings again: since the rename from rustd it
  stored ra-jit's as `ra-jit_s` and printed `ra_jit_s`, a `KeyError` after every workload.
  wasmrun builds wasmtime 36.0.19 (was 25.0.3, which Dependabot's alerts name); its workload
  is the same as rustc's on both.
- `cargo ra-jit run` from many clients of one checkout at once starts each client's program,
  on macOS too. A package with a `default-run` (or one program of several) went to plain
  `cargo run`, which on macOS removes and copies again the program cargo has under
  `target/debug` before it starts it, so a client that started it meanwhile found no file
  ("No such file or directory") or hung before its first instruction; and rustc built the
  edit again. ra-jit now runs the `default-run` itself, holds cargo's lock on the build
  directory from putting the daemon's programs in place until they have started (a `cargo
  test` executable too), takes it to give cargo back its own, and on macOS puts a program in
  place as a clone, as cargo does: a program started from a hard link just made can be
  killed as it starts ([rust-lang/cargo#7821](https://github.com/rust-lang/cargo/issues/7821)).
  `cargo_ra_jit.py`'s 16 clients on 4 worktrees run 8 rounds of edits, all by the daemon.
- A program or test `cargo ra-jit` starts itself has what the `[env]` of cargo's
  configuration sets, as under `cargo run` and `cargo test`: the nearest `.cargo/config.toml`
  (or `config`) says a variable, then `$CARGO_HOME`'s, with `force` and `relative`.
- `cargo_ra_jit.py`'s life workspace no longer races with itself on macOS: the test that runs
  `cargo build` inside and the one that starts `other` take turns, since cargo removes and
  copies each program again there on every build and the second sometimes found none (plain
  cargo failed it as often as ra-jit).
- A workspace the daemon loads after a change whose build scripts did not all run is not
  loaded: the crates of a script that failed were read without its `OUT_DIR`, so thiserror's
  `include!` went missing, every derived `Error` was unimplemented and `?` resolved to the
  body-less `FromResidual::from_residual` for the rest of the daemon's life (what hyperfine's
  edits under plain cargo met now and then on Linux before the edit stopped running every
  script through cargo). It says which script failed instead, as an edit that runs the
  scripts again already did.
- `bench.py` without `--reference` built plain cargo into `target-ref` inside the checkout,
  which cargo takes for a source of the package: ripgrep's, fd's and hyperfine's build scripts
  name no `rerun-if-changed`, so cargo ran them again and built the package again at every
  build, and "nothing changed" took cargo 300 to 390 ms instead of 30 to 60. The README run of
  2026-10-20 at `c8f58ac7d9` had it; cargo's target directories are now outside the checkout
  (`WORK/reference` by default).
- `nix profile install github:indexable-inc/ra-jit#ra-jit`, as the README says to run it,
  installs ra-jit and the programs it runs; the flake had no package of that name.
- A release's archive and the flake's build have `ra-jit-rustc-first`, which `cargo ra-jit`
  runs in the first build of a workspace; they had only ra-jit, cargo-ra-jit and ra-jit-rustc.
- A worker that loaded its workspace beside cargo's first build
  ([#11](https://github.com/indexable-inc/ra-jit/issues/11)) ends when its daemon does, as
  every other worker does: once told what cargo built, it waited for a lock on its standard
  input that it held itself, so a daemon that crashed or was killed left the worker running,
  holding its memory (2 GB on fd) and its socket, and the next daemon's first build fell
  back to cargo ("another daemon serves"). `cargo_ra_jit.py`'s "workers of the killed
  daemon are gone" fails without this.
- fd, tokei and eza build whole and run as cargo's builds do: a crate rustc built nothing
  of keeps its `static`s ra-jit's own (fd's `INIT`); a package built only for a build script
  or a proc macro, which `cargo metadata`'s single graph can still show as a dependency of
  the program's, checks overflow as the program does, so rust-analyzer's view of it no
  longer mixes two profiles; and `runbench.py` turns the release profile's `lto` off for the
  whole build, which needs rustc's libraries. bat and nushell do not link yet.
- A program ra-jit builds whole, with its own `main` (`runbench.py`'s builds), ran: on macOS
  the code-free stand-in rustc builds for the program had a weak `main` of its own, and
  ra-jit's object was weakened too, so the linker kept the stand-in's and the program
  exited 0 at once, printing nothing. ra-jit's `main` now stays strong. On macOS and Linux
  a `#[global_allocator]` was ignored in such a program, as the stand-ins declare none and
  rustc's allocator shim went to the default allocator; a stand-in now declares one that
  forwards to functions rust-analyzer's expansion of the attribute defines, under the
  `#[cfg]`s of the static and of the modules it is in, and not for one in a string, as
  heapless's docs have (`wholeprogram.py`).
- `&(1 / 2)`, a division by a constant, and `&mut []` are promoted to constants as rustc
  promotes them, so a reference to one lives as long as the program.
- Before the 2024 edition, a panic in a block's tail drops the tail's temporaries after the
  block's locals, as rustc does; ra-jit dropped them first, so a `Drop` that printed or
  locked saw the other order (`breakers.py unwind_tail_after_local`).
- What leaving a scope early drops (`break`, `continue`, `return`, a future dropped at an
  `await`) is placed on the line where the scope ends, as rustc places it, so a backtrace or
  a debugger's step through those drops shows the same lines under ra-jit as under rustc.
- On macOS, signing a splice reuses the hash of each appended page that is the same as the
  last splice's: 21 ms to 5 ms of each edit of nushell.
- The value of a `match` arm's guard was marked dead before the branch that reads it; with
  storage markers kept, as rustc's optimized MIR keeps them above `-Copt-level=0`, the guard
  could take the wrong arm.
- The code of a function grows in a straight line with the arms of a match that leave
  through `?`, as `#[derive(Serialize, Deserialize)]` writes for an enum: it grew with
  their square, 329 MB for one function of 285 arms, which an AArch64 branch cannot reach
  across ([#5](https://github.com/indexable-inc/ra-jit/issues/5)). `harness/growth.py`
  checks it.
- An edit that arrives while a function is being compiled at a higher level no longer waits
  for LLVM: the compile lets go of the database before LLVM runs.
- A workspace under a directory whose name has a space in it is built against what rustc
  built of it ([#14](https://github.com/indexable-inc/ra-jit/issues/14)). The daemon cut
  the dep-info file that names a crate's sources at every space, found none of the
  workspace's crates among rustc's, and named their functions itself, so a function it
  refused had no copy of rustc's to link to. Dep-info is now read and written as rustc writes
  it, in one place: a space in a path is `\ `, and `$`, `#` and backslashes are themselves;
  the values of the variables in it are unescaped too. `harness/spaces.py` builds a
  workspace under such a directory end to end.
- On macOS a program whose unwind table is larger than 1 MB takes edits as splices, and so
  live in the running program ([#13](https://github.com/indexable-inc/ra-jit/issues/13)).
  The daemon gave Apple's linker a fixed `-headerpad` of 1 MiB, and a splice moves the
  table into that room, so a GUI app of 570 crates with 3 MB of table, or nushell with 3.2
  MiB, linked every edit in full. The link now reads its headerpad from a file beside the
  executable, last on the line: 1 MiB, or what the line's own flags ask for. After each full
  link the daemon reads the header, and when the room is short it links once again with what
  the table needs and a quarter more, which the links after start from, in later daemons
  too. nushell's edit of `main` went from 878 ms to 276 ms on an M5 Max, and a library edit
  from 693 ms to 166 ms. `harness/headerpad.py` checks it.
- The first edit after the daemon is ready is as fast as the ones after it
  ([#12](https://github.com/indexable-inc/ra-jit/issues/12)). Two things made it slow.
  On Linux the first splice after a full link took 2.5 s of nushell's: it copied every
  debug section it added to, about 500 MB, behind the code it appended, and it cut the
  file to size, which on XFS writes out at the close all of the file the linker had not
  yet written. The sections now go where splices write them right after the link, and a
  splice only lengthens the file, so the first splice's link takes 7 ms, as those after it
  do. And code the daemon took from the store does not say where in the checkout it is
  from, so the first edit of a crate it came from compiled all of it and wrote the whole
  object again: 71,661 functions of nushell. The worker now compiles that code again
  by itself after its first build, and the daemon says the checkout is ready once it has.
  nushell's first edit of `main` after a cold build from a warm store went from 9.1 s to
  0.37 s on Linux and from 7-8 s to 0.3-0.4 s on an M5 Max. `harness/storefirst.py`
  checks it.
- On macOS a spliced executable lists its sections in the order of their addresses, and
  each segment a splice adds has a section, as `dyld_info` requires: it read none of the
  spliced executables on macOS 27 (`harness/splices.py`: 11 of 71 right, now 71).
- Tiered code that calls what the executable has no symbol of (`memcmp` where rustc's code
  calls `bcmp`, a `pthread_` function only rustc's inlined code reached) no longer has the
  program linked in full and started over at level 1, after which every later patch of it
  was linked in full too: that function stays where it was, and says why
  ([docs/tiers.md](docs/tiers.md), "Code no patch can hold").
- The thread that compiles at a higher level has a stack of 64 MiB, and the bodies a body
  built in shows are ten deep at most: a deep chain of them overflowed the 2 MiB default and
  took the worker down.
- Tiered code that calls what the executable has no symbol of (`memcmp` where rustc's code
  calls `bcmp`, a `pthread_` function only rustc's inlined code reached) no longer has the
  program linked in full and started over at level 1, after which every later patch of it
  was linked in full too: that function stays where it was, and says why
  ([docs/tiers.md](docs/tiers.md), "Code no patch can hold").
- The thread that compiles at a higher level has a stack of 64 MiB, and the bodies a body
  built in shows are ten deep at most: a deep chain of them overflowed the 2 MiB default and
  took the worker down.

## [0.1.0] - 2026-10-20

The first release.

### Added

- `ra-jit`, a compiler daemon that keeps a workspace loaded in rust-analyzer and generates
  code from its MIR with Cranelift. After the first build, an edit rebuilds only the
  functions that changed.
- `cargo ra-jit build|run|test|check`, with cargo's own flags. Anything the daemon cannot
  build falls back to cargo, and says so.
- `ra-jit enable` and `ra-jit disable`: route plain `cargo build|run|test|check` through
  ra-jit, and put cargo's config back.
- `ra-jit-live` (on crates.io), a crate that applies edits to a running program, keeping
  its state (macOS on Apple silicon).
- A nix flake: `nix run github:indexable-inc/ra-jit` opens a Game of Life you can edit
  while it runs; `#ra-jit` is the cargo wrapper.
- Prebuilt binaries for Linux (x86_64, aarch64) and macOS (aarch64).
- `cargo install ra-jit` from crates.io (Cranelift; `--features llvm` needs LLVM 20). The
  rust-analyzer and rustc_abi forks it builds on are published as `ra-jit-<name>`.

### Known limits

- Rust 1.99.0 only.
- No borrow checker: keep running `cargo check`.
- Changing a struct the running program uses asks for a restart.

[Unreleased]: https://github.com/indexable-inc/ra-jit/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/indexable-inc/ra-jit/releases/tag/v0.1.0

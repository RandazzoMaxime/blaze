# Changelog

All notable changes to ra-jit are written here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

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
  output differs from cargo's is never published; none is yet, since on macOS a program ra-jit
  builds every crate of, with its own `main`, does not run (docs/research/mir-gap.md).
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

### Changed

- `vendor/rust-analyzer` and `vendor/rustc_abi` are git subtrees instead of submodules: a
  plain `git clone` builds, and the flake builds from its own tree with no second pin of the
  forks.
- `vendor/subtree.py` replaces `vendor/subtree.sh`. `push` sends what main changed under
  `vendor/` to the fork, one fork commit per commit of main, and only fast-forwards the fork;
  the `fork sync` workflow runs it on every push to main, so the forks no longer fall behind
  (rust-analyzer's had: "warm edits" never reached it). `upstream` merges an upstream
  rust-analyzer release into `vendor/rust-analyzer` in one command, with conflicts left in
  the tree as `git merge` leaves them; before, that took a clone of the fork and a pull.

### Fixed

- A program ra-jit builds whole, with its own `main` (`runbench.py`'s builds), ran: on macOS
  the code-free stand-in rustc builds for the program had a weak `main` of its own, and
  ra-jit's object was weakened too, so the linker kept the stand-in's and the program
  exited 0 at once, printing nothing. ra-jit's `main` now stays strong. On macOS and Linux
  a `#[global_allocator]` was ignored in such a program, as the stand-ins declare none and
  rustc's allocator shim went to the default allocator; a stand-in now declares one that
  forwards to functions rust-analyzer's expansion of the attribute defines, under the
  `#[cfg]`s of the static and of the modules it is in, and not for one in a string, as
  heapless's docs have (`wholeprogram.py`).
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

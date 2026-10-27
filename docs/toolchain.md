# blaze's own toolchain

blaze is its own Rust toolchain. It builds every project with one toolchain that blaze
controls, names and ships, whatever the project pins: `blaze-2026-10-25` today. It is not the
project's toolchain and it does not try to be. The project's own toolchain, and its CI, stay
the final word on whether the code builds.

Andrew, 2026-10-26, answering "blaze uses its pinned nightly even when the project pins stable":
"you should use YOUR OWN NIGHTLY that you control since you are trying to replace rustc".

## Why one toolchain, and why blaze's

blaze compiles your code itself (Cranelift or LLVM, from rust-analyzer's MIR) and links it with
code rustc compiled. The two must agree on how types are laid out, how functions are called,
what a `TypeId` hashes, the names of symbols, libtest's entry point, the proc-macro bridge and
what rustc answers about targets. blaze knows those facts for one toolchain, checked by the
corpus difftest, and that toolchain is the base blaze replaces piece by piece
([no-rustc.md](no-rustc.md): cargo, then rustc, then the standard library's build). Following
each project's pin would mean knowing every release's facts and checking every release; the
goal is the reverse, no rustc at all. So the toolchain is part of blaze, versioned with it, like
the standard library is part of a compiler.

## What it is today

- **Name:** `blaze-<date>` (`blaze-2026-10-25`), after the date of the Rust nightly it is built
  from (`harness/toolchain`, `nightly-2026-10-25`). `blaze toolchain` prints it and where it
  is; `blaze toolchain --path` prints the directory alone.
- **Bytes:** the upstream nightly's `rustc`, `cargo`, `rust-std` and `rust-src`, unchanged, from
  static.rust-lang.org, each checked against the sha256 compiled into blaze
  (`crates/ra-jit/src/cargo/toolchain/pins.txt`, `harness/toolchain_pins.py`). So `rustc -V`
  still says `1.101.0-nightly (12433c307 2026-10-24)`.
- **Where:** `~/.blaze/toolchains/nightly-2026-10-25-<host>`, or rustup's `nightly-2026-10-25`
  when rustup already has it with `rust-src` (the same bytes). The installer downloads it once;
  `blaze toolchain --path` prints its directory.
- **Who uses it:** cargo, rustc and the daemon that blaze starts (`CARGO`, `RUSTC`, `RUSTDOC`,
  `PATH` for its own processes). The user's `cargo`, `rustc` and pins are untouched.
- **Bumps:** every day, to the newest nightly, by a workflow, with the merge gate and the
  release train as the gate ([The daily bump](#the-daily-bump)).

## The daily bump

blaze tracks the latest Rust nightly. `.github/workflows/toolchain-bump.yaml` runs every day at
05:40 UTC (nightlies are published at about 00:00 to 02:00 UTC), and by hand with a date
(`gh workflow run toolchain-bump.yaml -f date=2026-11-02`):

1. On the branch `toolchain/bump` (the open bump's with main merged in, or a new one from main),
   `harness/toolchain_bump.py` pins the newest nightly whose manifest has every component blaze
   needs on every host it ships for. It writes `harness/toolchain` and `pins.txt`, re-vendors
   `vendor/rustc_abi` and the proc-macro server from rust-lang/rust at the new commit, and
   moves the pin in `install.sh`, the server's manifest, `Cargo.toml` and the flake.
2. `harness/answers.py --write` on Linux x86-64, Linux AArch64 and macOS arm64, since the
   target specs depend on the host rustc was built for; the three are merged into
   `answers/rustc.json`.
3. The pull request `toolchain: pin nightly-YYYY-MM-DD` is opened, or updated when one is open,
   which runs the merge gate, and the release train is started on the branch. The train is
   what a bump needs beyond the gate: the corpus difftest with `--layouts` and every other
   harness on four platforms, rustc's borrow-check UI tests in both modes against the new
   nightly's own verdicts (at its commit, `--min-pass` as the floor), `fork alone` and
   `cargo ra-jit` on real projects.
4. When the train finishes, `harness/stdcount.py` compiles the standard library with the bump's
   build and with main's, each against its own nightly's `rust-src`, and counts the functions
   each refuses. The verdict (`.github/scripts/toolchain_bump_ci.py verdict`):
   - **green**: no job or harness red that is green on main's newest finished train, and no
     more std refusals than main: `gh pr merge --auto`, so the merge queue lands it once the
     gate passes, and the next release of blaze carries the new pin. A job red on main too is
     listed and not counted against the bump; the train's own issue covers it.
   - **red**: a comment on the pull request with what broke, and the issue
     "Toolchain bump to <date> is blocked", opened or commented on and retitled to the
     newest date, with how many days main's pin is behind. The pull request stays open.
5. A step before the pull request that fails (a vendoring conflict, no usable nightly in two
   weeks, a conflict merging main into the bump) opens or comments on the same issue with what
   failed, and so does a bump open for a day without merging. Nothing is skipped quietly.

Fixes for what a nightly breaks (layouts, the ABI, std's source, intrinsics, borrow-check
verdicts) are pushed to `toolchain/bump`. The next day's run merges main into it and moves it to
that day's nightly from there, so they are kept, and the train runs again.

## What it means for a project

- A project that pins another toolchain (a stable release, an older nightly) builds with
  blaze's. blaze says so once per project and pin, as a note, not an error, and switches
  nothing:
  `note: blaze builds with its own toolchain, blaze-2026-10-25, not rustc 1.99.0 (...), which
  the project's rust-toolchain file names. ...`
- Rust keeps building old code, so stable code builds; `#![feature(...)]` code builds too.
- Code blaze's toolchain cannot build (a nightly feature it removed, a library item newer than
  it, `rust-version` above 1.101) fails with rustc's error and a note that blaze built it with
  its own toolchain, not the project's.
- Keep `cargo build`/`cargo test` with the project's toolchain in CI. blaze is for the edit
  loop; the project's toolchain decides.

## Where it goes: no rustc

blaze is moving off rustc ([no-rustc.md](no-rustc.md)); the rule is in [AGENTS.md](../AGENTS.md).
blaze's nightly is the definition of the language and the standard library blaze matches while
rustc is still in the tree: its layouts, ABI, `TypeId`s and answers are what blaze agrees with.
It is not a rustc blaze maintains. As the no-rustc steps land its pieces go: cargo, then rustc
with rust-lld and the proc-macro server, then `rust-std`; what is left is `rust-src`, shipped as
part of blaze, and the toolchain is blaze alone.

- [#402](https://github.com/indexable-inc/blaze/issues/402): ship the standard library's
  source (`rust-src` of the pinned nightly) with blaze's release, so blaze's toolchain needs no
  upstream download once rustc is gone.
- [#403](https://github.com/indexable-inc/blaze/issues/403): shrink the download as each
  no-rustc step removes a piece (`cargo`, `rustc`, `rust-std`), down to blaze alone.
- [#404](https://github.com/indexable-inc/blaze/issues/404): the bump, every day
  ([The daily bump](#the-daily-bump)), the `blaze-<date>` name moving with it.

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
  when rustup already has it with `rust-src` (the same bytes). The installer downloads it once
  and, when rustup is there, links it as the rustup toolchain `blaze`, so `cargo +blaze` and
  `rustc +blaze` are the toolchain blaze builds with.
- **Who uses it:** cargo, rustc and the daemon that blaze starts (`CARGO`, `RUSTC`, `RUSTDOC`,
  `PATH` for its own processes). The user's `cargo`, `rustc` and pins are untouched.
- **Bumps:** on blaze's schedule, deliberately, with `harness/toolchain_bump.py`: it moves the
  pin, re-vendors `rustc_abi` and the proc-macro server, regenerates the pins and answers, and
  the corpus difftest (`harness/difftest.py corpus`) and the edit harnesses are the gate.

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

## Where it goes

1. **blaze's own build of the nightly** ([#402](https://github.com/indexable-inc/blaze/issues/402)): a snapshot of rust-lang/rust under
   indexable-inc at the pinned commit, built by blaze's release workflow (rustc, std,
   rust-src), so `rustc -V` names blaze's toolchain and blaze can carry the patches it needs
   (its own proc-macro server, its symbol and layout hooks) instead of matching upstream from
   outside.
2. **Hosting** ([#403](https://github.com/indexable-inc/blaze/issues/403)): the builds published with blaze's releases and pinned by hash
   as today's upstream bytes are, so the installer fetches blaze's toolchain and not
   static.rust-lang.org's.
3. **The bump process** ([#404](https://github.com/indexable-inc/blaze/issues/404)): a scheduled, gated move to a newer upstream nightly:
   rebase the patches, rebuild, run the corpus and edit harnesses, then publish.
4. Then the pieces go away as no-rustc lands: cargo, rustc with rust-lld and the proc-macro
   server, then `rust-std`; `rust-src` stays as part of blaze's release.

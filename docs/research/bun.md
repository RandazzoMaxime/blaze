# bun's Rust port through ra-jit

2026-10-21. Claims are marked **read** (in a source we read, or measured here, with the
command), **reported** (someone says so, unchecked) or **unverified** (our inference). What was
fetched is in [docs/upstream/bun/](../upstream/bun/SOURCE.md).

## What bun is now

- bun was rewritten from Zig to Rust in one pull request, oven-sh/bun#30412, "Rewrite Bun in
  Rust", merged 2026-05-14: 1,009,257 lines added, 4,024 removed, 2,188 files (read,
  `pr-30412.json`).
- The port was mechanical, file by file from 1,448 `.zig` files, to behave as the Zig did and
  pass the same test suite, and is to be made idiomatic after 1.4 ships (reported, the blog
  post "Rewriting Bun in Rust", 2026-07-08). Bun was 535,496 lines of Zig without comments, and
  about 4% of the Rust (~27,000 of ~780,000 lines) is in `unsafe` blocks (reported, same post).
- The pin here is the tag `bun-v1.4.3`, commit `c6da4a4d3010e5553438c60f6bd76d981976867c`
  (read, `git ls-remote`). Its Rust is 1,247,251 lines in `src/**/*.rs` (972,091 that are not
  blank or a line comment) in 103 workspace crates (read, `wc -l` and `Cargo.toml`'s members at
  that commit), on `nightly-2026-09-15` (read, `rust-toolchain.toml`).
- It uses nightly features ra-jit's frontend does not all have: `adt_const_params` (on in 11
  crates), `core::mem::type_info` in `bun_collections::MultiArrayList`, and
  `inherent_associated_types` in `bun_bundler` (read, the sources).

## How bun is built, and what the harness builds

- bun's own build (`bun scripts/build.ts`, ninja) compiles its C and C++ (JavaScriptCore from a
  prebuilt WebKit, BoringSSL, uSockets, ...), builds its Rust as one static library,
  `bun_runtime`, and links the two with clang (read, `build.ninja` from `--configure-only`).
  It needs LLVM 23, cmake, ninja, go and a bun (read, `docs/project/contributing.mdx`).
- ra-jit builds and splices executables rustc links. `harness/bunprep.py` has bun's build make
  everything but the Rust once per revision (1,227 of its 1,478 ninja steps on macOS arm64;
  read), writes the native half of bun's link to a file of link arguments, and adds one
  workspace member, `bun-exe`: a `#![no_main]` binary of `bun_runtime` whose build script puts
  those arguments on rustc's link line. bun's tree changes in four places: the member in the workspace,
  `debug-assertions = false` in `[profile.dev]` (bun's C++ is its release build, and a Rust
  build with assertions calls functions only a C++ build with them has), a feature that leaves
  the allocator shim's marker to rustc, and on macOS the system linker in place of ld64.lld
  (read, `bunprep.py`'s `overlay`).
- The program that comes out is bun: `--version`, `--revision`, `-e`, `build --no-bundle
  --minify` and `Bun.Transpiler` print what bun's own release build prints (read, `apps.json`'s
  runs, compared by `runbench.py` between cargo's build and bun's).

## What broke, and what was fixed

Each fix is its own commit on `main` with a regression case (read, the commits):

| What | Where it showed | Case |
|---|---|---|
| `cargo ra-jit` went to cargo for any rustc but 1.99.0; `RA_JIT_OTHER_TOOLCHAIN=1` builds with it | every build of bun | `soakrows.py`, "a project pinned to a nightly" |
| nightly cargo passes `-Z embed-metadata=no`, which sent the build to cargo | every build | the same row |
| nightly cargo's second build-dir layout: build scripts' output and the seeded target were not found | every build | the same row |
| an edit that makes rustc's copy of a function left to rustc out of date failed the build; it is cargo's to build | signature edits | `soakrows.py`, "an item added beside a function left to rustc" |
| a module whose file is outside its package (`#[path = "../.."]`) was not loaded | `bun_runtime` | `soakrows.py`, "a module file outside its package" |
| a `macro_rules!` defined by another crate's macro (`bun_core::define_scoped_log!`) was not in scope in later modules (`IncompleteExpr`) | sent every signature edit of `bun_runtime` to cargo | breaker `macro_defined_by_foreign_macro`, hir-def test |
| the stand-in for a crate had an `extern crate` per `--extern`, and nightly cargo names each dependency twice (`E0259`) | whole-program build | stub.rs test |
| whole-program builds stopped when cargo's link of the stand-ins failed for want of `#[no_mangle]` functions C calls | whole-program build | `apps.json` `nativecalls` |
| LLVM's `instcombine` named in the `-O3` pipeline aborts the process when one run leaves work (AArch64) | whole-program build | `llvm.rs` test |
| `simd_reduce_max`/`min` (NEON `vmaxvq_u8`) were not compiled | 164 functions | breaker `simd_reduce_max` |
| a `Box<[T], &Arena>` (an allocator with a size) was not dereferenced, and its drop read the pointer at offset 0 | 151 functions | breaker crate `allocated` |
| a borrowed constant with a cast in it (`&[b"browser" as &[u8], b"module"][..]`) was built on the stack, not promoted, and its slice outlived the frame | `new Bun.Transpiler()` crashed in the dev-loop build | breaker `promoted_index_of_casts` |

Filed, not fixed (read, the issues):

- [#33](https://github.com/indexable-inc/ra-jit/issues/33): an executable linked with
  `-no_compact_unwind` is linked in full at every edit on macOS. bunprep drops the flag.
  Fixed since (2026-10-22, `harness/nocompact.py`); bunprep drops it still, so the numbers
  here stay comparable.
- [#34](https://github.com/indexable-inc/ra-jit/issues/34): `adt_const_params` constants and
  `type_info` in const eval. Functions that reach them are left to rustc: in the dev loop rustc's
  copy is linked; the whole-program report left 1,645 functions out, including 907 reaching
  `type_info` and 304 reaching `adt_const_params`.
- [#35](https://github.com/indexable-inc/ra-jit/issues/35): everything the whole-program build
  can't compile yet, by cause, of which #34 is most.
- [#47](https://github.com/indexable-inc/ra-jit/issues/47): the cold build starts over once per
  static a function left to rustc uses ("a static is used by a function left to rustc", five
  rounds on bun), where one round could leave them all.
- [#32](https://github.com/indexable-inc/ra-jit/issues/32) (commented): a signature edit to
  `bun_runtime` goes to cargo, because a function left to rustc (`posix_signal_handler`) uses a
  writable static (`SHOULD_ABORT`) rustc's copy has no symbol for.

## Numbers

`bench.py bun --backend cranelift --runs 3` on macOS arm64 (Apple M5 Max, 18 cores, 128 GiB),
ra-jit `1645168dc4`, 2026-10-21; the machine was shared, load average 28 to 45 while it ran, so
every number is noisy (read, [docs/benchmarks/bun-macos-arm64.json](../benchmarks/bun-macos-arm64.json)).
Median of three edits, warm, incremental `cargo build` → `cargo ra-jit build`:

| Edit | cargo | ra-jit | How ra-jit built it |
|---|---:|---:|---|
| nothing changed | 0.17 s | 0.07 s | daemon |
| a function's body (`bun_runtime`'s CLI) | 12.2 s | 1.1 s | spliced into the executable |
| a widely used function (`bun_core`'s `StringBuilder::append`) | 26.7 s | 1.8 s | spliced |
| a generic function (`MultiArrayList::push`) | 31.8 s | 72.5 s | daemon, the whole object written and linked again |
| a new function | 12.3 s | 50.5 s | daemon, linked again |
| a signature (`bun_runtime`'s `multi_run`) | 9.8 s | 126 s | went to cargo (#32): `run` calls `posix_signal_handler`, left to rustc, which writes the static `SHOULD_ABORT` |

- Cold, until the daemon is ready: 291 s (164 s of build and 128 s until ready), against
  106 s for a clean `cargo build`; with another checkout's store, 97 s + 98 s. The daemon holds
  28 GB once ready; an edit peaks near 40 GB against cargo's 2 GB (read, same file).
- The executable runs: `Bun.Transpiler` minifying five of bun's TypeScript modules twenty times
  prints what cargo's build prints, in 1.06 s against 0.75 s for cargo's debug build and 0.12 s
  for its release build (median of ten; read, the result's `runtime`).
- Before the promotion fix in the table above, that run crashed (`SIGSEGV`): the dev loop's
  binary is the one bun's users of `Bun.Transpiler` would run (read, `bench.py` on the commit
  before it).
- How fast ra-jit's whole-program LLVM build runs is not measured: it does not link (#35).

The reduced nightly fixtures in `harness/bun-repros` keep these two refusals measurable without
building Bun's native C and C++ dependencies. Run them with `python3 harness/bun_repros.py` after
installing `nightly-2026-09-15`. The fixtures compare ra-jit's binary with cargo's binary and fail
if the expected refusal reason disappears without changing the fixture's expectation.

## What is not known

- Whether the Linux x86-64 path works: `nix develop .#bench-bun` evaluates, and the `bench-bun`
  workflow has not run yet (unverified).
- bun's own test suite (`bun bd test`) was not run against ra-jit's binary; the comparison is
  the harness's command lines (unverified beyond those).

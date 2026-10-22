<h1 align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/brand/ix-mark-on-dark.svg">
    <img alt="ix" src="docs/brand/ix-mark-on-light.svg" height="40" align="absmiddle">
  </picture>
  ra-jit
</h1>

Edit Rust. See it change in milliseconds, in the program that is already running.

```
nix run github:indexable-inc/ra-jit
```

That opens a Game of Life window you can edit while it runs (live on macOS Apple silicon).

![Editing the running Game of Life: rule, colours and speed change live](docs/demo.avif)

<p align="center">
  <a href="https://ra-jit.ix.dev/updates?src=readme">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="docs/brand/signup-on-dark.svg">
      <img alt="Tell us you use ra-jit" src="docs/brand/signup-on-light.svg" height="36">
    </picture>
  </a>
</p>

<p align="center">For release news: Watch → Custom → Releases on GitHub.</p>

## What you get

- **Live edits.** Save a file and the running program has the new code about a millisecond
  later. Its state stays. ([docs/live.md](docs/live.md))
- **Fast builds.** After the first build, an edit rebuilds only the functions you changed.
  No `rustc`, no waiting.
- **Same cargo.** `cargo ra-jit build|run|test|check` in any project.

## How fast

Live edit to running code, Game of Life demo (median of 20 edits, one 96-core Linux machine):

| Edit | incremental `cargo build` | ra-jit live | |
|---|---:|---:|---:|
| One function | 169 ms | 7 ms | **24x** |
| One function, 300 new functions added | 282 ms | 60 ms | 4.7x |

<!-- bench:start -->
Edit, then build, on real projects at pinned commits. Each cell is incremental `cargo build` → `cargo ra-jit build`, both warm, same linker, median of 5 edits. Linux x86-64, AWS c7i.16xlarge (dev-aws-7), pinned to 16 cores with taskset (Intel(R) Xeon(R) Platinum 8488C, 16 of its 64 cores, 124 GiB), ra-jit [`2b2f2dcd5b`](https://github.com/indexable-inc/ra-jit/commit/2b2f2dcd5bc4d7132fe62bf36669a34c657dafe7) on 2026-10-20.

| Project | Edit a function | Edit a widely used function | Change a signature | Nothing changed |
|---|---:|---:|---:|---:|
| ripgrep 15.1.0 | 655 ms → **104 ms** (6.3x) | 1.00 s → **62 ms** (16x) | 809 ms → **90 ms** (9.0x) | 30 ms → **33 ms** (0.9x) |
| fd 10.3.0 | 649 ms → **115 ms** (5.6x) | 600 ms → **103 ms** (5.8x) | 652 ms → **129 ms** (5.1x) | 43 ms → **35 ms** (1.2x) |
| bat 0.26.0 | 650 ms → **69 ms** (9.4x) | 936 ms → **58 ms** (16x) | 900 ms → **64 ms** (14x) | 81 ms → **32 ms** (2.5x) |
| hyperfine 1.19.0 | 528 ms → **117 ms** (4.5x) | 569 ms → **117 ms** (4.9x) | 583 ms → **162 ms** (3.6x) | 56 ms → **32 ms** (1.8x) |
| just 1.43.0 | 899 ms → **58 ms** (16x) | 926 ms → **58 ms** (16x) | 900 ms → **93 ms** (9.7x) | 54 ms → **30 ms** (1.8x) |
| axum 0.8.6 | 559 ms → **69 ms** (8.1x) | 1.11 s → **63 ms** (18x) |  | 102 ms → **34 ms** (3.0x) |
| nushell 0.108.0 | 1.62 s → **279 ms** (5.8x) | 5.78 s → **271 ms** (21x) | 5.82 s → **336 ms** (17x) | 240 ms → **38 ms** (6.3x) |
| helix 25.07.1 | 2.80 s → **231 ms** (12x) | 4.16 s → **86 ms** (48x) | 3.56 s → **354 ms** (10x) | 114 ms → **34 ms** (3.4x) |
| bevy 0.20.0 |  |  |  | 264 ms → **41 ms** (6.4x) |

- bevy 0.20.0: only the steps shown are measured, then the build failed: examples/showcase/breakout.rs:67:18: no method `chain` on type `(fn apply_velocity(Query<'_, '_, (&mut Transform, &Velocity), ()>, Res<'_,` …

Measured by hand with `harness/bench.py`; the `bench-readme` workflow re-runs it on the dedicated benchmark machine. The ra-jit measured is `cargo build --release`; the binaries a release ships are the `dist` profile with PGO, which start and edit faster ([how](https://github.com/indexable-inc/ra-jit/blob/main/docs/reports/RELEASE-BUILD.md)). Cold builds, the spread of every number, the LLVM tier and how fast the built programs run: [docs/benchmarks.md](docs/benchmarks.md) ([raw JSON](docs/benchmarks/linux-x86_64.json)).
<!-- bench:end -->

<!-- runbench:start -->
How fast the programs run, built whole by ra-jit with LLVM (every crate of the program compiled by ra-jit, with its own `main`), each time ÷ the time of `cargo build --release`. Lower is faster; 1.00x is as fast as release. Every program's exit status and output were the same as the release build's; a run in which any differed is never published. Linux x86-64: AWS c6i.16xlarge (Intel Xeon Platinum 8375C, 64 vCPUs, 123 GiB), NixOS 26.05, Linux 6.18, rustc 1.99.0, ra-jit `13ee5a5339` on branch `runtime-parity`, 2026-10-21. Median of 5 runs, the builds taking turns (numeric: 15).

| Benchmark | `cargo build --release` | release with overflow checks | ra-jit `-O3`, each crate alone | ra-jit `-O2` | ra-jit `-O3` |
|---|---:|---:|---:|---:|---:|
| sieve | 0.069 s | 1.00x | 4.35x | 0.93x | **0.97x** |
| collatz | 0.135 s | 1.94x | 1.03x | 1.02x | **1.01x** |
| fnv | 0.077 s | 1.00x | 2.74x | 1.00x | **1.00x** |
| qsort | 0.279 s | 1.03x | 2.04x | 1.05x | **1.04x** |
| matmul | 0.034 s | 1.00x | 9.62x | 1.14x | **1.15x** |
| nbody | 0.028 s | 1.00x | 14x | 1.42x | **1.42x** |
| mandelbrot | 0.348 s | 1.01x | 1.02x | 1.00x | **1.00x** |
| words | 0.066 s | 1.06x | 6.63x | 1.31x | **1.31x** |
| numeric (rayon) | 0.032 s | 1.16x | 3.89x | 1.09x | **1.04x** |
| jsonrt (serde_json, simd-json) | 1.412 s | 1.06x | 3.81x | 1.34x | **1.34x** |
| ripgrep 15.1.0 | 0.053 s | 1.09x | 8.25x | 1.16x | **1.16x** |
| fd 10.3.0 | 0.017 s | 1.00x | 4.50x | 1.25x | **1.25x** |
| tokei | 0.081 s | 1.10x | 8.31x | 1.62x | **1.53x** |
| eza | 0.362 s | 0.95x | 1.76x | 1.09x | **1.05x** |
| bat | 0.098 s |  |  |  | **1.24x** |
| **Geometric mean** (without bat) | | 1.08x | 3.89x | 1.16x | **1.15x** |

A program built whole has what its functions call in any crate built into them or shown to LLVM, the standard library's generic and `#[inline]` functions as rustc does across crates; *each crate alone* is `RA_JIT_INLINE=crate`, which keeps that to the function's own crate. Building across crates takes the programs other than the kernels 2.3 to 3.7 times as long: ripgrep 30.1 s against 8.5 s, tokei 35.9 s against 9.9 s. *Release with overflow checks* is `cargo build --release` with `-C overflow-checks=on -C debug-assertions=on`. The kernels (sieve to words) are `harness/corpus/kernels`; ripgrep counts `\w+ing\b` in its vendored dependencies, fd lists the `.rs` files of its own, tokei and eza walk theirs, bat prints three of its own files highlighted. Not built yet, so not in the table: nushell (some functions are not compiled: an `async` block whose layout rust-analyzer does not know, a constant of rustls that its evaluation fails on, and SIMD intrinsics ra-jit does not have, such as AVX-512's `simd_select_bitmask`). The geometric mean at `-O3` was 1.20x at ra-jit `f3eeb5295e`. On macOS arm64 (MacBook Pro, Apple M5 Max, macOS 27.0.1, median of 3) ripgrep, fd, bat and jsonrt build whole too, at 1.15x, 1.01x, 1.16x and 1.26x. Measured with `harness/runbench.py`; raw numbers in [docs/benchmarks/runtime-whole-program-linux-x86_64.json](docs/benchmarks/runtime-whole-program-linux-x86_64.json).
<!-- runbench:end -->

Not measured yet: workspaces of a million lines or more, and test suites
([#benchmarks](https://github.com/indexable-inc/ra-jit/issues?q=label%3Abenchmarks)).

## Try it on your project

```
nix run github:indexable-inc/ra-jit#ra-jit -- build
```

No nix? Put the binaries from a [release](https://github.com/indexable-inc/ra-jit/releases)
on your PATH, or build them with `cargo +1.99.0 install --locked --profile dist ra-jit`. Then
use Rust 1.99.0 with its source:
`rustup toolchain install 1.99.0 -c rust-src && rustup override set 1.99.0`.

A release's binaries and nix build the same ra-jit: the `dist` profile (thin LTO), optimized
with the profile of its own work that [pgo/profiles.json](pgo/profiles.json) pins
([how](docs/reports/RELEASE-BUILD.md)). `cargo install` builds `dist` without that last step,
which on Linux starts about 10% slower and edits about 3% slower than a release's binaries:
cargo takes no compiler flags from a crate, and the crates it builds from crates.io are not
the ones the profile was made of, so none could be handed to it either. (`--profile dist`
comes with the first release after 0.1.0.)

### Use it as your normal `cargo build`

```
nix profile install github:indexable-inc/ra-jit#ra-jit
ra-jit enable
```

From then on plain `cargo build`, `run`, `test` and `check` in any project go through ra-jit.
`ra-jit disable` puts your cargo config back.

### Live edits

Add the crate and poll once per frame:

```toml
ra-jit-live = "0.1"
```

```rust
let mut live = ra_jit_live::Live::start().ok();
loop {
    if let Some(outcome) = live.as_mut().and_then(|it| it.poll()) {
        eprintln!("ra-jit: {outcome}");
    }
    // ...your frame
}
```

### Cached functions

`#[ra_jit::cache]` on a pure function keeps its results across runs, until an edit changes
the code it reaches ([docs/fn-cache.md](docs/fn-cache.md)).

## What to know

- Live edits work on macOS Apple silicon only. Elsewhere you still get fast builds.
- An edit with a borrow-check error is refused, but the borrow checker still misses about a
  third of rustc's errors: `cargo check` stays the final word.
- Rust 1.99.0 only.
- Changing a struct the program is using asks for a restart.

More: [how it works](docs/architecture.md), [full results](docs/results.md),
[what is supported](docs/support.md).

## Credits

ra-jit stands on the shoulders of giants. Thank you to the people who built:

- [rust-analyzer](https://github.com/rust-lang/rust-analyzer), whose analysis and MIR ra-jit compiles from
- [Cranelift](https://cranelift.dev) (Bytecode Alliance), the fast code generator
- [LLVM](https://llvm.org) and [inkwell](https://github.com/TheDan64/inkwell), for the optimised tiers
- [salsa](https://github.com/salsa-rs/salsa), the incremental engine underneath
- [object](https://github.com/gimli-rs/object), for reading and writing executables
- [Rust](https://github.com/rust-lang/rust) and its compiler crates
- [rustd](https://github.com/DioxusLabs/rustd) (DioxusLabs), whose code ra-jit includes

None of these projects endorses ra-jit.

Made at [ix](https://ix.dev) for den's native app. MIT or Apache-2.0.

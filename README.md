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
Edit, then build, on real projects at pinned commits. Each cell is incremental `cargo build` → `cargo ra-jit build`, both warm, same linker, median of 5 edits. Linux x86-64, AWS c8a.metal-24xl, bare metal, pinned with taskset (AMD EPYC 9R45, 16 of its 96 cores, 188 GiB), ra-jit [`c8f58ac7d9`](https://github.com/indexable-inc/ra-jit/commit/c8f58ac7d9fb8c970b36f57f39ed34332b03bcef) on 2026-10-20.

| Project | Edit a function | Edit a widely used function | Change a signature | Nothing changed |
|---|---:|---:|---:|---:|
| ripgrep 15.1.0 | 453 ms → **50 ms** (9.1x) | 724 ms → **37 ms** (20x) | 562 ms → **68 ms** (8.3x) | 386 ms → **19 ms** (20x) |
| fd 10.3.0 | 434 ms → **43 ms** (10x) | 400 ms → **32 ms** (12x) | 638 ms → **71 ms** (9.0x) | 385 ms → **22 ms** (18x) |
| bat 0.26.0 | 435 ms → **45 ms** (9.7x) | 678 ms → **45 ms** (15x) | 948 ms → **53 ms** (18x) | 58 ms → **16 ms** (3.6x) |
| hyperfine 1.19.0 | 318 ms → **34 ms** (9.4x) | 350 ms → **34 ms** (10x) | 355 ms → **71 ms** (5.0x) | 300 ms → **21 ms** (14x) |
| just 1.43.0 | 662 ms → **38 ms** (17x) | 700 ms → **37 ms** (19x) | 677 ms → **62 ms** (11x) | 40 ms → **14 ms** (2.9x) |
| axum 0.8.6 | 350 ms → **50 ms** (7.0x) | 814 ms → **43 ms** (19x) |  | 71 ms → **16 ms** (4.4x) |
| nushell 0.108.0 | 1.44 s → **261 ms** (5.5x) | 4.80 s → **231 ms** (21x) | 4.85 s → **308 ms** (16x) | 178 ms → **26 ms** (6.8x) |

Measured by hand with `harness/bench.py`; the `bench-readme` workflow re-runs it on the dedicated benchmark machine. The ra-jit measured is `cargo build --release`; the binaries a release ships are the `dist` profile with PGO, which start and edit faster ([how](https://github.com/indexable-inc/ra-jit/blob/main/docs/reports/RELEASE-BUILD.md)). Cold builds, the spread of every number, the LLVM tier and how fast the built programs run: [docs/benchmarks.md](docs/benchmarks.md) ([raw JSON](docs/benchmarks/linux-x86_64.json)).
<!-- bench:end -->

<!-- runbench:start -->
How fast the programs run, built whole by ra-jit with LLVM (every crate of the program compiled by ra-jit, with its own `main`), each time ÷ the time of `cargo build --release`. Lower is faster; 1.00x is as fast as release. Every program's exit status and output were the same as the release build's; a run in which any differed is never published. Linux x86-64: AWS c6i.16xlarge (Intel Xeon Platinum 8375C, 64 vCPUs, 123 GiB), NixOS 26.05, Linux 6.18, rustc 1.99.0, ra-jit `f3eeb5295e` on branch `whole-program-main`, 2026-10-21. Median of 5 runs, the builds taking turns (numeric: 15).

| Benchmark | `cargo build --release` | release with overflow checks | ra-jit `-O3` | ra-jit `-O2`, across crates | ra-jit `-O3`, across crates |
|---|---:|---:|---:|---:|---:|
| sieve | 0.068 s | 0.99x | 5.32x | 0.96x | **0.97x** |
| collatz | 0.135 s | 1.94x | 1.03x | 1.02x | **1.02x** |
| fnv | 0.077 s | 1.00x | 2.71x | 1.00x | **1.00x** |
| qsort | 0.279 s | 1.03x | 2.10x | 1.11x | **1.11x** |
| matmul | 0.034 s | 1.00x | 10x | 1.15x | **1.15x** |
| nbody | 0.028 s | 1.00x | 15x | 1.42x | **1.41x** |
| mandelbrot | 0.348 s | 1.01x | 1.02x | 1.00x | **1.00x** |
| words | 0.065 s | 1.06x | 6.98x | 1.64x | **1.62x** |
| numeric (rayon) | 0.035 s | 0.97x | 3.64x | 1.04x | **1.00x** |
| jsonrt (serde_json, simd-json) | 1.448 s | 1.06x | 3.94x | 1.57x | **1.58x** |
| ripgrep 15.1.0 | 0.055 s | 1.08x | 7.87x | 1.32x | **1.33x** |
| fd 10.3.0 | 0.017 s | 1.01x | 4.72x | 1.40x | **1.37x** |
| tokei | 0.107 s | 0.98x | 6.13x | 1.40x | **1.30x** |
| eza | 0.354 s | 1.06x | 1.85x | 1.19x | **1.17x** |
| **Geometric mean** | | 1.07x | 3.92x | 1.21x | **1.20x** |

*Across crates* is `RA_JIT_INLINE=all`: what a function calls in any crate is built into it or shown to LLVM, the standard library's generic and `#[inline]` functions as rustc does. *Release with overflow checks* is `cargo build --release` with `-C overflow-checks=on -C debug-assertions=on`. The kernels (sieve to words) are `harness/corpus/kernels`; ripgrep counts `\w+ing\b` in its vendored dependencies, fd lists the `.rs` files of its own, tokei and eza walk theirs. Not built yet, so not in the table: bat (rust-analyzer's types for bincode's enum deserializer have an error, so two functions are not compiled and the link fails) and nushell (some functions are not compiled, among them ones behind `#[target_feature]`, so the link fails; at `-O3` across crates LLVM's instruction combining also gives up on one function). Measured with `harness/runbench.py`; raw numbers in [docs/benchmarks/runtime-whole-program-linux-x86_64.json](docs/benchmarks/runtime-whole-program-linux-x86_64.json).
<!-- runbench:end -->

Not measured yet: workspaces of a million lines or more, and test suites
([#benchmarks](https://github.com/indexable-inc/ra-jit/issues?q=label%3Abenchmarks)).

## Try it on your project

```
nix run github:indexable-inc/ra-jit#ra-jit -- build
```

No nix? `cargo +1.99.0 install --locked ra-jit`, or put the binaries from a
[release](https://github.com/indexable-inc/ra-jit/releases) on your PATH. Then use Rust
1.99.0 with its source:
`rustup toolchain install 1.99.0 -c rust-src && rustup override set 1.99.0`.

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
- The borrow checker is partial and does not stop a build yet: keep running `cargo check`.
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

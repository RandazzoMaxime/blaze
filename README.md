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
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/benchmarks/charts/edits-dark.svg"><img alt="How many times faster an edit builds under ra-jit than under cargo, by platform: ripgrep 15.1.0: macOS arm64 8.1×, Linux x86-64 11×; fd 10.3.0: macOS arm64 6.0×, Linux x86-64 5.6×; bat 0.26.0: Linux x86-64 14×; hyperfine 1.19.0: Linux x86-64 4.4×; just 1.43.0: Linux x86-64 16×; axum 0.8.6: Linux x86-64 8.1×; nushell 0.108.0: macOS arm64 10.0×, Linux x86-64 19×; helix 25.07.1: Linux x86-64 12×" src="docs/benchmarks/charts/edits-light.svg" width="760"></picture>

Edit, then build, on real projects at pinned commits: incremental `cargo build` → `cargo ra-jit build`, both warm, same linker, Cranelift, the median of each run's edits. Each platform is a machine of its own: compare numbers within a platform, not across.

An edit to one function, cargo → ra-jit:

| Project | macOS arm64 | Linux aarch64 | Linux x86-64 |
|---|---:|---:|---:|
| ripgrep 15.1.0 | 993 ms → **126 ms** (7.9x) | - | 655 ms → **104 ms** (6.3x) |
| fd 10.3.0 | 630 ms → **109 ms** (5.8x) | - | 649 ms → **115 ms** (5.6x) |
| bat 0.26.0 | - | - | 650 ms → **69 ms** (9.4x) |
| hyperfine 1.19.0 | - | - | 528 ms → **117 ms** (4.5x) |
| just 1.43.0 | - | - | 899 ms → **58 ms** (16x) |
| axum 0.8.6 | - | - | 559 ms → **69 ms** (8.1x) |
| nushell 0.108.0 | 1.81 s → **454 ms** (4.0x) | - | 1.62 s → **279 ms** (5.8x) |
| helix 25.07.1 | - | - | 2.80 s → **231 ms** (12x) |
| bevy 0.20.0 | - | - | - |

- **macOS arm64**: MacBook Pro (Mac17,6), macOS 27.0.1, with other work running (Apple M5 Max, 18 cores, 128 GiB), ra-jit [`84760c3f51`](https://github.com/indexable-inc/ra-jit/commit/84760c3f517a76f6e8a3aa41ccd81fc3f26b87c7) on 2026-10-21, median of 3 edits, load average 48 at the start, by hand.
- **Linux aarch64**: not measured yet ([the machines](docs/benchmarks.md#the-machines)).
- **Linux x86-64**: AWS c7i.16xlarge (dev-aws-7), pinned to 16 cores with taskset (Intel(R) Xeon(R) Platinum 8488C, 16 of its 64 cores, 124 GiB), ra-jit [`2b2f2dcd5b`](https://github.com/indexable-inc/ra-jit/commit/2b2f2dcd5bc4d7132fe62bf36669a34c657dafe7) on 2026-10-20, median of 5 edits, by hand.

<details><summary>Every kind of edit, by platform</summary>

**macOS arm64**

| Project | Edit a function | Edit a widely used function | Change a signature | Nothing changed |
|---|---:|---:|---:|---:|
| ripgrep 15.1.0 | 993 ms → **126 ms** (7.9x) | 1.31 s → **159 ms** (8.2x) | 828 ms → **175 ms** (4.7x) | 67 ms → **54 ms** (1.2x) |
| fd 10.3.0 | 630 ms → **109 ms** (5.8x) | 734 ms → **102 ms** (7.2x) | 681 ms → **113 ms** (6.0x) | 144 ms → **49 ms** (2.9x) |
| nushell 0.108.0 | 1.81 s → **454 ms** (4.0x) | 14.39 s → **850 ms** (17x) | 10.28 s → **716 ms** (14x) | 407 ms → **45 ms** (9.0x) |

**Linux x86-64**

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

</details>

Measured by hand with `harness/bench.py`, or by the `bench-readme` workflow on each platform's benchmark machine once it is registered. The ra-jit measured is `cargo build --release`; the binaries a release ships are the `dist` profile with PGO, which start and edit faster ([how](https://github.com/indexable-inc/ra-jit/blob/main/docs/reports/RELEASE-BUILD.md)). Cold builds, the spread of every number, the LLVM tier and how fast the built programs run: [docs/benchmarks.md](docs/benchmarks.md). Raw JSON: [macOS arm64](docs/benchmarks/macos-arm64.json), [Linux x86-64](docs/benchmarks/linux-x86-64.json).
<!-- bench:end -->

<!-- benchmem:start -->
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/benchmarks/charts/memory-dark.svg"><img alt="Peak memory while an edit to one function builds, by platform: ripgrep 15.1.0: macOS arm64 2.9 GB for ra-jit, 235 MB for cargo; fd 10.3.0: macOS arm64 2.6 GB for ra-jit, 212 MB for cargo; nushell 0.108.0: macOS arm64 12.5 GB for ra-jit, 1.1 GB for cargo" src="docs/benchmarks/charts/memory-light.svg" width="760"></picture>

Memory and disk, Cranelift, from the same runs as the build table. Peak: the most every process doing the work held at once, cargo → ra-jit. For ra-jit that is the daemon, its workers and what they run, and `cargo ra-jit build` with any cargo it ran; for plain cargo, cargo itself, rustc, build scripts and the linker. How it is measured: [docs/benchmarks.md](docs/benchmarks.md#memory-and-disk).

Peak while an edit to one function builds, cargo → ra-jit:

| Project | macOS arm64 | Linux aarch64 | Linux x86-64 |
|---|---:|---:|---:|
| ripgrep 15.1.0 | 235 MB → **2.9 GB** | - | - |
| fd 10.3.0 | 212 MB → **2.6 GB** | - | - |
| nushell 0.108.0 | 1.1 GB → **12.5 GB** | - | - |

Linux x86-64: its stored run is from before memory was measured.

<details><summary>The daemon between edits, cold builds and disk, by platform</summary>

**macOS arm64**

| Project | Edit a function, peak | Daemon between edits | Cold until ready, peak | On disk |
|---|---:|---:|---:|---:|
| ripgrep 15.1.0 | 235 MB → **2.9 GB** | 2.2 GB ready, 2.8 GB after 39 builds | 541 MB → **2.4 GB** | 416 MB → 598 MB |
| fd 10.3.0 | 212 MB → **2.6 GB** | 1.8 GB ready, 2.6 GB after 33 builds | 897 MB → **1.9 GB** | 355 MB → 460 MB |
| nushell 0.108.0 | 1.1 GB → **12.5 GB** | 11.5 GB ready, 12.5 GB after 39 builds | 2.4 GB → **16.1 GB** | 5.3 GB → 6.9 GB |

On disk: cargo's target directory → ra-jit's target directory, store and home.

</details>
<!-- benchmem:end -->

<!-- runbench:start -->
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/benchmarks/charts/runtime-dark.svg"><img alt="Time of each benchmark, ra-jit's build ÷ cargo build --release, by platform: Linux x86-64 geometric mean 1.21×" src="docs/benchmarks/charts/runtime-light.svg" width="760"></picture>

How fast the programs run. Each program is built by `cargo build --release` and by ra-jit with LLVM at `-O3`, every crate of it compiled by ra-jit, with the release profile; each benchmark is then run several times, the builds taking turns, median. A run is published only when every program did what the release build's did. Each platform is a machine of its own, and each number is ra-jit's time ÷ release's on it: 1x is as fast.

| Benchmark | macOS arm64 | Linux aarch64 | Linux x86-64 |
|---|---:|---:|---:|
| sieve | - | - | 0.95x |
| collatz | - | - | 1.02x |
| fnv | - | - | 1.00x |
| qsort | - | - | 1.04x |
| matmul | - | - | 1.16x |
| nbody | - | - | 1.41x |
| mandelbrot | - | - | 1.00x |
| words | - | - | 1.33x |
| numeric | - | - | 1.83x |
| jsonrt | - | - | 1.34x |
| ripgrep | - | - | 1.15x |
| fd | - | - | 1.26x |
| tokei | - | - | 1.67x |
| eza | - | - | 1.10x |
| bat | - | - | 1.25x |
| **Geometric mean** | - | - | **1.21x** |

- **macOS arm64**: not measured yet ([the machines](docs/benchmarks.md#the-machines)).
- **Linux aarch64**: not measured yet ([the machines](docs/benchmarks.md#the-machines)).
- **Linux x86-64**: AWS c6i.16xlarge (dev-aws-6) (Intel(R) Xeon(R) Platinum 8375C CPU @ 2.90GHz, 64 cores, 124 GiB), ra-jit [`2302d0f50b`](https://github.com/indexable-inc/ra-jit/commit/2302d0f50b9a8f7d9002e706434011141a887b43) on 2026-10-22, each benchmark run 5 times, by hand.

<details><summary>Times, memory and builds, by platform</summary>

**Linux x86-64**

| Benchmark | `cargo build --release` | ra-jit, LLVM | ra-jit ÷ release | release with dev checks ÷ release | Peak memory, release → ra-jit |
|---|---:|---:|---:|---:|---:|
| sieve | 69 ms | **66 ms** | 0.95x | 1.00x | 19 MB → 19 MB |
| collatz | 135 ms | **137 ms** | 1.02x | 1.94x | 19 MB → 19 MB |
| fnv | 77 ms | **77 ms** | 1.00x | 1.00x | 19 MB → 19 MB |
| qsort | 279 ms | **291 ms** | 1.04x | 1.03x | 19 MB → 19 MB |
| matmul | 34 ms | **40 ms** | 1.16x | 1.00x | 19 MB → 19 MB |
| nbody | 28 ms | **40 ms** | 1.41x | 1.00x | 19 MB → 19 MB |
| mandelbrot | 348 ms | **349 ms** | 1.00x | 1.01x | 19 MB → 19 MB |
| words | 66 ms | **87 ms** | 1.33x | 1.07x | 19 MB → 19 MB |
| numeric | 31 ms | **57 ms** | 1.83x | 1.08x | 19 MB → 19 MB |
| jsonrt | 1.42 s | **1.90 s** | 1.34x | 1.06x | 543 MB → 544 MB |
| ripgrep | 54 ms | **62 ms** | 1.15x | 1.07x | 19 MB → 19 MB |
| fd | 17 ms | **21 ms** | 1.26x | 1.01x | 19 MB → 19 MB |
| tokei | 81 ms | **136 ms** | 1.67x | 1.17x | 54 MB → 54 MB |
| eza | 352 ms | **387 ms** | 1.10x | 1.01x | 25 MB → 25 MB |
| bat | 97 ms | **122 ms** | 1.25x | 1.05x | 25 MB → 25 MB |
| **Geometric mean** | | | **1.21x** | 1.08x | |

What each program takes, release → ra-jit: its build from cold (for ra-jit, cargo running the build scripts and proc macros, ra-jit's compile of every crate and the link), its size, and its build's memory (the most the build's processes held at once, and the largest one process: rustc or ra-jit, or the linker).

| Program | Build | Size | Build, peak | Build, largest process |
|---|---:|---:|---:|---:|
| kernels | 400 ms → 4.40 s | 0.5 MB → 1.53 MB | 172 MB → 1.9 GB | 144 MB → 1.9 GB |
| numeric | 3.20 s → 9.20 s | 0.81 MB → 2.18 MB | 515 MB → 3.0 GB | 284 MB → 3.0 GB |
| jsonrt | 6.10 s → 18.10 s | 1.07 MB → 2.81 MB | 1.8 GB → 4.4 GB | 411 MB → 4.4 GB |
| ripgrep | 8.40 s → 31.70 s | 31.4 MB → 21.62 MB | 1.9 GB → 6.6 GB | 575 MB → 6.6 GB |
| fd | 41.70 s → 64.90 s | 4.13 MB → 7.85 MB | 5.0 GB → 6.4 GB | 454 MB → 6.3 GB |
| tokei | 13.20 s → 50.70 s | 5.51 MB → 12.54 MB | 4.5 GB → 7.6 GB | 841 MB → 7.5 GB |
| eza | 34.60 s → 38.90 s | 2.64 MB → 6.92 MB | 4.4 GB → 6.9 GB | 463 MB → 6.9 GB |
| bat | 53.30 s → 125.00 s | 7.55 MB → 14.64 MB | 5.9 GB → 9.3 GB | 688 MB → 9.3 GB |

Release with dev checks: `cargo build --release` with `-C overflow-checks=on -C debug-assertions=on`.

</details>

Measured by hand with `harness/bench.py --runtime` (harness/runbench.py), or by the `bench-readme` workflow on each platform's benchmark machine once it is registered. Raw JSON: [Linux x86-64](docs/benchmarks/runtime-linux-x86-64.json).
<!-- runbench:end -->

### bun

[bun](https://github.com/oven-sh/bun)'s Rust port, 1.4.3: 1.25M lines of Rust in 103 crates
on a nightly, beside a JavaScript engine's C and C++ ([what it took](docs/research/bun.md)).

<!-- bench-bun:start -->
Edit, then build, bun at its pinned commit: incremental `cargo build` → `cargo ra-jit build`, both warm, same linker, Cranelift, the median of each run's edits. Each platform is a machine of its own: compare numbers within a platform, not across.

| Platform | Edit a function | Edit a widely used function | Change a signature | Nothing changed |
|---|---:|---:|---:|---:|
| macOS arm64 | 12.16 s → **1.09 s** (11x) | 26.75 s → **1.81 s** (15x) | 9.83 s → **126.44 s** (c) (0.1x) | 168 ms → **73 ms** (2.3x) |
| Linux aarch64 | not measured yet | | | |
| Linux x86-64 | not measured yet | | | |

- **macOS arm64**: one machine (Apple M5 Max, 18 cores, 128 GiB), ra-jit [`1645168dc4`](https://github.com/indexable-inc/ra-jit/commit/1645168dc4abbdfab2e234232efb73a5c1ec2935) on 2026-10-21, load average 28 at the start, by hand.
- **Linux aarch64**: not measured yet ([the machines](docs/benchmarks.md#the-machines)).
- **Linux x86-64**: not measured yet ([the machines](docs/benchmarks.md#the-machines)).

(c): the daemon handed this edit to cargo (a fallback, so about cargo's time).

Measured by hand with `harness/bench.py`, or by the `bench-bun` workflow on each platform's benchmark machine once it is registered. The ra-jit measured is `cargo build --release`; the binaries a release ships are the `dist` profile with PGO, which start and edit faster ([how](https://github.com/indexable-inc/ra-jit/blob/main/docs/reports/RELEASE-BUILD.md)). Raw JSON: [macOS arm64](docs/benchmarks/bun-macos-arm64.json).
<!-- bench-bun:end -->

<!-- runbench-bun:start -->
ra-jit's whole-program build of bun does not link yet: about 1,300 of its functions are not
compiled ([#35](https://github.com/indexable-inc/ra-jit/issues/35)), so how fast it runs is not
measured.
<!-- runbench-bun:end -->

Not measured yet: test suites
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

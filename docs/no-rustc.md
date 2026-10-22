# ra-jit never runs rustc

## The rule

ra-jit compiles everything itself. When someone builds, runs, tests or checks a program with
ra-jit, no rustc runs: not for a function, a crate, the standard library, a build script, a
proc macro, a check, a test, a doctest or a link, and not through cargo either. There is no
path that falls back to cargo or to rustc when ra-jit cannot do something. ra-jit says what
it could not do, the build fails, and that failure is a bug to fix.

Andrew, 2026-10-22: "Everything should all be us 100%."

This describes where ra-jit is going, not where it is. Today rustc still does much of the work:
it builds every program once before the daemon takes over, it compiles the standard library,
the build scripts and the proc macros, it has the last word on errors, and it keeps a copy of
every function ra-jit refuses. [Today](#today) lists those places. Each one is a bug with an
issue labelled `no-rustc`. They are removed one at a time; none is extended, and no new one is
added.

## What counts as running rustc

Any of these, whoever starts the process:

- ra-jit starting `rustc`, `rustdoc` or another rustc driver, including to ask it something
  (`-vV`, `--print cfg`, `--print sysroot`) or to assemble an `asm!` block.
- ra-jit starting a cargo command that runs rustc. Cargo runs `$RUSTC` for everything it
  compiles and also to learn about the target (`rustc -vV`, `rustc - --print=file-names ...`),
  so `cargo build`, `check`, `test`, `rustc` and `metadata` (except with `--no-deps`) all run it.
- A process ra-jit runs that runs rustc in turn: rust-analyzer's `cargo check
  --compile-time-deps` for build scripts and proc macros, or a build script that probes
  `$RUSTC` (`autocfg`, `version_check`, `rustversion`, the feature probes of `anyhow` and
  `proc-macro2`).
- ra-jit using what rustc wrote, even when rustc ran earlier or on another machine: the
  standard library's rlibs in the sysroot, the rlibs and objects cargo had rustc build, rustc's
  link line (`--print link-args`), an executable rustc linked as the base of patches, a proc
  macro's dynamic library or a build script's executable that rustc compiled, and rustc's copy
  of a function ra-jit refused ("left to rustc").

What it does not cover:

- The harnesses' reference builds. rustc is the reference
  ([contributing.md](contributing.md#rules-for-code), rule 1): the harnesses build with cargo
  and rustc and compare. The oracle stays rustc's. What changes is every expectation that ra-jit
  hands something to rustc or cargo.
- The platform's linker and C toolchain: `cc`, `ld`, `lld`, `link.exe`, and the C or C++ a
  build script compiles with the `cc` crate. They are not rustc. ra-jit runs the linker with a
  line it works out itself, not with rustc's.
- ra-jit's own executable, which is built with cargo and rustc 1.99.0. Building ra-jit with
  ra-jit is a later goal, not part of this rule.
- The standard library's source (`rust-src`). It is source, and stays an input.
- Tools cargo runs that compile no Rust: rustfmt, the resolver, fetching.

## The end state

What "100%" means, piece by piece. Each piece is where one of today's uses goes.

1. **The standard library is compiled by ra-jit.** `core`, `alloc`, `std`,
   `compiler_builtins`, `panic_unwind`, `panic_abort`, `unwind`, `proc_macro`, `test` and what
   they depend on, from `rust-src`, kept in the store per target and profile like any other
   crate. No rlib of the sysroot is read or linked.
2. **Proc macros are compiled by ra-jit and loaded by ra-jit.** The proc-macro crate and its
   dependencies (`syn`, `quote`, `proc-macro2`) are compiled by ra-jit into a library that
   rust-analyzer's proc-macro server, running inside ra-jit, loads. Not the toolchain's
   `rust-analyzer-proc-macro-srv`, and not a library rustc built.
3. **Build scripts are compiled, linked and run by ra-jit.** `RUSTC` names ra-jit while they
   run, so that their own probes are answered by ra-jit, the way rustc 1.99.0 answers them.
4. **The cold build is entirely ra-jit's.** No first build by cargo and rustc, no executable
   of rustc's as the base of the first edits ([#11](https://github.com/indexable-inc/ra-jit/issues/11)):
   the first edits patch ra-jit's own executable, and the store and the remote store are what
   make that first build fast.
5. **No function is left to rustc.** A function ra-jit refuses fails the build, with the
   reason. The classes refused today ([#34](https://github.com/indexable-inc/ra-jit/issues/34),
   [#35](https://github.com/indexable-inc/ra-jit/issues/35), [support.md](support.md#language))
   are compiled.
6. **No fallback to cargo.** Not for source with an error, not for an item ra-jit cannot
   resolve ([#27](https://github.com/indexable-inc/ra-jit/issues/27) made such builds go to
   cargo; that goes), not for a flag ra-jit does not handle, not for a changed manifest. ra-jit
   reports an error itself, written as rustc writes it, and ends with the status cargo would.
7. **ra-jit's check is the final word.** `cargo ra-jit check`, and the daemon's verdict on the
   source it builds, are rust-analyzer's diagnostics and ra-jit's borrow checker
   ([borrowck.md](borrowck.md)), which therefore have to find what rustc finds, lints that deny
   included. `cargo check` is not asked.
8. **Doctests are compiled and run by ra-jit.** The examples are built by the daemon, which has
   the workspace loaded, and never by rustc or rustdoc
   ([#28](https://github.com/indexable-inc/ra-jit/issues/28)).
9. **Tests are ra-jit's.** The test harness's `main`, the list of `#[test]` functions, is
   written by ra-jit, so adding a test is a build like any other.
10. **The link is ra-jit's.** The link line (the native libraries of `#[link]` and of build
    scripts, `-C link-arg`, the linker cargo's configuration names) and the shims rustc adds at
    link time (the allocator's, `main`) are ra-jit's.
11. **`asm!` is assembled in process.**
12. **rustc's questions are answered by ra-jit.** `-vV`, `--print cfg`, `sysroot`,
    `target-libdir`, `target-spec-json`, `file-names`, from ra-jit's own targets.
13. **Nothing of the wrapper is left.** No `ra-jit-rustc`, no `ra-jit-rustc-first`, no
    workspace wrapper and no stand-ins built by rustc.

Once nothing rustc compiled is linked with ra-jit's code, ra-jit no longer has to agree with
rustc on symbol names, the unstable Rust ABI, vtable layout or `TypeId` hashes for the two to
link. Its programs still have to behave as rustc's do.

## How ra-jit relates to cargo

Cargo stays the command people type and the source of a build's shape: manifests, the lock
file, features, profiles and configuration. Using cargo as a front end is fine as long as cargo
never starts rustc. Since cargo runs `$RUSTC` for everything it compiles and for what it asks
about the target, that holds only when `RUSTC` and `RUSTDOC` name ra-jit and ra-jit answers
all of it:

- **`cargo ra-jit build`, `run`, `test` and `check`.** ra-jit plans the build itself:
  `crates/ra-jit/src/plan.rs` already works out the units, with the features and profile
  settings of each. It compiles and links them without starting `cargo build`. Where ra-jit
  needs cargo's resolution (`cargo metadata`), it runs cargo with `RUSTC` naming ra-jit.
- **Plain cargo after `ra-jit enable`.** Cargo's configuration names ra-jit as `build.rustc`,
  not as `build.rustc-workspace-wrapper`, which reaches only the workspace's crates and leaves
  rustc to build the rest. To cargo, ra-jit is then rustc: it answers the questions, and for
  each unit cargo asks for it writes the files cargo expects to find (the names
  `--print file-names` gave, and the dep-info), compiled through the daemon.
- **What goes to cargo today** (`Invocation::only_cargo` in `cargo/cli.rs`: `--message-format`,
  `-Z`, `--exclude`, several targets, an unknown triple) becomes supported, or refused with the
  reason. None of it goes to cargo.

Subcommands ra-jit does not claim (`cargo ra-jit doc`, `clippy`, `fmt`, `publish`) are passed
to cargo as typed today. Producing HTML documentation and running clippy's lints are not
builds of the program; whether they stay cargo's is an open question for Andrew. Under
`ra-jit enable` the global wrapper also sees those commands' rustc runs; it must then only
pass them by, never add a rustc run of its own.

## While the paths remain

- A remaining path may be fixed where it is wrong: it must not miscompile while it exists. It
  gets no new capability and no new caller.
- New code adds none. A feature that would need a rustc run or rustc's output waits for, or
  builds, the piece of ra-jit that replaces it. Review turns down a new `Command` running rustc,
  rustdoc or a compiling cargo; a new read of `RUSTC` or `RUSTDOC`; a new reason to build "with
  cargo"; a new class of function "left to rustc"; and a new use of rustc's rlibs, objects or
  symbols.
- A lane whose work makes a rustc path faster or able to do more, rather than removing it, is
  redirected.
- Each remaining path is counted, and the counts only go down.

## Today

These are the places ra-jit still runs rustc or relies on what it wrote, as of main at
41e7906192 (2026-10-22). [How often](#how-often) has the counts on real projects;
[the plan](#the-plan) has the order they go in.

- The first build of every workspace is cargo's and rustc's, with ra-jit as the workspace
  wrapper recording what rustc built (`cargo/wrapper.rs`, `ra-jit-rustc`,
  `ra-jit-rustc-first`).
- The first edits patch the executable rustc linked ([#11](https://github.com/indexable-inc/ra-jit/issues/11),
  `patch::Base::of_rustc`), which is what [#52](https://github.com/indexable-inc/ra-jit/issues/52)
  is about.
- Every program is linked with rustc's link line, ahead of rustc's rlibs of the same crates,
  and against the standard library's rlibs from the sysroot.
- A function ra-jit refuses is left to rustc's copy; when that copy is stale, or rust-analyzer
  finds an error, or an input ra-jit does not model changed, the build goes to cargo.
- rust-analyzer loads a workspace with `cargo metadata`, `cargo check --compile-time-deps` (which
  compiles the build scripts and proc macros with rustc and runs the scripts) and rustc's
  answers to `--print`; proc macros are rustc's libraries run by the toolchain's proc-macro
  server.
- Doctest examples are compiled by rustc, and a library whose doctests ra-jit cannot take is
  left to rustdoc ([#28](https://github.com/indexable-inc/ra-jit/issues/28)).
- Under plain cargo after `ra-jit enable`, rustc builds every crate of the workspace except the
  programs the daemon has loaded, and writes the dep-info of those.
- `cargo ra-jit check` is `cargo check`.
- Under Cranelift, every `asm!` block is assembled by running rustc (`hir-ty-codegen/asm.rs`).

### Where, and when

Each place in the code, what starts it, and the step of [the plan](#the-plan) that removes it.
Paths are under `crates/`; line numbers are of 41e7906192.

**The first build and the wrapper** (step K, then M)

| Where | What it does | When |
|---|---|---|
| `ra-jit/src/cargo/state.rs:366` `Built::by` | runs `cargo build` or `cargo test` with ra-jit as `RUSTC_WORKSPACE_WRAPPER` | the first build of every workspace and profile, and every fallback below |
| `ra-jit/src/cargo/wrapper.rs:63` `command` | runs rustc for each crate of the workspace, adding `--print=link-args` and `-Csave-temps` to record the link | every rustc run cargo makes for a workspace crate |
| `ra-jit/src/cargo/wrapper.rs:315` `dep_info` | runs rustc for dep-info only when the daemon built the crate | plain cargo after `ra-jit enable`, every build the daemon serves |
| `ra-jit/src/cargo/wrapper.rs:382` `stood_in`, `ra-jit/src/stub.rs:49` | has rustc build a few-line stand-in of each workspace crate | `RA_JIT_FIRST_BUILD=1` only; off by default |
| `ra-jit/src/cargo/wrapper.rs:157`, `ra-jit/src/cargo/mod.rs:85` `settle` | after cargo has linked a program, runs cargo again in the background to load the build | plain cargo, after every command that linked |
| `ra-jit/src/bin/ra-jit-rustc.rs:24` | runs rustc itself when `ra-jit` beside it does not start | a broken install |
| `ra-jit/src/bin/ra-jit-rustc-first.rs`, `ra-jit/src/cargo/switch.rs:61` | the wrapper under a second path, for the first build | the first build |
| `ra-jit/src/cargo/script.rs:95` | runs again the build-script executables rustc linked | before each build, when a script's inputs changed |
| `ra-jit/src/cargo/units.rs`, `ra-jit/src/deps.rs` | read, push and pull what rustc built | every build; `ra-jit deps` |

**The fallback to cargo** (steps D and E)

| Where | What it does | When |
|---|---|---|
| `ra-jit/src/cargo/mod.rs:305` `by_daemon`, `:300` `ToCargo`, `:395` `by_cargo` | sends the whole build to cargo | the source has an error; rustc's copy of a refused function is stale; a build script's or proc macro's source changed; a manifest changed; the daemon did not build a program |
| `ra-jit/src/cargo/cli.rs:297` `only_cargo` | sends the command to cargo untouched | `--message-format`, `-Z`, `--exclude`, several targets, an unknown triple |
| `ra-jit/src/cargo/mod.rs:139` `plain_cargo` | runs cargo with nothing of ra-jit's | `cargo ra-jit check` and subcommands ra-jit does not claim |

**What rustc compiled, linked into ra-jit's programs** (steps E, F, G)

| Where | What it does | When |
|---|---|---|
| `hir-ty-codegen/mod.rs:149` `NoSymbol`, `:152` `Private` | leaves a function to rustc's copy because rustc exported no symbol ra-jit's could replace | every build; most of the functions left to rustc |
| `hir-ty-codegen/mod.rs:772` and the reasons of [support.md](support.md#language) | leaves a function ra-jit cannot compile to rustc's copy | every build |
| `ra-jit/src/main.rs:3300` `rustc_symbols`, `ra-jit/src/symbols.rs` | reads rustc's symbol names from its rlibs | every build |
| `ra-jit/src/main.rs:1592` `links_core`, `ra-jit/src/cargo/plan.rs` | links with rustc's recorded link line, ahead of its rlibs and the sysroot's | every full link |
| `ra-jit/src/weaken.rs` | makes rustc's Mach-O objects weak so ra-jit's win | every full link on macOS |
| `ra-jit/src/main.rs:2720` `rustcs_to_patch`, `hir-ty-codegen/patch.rs:948` `Base::of_rustc` | splices the first edits into rustc's executable (#11) | the first edits after a cold build, unless `RA_JIT_RUSTC_BASE=0` |
| `ra-jit/src/cargo/state.rs:1071` `sysroot_libraries` | asks rustc where the sysroot's rlibs are | the first build |

**Loading the workspace** (steps B, I, J)

| Where | What it does | When |
|---|---|---|
| `ra-jit/src/main.rs:2261` `ProjectWorkspace::load` | `cargo metadata`, plus rustc's `-vV`, `--print sysroot`, `--print cfg`, `--print target-spec-json` through `cargo rustc --print` (`vendor/rust-analyzer/crates/project-model/src/toolchain_info/`) | every load: the daemon's workers and spares, and each doctest process |
| `ra-jit/src/main.rs:2272` | asks rustc for the target triple | every load |
| `ra-jit/src/plan.rs:475` | `cargo build --unit-graph`, which asks rustc about the target | every load, and every doctest run per library |
| `ra-jit/src/main.rs:2314`, `ra-jit/src/inputs.rs:282` `run_build_scripts` | rust-analyzer's `cargo check --compile-time-deps`: rustc compiles every build script and proc macro and their dependencies, and the scripts run | every load whose target directory does not have them yet; after an edit to a proc macro or build script |
| `ra-jit/src/main.rs:5287`, `ra-jit/src/borrowck.rs:113` `ProcMacroServerChoice::Sysroot` | expands with the toolchain's `rust-analyzer-proc-macro-srv` loading rustc's proc-macro libraries | every load that has a proc macro |
| `ra-jit/src/cargo/state.rs:948` `metadata`, `:1013` `other_toolchain`, `ra-jit/src/cargo/mod.rs:122` | `cargo metadata --no-deps` (no rustc), `rustc --version`, `--print=sysroot` | every client command |

**Tests, doctests, check, assembly** (steps C, H, L, D)

| Where | What it does | When |
|---|---|---|
| `ra-jit/src/cargo/run.rs:136` | `cargo test --doc` with ra-jit as `RUSTDOC` | `cargo ra-jit test` of a library with doctests |
| `ra-jit/src/doctest.rs:203`, `:246` `compile` | rustc links a probe program to learn the link line, and compiles or checks each example | every library's doctests |
| `ra-jit/src/doctest.rs:301` `leave_to_rustdoc` | hands the library to rustdoc | `-q`, a library ra-jit cannot load, and today every package whose features `cargo build --unit-graph` rejects (below) |
| `ra-jit/src/cargo/mod.rs:139` | `cargo ra-jit check` is `cargo check` | every check |
| `hir-ty-codegen/asm.rs:717` `run_assembler` | rustc assembles each `asm!` block (cached by its text) | Cranelift, the first build of a program with `asm!` |
| `ra-jit/src/main.rs:5084`, `:5127` `RA_JIT_WRAPS_RUSTC` | ra-jit standing in for rustc for a whole build | the harness's "alone" builds |
| `harness/apps.py:113` | the harness's "alone" mode: cargo with ra-jit as `RUSTC_WRAPPER`, linking the sysroot's rlibs | the harness only |

### How often

Counted on dev-aws-8 (Linux x86-64, 96 cores given to the run, Cranelift), ra-jit at
494f0ee948, rustc 1.99.0, with the bench apps of `harness/bench.py` at their pinned revisions
and edits. Every `rustc`, `rustdoc` and `cargo` of a copy of the toolchain was replaced by a
script that logs its arguments and its parent and grandparent, and each run was put in the
step it fell in; a step ends when no process of the toolchain runs and the log has been
still for 10 s, so what the daemon does in the background after a command is counted with it.
Each step is one run. "Queries" are runs that compile nothing: `-vV`, `--print`, and cargo's
`- --crate-name ___ --print=file-names` probe of the target. "Compiles" are the rest:
compiling, checking or linking a crate, a build script, a proc macro, a doctest example, a
build script's probe, or an `asm!` block. bevy has no test step in bench.py, and its native
libraries (ALSA, udev, Wayland, X11, Vulkan) and C compiler came from a nixpkgs shell. The
scripts are in `harness/norustc/`: setup.sh, wrap.sh, count.py, classify.py and left.py.

`cargo ra-jit build` and `test`, rustc runs per step (compiles + queries):

| Step | ripgrep | nushell | bevy (`--example breakout`) |
|---|---|---|---|
| cold build | 116 (43 + 73), 6.2 s | 618 (523 + 95), 40.0 s | 641 (553 + 88), 83.5 s |
| no-op build | 0 | 0 | 0 |
| body edit | 0 | 0 | 23 (2 + 21), 484 s: to cargo |
| body edit again | 0 | 0 | 44 (1 + 43), 467 s: to cargo |
| leaf edit | 0 | 0 | 80 (37 + 43), 489 s: to cargo, which compiles 36 workspace libraries |
| new function | 0 | 0 | 44 (1 + 43), 465 s: to cargo |
| signature edit | 0 | 128 (83 + 45), 8.6 s: to cargo | (no such edit) |
| an error | 6 (6 + 0): to cargo, rustc prints it | 4 (1 + 3): rustc prints it | 26 (1 + 25), 468 s: to cargo, rustc prints it |
| the error mended | 0 | 0 | 59 (1 + 58), 481 s: to cargo |
| first `test --no-run` | 106 (45 + 61), 5.5 s | 156 (115 + 41), 15.9 s | (not a step) |
| `test --no-run` after a body edit | 0 | 22 (0 + 22), 6.0 s | |
| `test --no-run` after a test was added | 75 (18 + 57): to cargo | 47 (11 + 36): to cargo | |
| doctests, first | 97 (27 + 70) and 2 rustdoc, 28.2 s | 212 (165 + 47) and 1 rustdoc, 13.3 s | |
| doctests, again | 90 (27 + 63) and 2 rustdoc, 28.0 s | 92 (78 + 14) and 1 rustdoc, 8.2 s | |
| `check` | 38 (35 + 3) | 413 (408 + 5) | 441 (419 + 22) |

Plain `cargo` on ripgrep after `ra-jit enable`, same counting ("cargo" there is the user's
command):

| Step | rustc runs | What |
|---|---|---|
| cold build | 111 (43 + 68), 6.7 s | cargo's build, as above |
| no-op build | 0 | |
| body edit, and again | 1 each | rustc writes the dep-info of the program the daemon built (wrapper.rs:315) |
| leaf edit | 6 | rustc compiles the 5 workspace libraries above the edited one, then the dep-info |
| first `test --no-run` | 77 (56 + 21) | cargo's build of the test profile |
| `test --no-run` after a body edit | 25 (3 + 22) | |
| doctests | 12 compiles and 9 rustdoc | all rustdoc's: plain `cargo test --doc` does not reach ra-jit |
| `check` | 34 | cargo's |

bun was not counted. Its workspace builds only after bun's own configure step has generated
code and built its C and C++ (`harness/bunprep.py` under `nix develop .#bench-bun`), which the
host did not have. What is known from [research/bun.md](research/bun.md) and the issues: bun
needs `RA_JIT_OTHER_TOOLCHAIN=1` for its pinned nightly; the whole-program report left 1,645
functions to rustc, 907 of them reaching `type_info` and 304 `adt_const_params` (#34, #35); a
signature edit goes to cargo because `posix_signal_handler` is left to rustc and writes a
static ([#32](https://github.com/indexable-inc/ra-jit/issues/32)); the cold build is 164 s of
cargo plus 128 s until the daemon is ready, against 106 s for cargo.

What the numbers say:

- **Under `cargo ra-jit`, edits of function bodies run no rustc** on ripgrep and nushell:
  no-op, body, leaf, new function, and a signature edit on ripgrep. That is what ra-jit was
  built for and it holds there. bevy and plain cargo are below.
- **Every cold build is rustc's.** ripgrep's 43 compiles are cargo's first build (25
  dependencies, 8 workspace libraries, 7 build scripts, the program's link) plus one `asm!`
  block and one build-script probe; nushell's 523 are 404 dependencies, 50 build scripts, 30
  proc macros, 26 workspace libraries, 11 probes, one `asm!`. The first `test` of a profile is
  the same again for the test profile.
- **Many runs are only questions**: 73 of 116 on ripgrep's cold build, 95 of 618 on
  nushell's, and all 22 of a nushell `test` after a body edit. Each worker
  and spare loads the workspace with `cargo metadata` (6 of them), `cargo rustc --print` (6) and
  `cargo build --unit-graph` (3), each asking rustc the same questions. The nushell `test`
  took 6.0 s for a body edit.
- **On bevy every edit went to cargo.** The first edit could not be spliced into rustc's
  executable ("the program is too large", [#79](https://github.com/indexable-inc/ra-jit/issues/79)),
  so the daemon built the whole program, which took about 8 minutes and then panicked in
  rust-analyzer ("inference vars shouldn't exist out of inference"); cargo then built the edit.
  Every later edit, the error and its mending included, found the daemon unable to load the
  workspace for the same panic, waited about 8 minutes, and went to cargo again. Those are bugs of their own; for this document they show how much of a build
  can end up rustc's without anyone seeing it.
- **Plain cargo after `ra-jit enable` runs rustc on every edit**: the dep-info of the program,
  and every workspace library above an edited one, since the wrapper reaches only what cargo
  asks it to and cargo recompiles the dependents. Its doctests are all rustdoc's.
- **The fallbacks to cargo seen:** an error in the source (rustc compiles and prints it);
  nushell's signature edit, which reaches `nu-derive-value`, a proc macro ("what runs during the
  build changed"), and then rust-analyzer's `cargo check --compile-time-deps` compiles 41
  libraries, 8 proc macros and 5 build scripts again; adding a test ("a library its unit tests
  are linked with as rustc built it changed"), since the list of tests is rustc's.
- **Doctests are the worst.** Each library starts a process that loads the workspace again
  (on nushell's first run that is 87 rustc compiles of rust-analyzer's `cargo check
  --compile-time-deps`), links a probe program with rustc to learn the link line, and has
  rustc check or link examples. And a bug sends whole packages to rustdoc: the doctest
  process runs `cargo build --unit-graph` on the workspace's root package with the
  library's features ("the package 'ripgrep' does not contain these features: default, log"),
  which fails, so `globset` and `grep-printer` on ripgrep and `nu-protocol` on nushell go to
  rustdoc.
- **`check` is all rustc's**: 35, 408 and 419 crates checked or compiled.
- **Functions left to rustc**, from the workers' logs: ripgrep's program had 1,769 of 1,769
  functions compiled by ra-jit; over every build (tests and doctests included) 53 distinct
  functions were left to rustc, all for "no rustc symbol". nushell: 534 distinct, 533 for "no
  rustc symbol", 1 for "frontend: mir: HasErrors". "No rustc symbol" is a consequence of linking
  ahead of rustc's rlibs: rustc exported nothing ra-jit's copy could take the place of, so
  rustc's copy is called. It goes away with step G, when nothing rustc built is linked.
  bun's [#34](https://github.com/indexable-inc/ra-jit/issues/34) and
  [#35](https://github.com/indexable-inc/ra-jit/issues/35) are the real refusals.

## The plan

The steps in the order they can land. Each has an issue labelled `no-rustc`, #81 to #93. Effort is a rough
size for one person: small is days, medium one to three weeks, large more. Each step ends with
the counts of [How often](#how-often) lower, measured by step A's check, and none higher.

| Step | Removes | Needs | Effort | Risk |
|---|---|---|---|---|
| [A](https://github.com/indexable-inc/ra-jit/issues/81). Count every rustc run and refuse new ones | nothing; makes the rest measurable | | small | low |
| [B](https://github.com/indexable-inc/ra-jit/issues/82). ra-jit answers rustc's questions | every query | A | medium | medium |
| [C](https://github.com/indexable-inc/ra-jit/issues/83). `asm!` assembled in process | the assembler run | A | small to medium | low |
| [D](https://github.com/indexable-inc/ra-jit/issues/84). ra-jit's verdict is the final word | the fallback on an error, `cargo ra-jit check` as `cargo check` | A; borrowck work | large | high |
| [E](https://github.com/indexable-inc/ra-jit/issues/85). No function left to rustc | the refused classes; a refusal fails the build | A | large, ongoing | medium |
| [F](https://github.com/indexable-inc/ra-jit/issues/86). The standard library compiled by ra-jit | the sysroot's rlibs | C, E | large | high |
| [G](https://github.com/indexable-inc/ra-jit/issues/87). Every crate compiled and linked by ra-jit | rustc's rlibs, objects, symbols and link line, "no rustc symbol" | E, F | large | high |
| [H](https://github.com/indexable-inc/ra-jit/issues/88). ra-jit writes the test harness | the fallback when a test is added | G | medium | medium |
| [I](https://github.com/indexable-inc/ra-jit/issues/89). Build scripts compiled and run by ra-jit | build scripts and their probes by rustc | B, G | medium | medium |
| [J](https://github.com/indexable-inc/ra-jit/issues/90). Proc macros compiled and loaded by ra-jit | proc macros by rustc, the toolchain's proc-macro server | F, G | large | high |
| [K](https://github.com/indexable-inc/ra-jit/issues/91). The cold build is ra-jit's | cargo's first build, the rustc base (#11) | G, I, J | medium | high |
| [L](https://github.com/indexable-inc/ra-jit/issues/92). Doctests by the daemon (#28) | probe and examples by rustc, rustdoc | B, G, D | medium | medium |
| [M](https://github.com/indexable-inc/ra-jit/issues/93). Remove the wrapper and the fallback | `ra-jit-rustc`, `ra-jit-rustc-first`, stand-ins, dep-info, settle, `by_cargo`, `only_cargo` | all | medium | low |

A, B and C can start now and in parallel. D and E run alongside everything: they are
correctness work whose bar rises as the others land. F, G, I and J are the core and go in that
order, I and J in parallel once G is in. K, H and L follow, M last.

**A. Count every rustc run and refuse new ones.** ([#81](https://github.com/indexable-inc/ra-jit/issues/81)) Every start of rustc, rustdoc or a cargo
that compiles goes through one function that records the reason, and `ra-jit cargo status`
prints the counts. The counting of this document becomes a harness script run nightly on the
bench apps, failing when a count goes up, and the fmt job refuses a new `Command` of rustc,
rustdoc or cargo outside that function. Also fix the doctest features bug above, which sends
packages to rustdoc for no reason.

**B. ra-jit answers rustc's questions.** ([#82](https://github.com/indexable-inc/ra-jit/issues/82)) `-vV`, `--print cfg`, `sysroot`, `target-libdir`,
`target-spec-json`, `file-names`, `split-debuginfo` and the rest are answered by ra-jit from
its own targets, byte for byte as rustc 1.99.0 answers them. rust-analyzer's `toolchain_info`
asks ra-jit in process. `cargo metadata`, `cargo rustc --print` and `cargo build --unit-graph`
run with `RUSTC` naming ra-jit, and a load asks them once, not once per worker and spare. Risk:
a wrong `cfg` builds the wrong code without a word; the harness compares every answer with
rustc's on every target ra-jit knows.

**C. `asm!` assembled in process.** ([#83](https://github.com/indexable-inc/ra-jit/issues/83)) The assembly text ra-jit already produces goes to LLVM's
assembler (the LLVM backend links LLVM already) or to an assembler crate, in process. Risk is
low: the cache key and the output are the same, and `harness/asmedits.py` compares.

**D. ra-jit's verdict is the final word.** ([#84](https://github.com/indexable-inc/ra-jit/issues/84)) An error is ra-jit's, written as rustc writes it,
with the status cargo returns; the build does not go to cargo to have rustc print it.
`cargo ra-jit check` is rust-analyzer's diagnostics and ra-jit's borrow checker, with the deny
lints. This needs ra-jit to reject what rustc rejects: [borrowck.md](borrowck.md)'s
measurements of the UI suite are the bar, and the step lands in parts as they pass (errors
first, then lints). [#27](https://github.com/indexable-inc/ra-jit/issues/27) is reversed: an
item ra-jit cannot resolve is an error to fix, not a reason to build with cargo. Risk: a
program rustc rejects runs, or one it accepts is refused.

**E. No function left to rustc.** ([#85](https://github.com/indexable-inc/ra-jit/issues/85)) The classes of #34 and #35 and the rest of
[support.md](support.md#language) are compiled; a refusal fails the build with its reason. Until
G lands, "no rustc symbol" cannot go; everything else can. Axionize's work on #34, #35 and #75
is this step.

**F. The standard library compiled by ra-jit.** ([#86](https://github.com/indexable-inc/ra-jit/issues/86)) `core`, `alloc`, `std`, `compiler_builtins`
(with its intrinsics and `#[naked]` functions), `panic_unwind`, `panic_abort`, `unwind`,
`proc_macro`, `test` and their dependencies, from `rust-src`, with std's own build script and
cfgs, kept in the store per target and profile and served by the remote. The harness's std
tests (11 of 2,846 refused today) are the bar. Risk: lang items, intrinsics and
`compiler_builtins` are where ra-jit has had least practice.

**G. Every crate compiled and linked by ra-jit.** ([#87](https://github.com/indexable-inc/ra-jit/issues/87)) The harness's "alone" build, where ra-jit
compiles every crate of the program, becomes the only build. ra-jit works out the link line
itself: the libraries of `#[link]` and of build scripts' `cargo:rustc-link-lib`, the target's
system libraries and start files, `-C link-arg` and the linker cargo's configuration names, and
the shims rustc adds (the allocator's, `main`). Nothing of rustc's is linked, so symbols.rs,
weaken.rs, `Base::of_rustc` and the "ahead of rustc's rlibs" link go, and with them "no rustc
symbol", ABI and mangling parity, and [#52](https://github.com/indexable-inc/ra-jit/issues/52).
Risk: a link line that differs from rustc's on a platform; the harness links every bench app
on Linux, macOS, Windows and wasm and compares the programs' behaviour.

**H. ra-jit writes the test harness.** ([#88](https://github.com/indexable-inc/ra-jit/issues/88)) The `main` of a test executable, the list of `#[test]`,
`#[should_panic]`, `#[ignore]` and benches as libtest's descriptors, is generated by ra-jit, so
adding a test is a build like another.

**I. Build scripts compiled and run by ra-jit.** ([#89](https://github.com/indexable-inc/ra-jit/issues/89)) The daemon compiles, links and runs build
scripts with `RUSTC` naming ra-jit, so that `autocfg`, `version_check`, `rustversion` and the
feature probes of `anyhow` and `proc-macro2` are answered by ra-jit (B for their questions, D's
check for their test programs). rust-analyzer's `cargo check --compile-time-deps` is replaced by
the daemon's own run. Risk: a probe answered differently from rustc turns a feature off
silently; the harness compares every script's output with what it prints under rustc.

**J. Proc macros compiled and loaded by ra-jit.** ([#90](https://github.com/indexable-inc/ra-jit/issues/90)) The proc-macro crate and its dependencies are
compiled by ra-jit into a library that rust-analyzer's proc-macro server, running inside ra-jit,
loads. `proc_macro` is ra-jit's from F, so the bridge matches the server, which is built from the
same rust-analyzer. [#38](https://github.com/indexable-inc/ra-jit/issues/38)'s expansion cache
makes recompiling a macro rare. Risk: the bridge's ABI between the library and the server.

**K. The cold build is ra-jit's.** ([#91](https://github.com/indexable-inc/ra-jit/issues/91)) `cargo ra-jit build` with nothing built plans the units
(plan.rs), and the daemon compiles them all, taking what the store and the remote have, links
the program with G's line and patches its own executable from the first edit on. No `cargo
build`, no rustc base. See [the cold start](#the-cold-start-without-rustc).

**L. Doctests by the daemon.** ([#92](https://github.com/indexable-inc/ra-jit/issues/92)) [#28](https://github.com/indexable-inc/ra-jit/issues/28): the
examples are compiled by the daemon that has the workspace loaded, linked with G's line; no probe
program, no process per library, no rustdoc. `compile_fail` examples need D.

**M. Remove the wrapper and the fallback.** ([#93](https://github.com/indexable-inc/ra-jit/issues/93)) Plain cargo after `ra-jit enable` names ra-jit as
`build.rustc`; ra-jit answers cargo's questions (B) and writes for each unit the files cargo
expects, built by the daemon. Then `ra-jit-rustc`, `ra-jit-rustc-first`, the stand-ins, the
dep-info runs, `settle`, `Built::by`, `by_cargo`, `ToCargo`, `only_cargo`, `RA_JIT_RUSTC_BASE`
and `RA_JIT_WRAPS_RUSTC` are deleted, and step A's count is zero.

### The cold start without rustc

Today `cargo ra-jit build` from nothing returns in about cargo's time (results.md: ripgrep 6.96
s against cargo's 6.16, nushell 37.19 against 35.51) because it is cargo's build, and the
daemon is ready for edits at about twice that (#11). The first edits patch rustc's executable
so that they need not wait for ra-jit's own link. Without rustc the cold build is ra-jit
compiling every crate, the standard library included.

That is kept fast by not compiling, rather than by compiling as fast as rustc:

- **The store and the remote.** A crate compiled once, by anyone, for a target, profile and set
  of features is fetched, not compiled. On results.md's remote measurement, ripgrep with every
  function served from the remote compiled none and was ready in 7.3 s. The standard library
  and the most used crates of crates.io are compiled ahead by ra-jit and served by the remote,
  so a fresh machine compiles only the workspace.
- **Frozen dependencies** ([#40](https://github.com/indexable-inc/ra-jit/issues/40)) and **a
  warm-start manifest** ([#41](https://github.com/indexable-inc/ra-jit/issues/41)): an
  unchanged dependency, or an unchanged checkout, is linked without being analysed. These
  become the cold-start path; they no longer only save memory.
- **Compiling less.** Not built, to evaluate in step K: compiling a function the first time the
  program reaches it, so that a cold build compiles what the program uses rather than every
  function of every dependency.

What has not been measured: ra-jit compiling every crate of a bench app, its dependencies and
the standard library, with an empty store. Step G's issue starts by measuring it. The step K
bar: with a warm remote, `cargo ra-jit build` from nothing is no slower than cargo's cold build;
with an empty store, it is measured and published beside cargo's, and the gap is an issue of its
own. Until K lands, the rustc base stays as it is; it is not made faster or extended.

### Lanes in flight

Lanes whose work makes a rustc path do more, to redirect:

- **The rustc base and the cold start** (#11, `cold-start-2`): make the daemon's first edits
  fast on ra-jit's own executable, toward step K, rather than tune patching rustc's.
- **rustc-dead-code** (#52, branch `rustc-dead-code`): keeps code in rustc's link so that edits
  spliced into rustc's executable can call it. It extends the rustc base; step G removes the
  problem. Its correctness fixes may land; the keeping should not.
- **The fallback to cargo** (#27, closed): "fails instead of going to cargo" was fixed by going to
  cargo. Step D reverses it; the resolution bug behind it (`.chain()` on a tuple of systems) is an
  E or D bug to fix in ra-jit.
- **bun-edits**: patches rustc's executable for const-generic statics so that functions left to
  rustc keep working. Its correctness fix (cfcb0c879b) is fine; the rest builds on functions left
  to rustc, which step E removes.
- **kept-rejections** ([#37](https://github.com/indexable-inc/ra-jit/issues/37)): sends callers
  to rustc's copies of rejected code. Redirect to compiling them (E).

Aligned: Axionize's #34, #35 and #75 work (step E), the proc-macro expansion cache #38 (J),
include-loop's own base (K), and glancer's #40 and #41 (the cold start).

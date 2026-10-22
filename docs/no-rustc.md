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
494f0ee948. The inventory with file and line, when each runs, how often on real projects, and
the ordered plan to remove each is the next section of this document.

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

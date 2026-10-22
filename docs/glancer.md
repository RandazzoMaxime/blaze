# A front end more like Glancer

2026-10-21. What Rust Glancer and matklad say, with sources, is in
[research/glancer.md](research/glancer.md). This file is the plan: what ra-jit already has
toward each of six ideas taken from them, where the time and memory actually go, and an
ordered list of steps, each of which merges on its own. Valid MIR is never given up: every
step either keeps the analysis rustc's semantics need or skips it only where a frozen copy
of the same result exists.

## Short version

- The largest cost on the workspaces that hurt was not analysis. helix's 631 s to ready
  and 100.7 GB were names: a store key spells out every generic argument, a closure that
  wraps closures holds each of their types twice, and helix-parsec's combinators made one
  name of 665 MB. Hashing a type once its spelling passes 1 KB took helix to **109 s and
  18.4 GB**, with the same 158,768 functions compiled (step 1, below; issue #29).
- What remains on a cold start is mostly ra-jit's own: on helix, a second round of 29 s
  because one static started the build over; on both, about 76 s of thread time in
  translation. Glancer's ideas act first on loading and name resolution, which is 12 s of
  nushell's 50 s and 7 s of helix's 109 s on a cold start, and on inference and MIR only
  once something is kept from an earlier start.
- Where the ideas do pay is the **warm** start. A store that has every function still costs
  nushell its whole analysis, because the instances to look up are found by analysing: in
  the cold-start lane's run, 165,752 functions found, and still 36 s of inference, 33 s of
  MIR, 21 s of monomorphization and 88 s of translation (thread time). Items 1 and 2 are
  what make a warm start a lookup.
- Order: measure memory per query first (step 0), bounded names (done), proc-macro
  expansion cache (item 3), memo eviction (item 5's quick part), frozen dependency artifacts
  (item 1) in stages each guarded by a differential check, then the workspace's own state
  (item 2, with the cold-start-2 lane). The flat syntax tree (item 4) has its own lane
  (#42); stripping the LSP parts (item 6) waits for numbers that say it matters. Who has
  what is under "Working with the other lanes".

## What was measured

All on dev-aws-7 (c7i.16xlarge, 123 GB), the daemon pinned to 16 cores with `taskset`,
Cranelift, an empty store, `harness/coldprof.py`, ra-jit at `5f26408566` (main on
2026-10-21) and the same with step 1. "ready" is wall time from the start of `cargo build`;
"cargo" is cargo's own build, which the daemon overlaps. Steps are summed thread time.

| | cargo | ready | peak RSS | compile step | rounds | store | stitching (resolve) |
|---|---:|---:|---:|---:|---:|---:|---:|
| helix, main | 34.0 s | 631.3 s | 100.7 GB | 590.3 s | 2 (first 290.7 s) | 14 GB | 131.5 s |
| helix, step 1 | 34.0 s | **109.1 s** | **18.4 GB** | 69.7 s | 2 (first 29.2 s) | 1.6 GB | 11.4 s |
| nushell, main | 36.2 s | 51.4 s | 16.3 GB | 10.9 s | 1 | 1.1 GB | 3.0 s |
| nushell, step 1 | 36.0 s | 49.9 s | 15.8 GB | 9.7 s | 1 | 866 MB | 1.9 s |

Thread time per step, helix with step 1: infer 12.8 s, MIR 16.1 s, monomorphize 13.6 s,
translate 76.1 s, describe 6.2 s, store write 1.3 s (on main: store read 28.1 s, store write
35.8 s). nushell with step 1: infer 32.0 s, MIR 27.9 s, monomorphize 15.4 s, translate
76.1 s.

From the committed benchmarks ([`docs/benchmarks/linux-x86_64.json`](https://github.com/indexable-inc/ra-jit/blob/b3e953eb13e8e4476a4673ab1c7dfbdff61880c6/docs/benchmarks/linux-x86_64.json), `2b2f2dcd5b`, same
machine type): ripgrep ready 12.5 s at 2.6 GB, load 0.9 s, resolve 0.5 s; bevy ready
115.8 s at 8.5 GB, load 14.4 s, resolve 12.6 s; nushell load 7.2 s, resolve 5.0 s; helix load
7.1 s, resolve 5.6 s. zed did not fit in 111 GB (#29).

**After cold-start-2 (2026-10-21, dev-aws-11, 187 GB, 16 cores, same harness).** cargo
is answered from what rustc linked, and the program edits patch (the own base) is built in
the background once nothing has asked for a second:

| | `cargo ra-jit build` returned | ready | RSS at ready | own base done | own base peak |
|---|---:|---:|---:|---:|---:|
| helix | 27.3 s | 29.4 s | 4.7 GB | 108.4 s | 19.1 GB (13.6 GB after) |
| zed v1.24.4 | 160.3 s | 214 s | 14.6 GB | given up at 48.1 GB | 49.8 GB, flat after |

- helix's own base is step 3's restart round now: its first round (39.1 s) is thrown away on
  `__RUST_STD_INTERNAL_VAL`, in the background. It does not loop, and memory does not grow
  without bound; the 55 GB seen before was a binary from before step 1.
- zed's own base restarted once a second (2131 interruptions in 40 minutes, flat at
  16.2 GB): `livekit_api`, an optional dependency a dev-dependency's feature turns on, includes
  a file under an `OUT_DIR` no build script wrote, and an include that could not be read was
  counted as one still to load before every turn. Fixed on `include-loop`
  (`harness/rustcbase.py`'s `unbuilt-member`).
- With the loop gone, zed's own base grows about 1.2 GB/s and passed 150 GB in two minutes. A
  heap profile at 73.6 GB live: `described` (debuginfo types) 51.2 GB, 69.6%; `infer`, MIR
  and `compiled_instance` each under 1 GB. That is #55's, and item 5's `described` sharing.
- The safety valve until then: a worker gives an own base up once it holds a quarter of the
  machine's memory (`RA_JIT_OWN_BASE_MEMORY`), and edits patch rustc's executable. It is not
  #39's result dropping: nothing is freed (salsa keeps the memos, #57), the build just stops.

How helix was found: during its compile step the worker ran on one core at 40 to 100 GB.
A `perf` sample of it was 14% `memmove`, 13% SHA-256, 11% `core::fmt::write`, 7% hex
formatting of bytes, 6% hashing of `Arc<str>`, and the rest in `StableNames::{ty, args,
item}` and `cached::Entries::{get, usable}`. The longest printable strings in the store
were stable names: 665,474,913 bytes for one `helix-parsec` closure, and the largest packs
(3.0 GB, 1.3 GB, 665 MB) whole multiples of it.

## What ra-jit already has, item by item

1. **Frozen dependency artifacts.** Nothing of analysis is kept across runs. The store
   (`crates/ra-jit-store`, [cache.md](cache.md)) keeps compiled code only, keyed
   by stable names; `content.rs` names a crate of the checkout by its source; `remote.rs`
   shares packs through a directory or S3; `deps.rs` pushes and pulls cargo's dependency
   units. Those are the shareable, content-addressed half of item 1, for code. rustc
   compiles every dependency; ra-jit analyses dependencies only as far as the instances it
   compiles (generic or inlinable functions homed there, `home.rs`), but rust-analyzer loads
   and resolves every crate first.
2. **The workspace's own state.** Nothing persisted. While cargo builds, a spare worker
   loads and resolves the sysroot and the workspace and compiles the functions of each crate
   cargo is done with on a quarter of the cores (#11). The cold-start-2 branch made that
   lighter and built no persistence: what is left of the command's cost, 5 to 7% over
   cargo's on just, ripgrep and nushell, is the spare resolving the sysroot and the
   workspace beside cargo, which is this item's to remove (and item 1's for the sysroot),
   through the one id serialization both need, not a mechanism of its own.
3. **Proc macros and build scripts.** Build scripts: `seed_own_target` copies cargo's target
   directory and commits `20708cd2da` and `c8f58ac7d9` stop re-running scripts cargo already
   ran, so this half is mostly done. Proc macros: one proc-macro server process
   (`proc_macro_processes: 1`), and no cache of expansions.
4. **Syntax trees.** rowan everywhere. The parse query keeps at most 128 trees, macro
   expansions 512 and 1,024, bodies 512 and 4,096 (salsa `lru` attributes in the subtree),
   but ra-jit's `body_syntax` held each anchor's green node, which kept whole files' trees
   past that lru until #42's step (a)
   ([flat-syntax.md](flat-syntax.md#step-a-no-tree-outlives-its-query)). Since #42's steps
   (b) and (c) the compiler's parses make flat trees (arrays) behind rowan's cursor.
5. **Memory discipline.** No eviction of inference or MIR: `infer`, `mir_body` and
   ra-jit's monomorphized MIR have no `lru`, and ra-jit drops nothing. Measuring is there:
   with `RA_JIT_MEMORY=<dir>`, `crates/ra-jit/src/memory.rs` reports resident memory after
   load, resolve, build, warm-up and each request answered (on macOS the physical footprint)
   and writes a table of every salsa ingredient (count,
   bytes of fields and of salsa's records; heap bytes only where an ingredient declares
   `heap_size`, which no rust-analyzer query does). A build with the `heap-profile` feature
   allocates with jemalloc, reports live bytes, and with `_RJEM_MALLOC_CONF=prof:true`
   writes a heap profile for `jeprof` (on macOS for `harness/heapfold.py`, with `heap.py` to
   give it to queries). rust-analyzer's own per-query report
   (`per_query_memory_usage` in `ide-db/src/apply_change.rs`) is commented out since its
   salsa migration.
6. **LSP-shaped parts.** ra-jit has its own watcher (`inputs.rs` for the manifests and
   cargo config, `tree.rs` for sources, "watching N files"); line tables serve only panic
   locations and debug info. The client already sends a sync request per build.

## The items

### Step 1: bounded names (done)

Not one of the six, but it is what #29 was. `StableNames` memoizes the spelling of each
interned type and of each crate, and a type whose spelling passes 1,024 bytes is written as
`$` and the SHA-256 of the spelling, with a `~` kept when the spelling had one, so that
`of_checkout` still sees types of the checkout. A name's size now grows with the number of
distinct types, not with the tree they unfold to. Names under 1 KB are unchanged, so the
entries of a store stay valid. `harness/deepnames.py` builds closures nested 20 deep: on
main the store is 156 MB, with step 1 it is 0.2 MB, and a second daemon finds the 114 entries
again. In `cargo_ra_jit.py helix`, the cold `cargo ra-jit run` takes 640.8 s on main and
46.9 s with step 1 (dev-aws-7; both runs exit 1 where cargo's exits 0, a separate issue).

### Item 3: proc-macro expansion cache

**Design.** A wrapper around each `ProcMacroExpander` the workspace loads, installed by
ra-jit after `load_workspace` and before the crate graph is set. The key is a SHA-256 of
the macro dylib's content hash, the macro's name and kind, the input token trees (and the
attribute's) with spans replaced by their index in a table of the distinct spans of the
input plus def, call and mixed site, the environment the expander is given, its current
directory, and ra-jit's binary hash. The value is the output token tree with the same span
indices. An expansion is not stored when it errored, when its output has a span not in the
table, or when the expander asked the server anything during it (`SourceText`,
`LocalFilePath`, `FilePath`, `ByteRange`, `LineColumn`, `SpanSource`): those read outside
the tokens. A per-macro opt-out (`RA_JIT_PROC_MACRO_CACHE=off` or a list of names) covers
macros that read files or the environment by themselves, which the server cannot see.

**Where.** The wrapper and its disk format in a new crate under `crates/` (it is machinery:
hashing, files, no Rust semantics), stored in the same store directory with its own key
domain. Seeing the sub-requests needs a hook in load-cargo's `Expander`
(`vendor/rust-analyzer/crates/load-cargo/src/lib.rs`, around line 703): a new module file
in load-cargo declared with one line, which counts the callback's requests per expansion.

**Payoff.** Unmeasured. bevy's 14.4 s load is where it should show (derive-heavy); step 0
times expansions to say how much of a load they are before this is built. A warm start
saves the expansion and the server round trip; a cold start gains nothing.

**Risks.** A macro that is impure without asking the server (reads a file through
`std::fs`, `env!` at expansion time) gives a stale expansion: the opt-out, and a check mode
that expands anyway and compares (like `RA_JIT_CACHE_CHECK` for the store), are the
guards. Span renumbering must give back the same spans, or diagnostics and hygiene drift:
the check mode compares spans too.

### Item 5: memo eviction and per-phase memory

**Design.** First the quick part: once a body's machine code is in the object, its
`infer` and `mir_body` values and ra-jit's monomorphized MIR are not needed until an edit
reaches it. salsa's LRU drops a value and keeps the memo's revisions and dependencies
(`evict_value_from_memo_for`, salsa 0.28.5 `function/memo.rs`), so an evicted memo is still
validated by an edit and recomputed only when asked for again. Then the Glancer
discipline where it fits: ra-jit already runs a phase across all crates (load, resolve,
compile ahead, compile), and the `RA_JIT_MEMORY` report after each phase says which phase
leaves what behind; compaction and a phase arena come after those
numbers, and only for what they show.

**Where.** `lru = N` on the three queries is an attribute in upstream files; ra-jit's
monomorphized MIR is ra-jit's own query.

**Payoff.** Unmeasured until step 0; helix's 18.4 GB and nushell's 15.8 GB are the
numbers to move. The bound is what infer and MIR hold of those, which step 0 reports.

**Risks.** An edit that reaches many bodies recomputes their MIR: the edit harnesses
(`editbench.py`, `movedlines.py`, `storefirst.py`) and edit-to-binary times are the check.

**Measured (2026-10-21, #39).** The `lru` route does not work as designed, and nothing of it
is merged ([research/daemon-memory.md](research/daemon-memory.md)):
- salsa 0.28.5 takes an evicted memo of a query with cycle handling, which `infer` and every
  MIR query have, for one a panic left, and throws. With `lru` on them nushell's worker died
  in its cold build.
- An `lru` evicts only when a revision begins, so a build's peak keeps everything.
- `lru = 4096` on `described`, which has no cycle handling, lowered ripgrep's live heap once
  ready from 1,622 to 1,416 MB, and its footprint not at all: what it frees stays with the
  allocator.

What holds the memory: on nushell's 10.0 GB live heap once ready, `described` (debuginfo
types, each memo a whole copy of a type and everything it is made of) is 2.7 GB,
`compiled_instance` 0.65 GB, `runtime_mir_body` 0.55 GB, the parse trees 0.8 GB, `mir_body`
0.37 GB, `infer` 0.29 GB, salsa's records 1.5 GB in all. And about 0.6-0.9 GB of each
project's figure in `bench.py` after the first request is not the project's: it is the spare
worker the daemon starts for the next project (940 MB on ripgrep). Sharing what `described`
keeps, the spare, and a patched salsa that can evict a query with cycle handling are the
next steps, each its own issue.

**Measured (2026-10-22, #56, #57; Linux x86-64, 16 cores).** The spare is gone once a
workspace is loaded: one is started for a build as it begins, and `RA_JIT_SPARE=ready` keeps
the old one. An idle daemon holds 1.28 GB instead of 2.64; the daemon after `bench.py`'s edits
2.31 GB instead of 3.63 on ripgrep and 11.36 instead of 12.70 on nushell, with the same edit
times. A second project's cold start is unchanged
and its warm start pays the library's load, 1.3 s on ripgrep and 0.9 s on nushell
([cargo.md](cargo.md#what-a-build-does)). A vendored salsa patched to tell an evicted memo from one
a panic left lets `lru` on `infer`, the MIR queries and `inlinable` through nushell's cold
build and edits, but it is not merged: it took nushell's daemon after edits from 12.9 to
12.45 GB and its signature edit from 0.42 to 2.43 s, since what depends on an evicted memo
cannot be backdated; on `infer` and `runtime_mir_body` alone, 0.35 GB and 0.42 to 0.64 s. A
forced `mi_collect` after each request returned 30 to 40 MB of ripgrep's worker. The rest is
the values the daemon keeps, `described` first (#55)
([research/daemon-memory.md](research/daemon-memory.md#2026-10-22-the-spare-and-eviction)).

### Item 1: frozen dependency artifacts

**Design.** One artifact per dependency crate, content-addressed by its source hash, cfg,
features, toolchain and ra-jit's binary hash, holding what other crates ask of it:
signatures, module exports, impls, layouts, and MIR only for generic, `#[inline]` and
`const fn` items (the set rustc encodes, `should_encode_mir` in rustc 1.99.0). Its own
format, mmapped, not `.rmeta`. A dependency whose source is edited (a path dependency, a
`[patch]`) thaws to salsa. Artifacts live in the store directory and travel through the
remote like packs.

The obstacle is that ids are salsa-interned per process and every lookup in rust-analyzer
bottoms out in `DefMap`, `ItemTree` and the parse. So item 1 is staged:

- **1a.** Tiers (frozen, thawed, live) for each crate, the content keys (a Merkle hash over
  a crate's source and its dependencies' keys), an empty container, and a differential
  checker that, for a frozen crate, computes the live salsa value next to the frozen one and
  fails on any difference. Nothing is served frozen yet.
- **1b.** Frozen query families one at a time, in the order of measured cost (step 0
  says which): a hook at the top of each query, in a new module file of `hir-def` or
  `hir-ty` declared with one line, returns the decoded value for a frozen crate. Decoding
  interns into the running database, so ids stay process-local. Each family merges after
  the gate passes with the checker on.
- **1c.** Values read without being memoized (straight from the mmap), then zero-copy
  views for MIR and impl indices.

**Payoff.** On a cold start with no artifacts, none. On every later start, on any checkout
that shares dependencies: the load and resolve of dependencies (bevy 14.4 s + 12.6 s,
nushell 7.2 s + 5.0 s, helix 7.1 s + 5.6 s, less the workspace's own share), and the
inference and MIR of dependency instances. Memory: salsa memos of dependencies become pages
of a mapped file, which the kernel can drop.

**Risks.** It is the largest change here and touches the most upstream behaviour. The
differential checker, kept as a mode, is the guard: a frozen value that differs from the
live one is a bug, never a fallback. salsa's own `persistence` feature is the alternative
for the time win (no memory win), at the cost of serde on hundreds of rust-analyzer types.

### Item 2: the workspace's own state

**Design.** The cheap form first, which is ra-jit's and not salsa's: a manifest per build,
keyed by the content of the checkout's crates, listing the stable names of every instance
the build compiled and the roots it started from. A warm start whose crates have the same
content looks the names up in the store and links, with no analysis; a mismatch falls back
to the full build. Then salsa's state proper (memo values and dependency graph) for the
crates of the checkout, so that a start after an edit revalidates instead of recomputing.

**Where.** The manifest beside the store, in `crates/ra-jit-store`. Salsa state with
cold-start-2.

**Payoff.** The warm start: nushell found all 165,752 functions in the store and still
spent 36 s of inference, 33 s of MIR, 21 s of monomorphization and 88 s of translation
(thread time) to find which (the cold-start lane, dev-aws-13, 16 vCPU, a warm store against
an empty one). The manifest skips all of it when nothing changed.

**Risks.** A manifest that matches content but not configuration (features, target,
cfg) would link the wrong program: it is keyed by everything the store key has, plus the
crate graph.

### Item 4: a flat syntax tree for files nobody edits

Its own lane (#42); the measurements and the design are in
[flat-syntax.md](flat-syntax.md). On main after the first request, syntax was 12.7% of
nushell's live heap (1,164 MB), 22% of ripgrep's (315 MB) and 8% of helix's (852 MB), most of
it green trees of whole files that `body_syntax` kept past the parse lru. Step (a), merged,
keeps an anchor's text and range instead and parses it again when asked: nushell's syntax
falls to 676 MB and helix's to 436 MB, resident memory right after the first request is
1.1 GB and 2.1 GB less, and ready time does not move. In cold time, building green trees and
walking red ones are 16% of ripgrep's samples to the first request and 7% of nushell's. Steps
(b) and (c), merged, put a flat tree behind rowan's cursor so that no lowering code changes:
building trees falls from 12.1% of ripgrep's cold-start CPU to 1.9% and from 5.4% to 1.1% on
nushell, trees as a whole take a quarter to a third less CPU, and ready time and resident
memory do not move. What is left of syntax is lexing and parsing to events (3.6% on nushell),
token trees (4.0%) and walking (5.2%), and in memory macro expansions' trees and span maps.
Item 1 would remove most dependency parsing anyway; each says here which share it took.

### Item 6: no LSP-shaped guessing

Later, and for correctness more than speed. The client already sends a request per build;
it can send the paths and content hashes cargo's build used (from cargo's dep-info), and the
daemon then compares hashes instead of trusting its watcher's timing. The watcher stays as
the trigger for builds nobody asked for. Line tables stay: panics and debug info need them.

## Steps, in order

Each is a branch off main, merged after the gate (`cargo test --release -p ra-jit`, corpus
`--mixed 8`, mixfutures `--mixed 16 --layouts`, `growth.py`, `movedlines.py`,
`storefirst.py`, `cargo_ra_jit.py`, `borrowck_ui.py`, hir-ide failing only its known 18)
and cold-start time and RSS on helix and nushell before and after.

1. Bounded names. Done with this file.
2. Step 0, measurement: helix and nushell under a `heap-profile` build with `RA_JIT_MEMORY`
   and `prof:true`, read with `jeprof` by the function that allocated, to say what share of
   the peak inference, MIR, monomorphized MIR, item trees and syntax hold; and proc-macro
   expansions counted and timed (the one new piece of code here).
3. The restart round: helix's first round of 29 s is thrown away because one instance needs
   a writable static (`__RUST_STD_INTERNAL_VAL`, std's thread-local) without a rustc symbol.
   Rejecting the instance's call chain without starting over saves it.
4. Proc-macro expansion cache (item 3), with its check mode.
5. Memo eviction (item 5), sized by step 0.
6. Item 1a, then 1b family by family, then 1c.
7. The warm-start manifest (item 2), then salsa state with cold-start-2.
8. zed measured again (its ready path fits since cold-start-2; its own base waits on #55);
   item 6 if the numbers say so. Item 4 has its own lane (#42).

## Working with the other lanes

Who has what, as of 2026-10-22. Each lane keeps its line here current when it merges or
stops.

| Work | Issue | Lane |
|---|---|---|
| The restart round (step 3) | #37 | glancer, gated on dev-aws-7, merging |
| Proc-macro expansion cache (item 3) | #38 | glancer, gated on dev-aws-7, merging after #37 |
| `described` copies whole type closures (item 5) | #55 | share-types, dev-aws-6 |
| The spare worker's memory (item 5) | #56 | spare, its own AWS host |
| salsa eviction of queries with cycle handling (item 5) | #57 | spare, after #56 |
| Frozen dependency artifacts (item 1) | #40 | glancer, after #38 |
| The workspace's own state (item 2) | #41 | glancer, after #40's 1a |
| A flat syntax tree (item 4) | #42 | flat-syntax: done, steps (a), (b) and (c) merged |
| ra-jit never runs rustc | #81 to #93 | no-rustc; steps A to M of `docs/no-rustc.md`, A to C ready to assign |

- **no-rustc.** Andrew's rule is that ra-jit must never run rustc. Since cold-start (#11)
  the time to ready is what rustc linked. The background own base, and #37's forwarding of
  callers to rustc's copies, build on that, so the warm start is cheap today because rustc
  built the program. Without rustc the daemon's own build is the ready path again. That
  makes items 1 and 2 (#40, #41) the warm start's path once more, as they were planned
  before #11: they skip the analysis of what is unchanged on the daemon's own build.
  `docs/no-rustc.md` says in which order rustc goes (its step K, #91, is the cold start). Items 1 and 2 are designed not to need rustc's
  output, and a step here that adds a new dependency on it says so in this table.
- **flat-syntax (#42)** and item 1 both change what dependency files cost. A frozen crate
  (1b) no longer parses for its item tree, and a flat tree makes the parses that remain
  cheaper. Both measure with step 0's tooling (`harness/heap.py`, `RA_JIT_MEMORY`), and
  each says in its item here which share of load and resolve it took, so the other doesn't
  count it again.
- **share-types (#55)** and **spare (#56, #57)** both report the ready heap and the edit
  peak with `harness/memory.py`, and write item 5's numbers here when they merge.
- **cold-start-2** owns persisting the analysed workspace. Item 2's manifest is about
  compiled names and sits next to the store; salsa state is theirs. Both need a serialization
  layer for ids that survive a process: whoever lands first puts it where the other can use
  it, and says so in this file.
- **architecture split**: the store is the crate `crates/ra-jit-store`, and the code
  generator moves to a crate of its own. New store-side code (item 3's cache, item 1's
  container, item 2's manifest) goes in or beside `ra-jit-store`, behind the store's
  interface, not into `hir-ty-codegen`'s module tree.

## Not checked

- bevy and ripgrep with step 1; zed's own base past the memory bound (#55).
- Whether `instance_path` and `home::Mentions`, which also walk types without a memo,
  grow the same way on other workspaces; on helix they were under 3% of the sample.
- How much of a load proc-macro expansion is (step 0).
- What share of RSS infer and MIR hold (step 0).

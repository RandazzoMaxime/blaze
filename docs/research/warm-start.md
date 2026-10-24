# A daemon's start on a warm store, with nothing of rustc's

2026-10-24, for [#41](https://github.com/indexable-inc/ra-jit/issues/41) (the workspace's own
state on disk). Measured on dev-aws-7 (c7i.16xlarge, 64 cores, 124 GiB, Linux x86-64),
Cranelift, `RA_JIT_RUSTC_BASE=0`, with `harness/warmstart.py` on bench.py's helix, ripgrep and
nushell. Before is main at `b6e58ceab9`; after is the warm-manifest branch on it. Each claim is
**read** (measured, or read in a source), **reported** (someone says so) or **unverified** (our
inference).

## Short version

- #41 asked for the workspace's own state on disk because a restart was thought to spend its
  time inferring again. It does not. In helix's "again" (the store has every function, the
  daemon starts anew), inference was 5.2 s of thread time and the first build 51.2 s of wall
  time (**read**).
- The store saved nothing on helix. "restart" found nothing in the store and compiled 148,105
  functions in 69.6 s. "again" took 118,698 entries from the store, compiled 50,440 instances
  and took 105.1 s (**read**).
- Three costs made it so, none of them reading from disk (**read**):
  1. The walk of the types an instance mentions, which picks the crate it is compiled as
     (`home::Mentions`), went into a type as often as the type was reached. A future's type
     holds the futures it awaits, so the same few types were reached along exponentially many
     ways. One thread held the build for about 25 s while 63 were idle, and the main thread's
     `resolve` phase took 11.4 s.
  2. The 64 compiling threads read the same store entries and sets of types at the same time,
     and interning a value takes a shard's write lock even when the value is already in it:
     571 s of thread time in "store read".
  3. The first edit after a start rebuilt the checkout's code from the store, so that the edit
     after it has marks to patch against: 46.2 s, waited for before the edit was answered.
- After, helix's "again" takes 38.0 s, its first edit's rebuild 14.0 s, "restart" 34.3 s and a
  second checkout's wait until ready 59.8 s (was 105.1 s). nushell's "again" is 35.8 s (was
  37.8 s) and ripgrep's 6.2 s (was 6.3 s). Every program's output is cargo's (**read**).

## How it was measured

- `harness/warmstart.py PROJECT` gives every build its own home and one store, with
  `RA_JIT_CACHE`, `RA_JIT_RUSTC_BASE=0` and `RA_JIT_STATS=1`. Its steps are `first` (an empty
  store), `warm` (a second checkout of the same commit on that store), `restart` (the second
  checkout's daemon stopped and started) and `again` (once more). After each it makes a body
  edit and undoes it, and checks the program's output against cargo's (**read**).
- The time of a step is `cargo ra-jit build` plus the wait until `cargo ra-jit status` says
  the checkout is no longer loading, so a rebuild for the first edit is in it. What each worker
  said is in `WORK/said-STEP.txt`, and for the edits in `said-STEP-edit.txt` (**read**).
- The stacks are from `gdb -p` on the worker every 3 s (`thread apply all bt 45`). Frames are
  cut at 45, so a deep walk shows no caller. perf with `-g` names nothing: the release build
  has no frame pointers (**read**).

## Before and after

Seconds, Cranelift, one run each (**read**). "ready" is the wait after `cargo ra-jit build`;
"build" includes it for the daemon's own builds (`restart`, `again`).

| | helix before | helix after | nushell before | nushell after | ripgrep before | ripgrep after |
|---|---|---|---|---|---|---|
| first: build + ready | 48.5 + 62.2 | 47.5 + 26.2 | 55.7 + 24.9 | 56.2 + 24.6 | 15.5 + 2.8 | 14.5 + 2.8 |
| warm: build + ready | 39.3 + 105.1 | 38.0 + 59.8 | 43.6 + 35.2 | 43.5 + 31.6 | 6.7 + 3.6 | 6.6 + 3.3 |
| restart | 69.6 | 34.3 | 34.0 | 33.6 | 5.8 | 5.7 |
| again | 105.1 | 38.0 | 37.8 | 35.8 | 6.3 | 6.2 |
| edit after first | 0.18 | 0.16 | 2.12 | 0.37 | 0.09 | 0.09 |

helix's "again", from what its worker said (**read**):

| | before | after |
|---|---|---|
| first build's compile | 51.2 s | 16.4 s |
| main thread's `resolve` | 11.4 s | 0.66 s |
| "store read", thread time | 571.2 s | 129.4 s |
| the rebuild for the first edit | 46.2 s | 14.0 s |
| its `resolve` | 11.8 s | 1.0 s |

## What changed

- `home::Mentions` and `returns_async_block`'s walk go into each type once. The other walks of
  types in `hir-ty-codegen` already did (**read**).
- `Entries::get` and `Entries::types` read each entry and set of types once. A thread that asks
  while another reads it waits for that read (**read**).
- The interner (`intern::Interned::new`, `new_gc`, `InternedSlice::from_header_and_slice`,
  `Symbol::intern`) looks a value up under the shard's read lock and takes the write lock only
  to insert one. A value removed by a drop or a collection is removed under the write lock as
  before (**read**).
- The entries read from the store are freed on another thread, as the types told of were. That
  was 6 s or more of the main thread after helix's build (**read**, three samples).
- An edit does not wait for the store to have what the build before it kept, and each write
  waits for the one before. After a build from nothing, helix's write took 21.2 s, longer than
  the build's compile (20.4 s), and the edit after it waited for all of it: 21.3 s. That was the
  first measurement's 16.8 s edit after `first` (**read**).
- A worker whose input ends (the daemon stopping) waits for its store writes before it exits,
  and the daemon gives each a minute. Without that, a stop just after an edit lost what the
  build before it kept: helix's `warm` compiled 131,609 functions instead of 115,706 (**read**).

## What is left

- "store read" is still 129 s of thread time in helix's "again". The 64 threads ask for the same
  callees together, and each set of types is interned type by type (**read**). How much of the
  build's 16.4 s it still costs was not measured apart.
- 1,615 functions are "rustc's copy of a function of the checkout" and are not kept, and 24,769
  entries call one of them, so they are compiled again on every start (**read**).
- The rebuild for the first edit (14.0 s on helix) is there because a patch's marks hash
  compiled code that holds salsa ids, which a new process does not share, so the marks cannot be
  stored (**read**, `patch.rs`). Marks from stable names would let a started daemon patch what
  it read from the store with no rebuild (**unverified**).
- Adopting the executable ra-jit linked last as a base, as `Base::of_rustc` adopts rustc's,
  needs the symbols as linked. A function's symbol in a build with nothing of rustc's is
  `ra-jit.{crate id}.{path}`, and the crate's id changes with its contents, so an edit renames
  the symbols of its crate. Generic instances are named per build (**read**, `main.rs`
  `function_named_in`).

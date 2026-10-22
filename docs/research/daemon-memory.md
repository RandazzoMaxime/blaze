# What the daemon's memory is, and what salsa's `lru` can do about it

2026-10-21, for [#39](https://github.com/indexable-inc/ra-jit/issues/39) (Glancer item 5,
[docs/glancer.md](../glancer.md#item-5-memo-eviction-and-per-phase-memory)). Measured on
Andrew's Mac (Apple silicon, macOS, shared with other builds), ra-jit at `8b25a045da`,
Cranelift, the `bench.py` projects. Each claim is **read** (measured, or read in a source),
**reported** (someone says so) or **unverified** (our inference). The salsa source read is in
[docs/upstream/daemon-memory/](../upstream/daemon-memory/SOURCE.md).

## Short version

- What `bench.py` calls the daemon is the daemon's whole process tree. After the first build,
  a **spare worker** is started for the next project, and it holds a loaded standard library:
  **940 MB** of ripgrep's 2,834 MB after an edit (the project's worker holds 1,909 MB).
  The worker itself does not grow at the first edit (**read**).
- Of the worker, the largest holder is `described` (what a debugger is told of a type): on
  nushell **2.44 GB of values and 0.29 GB of salsa's records**, about 30% of the live heap.
  Each of its 77,114 memos is a type with every type it is made of, copied whole, names
  included (**read**).
- salsa 0.28.5's `lru` cannot be put on `infer` or the MIR queries: they have cycle handling,
  and salsa takes an evicted memo for one a panic left and throws (**read**, it killed
  nushell's worker in a cold build).
- An `lru` evicts only when a revision begins, so it lowers nothing during a build, and what it
  frees stays with the allocator: `lru = 4096` on `described` lowered ripgrep's live heap from
  1,622 MB to 1,416 MB and its footprint not at all (**read**).

## How it was measured

- `RA_JIT_MEMORY=<dir>` reports after load, resolve, build and warm-up, and now after each
  request a worker answers (`request-N`). On macOS the resident figure is now the physical
  footprint (`proc_pid_rusage`), which was 0 before (**read**).
- A `--features heap-profile` build allocates with jemalloc and, under
  `_RJEM_MALLOC_CONF=prof:true`, writes a heap profile per report. jeprof names nothing right on
  macOS: the profile has no load address. The daemon now writes it beside the profile
  (`<step>.load`), and `harness/heapfold.py` names the stacks with `atos` and `rustfilt`.
  `harness/heap.py` gives each allocation to the innermost salsa ingredient on its stack, as a
  query's value or salsa's record of the memo (**read**).
- Sampled bytes are scaled as jeprof scales them, `size / (1 - e^(-avg/rate))`; the sampled
  total is within 15% of jemalloc's live count (ripgrep 1,444 against 1,622 MB) (**read**).

## What holds it

Live heap by phase, jemalloc's count (**read**):

| | load | resolve | build | ready |
|---|---|---|---|---|
| ripgrep | 826 MB | 861 MB | 1,785 MB | 1,622 MB |
| nushell | 1,880 MB | 2,206 MB | 10,249 MB | 10,030 MB |

nushell's cold build compiled 6,399 functions and 152,692 instances in 14.3 s. Its heap once
ready, by holder (9.1 GB sampled: values 6.2 GB, salsa's records 1.5 GB, ra-jit's own
1.0 GB, interned 0.4 GB) (**read**):

| holder | value | record | memos |
|---|---|---|---|
| `described` (debuginfo types) | 2,440 MB | 289 MB | 77,114 |
| `runtime_mir_body` | 550 MB | | 22,285 |
| `compiled_instance` | 439 MB | 209 MB | 158,496 |
| `parse_macro_expansion` | 415 MB | | |
| `parse` | 400 MB | | |
| `mir_body` | 371 MB | | 22,615 |
| vfs and loading | 277-293 MB | | |
| splice (ra-jit) | 249 MB | | |
| `infer` | 236 MB | 53 MB | 22,559 |
| symbol table (ra-jit) | 229 MB | | |
| `Body::relative` | 186 MB | | 135,996 |
| `inlinable` | 121 MB | 140 MB | 118,693 |
| `crate_local_def_map` | 99 MB | | |
| `body_syntax` | 91 MB | 66 MB | 135,996 |
| `with_source_map` | 85 MB | | |
| `compiled_root` | 67 MB | | 9,585 |

ripgrep once ready (1,444 MB sampled): `parse` 186 MB, `described` 180 MB and 20 MB of
records, `parse_macro_expansion` 91 MB, loading 88 MB, `Body::relative` 67 MB,
`runtime_mir_body` 55 MB, `mir_body` 46 MB, `compiled_instance` 44 MB and 14 MB of records,
`infer` 44 MB and 14 MB of records; salsa's records 254 MB in all (**read**).

`described`, split by what allocated its bytes on nushell: 1,331 MB copying a type's set into
its `Arc<[DebugType]>`, 893 MB formatting names, 227 MB vectors of fields, 285 MB salsa's
completion (**read**). A set is about 32 KB: a type and the types it is made of, two pointers
deep, each with its name as a `String`. The sets of `Vec<A>` and `Vec<B>` each hold their own
copy of everything they share. It became a query in `ae53631abb` (2026-10-20) so that an edit
does not describe the types of the functions it compiles again, part of nushell's main edit
going from 0.40 s to 0.30 s (**read**, that commit's message).

parse trees stay although `parse` has `lru = 128`: `body_syntax` (135,996 memos on nushell)
holds the green subtree of every body, which keeps the trees' nodes alive. `Body::of` keeps an
`Arc<Body>` for every body, so `Body::relative`'s `lru = 4096` frees little (**unverified**:
read from the code and the counts, not measured by evicting).

The spare: after the first request the daemon starts `ra-jit --spare` with the 21 crates of the
standard library loaded ("spare: 21 crates in 5.4s"), for the next project to start from. On
ripgrep after an edit: worker 1,909 MB, spare 940 MB (`footprint` of each process) (**read**).
That is the step from "once ready" to "after the no-op" in `bench.py`: ripgrep 2,309 to
2,826 MB, fd 1,722 to 2,566 MB, nushell 11,330 to 11,970 MB (**unverified** for fd and
nushell, whose processes were not measured one by one).

## salsa's `lru`, read and tried

- `#[salsa::tracked(lru = N)]` evicts in `reset_for_new_revision`, which runs when a revision
  begins; nothing is evicted during the revision a build runs in (**read**,
  `function.rs:505`).
- Eviction sets the memo's value to `None` and keeps its revisions and dependencies; only a
  `Derived` memo is evicted (**read**, `function/memo.rs:87`). An evicted memo whose inputs
  changed cannot be backdated: there is no old value to compare, so what depends on it
  re-executes too (**read**).
- A query with cycle handling (`cycle_result` or `cycle_fn`) re-executes through
  `execute_maybe_iterate`. When the old memo was verified in the current revision and has no
  value, `previous_iteration` takes it for a cycle head that panicked earlier in the revision
  and throws `Cancelled::PropagatedPanic` (**read**, `function/execute.rs:364`). An evicted
  memo that an edit has deep-verified and something then asks the value of is exactly that.
  The memo a panic leaves has a `fixpoint_initial` origin and an evicted one a `Derived`
  origin, so salsa could tell them apart (**unverified**: not tried). The same code is in
  salsa's latest release (**read**, docs.rs).
- Tried: `lru` on `infer` (2,048), `mir_body` and `runtime_mir_body` (2,048 each) and their
  closure queries (1,024 each). ripgrep built and edited; nushell's worker exited with status
  101 and no message in its cold build, after "symbols: 230220". Under lldb the throw was
  `Cancelled::throw` from `MemoHeader::previous_iteration`, inside `infer`'s `execute`,
  reached from `compiled_root`'s `maybe_changed_after` as `root_rows` was fetched on a
  `lower_until` thread (**read**). `Cancelled` is resumed with `resume_unwind`, which skips the
  panic hook, which is why nothing was said (**read**). Every MIR query and `inlinable` has
  cycle handling too.
- Tried: `lru = 4096` on `described` alone (no cycle handling). ripgrep with jemalloc: ready
  1,416 MB live (1,622 on main), 1,403 and 1,394 MB after two body edits; footprint 1,982 MB
  ready. With the release allocator (mimalloc), the worker's footprint after an edit was
  1,907 MB against main's 1,925 MB (**read**, one run each). `bench.py --runs 3`: ripgrep
  ready 2,179 MB (2,309 on main), fd 1,818 (1,722), nushell 10,876 (11,330); after edits
  nushell 11.9-12.3 GB as on main (**read**; the machine was busy, cold builds took about
  twice main's time in that run). The edits it slows are the ones that write the whole object,
  which describe every variable's type again (nushell's generic, struct and new function edits
  were full links in that run) (**unverified** as a cost: those edits were not interleaved
  with main's).

## What would cut it

1. **Share what `described` keeps** (nushell about 2.4 GB, ripgrep about 180 MB): a memo per
   type of the type alone, naming what it is made of by type rather than copying it, names
   interned. The writer and the store's `stored_types` read sets today; both change. Keeps
   edits as fast, since it is still a query.
2. **The spare worker** (about 0.6-0.9 GB of each project's figure): start it when a second
   project asks, or let it go when a project is served. A second project's cold start pays
   loading the standard library again (5.4 s on this Mac).
3. **Eviction salsa can do**: patch `previous_iteration` to throw only for a
   `fixpoint_initial` memo (a vendored salsa, as `vendor/rustc_abi` is patched in), then
   `lru` on `infer` and the MIR queries. Bounded by what those hold (nushell about
   1.2 GB of values) and by the allocator returning it. An edit that reaches an evicted body
   re-executes what depends on it rather than backdating.
4. **What the allocator keeps**: jemalloc's footprint was 450 MB over its live count on
   ripgrep once ready, and mimalloc's eviction above returned nothing. Whatever is evicted is
   worth measuring as footprint, not only as live bytes.

# A lean frontend for compiling: where the time and memory go

2026-10-24, for [#236](https://github.com/indexable-inc/ra-jit/issues/236) (a strict, lean
frontend: lossless trees and error tolerance off the codegen path), beside [#40](https://github.com/indexable-inc/ra-jit/issues/40)
(frozen dependencies), [#41](https://github.com/indexable-inc/ra-jit/issues/41) (the warm
manifest), [#188](https://github.com/indexable-inc/ra-jit/issues/188),
[#216](https://github.com/indexable-inc/ra-jit/issues/216) and
[#11](https://github.com/indexable-inc/ra-jit/issues/11). The question: rust-analyzer was built
for an editor (a lossless tree per file, inference that tolerates any error, salsa recording
every read so any edit can be answered incrementally); which of that does a compiler pay for,
and how much? Measured on dev-aws-8 (AMD EPYC 9R14, 192 vCPU, 369 GB, Linux), ra-jit at
`c48ca3f800` and, for the listing change, `07217080d0`; release builds, Cranelift, nushell 0.108.0
(`bench.py`'s pin), the worker pinned to 16 cores (`taskset -c 120-135`) and builds on others.
Each claim is **read** (measured, or read in a source), **reported** (someone says so) or
**unverified** (our inference).

## Short version

- The two things the critique names first cost little on the codegen path. Error tolerance:
  frames that only collect diagnostics are under 0.1% of the cold start's CPU, and the
  diagnostics kept are 18 MB (0.3%) of the heap; the `*_with_diagnostics` queries (7.7% on the
  stack) spend it computing the values the compiler needs. Lossless trees: after the flat trees
  of [flat-syntax.md](../flat-syntax.md), building trees is 3.5% and walking them 1.6% of the
  cold start; trivia handling is about 0.4% (**read**).
- What the cold start (52 s to ready, 41 s of it loading the workspace) spends is lexing and
  parsing (11.5% innermost, 13.4% on the stack), macro expansion (30.8% on the stack, token
  trees 4.3% innermost) and code generation (28.6% on the stack). A parser without trivia
  would still lex and parse; what a compiler saves is in not doing it again, which is #40
  and #41's (**read** for the shares, **unverified** for the rest).
- On edits, the cost is salsa's bookkeeping: revalidating memos is on the stack in 38% of a
  body edit's samples and 47% of a signature edit's. A signature edit in nu-protocol walks
  30.6M edges, 27.6M of them to memos already verified in that revision, for 1.7M deep
  verifications and 4.7k re-executions; `compiled_instance` is 12.2M of the edges (**read**).
- Durability does not help: dependencies are already verified shallowly (17 deep
  verifications at MEDIUM per body edit). Giving workspace files MEDIUM made the first
  signature edit 3.7 s, as salsa cannot backdate a memo whose durability falls (**read**).
- Salsa's records are the largest holder of the heap after the base build: 1.3 GB, 22.6% of
  5.8 GB sampled, about 64M edges, a third of them on memos of dependencies (**read**).
- Landed: the crates a build asks for are listed on every core, not one after another: a
  signature edit's `roots` stage 53 to 30 ms and its edit-to-binary 410 to 373 ms (−9%)
  ([below](#slice-1-crates-listed-on-every-core)).

## How it was measured

- `harness/leanprof.py PROJECT --work W` runs a project's first hour as phases and writes when
  each began and ended (monotonic clock): `cold` (`cargo ra-jit build` from nothing until
  ready), `base` (the own base the daemon builds once idle), each edit of `--edits` made and
  undone `--rounds` times, then `warm` (the daemon stopped and started on the same store) and
  `warm base`. `--no-warm` stops after the edits (**read**).
- `perf record -k CLOCK_MONOTONIC -g -F 199` of the whole run, with frame pointers, cut by
  `harness/cpufold.py --phases W/phases.json`: per phase, the share of samples with a kind of
  work on the stack, the kind of the innermost frame, and for samples revalidating memos, the
  query whose memo was verified, the query that asked and the ra-jit function that asked
  (**read**).
- A build with counters in salsa 0.28.5 (not committed: a patched copy of salsa and a dump at
  each request) gave per revision the deep verifications by durability and query, edges walked
  and executions. The heap: a `heap-profile` build under `_RJEM_MALLOC_CONF=prof:true` and
  `RA_JIT_MEMORY`, read with `heap.py` (**read**).
- `RA_JIT_MEMORY` makes edits slower (to 1.5 s), so timings come from runs without it and
  memory from runs with it (**read**).

## The cold start and the base build

nushell, one run: ready in 52.1 s (`load workspace: 41.4s`; the reference `cargo build` beside
it 46.3 s), resident 5.2 GB; the own base then took 21.2 s, peak 11.2 GB, 7.5 GB after. A warm
start took 37.2 s and recompiled everything, none reused: that is #41's and #216's (**read**).

CPU on the stack (a sample counts under every kind on its stack, so columns add up to more than
100%) and by the innermost frame of a kind (adds up to 100%) (**read**):

| kind | cold, on stack | base, on stack |
|---|---|---|
| parse a file | 13.4% | 1.3% |
| parse a macro expansion | 2.9% | 1.0% |
| item trees | 7.0% | 0.5% |
| bodies and signatures lowered | 6.2% | 2.9% |
| macro expansion | 30.8% | 8.5% |
| name resolution | 8.2% | 0.5% |
| inference | 10.8% | 14.0% |
| MIR building | 9.2% | 23.6% |
| MIR passes and monomorphization | 5.6% | 22.2% |
| code generation | 28.6% | 88.3% |
| the store | 6.0% | 0.8% |
| `*_with_diagnostics` and diagnostics | 7.7% | 2.4% |
| salsa revalidating | 5.1% | 7.8% |
| salsa interning | 3.3% | 3.3% |

| innermost frame, cold | share |
|---|---|
| other (allocator, kernel, std) | 24.2% |
| inference and traits | 12.7% |
| lexed and parsed (events) | 11.5% |
| salsa: memos fetched and stored | 10.9% |
| code generation | 10.3% |
| macro expansion | 4.7% |
| name resolution | 4.7% |
| token trees (syntax to and from) | 4.3% |
| salsa: memos revalidated | 4.1% |
| flat trees built | 3.2% |
| trees walked | 1.6% |
| ast id maps | 1.5% |

Within the parser's samples, `LexedStr::new` is 10%, `event::process` 8%, the flat tree builder
6.5% and putting trivia between tokens (`intersperse_trivia`) 3.6% (**read**). So a parser that
kept no trivia would save at most about 0.4% of the cold start, and a tree format leaner than
the flat one at most its 3.2% plus 1.6% (**unverified**: an upper bound, not a measurement).

Of the cold start's revalidation (5.1%), 2.3% is `Body::of` verifying name-resolution memos
(`ModuleScope::name`, `ModuleScope::legacy_macro`) during lowering (**read**).

## Edits

Body edits (a line in `main` of nushell's `src/main.rs`) took 0.33 to 0.39 s and signature
edits (`coerce_bool` in nu-protocol, called across the workspace) 0.79 s the first time, then
0.44 to 0.47 s (**read**). Where a body edit's 300 ms goes, from the daemon's `timeline-us`
(**read**):

| stage | body edit | signature edit |
|---|---|---|
| notice (the watcher) | 35 ms | 47 to 49 ms |
| apply (outlines lowered) | 21 ms | 31 ms |
| roots (the crates listed again) | 6 ms | 50 to 60 ms |
| check (stale functions found and compiled) | 155 to 200 ms | 220 to 257 ms |
| build-rest (waiting for the diagnostics) | 35 to 76 ms | 36 ms |
| refuse and link | 28 ms | 32 ms |

- A body edit's `check` is real work: `main` is inferred, lowered to MIR and compiled again.
  The diagnostics pass (`broken_in`, 205 to 250 ms on one thread for the one edited file) runs
  beside the build and is on the critical path by 35 to 76 ms; half of it is the borrow check of
  `main`, the rest waiting on queries the build is computing (**read**).
- Salsa per body edit: 264k deep verifications at LOW walking 1.44M edges, 3.6k executed,
  mostly `row_place` (1,806) and `source_place` (804) for the lines that moved, then
  `macro_arg` (281) and `parse_macro_expansion` (157); 17 at MEDIUM (**read**).
- A signature edit asks the patch base about 6,269 roots and 106,256 instances, because every
  instance whose home crate depends on nu-protocol might be stale. That is 1.7M deep
  verifications walking 30.6M edges (27.6M to memos already verified this revision) and 4.7k
  executions; `compiled_instance` alone is 106k verifications and 12.2M edges. Interned
  arguments of tracked functions are verified 3.67M times
  (`TraitImpls::in_crate_and_deps_for_trait_and_self_ty` 1.25M, `layout_of_ty` 0.67M,
  `blanket_in_crate_and_deps` 0.47M) (**read**).
- The 19 crates touched by the signature edit were listed again one after another, 0.2 to 7 ms
  each: that is most of the `roots` stage (**read**).

## Memory

The worker's heap after the base build, jemalloc's sampled bytes given to the holder by
`heap.py` (5,789 MB sampled) (**read**):

| holder | MB | share |
|---|---|---|
| salsa's records (edges, revisions) | 1,307 | 22.6% |
| ra-jit's own (vfs, symbol table, splice) | 988 | 17.1% |
| inference and traits | 639 | 11.0% |
| codegen and monomorphization | 617 | 10.7% |
| macro expansion (trees, arguments, span maps) | 443 | 7.7% |
| MIR | 435 | 7.5% |
| interned values | 411 | 7.1% |
| source maps | 248 | 4.3% |
| bodies and signatures | 242 | 4.2% |
| syntax held (parse, ast id maps, body syntax) | 169 | 2.9% |
| name resolution | 138 | 2.4% |
| layout | 127 | 2.2% |
| diagnostics kept | 18 | 0.3% |

`heap.py --syntax` counts all syntax, expansions included, at 587 MB (10.1%): expansion trees and
span maps 324 MB, ast id maps of bodies 106 MB; after the edits 4.8%. The parse trees the
build report shows (754 MB) go at the next revision, when the parse lru evicts (**read**).
Salsa's own count of its metadata is 898 MB (**read**).

## Experiments

- **Workspace files at MEDIUM durability, except the ones being edited** (a temporary branch
  setting their text again after load). Body edits: no change. The first signature edit:
  3.7 s, against 0.79 s, as every memo that read a file now at LOW could not backdate. Rejected
  (**read**).
- **Interned arguments that are never collected** (salsa-macro-rules' implicit interner of
  tracked functions with more than one argument given `REVISIONS = usize::MAX`, so no read
  edge is recorded and none verified): salsa's metadata 898 to 859 MB, a signature edit's
  revalidation about 245 to 220 ms and its edit-to-binary median 401 to 374 ms (7%); body edits
  unchanged. It needs a fork of salsa-macro-rules, and the arguments are never freed in a long
  session. An argument that holds an interned value salsa does collect could outlive it and
  name whatever takes its slot; that every argument's values are themselves never collected
  was not checked (**read** for the numbers, **unverified** for the rest).
- **Dependencies' inputs at `NEVER_CHANGE`** (not run): salsa drops edges to them, about 250 MB of
  the records, but setting one again panics, so every input the dependencies' memos read would
  have to be one, and a change to the crate graph would need a new worker (**read** in salsa's
  source, **unverified** for the size).

## What a lean path is, ranked by what was measured

1. **List the crates a build asks for on every core.** Landed
   ([below](#slice-1-crates-listed-on-every-core)).
2. **Ask the patch base about fewer instances on a wide edit.** The largest cost found:
   `compiled_instance`'s 12.2M edges and the interned arguments under it, about half of a
   signature edit's `check`. Either a firewall per instance (a query of what the instance's
   code depends on that backdates, so verification stops there) or asking only about instances
   that mention a changed item. The stale check is codegen's, so this goes in `crates/`
   (**unverified**: no prototype).
3. **Take the diagnostics pass off a body edit's critical path** (35 to 76 ms of about 300):
   split one file's diagnostics across cores by definition, and leave out the definitions the
   edit did not change, whose diagnostics are memoized. `cargo ra-jit check` keeps the
   tolerant path whole: what it reports must stay rustc's (#222) (**unverified**).
4. **Interned arguments that are never collected**: 7% of a signature edit and 39 MB, from a
   one-line change in a pinned fork of salsa-macro-rules, once it is shown that no argument
   holds a value salsa collects (**read** for the numbers, as above).
5. **Dependencies never revalidated** (`NEVER_CHANGE` inputs, or frozen parts): memory, not
   time, as their memos are already verified shallowly. This is #40; nothing separate here.
6. **Syntax**: a parser that keeps no trivia (about 0.4% of the cold start) or drops trees once
   lowered (the parse lru and [flat-syntax.md](../flat-syntax.md)'s step (a) already do most of
   it: 2.9% of the heap). An item-only parse for item trees would save more of the parse but
   needs ast id maps and `def_syntax` that do not need the whole file (**unverified**).
7. **A strict mode without recovery or diagnostics**: under 0.1% of the CPU and 0.3% of the heap
   on a project without errors. Not worth a second path (**read**).

The cold start's biggest shares, macro expansion and parsing, are not the editor's design but
work done again that rustc keeps in `.rmeta`; not doing it again is #40 (frozen dependencies),
#41 (the warm manifest) and the expansion cache of [proc-macro-cache.md](proc-macro-cache.md).

## Slice 1: crates listed on every core

[#239](https://github.com/indexable-inc/ra-jit/pull/239). `build` listed each crate it builds (its functions, what it exports) one after another, though
`Listed::of_all` lists crates one to a core and was used only for linked crates. It now lists
them all first with `of_all`, which skips crates already listed. What is listed is the same.

nushell, main at `07217080d0` against the change on it, two interleaved runs each of
`leanprof.py nushell --no-warm --env RA_JIT_STATS=1` on cores 16 to 31, medians of the
ten edits after the first of each kind (**read**):

| | main | listed on every core |
|---|---|---|
| signature edit, `roots` stage | 53.1 ms | 29.9 ms |
| signature edit, edit-to-binary | 410 ms | 373 ms (−9%) |
| signature edit, the harness's wall time | 0.461 s | 0.426 s (−7.5%) |
| body edit, the harness's wall time | 0.346 s | 0.346 s |
| own base | 21.3 s, 21.3 s | 21.3 s, 20.8 s |
| peak during the own base | 11.3 GB, 11.1 GB | 14.1 GB, 11.2 GB |
| peak during signature edits | 7.13 GB, 7.10 GB | 6.93 GB, 7.12 GB |

The cold start does not move (51.2 and 53.2 s, against 52.1 and 52.3 s): its 289 listings are
made before the first request, not by `build`. The own base's 17 listings took about 600 ms one after
another and 155 ms together, which its 18 s of compiling hides. The one peak of 14.1 GB was not
there again; the listings end before compiling begins, where the peak is (**read** for the
numbers, **unverified** that it was noise). The signature edit's 19 listings sum to 27 ms one
after another, but each takes 8 to 18 ms beside the others, verifying the same memos of
nu-protocol: what is left of the stage is that and the exported functions (**read**).

## Not checked

- helix was not measured; nushell is the only project here.
- The counters' build was a different binary (patched salsa), so its counts and the timings
  come from different runs of the same commit.

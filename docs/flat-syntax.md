# Flat syntax trees for files nobody edits

2026-10-22, for [#42](https://github.com/indexable-inc/ra-jit/issues/42) (Glancer item 4,
[glancer.md](glancer.md#item-4-a-flat-syntax-tree-for-files-nobody-edits)). Measured on an
ephemeral AWS host (64 vCPU, Linux, release builds with mimalloc; `heap-profile` builds with
jemalloc for the heap profiles), ra-jit at `c43c6443bd` (main) and the `flat-syntax` branch,
Cranelift, the `bench.py` projects ripgrep, nushell and helix, each a cold start then the
first request. Each claim is **read** (measured, or read in a source), or **unverified**
(our inference).

## Short version

- On main, syntax holds 12.7% of nushell's live heap after the first request (1,164 MB),
  22% of ripgrep's (315 MB) and 8% of helix's (852 MB). Most of it is the green trees of
  whole files (**read**).
- They outlived the parse query's lru of 128 because each body's `body_syntax` held its
  anchor's green node, and that is a piece of its file's tree. Step (a) keeps the anchor's
  text and range and parses it again on demand: the green trees of files fall from 544 MB to
  87 MB on nushell and from 449 MB to 41 MB on helix, and syntax as a whole to about half
  (nushell 676 MB, helix 436 MB) (**read**).
- What that does to the daemon: resident memory right after the first request is 2.1 GB less
  on helix and 1.1 GB less on nushell, and 0.1 to 0.3 GB less once it settles; the time to
  ready and to the first answer does not move; parsing anchors again costs 1 to 2% more CPU
  (**read**).
- The time syntax costs is CPU, not memory: building green trees and walking red ones are
  16% of ripgrep's cold start to the first request and 7% of nushell's (**read**). A flat
  tree (arrays in preorder) is built 5 times faster than rowan's from the same parser events
  and walked 10 times faster (**read**, on 962 MB of Rust).
- Steps (b) and (c), next: a fork of rowan whose cursor has a second, flat kind of tree, so
  `syntax::ast` and all lowering are unchanged and the compiler path builds no green tree.

## What syntax holds (step 1, memory)

`harness/heap.py --syntax` gives each live allocation of a jemalloc heap profile to green
nodes, red nodes, span maps, ast id maps, token trees or the lexer by the frames on its
stack, and to the salsa ingredient that made it. After the first request (`request-1`):

| project | live heap | syntax | green, files | green, expansions | span maps | ast id maps | token trees | lexer |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| ripgrep, main | 1,407 MB | 315 MB (22%) | 201 | 37 | 29 | 12 | 16 | 17 |
| ripgrep, (a) | 1,276 MB | 210 MB (16%) | 92 | 42 | 16 | 18 | 20 | 21 |
| nushell, main | 9,177 MB | 1,164 MB (13%) | 544 | 204 | 151 | 117 | 89 | 56 |
| nushell, (a) | 8,519 MB | 676 MB (8%) | 87 | 219 | 157 | 89 | 78 | 42 |
| helix, main | 10,820 MB | 852 MB (8%) | 449 | 159 | 91 | 32 | 57 | 63 |
| helix, (a) | 10,139 MB | 436 MB (4%) | 41 | 158 | 101 | 31 | 57 | 45 |

All **read**; MB of sampled live bytes, scaled as jeprof scales them. The ripgrep (a) row
is an earlier build of the branch, which differs only in hashing each anchor more than once.
How much of the expansions' trees is live at the report moves by tens of MB between runs:
it is what their lru of 512 holds when the report is taken. Red nodes are under 3 MB everywhere: rowan's cursors are
transient. After warm-up, before the first request, nushell's syntax is 689 MB on main and
419 MB with (a), helix's 434 and 176 MB. On helix 5.3 GB of the 10.8 GB is `described`
([#55](https://github.com/indexable-inc/ra-jit/issues/55)), not syntax.

What is left after (a) is mostly macro expansions (`parse_macro_expansion`, lru 512, with
their span maps) and ast id maps that `body_syntax` makes for an anchor parsed alone.

## What syntax costs (step 1, time)

`harness/cpufold.py --dso <ra-jit>` folds `perf record -g` of a cold start through the first
request (ra-jit's own samples only) by what the innermost frame of a kind does:

| work | ripgrep (8,516 samples) | nushell (56,607) |
|---|---:|---:|
| walking red nodes (rowan's cursor) | 8.5% | 3.7% |
| building green trees (builder and node cache) | 7.8% | 3.6% |
| lexing and parsing to events | 6.7% | 3.1% |
| token trees (syntax to and from) | 6.5% | 3.9% |
| ast id maps | 1.3% | 0.5% |
| span maps | 0.8% | 0.4% |
| code generation | 16.8% | 27.6% |
| salsa | 16.0% | 14.1% |
| inference and traits | 13.4% | 26.4% |

All **read**. A flat tree replaces the first two rows; the parser's events stay.

## Where trees are made and kept

| query | what it keeps | lru |
|---|---|---:|
| `EditionedFileId::parse` | the file's green tree | 128 |
| `parse_macro_expansion` | the expansion's green tree and its span map | 512 |
| `ast_id_map` | pointers (kind and range) to items, no tree | 1,024 |
| `real_span_map` | ranges of items, no tree | none |
| `file_item_tree_query` | the item tree, no tree | none |
| `macro_arg` | the token tree of a macro call, no tree | none |
| `body_syntax` (ra-jit) | on main the anchor's green node; with (a) its text range and a hash | none |
| `Body::with_source_map` | the body and its source map (pointers) | 512 |
| `Body::relative` | the body lowered relative to its anchor | 4,096 |

All **read** in the subtree. Nothing but `parse`, `parse_macro_expansion` and, on main,
`body_syntax` holds a tree; everything else holds pointers (a kind and a range) that are
resolved against a parse when asked.

## What needs a tree after lowering

- **Diagnostics with spans** (type errors, borrowck, wfcheck): a source map gives an
  `AstPtr`, and `to_node` resolves it against the file's parse, made again when evicted.
  ra-jit's diagnostics go through `with_source_map`, which with (a) finds the anchor by its
  start and hash in the file's parse.
- **Places** (`hir-ty-codegen/places.rs`, panic locations and debug lines): the anchor's
  text, for its row starts. With (a) that is a slice of the file's text, no tree.
- **Relative bodies** (`Body::relative`, the body as if its anchor were a file of its own,
  so an edit above it does not change it): the anchor's own tree, parsed again from its text.
- **Macro inputs**: `macro_arg` lowers the call's syntax to a token tree once and keeps that;
  an attribute macro needs its item's syntax at that moment only.
- **Edit invalidation**: a changed file text reparses the file; queries that hold pointers
  compare equal when the items did not move, and `body_syntax` compares the anchor's hash,
  so an unchanged body backdates.

## Step (a): no tree outlives its query

**What changed.** `BodySyntax` keeps the file's text (shared with `file_text`), the
anchor's range, kind, edition and a 128-bit hash of its green tree. `root()` takes the tree
from a per-thread cache of the last 32, parses the anchor's text alone, or, when that parses
differently (a `macro_rules!` body that means something else at top level), finds it in the
file's parse by range and hash. `RA_CHECK_RELATIVE_BODIES=1` skips the cache and prints each
anchor that falls back. And the daemon now asks salsa to evict lru values once no build and
no tier compile holds a clone of the database (`let_go`): an lru evicts only when a revision
begins, and a daemon nobody edits begins none.

**Measured.** The heap rows marked (a) above. Resident memory after the first request and
time to ready and to the first request, two quiet rounds each, are in the table below.

| project | ready | first request answered | largest process at its peak | worker after the first request | a few seconds later |
|---|---:|---:|---:|---:|---:|
| ripgrep, main | 6.41 / 6.10 s | 9.22 / 8.88 s | 2,476 / 2,466 MB | 2,472 / 2,313 MB | 2,238 / 2,321 MB |
| ripgrep, (a) | 6.25 / 6.07 s | 9.18 / 9.00 s | 2,366 / 2,341 MB | 2,120 / 2,332 MB | 2,044 / 2,273 MB |
| nushell, main | 39.7 / 37.4 s | 66.2 / 54.5 s | 14,158 / 15,118 MB | 11,864 / 12,742 MB | 10,990 / 11,686 MB |
| nushell, (a) | 40.3 / 37.2 s | 58.2 / 54.9 s | 15,256 / 15,291 MB | 11,231 / 11,232 MB | 11,169 / 11,290 MB |
| helix, main | 37.1 / 36.4 s | 128.5 / 128.6 s | 17,991 / 18,047 MB | 14,329 / 14,300 MB | 12,551 / 12,560 MB |
| helix, (a) | 37.9 / 35.9 s | 137.5 / 126.7 s | 17,587 / 17,554 MB | 12,223 / 12,239 MB | 12,273 / 12,331 MB |

Two rounds each (`harness/coldprof.py`, release builds, nothing else running; seconds from
the start of the cold start). All **read**. The time to ready and to the first answer does not
change beyond the spread of two runs. Resident memory right after the first request falls by
2.1 GB on helix and 1.1 GB on nushell; a few seconds later main's has fallen too (the
allocator returns what the build let go of), and what stays is 0.1 to 0.3 GB less. The peak
is the build's, which (a) does not touch: trees are made and let go of during it as before.

**Cost.** Parsing an anchor again costs CPU: on nushell's cold start to the first request,
building green trees goes from 3.6% to 5.4% of ra-jit's samples, lexing and parsing from 3.1%
to 3.7% (helix 1.2% to 2.2%) (**read**, perf runs beside other work). That is the cost the
flat tree removes.

**Gate.** Unit tests (ra-jit 138, hir-ty-codegen 13), hir-ide (its 18 known failures),
corpus `--mixed 8` under Cranelift and LLVM (MATCH, 0 of 902 layouts differ), the corpus
again with `RA_CHECK_RELATIVE_BODIES=1 RA_JIT_CHECK_PLACES=1`, mixfutures `--mixed 16
--layouts` under both, growth, movedlines (6 of 6), borrowck_ui (as main), the pinned ui
tests (15 of 15) and the ui run-pass claims against `ui-known.txt`: the differences are the
host's (main has the same five).

## Steps (b) and (c): a flat tree behind rowan's cursor

**Representation.** Every node and token in preorder, as arrays: a kind (`u16`), a start
(`u32`), the number after the subtree (`u32`) and the parent (`u32`, its top bit for a
token). The end of an element is the start of the one after its subtree; a token's text is a
slice of the tree's text, kept once. This is the layout of Carbon's parse tree and Zig's AST,
and what Glancer keeps.

**How the parser emits it.** The same `LexedStr::intersperse_trivia` steps rowan's builder
takes: an enter pushes a node and remembers it, an exit writes where its subtree ends, a
token pushes a token. The parser does not change.

**How lowering consumes it.** Unchanged. rowan 0.17's cursor is about 35 functions over
`NodeData` (a green pointer, a parent, an offset). A fork in `vendor/rowan` (we pin every
layer) gives the cursor a second kind of tree: a `SyntaxNode` that is a flat tree's root and an
element's number, with nothing allocated per node, and moves that are arithmetic on the
arrays. `syntax::ast`, `AstPtr`, ast id maps, item trees and body lowering compile and run as
they are. A green tree of a part of a flat one is made only when something asks for it (the
syntax editor that builtin derives use, IDE assists): `green()` builds that part's and keeps
it with the root. `Parse` holds a green tree or a flat one; parses make flat ones, and
`RA_JIT_GREEN_TREES=1` makes green ones, to compare. Steps (b) and (c) are the one change:
with the cursor serving flat trees nothing in the compiler path makes a green tree.

**Measured first** with a prototype (the flat tree alone, one thread, nushell's sources and
the 1,384 crates in the host's cargo registry: 43,196 files, 962 MB, 465 million nodes and
tokens):

| | rowan | flat |
|---|---:|---:|
| lex and parse to events | 20.2 s | 20.2 s |
| build the tree from the events | 16.9 s | 3.2 s |
| hold all the trees (10 bytes an element) | 6,518 MB | 4,435 MB |
| walk every node and token (kind and range) | 15.0 s | 1.5 s |

**Read** (the host ran other work, so times are within 20%). rowan's trees are smaller than
their 16 bytes per child suggests because its node cache shares equal tokens and small nodes.
With a parent for every element, which the cursor needs, the flat tree is 14 bytes an element
and holds what rowan does: the win is time, not memory.

**Risks.** Equality of parses (salsa backdates a reparse whose tree is equal) is by arrays and
text instead of green pointers; the gate's edit harnesses (growth, movedlines) cover it.

## Not checked

- bun: not run.
- The flat cursor's effect on the daemon: on its own branch until its gate and numbers are in.

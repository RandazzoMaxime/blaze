# What ra-jit keeps on disk, against cargo

2026-10-23. Measured on dev-aws-14 (c8a.48xlarge, `/home` on XFS, which shares blocks between
reflinked files), Cranelift, `harness/bench.py` with `--keep`, rustup's 1.99.0. Each claim is
**read** (measured, or read in the source), **reported** (someone says so) or **unverified** (our
inference). The first pass (the store, #131 and #136) is at the end; the second pass, on
`origin/main` at 1c4cbb9 against the branch that loads build scripts from cargo's messages and
sends packs compressed once (#153), then main at 04bdd5b against the one that keeps `ra-jit.o`
packed, is first.

## Short version

- The 31.6 GB measured for bevy in the first pass was a run whose edits all fell out of the
  splice to a full 300k-function compile, which wrote a 2.4 GB `ra-jit.o` and 11.8 GB of
  `.ra-jit-units`. On main at 1c4cbb9 bevy's edits splice, and its target is 16,241 MB against
  cargo's 17,550: already below cargo on XFS (**read**).
- On XFS the pieces F/G removes (`.ra-jit-units` and rustc's program kept) free nothing on
  bevy, where every block is shared with cargo's rlibs or the daemon's program, and 53-538 MB
  on the others, where the daemon linked a program of its own (**read**, FIEMAP). On bevy they
  cost 6.2 GB only where nothing is reflinked (ext4) (**read**, `du`).
- `ra-jit.o`, the whole program as the daemon compiled it, is kept zstd-packed between the
  links that read it: ripgrep 532 → 503 MB, helix 5,022 → 4,534, nushell 6,181 → 5,774
  (**read**).
- After F/G, projected on XFS: ripgrep 450 MB against cargo's 407, helix 4,110 against 3,713,
  nushell 5,236 against 5,343, bevy 15,771 against 17,550. What is left over cargo's is the
  daemon's program being larger than rustc's (helix 748 MB against 424).
- rust-analyzer built every build script and proc macro again in `target/rust-analyzer`
  (471 MB on bevy). The daemon now loads them from what cargo said of its build, and builds
  none: bevy 16,241 → 15,771 MB, its warm-up 30.2 → 20.2 s (**read**).
- Packs sent to a remote were zstd again over zstd blocks. They are sent as they are now.
- Compressed debug information: zlib is the only one every reader takes (std's backtrace, gdb,
  lldb, addr2line); lldb 20 cannot read zstd. On bevy's program it is 2,021 → 768 MB, but a
  panic's backtrace inflates it all (~4-6 s, ~1.6 GB) (**read**). Not shipped: a choice
  for Andrew.

## After F/G, projected

bevy after the bench's edits, MB. "XFS" counts every block once (FIEMAP); "no reflinks" is
`du`, what each part costs on ext4, where every copy is a copy (**read**, both measured on the
branch's target). F/G's column is the part removed and nothing else (**unverified**: it
assumes F/G changes neither `deps` nor `incremental`).

| part | what it is | cargo's | ra-jit XFS | ra-jit no reflinks | after F/G, XFS | after F/G, no reflinks |
|---|---|---:|---:|---:|---:|---:|
| `debug/incremental` | rustc's incremental state; ra-jit's is smaller as the daemon, not rustc, builds the edits | 8,466 | 6,505 | 6,651 | 6,505 | 6,651 |
| `debug/deps/*.rlib` | cargo's rlibs, the same | 5,886 | 5,887 | 5,887 | 5,887 | 5,887 |
| `debug/.ra-jit-units/*.rlib` | the rlibs ra-jit links in place of rustc's (F/G) | none | 0 (4,215 shared with `deps`) | 4,216 | 0 | 0 |
| `debug/examples` | the program: cargo's, or the daemon's with its spliced code and a second copy of the debug sections | 2,016 | 2,197 | 3,749 | 2,197 | 3,749 |
| `ra-jit/<build>/programs/*/cargo` | rustc's program, kept for the first edits' splices (F/G) | none | 0 (shared with the daemon's) | 2,016 | 0 | 0 |
| `ra-jit/<build>/scripts` | what cargo said of its build scripts and proc macros | none | 0.03 | 0.03 | 0.03 | 0.03 |
| `target/rust-analyzer` | rust-analyzer's own build scripts and proc macros (gone) | none | 0 (471 on main) | 0 | 0 | 0 |
| `ra-jit/<build>/programs/*/ra-jit.o` | the program as ra-jit compiled it, when the daemon links its own (none on bevy) | none | 0 | 0 | 0 | 0 |
| `deps/*.rmeta`, `*.so`, `build`, `.fingerprint` | cargo's, the same | 1,177 | 1,176 | 1,058 | 1,176 | 1,058 |
| **total** | | **17,550** | **15,771** | **23,577** | **15,771** | **17,345** |

So on XFS ra-jit is 1.8 GB below cargo today and F/G does not move it. Where nothing is
reflinked F/G brings it from 23.6 GB to 17.3 GB, just below cargo, and the program's second
copy of its debug sections (1.55 GB there, below) is what is left over cargo's.

All four, MB, XFS (`bench.py`'s `disk_mb` and the breakdowns of the kept targets, **read**).
"main" is 1c4cbb9, before this pass; "scripts" is #153 (build scripts from cargo's messages);
"packed" is `ra-jit.o` kept packed (below), measured against main at 04bdd5b on the same
host and cores (main there: ripgrep 532, helix 5,022, nushell 6,181). "F/G frees" is what
removing `.ra-jit-units` and rustc's program kept would free, and "after F/G" what is left
of "packed" then.

| project | main | scripts | packed | cargo | F/G frees | after F/G | `ra-jit.o` | the daemon's program against cargo's |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| ripgrep | 533 | 533 | 503 | 407 | 53 | 450 | 35 → 6 | 83 against 53 |
| helix | 5,022 | 5,010 | 4,534 | 3,713 | 424 | 4,110 | 572 → 93 | 748 against 424 |
| nushell | 6,175 | 6,176 | 5,774 | 5,343 | 538 | 5,236 | 469 → 75 | 817 against 538 |
| bevy | 16,241 | 15,771 | 15,771 | 17,550 | 0 | 15,771 | none | 2,197 against 2,016 |

bevy's "packed" is not measured: it has no `ra-jit.o` to pack, its first edits being spliced
into rustc's program. After F/G it will have one, about 2.4 GB as the first pass saw it, a
sixth of that packed, and its own program in place of rustc's (**unverified**).

On ripgrep, helix and nushell the daemon's spare worker loads the workspace while cargo builds,
from cargo's messages as they come, so main had no `target/rust-analyzer` there either; bevy's
worker loaded after the build, which built them again. After F/G what is left over cargo's
is the program the daemon linked being larger than rustc's (its code and debug information,
Cranelift's) and the packed `ra-jit.o`, against an `incremental` smaller than cargo's: helix
397 MB over, ripgrep 43; nushell 107 under, bevy 1,779 under.

## The program's second copy of its debug sections

- What it is (**read**, `splice.rs`, `splice/debug.rs`): a splice appends code behind the
  image and copies the sections a debugger reads behind that code (64 MiB of room), pointing
  the section headers at the copies. Each splice sources the copies from the linker's own
  offsets, which are never written, and a copy the appended code has grown into is copied
  again from there. Undoing the last splice is already a journal of the ranges it overwrote
  (`self.undo`): the linker's copy is the source of the next copy, not of the undo.
- What it costs: on XFS the copies are `copy_file_range` reflinks of the linker's blocks,
  181 MB on bevy; with no reflinks it is the sections again, 1,553 MB (**read**).
- Not changed here. A copy a section that outgrew its room moves is written over the copies
  behind it, and today that is undone by copying from the linker's offsets again. Without them
  the undo would have to journal whole copies, unless a section only ever moves to space past
  the end of the file: the headers pointed at the copies when they are first laid out
  (`Described::laid_out`), the linker's ranges punched then, and a copy that moved punched
  once the splice that moved it can no longer be undone, at the next splice. That changes
  the order a splice writes in (`splice.rs`) and is worth 1.5 GB on bevy only where nothing
  is reflinked; on XFS 181 MB (**unverified**, the design; the cost is **read**).

## `ra-jit.o`

- It is the whole program as the daemon compiled it, written whenever the daemon links a
  program of its own rather than splicing into rustc's: ripgrep 35 MB, nushell 474 and helix
  571 in this bench, during the warm-up; bevy, whose first edits were spliced into rustc's
  program, has none (**read**, the files' times). A patch is laid out against it, and a full
  link links it (`main.rs` `sync`, `cargo/plan.rs`). Once F/G has rustc build no program,
  every project has one.
- helix's body edit links in full: "no patch: a closure in what rustc linked is another now",
  a 151k-function compile and 40 s against cargo's 2.3 s (**read**, its bench log), on main
  and on the branch.
- Only a link reads it on Linux: the splice of an ELF executable reads the executable and
  the patch, not the object (`splice.rs` `Baseline::parse`), and the debugger reads the
  executable. So the daemon keeps it zstd-packed beside where it was, `ra-jit.o.zst`, from the
  end of each link to the next that reads it, which inflates it there first (`whole.rs`).
  Not on macOS, where a debugger reads the object, nor with `split-debuginfo`, nor for the
  harness's batch builds, which read the object.
- Not in the global store: no other checkout or daemon has the same bytes, and the store's
  collection could drop it while the daemon still links with it (**unverified**).
- Packing takes 12-126 ms after a link, 16 threads at level 1 (**read**, the daemon logs);
  inflating happens only for a link of an object a patch was spliced over, which a full build
  does not need (it writes a new one).

## Compressed debug information

Compressed with Python over the ELF section headers (`SHF_COMPRESSED`), since neither
nixpkgs' binutils 2.46 `objcopy` nor LLVM 20's `llvm-objcopy` writes zstd (**read**).

| reader | zlib | zstd |
|---|---|---|
| std's backtrace (1.99.0) | file and line | file and line |
| gdb 17.2 | breakpoints, `bt`, locals | the same |
| lldb 20.1.8 | breakpoints, `bt`, locals | no line table: fails |
| binutils 2.46 `addr2line` | file and line | not built with zstd |

(**read**, a fixture and bevy's `breakout`, each reader run on each.)

- zlib is the one path every reader takes. On bevy's program: debug sections 1,576 → 376 MB,
  the file 2,021 → 768 MB; inflating them all takes 3.8 s on one core (**read**).
- What it costs: a panic's backtrace inflates every section, about 4-6 s and 1.6 GB of memory
  per symbolized panic on bevy (**unverified**, from the inflate time); gdb's start 11.47 →
  8.32 s, lldb's 3.49 → 6.41 s (**read**).
- Not shipped. Split DWARF in the store would keep the program small without the panic's cost,
  but lldb and std read `.dwo` and `.dwp` differently, and it is not measured here.

## The store

- 64K blocks compress helix's store 4.2x at zstd 1 and 4.5x at 3; 256K blocks 5.4x and 5.9x;
  about 700-780 MB/s to compress, 1.4-1.6 GB/s to read back, one core (**read**,
  `packstat.py` on helix's 229,358 records). 128K at level 3 is what shipped: 5.0x on helix,
  5.1x on bevy.
- Its bound is a tenth of what the disk has for it (free space plus the store), at least
  256 MiB and at most 32 GiB, recomputed at each collection; `RA_JIT_CACHE_MAX` sets bytes or
  `off`, `ra-jit cache gc` collects, `ra-jit status` says the size and the bound
  ([cache.md](../cache.md)) (**read**).
- `bench.py` says of each project how much of its store no project before it in the run had
  stored (`store_added_mb`).
- A pack sent to a remote is `packs/<sha256>.pack` as the store has it, zstd blocks already;
  it was zstd again as `.pack.zst`, which an older ra-jit's remote may still hold and is still
  read ([cache.md](../cache.md)) (**read**).

## Left

- The daemon's program larger than rustc's: helix 748 MB against 424.
- helix's closure fallback, which links in full.
- The second copy of the debug sections where nothing is reflinked (above).
- Whether to compress the program's debug information with zlib (above).

## First pass (2026-10-22 main, against #136)

Every block counted once, MB, after the bench's edits.

| project | ra-jit target | store before | store after | ra-jit total before → after | cargo target |
|---|---:|---:|---:|---:|---:|
| ripgrep | 532 | 20 | 6 | 552 → 538 | 407 |
| helix | 4,774 | 1,686 | 339 | 6,462 → 5,113 | 3,713 |
| nushell | 5,927 | 219 | 64 | 6,159 → 5,991 | 5,343 |
| bevy | 31,575 | 8,334 | 1,623 | 39,919 → 33,198 | 17,549 |

bevy's 31,575 is the run whose edits fell to full links (above). Counted by `du`, as
`bench.py` did before #131, it was 43,063 MB: reflinked copies counted twice.

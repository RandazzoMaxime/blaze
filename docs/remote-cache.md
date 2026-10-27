# A shared remote cache: one store for CI and everyone

2026-10-27. Status: design, nothing built. Written against blaze main `e4bbb88`. Each claim is
marked **read** (in a source we read, or measured by us), **reported** (a document or someone
says so, not checked again here) or **unverified** (our inference or an estimate). "Slice R2"
below is [ci-runner.md](ci-runner.md)'s second slice; "R2" alone is Cloudflare's object store.

Andrew wants CI and local testing to be "uber fast", and wants other people to benefit from the
cache. [AGENTS.md](../AGENTS.md) makes blaze's own store "the one cache", locally and in the
shared remote CI fills, and rules out sccache. This file is the design of that shared remote:
how CI on main fills it, how CI on every branch and any user reads it, and how nobody else can
put code in it. It builds on [cache.md](cache.md) (the store and today's remote), on
[ci-runner.md](ci-runner.md) (test results keyed by content) and on [cargo.md](cargo.md) (the
sandbox and `RA_JIT_TESTS=affected`).

## Short version

- **Two layers.** A content-addressed store (CAS) of immutable blobs, each named by the SHA-256
  of its uncompressed bytes, kept in packs of 16 to 64 MB. Above it an index that maps an input
  key (a function's key, a dependency unit's key, a test's key) to the blob that is its output.
  The CAS cannot lie: every blob is hashed after it is fetched. The index can, so every entry is
  under an Ed25519 signature that clients check, and only CI on main holds the key.
- **Backend: Cloudflare R2.** Writes go to its S3 API with a token only main's CI has. Reads go
  through an R2 custom domain on Cloudflare's CDN: no egress charge, served from the nearest
  point of presence, cached at the edge. A user can stack a private tier of their own above it.
- **The key stops naming the blaze binary.** Today an entry is keyed by what blaze was built from
  (or, for the published crate, the bytes of its executable), so every release, several a day,
  misses everything. The dev-aws-14 store shows it: seven near-identical 227 MB packs written in
  ten hours, the same code stored once per blaze build (**read**). The new key names a codegen
  version that changes only when output would, and a CI check on a fixed corpus keeps it honest.
- **Fetch time.** blaze's own workspace is about 270 to 320 MB in the store (**read**, below). At
  the measured 210 to 225 MiB/s from R2 to a GitHub runner over 16 streams that is about 1.3 s,
  against about 8 s on one stream (**unverified** arithmetic on **reported** rates). The fetch
  starts when the daemon starts and overlaps the workspace load.
- **Sans-IO.** Hashing, chunking, packing, the index, signatures and the protocol are pure state
  machines in their own crates. Requests and responses cross one trait, with tokio over HTTP/1.1
  and HTTP/2 as the default backend, and io_uring and HTTP/3 as backends behind measurements.
- **Chunking** with VectorCDC (the SIMD form of AE, a hashless content-defined chunker) for large
  opaque files: rlibs, dependency units, the toolchain. Compiled functions are not chunked; their
  key already deduplicates them. **Compression** is zstd per blob, a high level on CI's upload,
  with trained dictionaries for the small function blobs.
- **GC is R2's lifecycle rules, no job**: the index expires 30 days after upload, blobs 90, and
  CI on main copies onto itself any blob it used that is older than 45 days, so a blob always
  outlives every entry that names it.
- **Rollout** in nine slices; the first two (R2 behind today's remote, then the codegen version)
  give most of the speed.

## Goals and non-goals

Goals:

- CI on any branch, and a user's first build of a project, take every function, dependency unit
  and test pass that main's CI already made, and compile or run only what differs.
- Anyone can read the shared tier with no account and no token, from anywhere, at no cost to them
  or to us beyond storage.
- Nobody but CI on main can make a client run code the shared tier serves, whatever they do to
  the bucket, the CDN or the network. A user's own tier is trusted by that user's keys only.
- A new blaze release that does not change what blaze compiles keeps every entry.
- Every part that decides what to fetch, how to check it and how to write it can be tested
  without a network, and the transport can be replaced.

Non-goals:

- Remote execution. A function compiles in milliseconds and a test runs in a sandbox locally in
  about 8 ms (**read**, [ci-runner.md](ci-runner.md#protocol-why-none-on-the-hot-path)); there is
  nothing to execute remotely. REAPI stays an edge adapter ([#226](https://github.com/indexable-inc/blaze/issues/226)).
- Writes to the shared tier from anyone but CI on main: no pull request, no fork, no user.
- A rust-analyzer analysis cache. The analysis is rebuilt at load, as today ([cache.md](cache.md)).
- Keeping every main commit forever. What main has not used for a month or three expires.
- Sharing between different codegen versions or toolchains. A different version is a different
  key, by design.

## What exists today

From [cache.md](cache.md) and `crates/ra-jit/src/remote.rs` (**read**):

- The remote is a directory, `s3://` (the AWS SDK) or a read-only `https://` address. It holds
  `packs/<sha256>.pack` (or `.pack.zst`), `keys/<sha256>` (the keys in one pack),
  `manifests/<project>` (one line per pack) and `units/`, `deps/` for cargo's dependency builds.
- A pack is named by the SHA-256 of its bytes and checked after the fetch. The manifest is the one
  mutable object; it is signed with Ed25519 when `RA_JIT_REMOTE_SIGN` is set, carries a turn
  number that only goes up, and a machine refuses a turn lower than one it took (`signed.rs`).
- A daemon with `RA_JIT_REMOTE` pulls before its first build (all packs, or with
  `RA_JIT_REMOTE_LAZY` only the key lists and then a pack when a key in it is first asked for),
  and pushes every five minutes and when it ends (`keeper.rs`).
- A fetch that fails its hash is thrown away, reported, and the functions are compiled again.
- Real S3 was never run: everything was measured against MinIO and SeaweedFS over loopback.
- No workflow in `.github/workflows` sets `RA_JIT_REMOTE` (**read**). CI fills no remote today.

The identity every store key is hashed with is `Identity(FORMAT, build, toolchain)`
(`crates/ra-jit-store/src/lib.rs`, **read**). `build` is a hash of every source file of the
workspace (`crates/ra-jit/build.rs`), or the executable's bytes for a blaze built elsewhere. So
any commit that touches a crate of blaze, a doc-comment fix in the CLI included, gives a new
identity and an empty cache.

## Prior art

| System | What it does | What we take, or not |
|---|---|---|
| Bazel REAPI ([remote_execution.proto](https://github.com/bazelbuild/remote-apis/blob/main/build/bazel/remote/execution/v2/remote_execution.proto)) | CAS of `Digest{hash, size}` plus an ActionCache from action digest to result. AC entries carry no signature; trust is the server's access control. Recently `GetChunkMapping`/`SpliceBlob` with FastCDC 2020 (avg 512 KiB recommended) | The CAS/AC split. Not its unsigned AC (we serve from a public CDN, so the index must carry its own proof), and not its per-entry unary lookups: 165,752 functions would be 165,752 calls (**read**, [ci-runner.md](ci-runner.md)) |
| [Attic](https://github.com/zhaofengli/attic) | Nix cache on S3: FastCDC 16/64/256 KiB for NARs of 64 KiB and up, zstd, narinfo signed by the server, GC by retention then orphans | Same chunk sizes as a start; the index expiring before the blobs it names. Not server-side signing: our server is a bucket |
| casync / [desync](https://github.com/folbricht/desync) | Chunk stores `<4 hex>/<id>.cacnk`, chunks 16:64:256 KB, zstd, SHA-512/256 or SHA-256, index files listing chunks | A chunk list per large file; over S3, HTTP and OCI alike |
| Nix binary cache ([narinfo](https://nix.dev/manual/nix/2.35/protocols/binary-cache/narinfo.html)) | Ed25519 over a fingerprint, `trusted-public-keys = name:key`, keys named `cache.example.org-1` and incremented to revoke; rotation signs with both keys ([nix#12976](https://github.com/NixOS/nix/pull/12976)) | Named, numbered keys and an overlap when rotating |
| sccache 0.19 ([S3.md](https://github.com/mozilla/sccache/blob/main/docs/S3.md)) | S3 and R2 (`SCCACHE_ENDPOINT`, `SCCACHE_REGION=auto`), `SCCACHE_S3_NO_CREDENTIALS` for public read-only PR builds. Keys by BLAKE3 over the sysroot's shared libraries, arguments, sources and env ([Caching.md](https://github.com/mozilla/sccache/blob/main/docs/Caching.md)) | Read-only anonymous access for PRs. Not the tool (AGENTS.md forbids it), and not keying by the compiler's bytes, which is the problem this design removes. Whether it hashes the rustc binary itself we did not confirm (**reported**) |
| [kache](https://github.com/kunobi-ninja/kache) | Content-addressed Rust cache over S3, GCS or OCI | Nothing new beyond the above |
| VectorCDC ([FAST '25](https://www.usenix.org/conference/fast25/presentation/udayashankar)) | SIMD forms of the hashless chunkers RAM and AE: VRAM 24 to 26 GB/s with AVX-512, VAE 5.2 GB/s with 256-bit vectors, against FastCDC 2 GB/s and Gear 0.95 GB/s on the paper's machine (**reported**) | The chunker, via ix's implementation (below) |

VectorCDC is not a gear hash: it vectorises AE and RAM, which find a local byte maximum and need
no rolling hash at all, and the paper reports that its SIMD approach did not speed up FastCDC
(**reported**). The decision stands; only the description changes.

## Key and identity

### Function keys

The key of a function is what [cache.md](cache.md) describes (backend, optimisation level,
target, debug info, overflow checks, the stable name with generic arguments and crate), and the
identity it is hashed with becomes:

    Identity = SHA-256(STORE_FORMAT, CODEGEN, toolchain commit)

with no `build` field. `CODEGEN` is a number in a new crate, `crates/blaze-semver` (its only
content), and there is one such number per kind of entry:

| constant | covers | bump when |
|---|---|---|
| `CODEGEN` | compiled functions: MIR lowering, the backends, layout, mangling, relocations, unwind and line tables, the record's encoding | the bytes of any stored function for some key would change |
| `EXPAND` | proc-macro expansions | an expansion's stored form or its key changes |
| `SCRIPT` | build-script runs | the stored run or its key changes |
| `UNIT` | dependency units | a unit's contents for one key would change |
| `TESTS` | test pass records | the reach hash, the sandbox's recording, the fixed environment or the harness changes what a pass means |

A toolchain bump (`harness/toolchain`) is still a new identity: the std and the language blaze
matches move with it ([AGENTS.md](../AGENTS.md), own-nightly). The local store takes the same
identity, so what the remote serves drops into it unchanged.

### Keeping the version honest

A version that is not bumped when output changes serves the old blaze's output to the new blaze:
a codegen fix that never takes effect on cached functions, or, worse, functions of two layouts
linked into one program. So the version is checked, not trusted.

- **The golden check.** A gate job (`harness/semver.py`, in the merge gate) builds a fixed corpus
  and hashes every stored value per kind into one digest per key, written to
  `harness/semver/<target>.txt` (key, value digest). The corpus is blaze's own workspace, the five
  bench apps, and every regression test of the codegen and difftest harnesses. If any digest
  differs from the committed file and the constant of that kind was not bumped in the same change,
  the gate fails and names the first keys that differ. A bump regenerates the file.
- **Determinism.** The job builds the corpus twice, in two fresh stores, and fails when the two
  differ. Nondeterministic output is a bug ([#200](https://github.com/indexable-inc/blaze/issues/200)
  for whole-program layout); without determinism no cache is sound.
- **Bug fixes force a bump.** A codegen bug fix comes with its reproducer in the corpus
  ([AGENTS.md](../AGENTS.md), best-long-term-design). The fix changes that reproducer's output, so
  the golden check fails until `CODEGEN` is bumped. A fix that changes nothing in the corpus is a
  fix without a test.
- **Coverage is the hole.** A change that alters output only for code outside the corpus passes.
  The corpus is large on purpose, and slice 2 measures how many keys it has against a typical
  project.
- **Dev builds.** A blaze built from a dirty tree, or not by CI, reads the shared tier and writes
  only the local store and private tiers. The shared tier is written only by CI on main, after
  the golden check passed on that commit.

### Other keys

- **Dependency units.** `SHA-256(UNIT, toolchain, target, package id, version, the
  Cargo.lock checksum or git commit, features, profile settings, the unit keys of its
  dependencies)`. The output is a unit record: each file's path, mode, size and chunk list.
- **Test passes.** As [ci-runner.md](ci-runner.md#what-a-test-result-is-keyed-by) says, in two
  steps, because what a test read is only known after it ran. The entry key is
  `SHA-256(TESTS, identity, target, reach hash, attributes, fixed environment, the content of each
  CARGO_BIN_EXE_* it runs)`. Its value is a list of pass records, each holding the paths the
  sandbox recorded with their digests (absent paths included) and the seed it ran with. A client
  checks those paths on its own machine; when all match, the test is skipped as passed. Only
  passes are kept: a failure is never a reason to skip.
- **Flakes.** Two verdicts under one key are nondeterminism by construction
  ([ci-runner.md](ci-runner.md#nondeterminism-and-flakes)). CI on main writes a `flake` entry under
  the same key when a rerun disagrees, so every client knows the test is main's bug and not the
  pull request's. The seed is a hash of the key, so a rerun sees the same randomness.
- **Proc-macro expansions and build-script runs**, as cache.md keys them, under `EXPAND` and
  `SCRIPT`.

## Object layout in the bucket

The bucket exists: `blaze-cache`, in Indexable's Cloudflare account, one tier. The repository
has the secrets `R2_ACCESS_KEY_ID`, `R2_SECRET_ACCESS_KEY` and `R2_ENDPOINT`, from an object
read and write token scoped to that bucket alone, checked with `aws s3 ls` (**reported**, by the
coordinator, 2026-10-27). Its one lifecycle rule so far aborts multipart uploads left incomplete
after a day. Everything except heads and the trust list is immutable and named by a SHA-256.

    trust/keys                            the signing keys, signed by the root key; never expires
    index/heads/p/<project>/<commit>      a signed head: one main commit of one project
    index/heads/p/<project>/latest        a signed head: the newest of the above
    index/heads/u/<unit-key>              a signed head: what any project reuses of one dependency unit
    index/seg/<sha256>                    an index segment (sorted entries, deterministic CBOR)
    cas/packs/<sha256>.bpk                a pack of blobs: frames, then its index
    cas/dict/<sha256>                     a zstd dictionary

The two top prefixes are the two lifetimes of [GC](#gc): `index/` lives about 30 days from its
upload, `cas/` about 90.

A `<project>` is a name CI picks (`blaze`, `ripgrep`, the bench apps). A `<commit>` is a main
commit's id. A `<unit-key>` is the hex of a dependency unit's key.

### Blobs and packs

A blob is named by the SHA-256 of its **uncompressed** bytes. That name survives a change of
compression level or dictionary, and it lets a client check one blob read by a range out of the
middle of a pack, without the rest. There are three kinds: a function record (what the store
holds today under one key), a chunk of a large file, and a small record (a unit record, a test
pass list, an expansion).

A pack (`.bpk`) is a header, then frames, then an index, then a fixed footer that gives the
index's offset and length so one ranged read of the tail finds it. Each frame is one zstd frame
holding one blob. The index lists, per frame: the blob's SHA-256, its offset and compressed
length in the pack, its uncompressed length, and the dictionary it was compressed with (by
SHA-256, or none). A pack's own name is the SHA-256 of all its bytes. Packs are 16 to 64 MB: big
enough that a project's whole store is tens of objects, small enough that one is a few ranged
reads and fits Cloudflare's 512 MB cacheable limit with room ([default cache behavior](https://developers.cloudflare.com/cache/concepts/default-cache-behavior/), **reported**).

This is also the store's next format (`STORE_FORMAT` 4): the local store keeps the same packs and
reads a value by decompressing its frame. A fetched pack is put in the store as it is and mapped,
not compressed again. Today's packs compress 128 KB blocks together, which a small value with a
dictionary can match on its own (to be measured in slice 6).

CI writes the functions of one crate together, in the order the daemon walks them, so a client
that needs most of a crate reads one contiguous range.

### Index segments and heads

A segment is a sorted table of entries, `(kind, key) → value`, encoded as deterministic CBOR (the
same encoding the agent API uses, [AGENTS.md](../AGENTS.md), no-mcp). A value names blobs by
SHA-256 and, for each, where it is: pack, offset, length. A segment is named by the SHA-256 of
its bytes and holds at most about 4 MB, so a project is tens of segments.

A head says what one main commit of one project used:

    head {
      name: "p/blaze/9f3c...", turn: 1834, created: 2026-10-27T03:40:00Z,
      versions: { store: 4, codegen: 41, expand: 7, script: 3, unit: 2, tests: 5 },
      toolchain: "nightly-2026-10-25", target: "x86_64-unknown-linux-gnu",
      segments: [ sha256, ... ],
      signer: "blaze-ci-1", signature: ed25519(over everything above)
    }

A project head names every segment that main's build at that commit read or wrote: it is the
prefetch list. A unit head (`index/heads/u/<unit-key>`) names the segments holding what any build has
of that dependency unit: its rlib chunks, its functions, its non-generic code. A user's project
that depends on `serde 1.0.228` with the same features finds those by the unit key, computed from
its own `Cargo.lock` before any request. That is how people other than us benefit: every
dependency blaze's CI has built, for its own workspace or the corpus, is reusable by anyone with
the same version, features, target and toolchain.

Generic instances are kept under the crate whose types they are instantiated with: an instance of
`Vec<serde_json::Value>` belongs to `serde_json`'s unit head when all its type arguments come from
that unit's dependency closure, and to the user's crate otherwise (the user's own crates are not
in the shared tier).

### Large files

A file over 64 KiB that is opaque to blaze (an rlib, a static library, an archive in a unit, the
toolchain's tarball) is cut into chunks by VectorCDC, minimum 16 KiB, average 64 KiB, maximum
256 KiB, and stored as a chunk list: the chunk SHA-256s in order, with the file's own SHA-256. Two
builds of a crate that differ in a few functions share most of their chunks. The sizes are those
of Attic, desync and ix's own chunker (**reported**), and are tuned in slice 5 against blaze's
units: dedup ratio across consecutive main commits against index size. A file under 64 KiB is one
blob.

The cut points are part of the format: the scalar implementation is the definition, every SIMD
path (AVX-512, AVX2, NEON) must find the same cuts, and a property test checks that on random and
on real inputs. Changing the parameters changes chunk lists, not blob names; dedup across the
change is lost, nothing else.

## Hash: SHA-256

SHA-256 matches the store, the existing remote, OCI and Nix, so nothing is converted at a
boundary. On the machines that matter it runs in hardware:

| machine | SHA-256, one core | source |
|---|---|---|
| AMD EPYC 9R45 (Zen 5), SHA-NI, OpenSSL 3.5.9 | 2.22 GB/s (BLAKE2b-512: 1.64 GB/s) | **read**, dev-aws-14, 1 MiB blocks |
| Apple M2 Max, Rust `sha2` | 2.38 GB/s | **reported**, [checksum-benchmarks](https://github.com/conradludgate/checksum-benchmarks) |
| Apple M4 Pro, CommonCrypto | 3.28 GiB/s | **reported**, [notonbluray](https://notonbluray.com/blog/benchmarking-aes-256-sha256-and-crc-64-on-apple-silicon/) |

Against the network: 16 streams from R2 deliver about 225 MB/s of compressed bytes, which
decompress to about five times that, 1.1 GB/s (compiled code compresses to about a fifth,
[cache.md](cache.md)). Hashing 1.1 GB/s takes half a core with SHA-NI. A GitHub runner has four.
BLAKE3 would save part of that half core (it roughly ties SHA-256 on one Apple core and is far
faster with AVX-512 and many threads, **reported**), and cost a second hash in every format and
tool. The Rust crate is `sha2` 0.11, which picks SHA-NI or the ARMv8 extensions at run time
(**reported**, [README](https://github.com/RustCrypto/hashes/blob/master/sha2/README.md)).

## Compression

- **zstd, one frame per blob.** CI compresses what it uploads at a high level: a blob is written
  once and read by every CI run and user after, and zstd decompresses at about the same speed
  whatever the level. Level 19 runs at a few MB/s a core (**unverified**, to be measured), which
  is fine for a main commit's new blobs (tens of MB) and slow for a cold fill after a `CODEGEN`
  bump (about 1.5 GB uncompressed, about a minute on 8 cores at 3 MB/s a core, **unverified**).
  So the level is a setting measured in slice 6: high for incremental pushes, lower for a cold
  fill that a background job compresses again later. The blob names do not change.
- **Dictionaries for small blobs.** A function record is about 400 bytes on average (nushell:
  165,752 functions in 64 MB after zstd, **read**, [ci-runner.md](ci-runner.md)), too small for
  zstd to find much in on its own. CI trains a dictionary (`zstd::dict::from_samples`, 112 KiB)
  per (codegen version, target, blob kind) from a sample of main's blobs, and compresses each
  small blob with it.
- **How dictionaries are named and versioned.** A dictionary is a blob like any other: stored at
  `cas/dict/<sha256 of its bytes>`, immutable, and named in each pack's index by that hash. The zstd
  frame also carries a 32-bit dictionary id, which is a hint only: we set it from the hash's first
  bytes, mapped into the range zstd leaves for private use (over 32,767 and under 2^31,
  [format](https://github.com/facebook/zstd/blob/dev/doc/zstd_compression_format.md)). A new
  dictionary is a new hash; packs written with the old one keep naming it, and a pack's refresh
  refreshes the dictionaries it names. Retraining happens on a `CODEGEN` bump and when slice 6's
  measurement says the ratio has drifted.
- **Large chunks** are compressed without a dictionary, and kept as they are when compression
  saves less than an eighth.

## Trust and signing

The CAS needs no trust: a blob that does not hash to its name is refused. The index is where a
writer could poison clients: an entry that maps a real key to other code. So every entry is under
a signature, and the signature is checked before anything it names is used.

### What is signed

One Ed25519 signature per head, over the head's fields, which name segments by SHA-256, which
hold the entries. So each entry is covered by exactly one signature check and one hash. Signing
each entry on its own would cost 64 bytes an entry and a verification each: about 165,000
verifications for nushell, several seconds of one core at tens of microseconds each
(**unverified**), against one per head. ed25519-dalek 3 does the checks.

### The keys

- **Root key** `blaze-root-1`: offline, held by Andrew. Its public half is compiled into blaze.
  It signs only `trust/keys`.
- **`trust/keys`**: a list of signing keys, each with a name (`blaze-ci-1`), its public key, its
  role (signs project and unit heads of this tier), not-before and not-after times, and a turn
  number. Signed by the root key. Clients fetch it at daemon start (it is small and cached for a
  minute at the edge) and remember its highest turn.
- **CI signing key** `blaze-ci-N`: an Ed25519 secret in a GitHub environment, `cache-writer`,
  that only workflows on `refs/heads/main` can enter. The R2 write token sits in the same
  environment. Today `R2_ACCESS_KEY_ID`, `R2_SECRET_ACCESS_KEY` and `R2_ENDPOINT` are repository
  secrets, which every workflow run on a branch of this repository can read (not a fork's); slice 1
  moves them into `cache-writer` before any workflow uses them. The job that signs and uploads is a separate job from the build: it receives the
  build's new blobs and entries as an artifact, so build scripts, proc macros and tests (which run
  sandboxed in the build job, [cargo.md](cargo.md#every-test-runs-in-a-sandbox)) never run in a
  process that can read the key.

### Rotation and revocation

- **Rotation**, every few months or at will: add `blaze-ci-(N+1)` to `trust/keys` (root-signed,
  turn + 1), switch CI to it, and keep `blaze-ci-N` until its not-after date, so heads it signed
  stay valid while they are still live. This is the overlap Nix uses ([nix#12976](https://github.com/NixOS/nix/pull/12976)).
- **Revocation**: publish `trust/keys` without the key (turn + 1). A client refuses any head
  signed by a key the current list does not have, so revocation reaches each client at its next
  daemon start and the heads the key signed stop being read. CI then signs fresh heads for the
  live main commits with the new key.
- **Root compromise**: a blaze release with a new root public key. Nothing in the bucket can
  recover from it, which is why the root key is offline and signs only the key list.

### Who can write

| who | reads | writes |
|---|---|---|
| CI on main, `cache-writer` job | shared tier | shared tier: packs, segments, dictionaries, heads |
| CI on a branch of the repository, a merge-queue run | shared tier | nothing (no environment, no token) |
| CI on a pull request from a fork | shared tier | nothing: GitHub gives fork runs no secrets |
| any user | shared tier, anonymously | their own private tier, with their own keys |

### What it guarantees, and what not

- Tampering with the bucket, the CDN or the network: caught. Blobs fail their hash; entries fail
  their signature or are absent.
- A stolen R2 write token without the signing key: can delete (caught as missing objects) and
  upload unsigned junk (refused). Cannot make a client run anything.
- A replayed older head or key list: refused by its turn, as `signed.rs` does for manifests now. A
  CDN that serves a stale `latest` makes a client miss, never run the wrong code.
- A compromised main build: not caught by signatures. CI signs what main's build produced. If a
  dependency's build script escaped the sandbox and changed an output before it was hashed, the
  signed entry is wrong. Two mitigations, neither complete: the signer is not reachable from the
  build (above), and a second CI job on another runner compiles a random sample of the new entries
  of each push and compares them with what is signed (this needs per-function determinism, which
  the golden check enforces). A mismatch fails main and revokes nothing automatically.
- Users' assumption, as today ([cache.md](cache.md#trust-and-signing)): two builds of blaze with
  the same versions compile the same key to equivalent code.

### Private tiers

`RA_JIT_REMOTE` becomes a list, read in order; the shared tier is the default last entry:

    RA_JIT_REMOTE="s3://acme-blaze/cache?endpoint=...&trust=acme-ci-1:<pubkey>,shared"

Each tier has its own trust: the shared tier's is the root key compiled into blaze; a private
tier names its keys in the address (`trust=`) or in `RA_JIT_REMOTE_TRUST`. A machine writes only
to tiers it holds a signing key and credentials for. An entry in a tier names only blobs of that
same tier, so one tier's GC never breaks another's entries; a push to a private tier uploads the
blobs it lacks even when the shared tier has them. `shared` alone is the default for released
blaze builds (see the open questions on whether reading it is on by default), and `off` turns
every tier off.

## The fetch path

### What happens when the daemon starts

1. **Before the workspace loads**: fetch `trust/keys` and the project head. The project is the
   checkout's `RA_JIT_PROJECT`, else a name derived from its git remote; the head is that of the
   merge base of `HEAD` with main (`index/heads/p/<project>/<merge-base>`), else the newest head of the
   first 50 first-parent ancestors, else `latest`. Two requests at a CDN round trip each (tens of
   milliseconds, **unverified**).
2. **Segments**, all at once: tens of objects of up to 4 MB, from the edge.
3. **Units**: from the checkout's `Cargo.lock`, compute every dependency unit key and fetch those
   unit heads the project head does not already cover, so a project with no head of its own
   (any user's) still gets its dependencies.
4. **Plan**: the union of blobs the segments name, minus what the local store has, grouped by pack
   and offset, ranges closer than 1 MiB merged, split into parts of 4 to 8 MB.
5. **Fetch**: 16 parts in flight (`RA_JIT_REMOTE_JOBS`, default 16 for R2). Each part is
   decompressed frame by frame, each blob hashed against its name, and written into the local
   store. A whole pack that arrives is put in the store as a pack.
6. **Meanwhile** rust-analyzer loads the workspace (5 s for ripgrep, 37 to 52 s cold for nushell,
   **read**, [ci-runner.md](ci-runner.md)). Store lookups during the load and the first build
   find what has arrived; a key whose blob is still in flight waits on that part rather than
   compiling it (the store's existing hook, `fetch_missing_with`, used by `RA_JIT_REMOTE_LAZY`
   today).

The lazy mode stays: with `RA_JIT_REMOTE_LAZY` step 5 fetches only what a build asks for. With a
project head the full prefetch is the default, since the head names what main's build actually
used.

### How much, and how long

blaze's own workspace in the store, measured on dev-aws-14 on 2026-10-27 (**read**): the
machine's shared store held 2.35 GB in 55 packs, written by several lanes over ten hours under
several blaze identities. The packs one self-host build wrote between 03:34 and 03:39 come to about
317 MB; one earlier group (00:22 to 00:23) to about 273 MB. Of those, one pack of 203 to 229 MB
appears seven times, each time under a new identity, which is today's per-release miss made
visible. So one identity's working set for blaze is about 270 to 320 MB at zstd level 3. With a
higher level and dictionaries it may be 15 to 25% smaller (**unverified**, slice 6 measures).

Rates from R2 to a GitHub `ubuntu` runner: about 38 MiB/s on one stream, 210 to 225 MiB/s with 16
in parallel (**reported**, [cloud-cache-action](https://xsavikx.github.io/cloud-cache-action/guide/performance.html)).

| what | size | 1 stream (38 MiB/s) | 16 streams (215 MiB/s) |
|---|---|---|---|
| blaze's workspace, whole, level 3 | 300 MB | 7.5 s | 1.3 s |
| the same at a higher level, 20% smaller | 240 MB | 6.0 s | 1.1 s |
| a branch that touched one crate: what main's head lacks | 10 to 30 MB | 0.3 to 0.8 s | 0.05 to 0.15 s plus a round trip |
| nushell's function store ([ci-runner.md](ci-runner.md)) | 64 MB | 1.6 s | 0.3 s |

All **unverified** arithmetic on the figures above. The CPU on the client: 300 MB compressed is
about 1.5 GB of blobs, which hash in about 0.7 core-seconds and decompress in about 1 to 1.5
core-seconds (**unverified**), spread over the runner's cores while the bytes arrive. On a warm
developer machine almost all of it is in the local store already and the fetch is the heads and
segments, a few hundred milliseconds.

After this the fetch is not the long pole. helix's warm start spent 129 to 131 s of thread time
turning store records back into the daemon's values ([research/warm-start.md](research/warm-start.md),
**read**); that cost is the store's, and #216 is its fix.

### HTTP versions

- R2's S3 endpoint answered no HTTP/3, and HTTP/2 was disabled on it in 2023 (**reported**,
  [answeroverflow](https://www.answeroverflow.com/m/971611340068245504),
  [community](https://community.cloudflare.com/t/cloudflare-r2-http-2-error/441326)). Writes are
  HTTP/1.1, many connections.
- The custom domain is a Cloudflare zone: HTTP/2 and, with the zone's switch on, HTTP/3 between
  the client and Cloudflare (not to the origin) ([HTTP/3](https://developers.cloudflare.com/speed/optimization/protocol/http3/),
  **reported**). Reads go there.
- The 16-stream figure was measured with parallel connections. Sixteen streams on one HTTP/2
  connection share one TCP congestion window and may not reach it; the default backend opens
  several connections and slice 2 measures how many.

### Edge caching

The custom domain needs a Cache Rule ("cache everything") for `cas/` and `index/seg/` with an
edge TTL of a week: objects there never change while they exist (a week, not longer, so the edge
does not keep serving an object long after its expiry). `index/heads/` and `trust/` are cached
for 60 s.
Cloudflare serves range requests from cached files and fills them with 1 MiB-aligned origin
ranges ([range requests](https://developers.cloudflare.com/cache/reference/range-requests/),
**reported**). The bucket's `r2.dev` address is rate-limited to hundreds of requests a second and
is never used ([limits](https://developers.cloudflare.com/r2/reference/limits/), **reported**).

## The write path

Only the `cache-writer` job on main. After the build and test job of a main commit passes the
golden check and finishes:

1. **Refresh** what the head will name that is old: every segment it reuses that was uploaded
   before today, and every pack (and the dictionaries its frames name) holding a blob the build
   used that was uploaded more than 45 days ago, is copied onto itself (a server-side
   `CopyObject` to the same name), which sets its upload date to now (**reported**, coordinator).
   A copy moves no bytes through the runner and costs one write.
2. **Collect** what the build wrote to its store that the shared tier lacks: the build job lists
   the keys it wrote, and the latest head's segments say what the tier already has.
3. **Pack**: compress each new blob at the upload level, with the dictionary for its kind, into
   packs of up to 64 MB, ordered by crate and walk order. Large files are chunked first.
4. **Upload packs and dictionaries** with `If-None-Match: *`. R2 evaluates it atomically
   ([consistency](https://developers.cloudflare.com/r2/reference/consistency/), **reported**); a
   412 means another writer put the same bytes there, which is success.
5. **Upload segments** the same way: entries for the new blobs, and for the project head a full
   list of segments its build used (old segments reused by hash).
6. **Sign and upload heads**: `index/heads/p/<project>/<commit>` with `If-None-Match: *`, every
   touched `index/heads/u/<unit-key>`, then `index/heads/p/<project>/latest` with `If-Match` on the
   ETag read before,
   retried a few times when another writer moved it, as `extend_list` does today.
7. **Test passes and flakes** of the commit's test run go into a segment of the same head.

Readers never see a head naming an object not yet written, because objects go up before the
heads that name them, and anything old a head names is refreshed before the head goes up. A
job killed midway leaves unnamed objects, which expire with their prefix. A push of a main commit that fails fails main's train run: a broken writer is seen
the same day.

Writes cost little: R2 bills $4.50 per million writes and $0.015 per GB-month, egress free
([pricing](https://developers.cloudflare.com/r2/pricing/), **reported**). A main commit is tens of
writes; a 100 GB tier is $1.50 a month.

## Failure modes

The rule ([AGENTS.md](../AGENTS.md), no-fallback): no quiet other path, and a clear error where
blaze cannot do what it was asked. A remote is a cache: with or without it, a build produces the
same output, so not reaching it changes speed, not what is built. A remote that answers wrongly
is a different matter: that is a bug or an attack, and must be seen and fixed.

| failure | what blaze does |
|---|---|
| a tier cannot be reached (DNS, connect, timeout, 5xx after retries) | locally: the build goes on without that tier, and **every** build's `Finished` line says so (`[ra-jit, remote down: cache.<domain>: timed out]`), not one line in a log. With `RA_JIT_REMOTE_REQUIRED=1`, which blaze's own CI sets, it is an error and the job fails |
| a blob that does not hash to its name | error, naming the object, the tier and the head; after one retry that bypasses the edge cache. Today the build compiles those functions instead; that changes to an error |
| a head or key list with a bad signature, an unknown or revoked key, or an older turn | error, naming the head and the key |
| a head names an object the tier does not have (404) | error: by the invariant of [GC](#gc) a blob outlives every entry that names it, so this is a bug |
| no head for this project, or none for this codegen version | not an error: the tier has nothing for this build yet. The `Finished` line says `remote: nothing for codegen 41` |
| the local disk fills while fetching | as today: `store: nothing more is kept of this build`, and the build goes on |
| the write job fails | main's train run fails, with the step and the request |

Each error says how to go on: `RA_JIT_REMOTE=off` for this command, or the tier to drop from the
list.

Why not stop on an unreachable tier by default too: a developer on a train with the shared tier
configured would have every build fail for a cache. The status on every build makes it
impossible to miss. CI, where a cache that silently stopped working is a regression nobody sees,
treats it as an error. This is the one choice here Andrew may want the other way (open questions).

## GC

There is no GC job. R2's lifecycle rules expire an object by its age since upload, not since its
last read, so the design makes upload age mean "last used by main":

| prefix | expires after | what keeps it |
|---|---|---|
| `index/` (heads and segments) | about 30 days | CI on main writes a new head for each commit, and refreshes the segments it reuses |
| `cas/` (packs and dictionaries) | about 90 days | CI on main copies onto itself any pack it used that is older than about 45 days |
| `trust/` | never | written by hand with the root key |

**The invariant: a blob always outlives every index entry that points to it.** An index object
written at time `t` expires at about `t + 30 days`. Every pack it names was refreshed if it was
older than 45 days at `t`, so it expires no earlier than `t + 45 days`. Every segment a head names
is new or was refreshed on the day the head was written, so it expires with the head or after it.
The margins (45 against 30 days, a day for segments) absorb the lifecycle's day granularity and a
write job that runs for hours. Readers check nothing of it: a 404 for a named object is a bug and
an error ([Failure modes](#failure-modes)).

What follows from it:

- A head nobody writes again (a project not built on main for a month, an old commit) disappears
  after 30 days, and then a branch based on that commit misses. The merge-base lookup goes back 50
  commits, so this matters only for branches over a month old.
- A pack main stopped using expires 90 days after its last refresh. A pack with one used blob and
  many dead ones is refreshed whole; the write job writes a new pack of the used blobs instead when
  under half of a pack's blobs are used, and names that one.
- Unit heads are rewritten when CI touches the unit; one untouched for 30 days expires, and a
  user's next build of that dependency misses until CI builds it again.
- Released blaze versions' std projects are heads like any other: kept while the release train
  writes them, expired 30 days after the last write.
- Cost of the refreshes: a copy per old pack and per reused segment per main commit, tens of
  writes at $4.50 a million.

The rules are added with `wrangler r2 bucket lifecycle add blaze-cache ...` (one per prefix, by
age) once the layout is final, which is the last step of slice 4; until then nothing expires but the
incomplete multipart uploads. A private tier on R2 uses the same rules; one on S3 the same with S3
lifecycle rules; a directory remote keeps today's `ra-jit cache gc --remote`.

## Crate layout

Sans-IO throughout: every crate below `blaze-remote-io` takes bytes and events and returns bytes
and requests, with no sockets, files or clocks of its own. Each can be tested, fuzzed and
simulated with a fake transport that drops, reorders, truncates and corrupts.

| crate | what it is | depends on |
|---|---|---|
| `blaze-semver` | the version constants, nothing else | none |
| `blaze-cas` | `Digest` (SHA-256), streaming verification (`Verifier::push(&[u8])`, `finish() -> Result`), chunk lists | `sha2` |
| `blaze-cdc` | VectorCDC AE: scalar reference, AVX-512, AVX2, NEON; a streaming chunker that is fed bytes and yields cut points | none. Ported from ix's `crates/storage/cdc` (below) |
| `blaze-pack` | the `.bpk` format: writer and reader over byte slices, the frame index, dictionary references, the range planner (wanted blobs to coalesced byte ranges) | `blaze-cas`, `zstd` |
| `blaze-index` | entry kinds, segments (deterministic CBOR), heads, `trust/keys`, Ed25519 signing and checking, turns | `blaze-cas`, `ed25519-dalek`, a CBOR crate |
| `blaze-remote-proto` | the protocol as state machines: `Fetch` (trust, head, segments, plan, parts), `Push` (collect, pack, conditional uploads in order), `Refresh` (which old packs and segments a head needs copied onto themselves). Each emits `Request { op: Get{range} \| Put{if_none_match \| if_match} \| Copy{onto_itself} \| List, name }` and consumes `Response` events. SigV4 signing of S3 requests is sans-IO too (`rusty-s3`) | the four above, `rusty-s3` |
| `blaze-remote-io` | `trait Transport { fn send(&self, Request) -> impl Future<Output = Response> }` with body streaming, plus the backends below | `blaze-remote-proto` |
| `ra-jit` (`remote.rs`, `keeper.rs`) | drives a `Fetch` at daemon start and a `Push` when told; owns policy (tiers, env vars, status lines) | `blaze-remote-io` |
| `ra-jit-store` | reads and writes format 4 packs through `blaze-pack` | `blaze-pack` |

Backends of `blaze-remote-io`:

- **tokio, HTTP/1.1 and HTTP/2** (hyper 1 and rustls): the default everywhere.
- **directory**: a mounted path, for tests and for today's `Dir` remote.
- **io_uring on Linux**, with compio (active; monoio's and tokio-uring's last releases are from
  2024, **reported**). Only for the bulk part reads into the store, behind a measurement in slice
  8 showing it beats tokio on a GitHub runner. The daemon is not inside the test sandbox, whose
  seccomp filter refuses `io_uring_setup` ([cargo.md](cargo.md#every-test-runs-in-a-sandbox)), so
  nothing stops it there.
- **HTTP/3**, for reads from the custom domain, with tokio-quiche (open-sourced 2025-11, now
  0.20, over quiche 0.30, **reported**). Its `ClientH3Driver` is a building block, not an HTTP
  client, so this is real work; worth it only if slice 8 shows it helps on lossy or long links
  (users far from a point of presence), since a datacenter runner is unlikely to gain.

Crates to reuse, and not:

- **Reuse**: `sha2` 0.11; `zstd` 0.14 with `zdict_builder`; `ed25519-dalek` 3 (already used by
  `signed.rs`); `rusty-s3` 0.10 (sans-IO S3 request signing, which fits the design better than the
  AWS SDK); hyper and rustls; compio and tokio-quiche behind their slices.
- **VectorCDC**: no crate on crates.io (**reported**). ix has one, `crates/storage/cdc` in
  indexable-inc/ix: AE with VectorCDC's SIMD, AVX-512, AVX2, NEON and a scalar reference that the
  tests compare against, defaults 16/64/256 KiB, no dependencies beyond `bytes` and `parking_lot`
  (**read**). Port it as `blaze-cdc` rather than depend on the ix workspace.
- **Not `fastcdc`**: a good crate (5.0, used by Attic and REAPI's reference), but the decision is
  VectorCDC, and AE needs no table. **Not `gearhash`**: VectorCDC is not a gear hash.
- **Not `aws-sdk-s3`** in the new crates: it does its own IO and cannot sit behind the transport
  trait. Today's `s3.rs` keeps it until slice 3 replaces it.
- **Not `object_store`**: it does its own IO as well.

## Rollout

Ranked by what each gives. Each slice lands on its own and is measured.

1. **R2 behind today's remote.** The bucket (made), a custom domain with the cache rules above,
   the secrets moved into the `cache-writer` environment, CI on main pushing to
   `s3://blaze-cache/v3` (today's layout, under a prefix of its own) with today's `ra-jit cache push` (the AWS SDK with R2's endpoint and `region=auto`,
   which sccache documents) and every CI run reading over `https://`. The `https` place learns
   real paths beside its release-asset names. First run of a real object store; measures the
   single and 16-stream rates on our own runners. No format change, and no expiry: `v3/` is
   removed by hand once slice 4 has replaced it.
2. **The codegen version.** `blaze-semver`, `Identity` without `build`, the golden check and the
   determinism check in the merge gate. Removes the per-release miss. Measures the corpus's keys
   against nushell's.
3. **Sans-IO crates and the new packs.** `blaze-cas`, `blaze-pack`, `blaze-remote-proto`,
   `blaze-remote-io` with the tokio backend; store format 4; ranged parallel reads; the prefetch
   at daemon start. Measures fetch time against slice 1.
4. **The signed index.** Segments, project and unit heads, `trust/keys`, the root and CI keys;
   manifests and key lists retired. Unit heads let other projects use what blaze's CI built. The
   write job's refresh step, then the lifecycle rules for `index/` and `cas/` with `wrangler r2
   bucket lifecycle add`.
5. **Large files.** `blaze-cdc` from ix; dependency units and the toolchain as chunk lists;
   measures dedup across consecutive main commits.
6. **Compression.** The upload level and the dictionaries, measured against level 3 in 128 KB
   blocks, on blaze's workspace and nushell.
7. **Test passes.** Slice R2 of [ci-runner.md](ci-runner.md) writes its records into this index;
   flake entries; `RA_JIT_TESTS=affected` reads them, so a branch skips what main passed.
8. **Backends.** io_uring and HTTP/3, each kept only if it measures faster where it is meant to.
9. **Private tiers.** The tier list and per-tier trust.

The refresh (slice 4) must ship before the lifecycle rules are added (its last step), or the
first rule would expire what a head names.

## Open questions

- **Is reading the shared tier on by default** for released blaze builds? On by default is what
  makes everyone benefit; it also tells Cloudflare and us which dependency units a machine
  builds (the unit keys it asks for). Proposed: on, with `RA_JIT_REMOTE=off`, and said in the
  install docs.
- **Should an unreachable tier stop a local build**, as it stops CI? Proposed: no, with the status
  on every `Finished` line; the no-fallback rule could be read either way.
- **Per-head or per-entry signatures.** The design signs each head, which covers every entry once.
  Per-entry signatures would let one entry be served alone, at 64 bytes and one verification each.
  Is per-head what you meant by "each entry signed"?
- **The cache's domain**: which name, and on which Cloudflare account (`cache.<domain>` above).
- **Who holds the root key**, and where (a hardware key, a printed backup).
- **Dev builds and the version**: should a blaze built from a dirty tree salt its local keys with
  a hash of the codegen crates, so a developer's codegen experiment never lands in their own
  store under the released version's keys?
- **Sampled rebuilds on clients**: should a client be able to recompile a fraction of what it
  fetched and compare (`RA_JIT_REMOTE_VERIFY=sample`), for users who trust CI less than we do?
  Needs per-function determinism, which the golden check gives.
- **Windows and macOS**: CI on main runs Linux x64 and arm64 and macOS arm64 (`train.yaml`,
  **read**). Each target is its own set of heads; is macOS arm64 filled from the start?

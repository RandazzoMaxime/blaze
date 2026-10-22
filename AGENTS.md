# ra-jit

How to build, test and change ra-jit, including the vendored rust-analyzer, is
[docs/contributing.md](docs/contributing.md); what the harnesses check is
[docs/harness.md](docs/harness.md).

- **Language semantics go in the rust-analyzer subtree.** MIR building and passes, borrow
  checking, const eval, layout and code-identity hashes as salsa queries (what rustc keeps in
  `rustc_middle`, `rustc_mir_transform`, `rustc_borrowck`) go in `vendor/rust-analyzer`, as a
  new crate there rather than edits to upstream's files where possible; codegen backends,
  object writing, the splice, the daemon, cargo integration, the store, tiers, the live runtime
  and the harnesses stay in `crates/`: [docs/contributing.md](docs/contributing.md#where-new-code-goes).
- **Heavy work runs on a dev host, the Mac does quick checks.** Release builds, harness sweeps
  and benchmarks run on an AWS dev host claimed in the host ledger (`fleet-ops` skill), with a
  lease as long as the run, released after: [docs/contributing.md](docs/contributing.md#working-in-parallel).
- **One worktree per lane, never `git stash`.** A uniquely named worktree with its own
  `CARGO_TARGET_DIR` or a shared sccache; the stash is shared by every worktree; harnesses run
  with `RUST_BACKTRACE=0`.
- **Merge small and often** once the pull request's fast tier is green (about 10 minutes); the
  full tier runs nightly or by hand: [docs/harness.md](docs/harness.md#ci).
- **Delete what a lane made when it is done:** its worktree, target directory and dev-host
  directories, and its merged branch.
- **Credit people.** A commit or pull request built on someone's suggestion, diagnosis,
  workaround or patch from an issue, review or discussion gets a
  `Co-authored-by: Name <id+login@users.noreply.github.com>` trailer for each of them, and names
  where it came from: [docs/contributing.md](docs/contributing.md#credit).

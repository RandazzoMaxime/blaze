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
- **Credit people.** A commit or pull request built on someone's suggestion, diagnosis,
  workaround or patch from an issue, review or discussion gets a
  `Co-authored-by: Name <id+login@users.noreply.github.com>` trailer for each of them, and names
  where it came from: [docs/contributing.md](docs/contributing.md#credit).

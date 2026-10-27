# `cargo blaze doc`

`cargo blaze doc` (and `blaze doc`) writes the HTML documentation of the packages asked for and
of what they depend on, as `cargo doc` does, with no rustdoc: blaze writes every page from
rust-analyzer's HIR (`crates/blaze-doc`). The pages are not rustdoc's byte for byte; they hold
what rustdoc's hold, look much like them, and work the same way (search, sidebar, themes,
source pages).

```sh
cargo blaze doc --open            # the package and its dependencies, then open its page
cargo blaze doc --no-deps -p foo  # one package, nothing it depends on
```

## What it writes

`target/doc/<crate>/index.html` for each crate, or `target/<triple>/doc/...` with `--target`,
and beside them:

- a page for every item rustdoc gives one (modules, structs, enums, unions, traits, trait
  aliases, functions, type aliases, constants, statics, macros, derive and attribute macros,
  primitives a crate documents), at rustdoc's file names, so a link into rustdoc's docs works
  into blaze's;
- on each: the declaration with every name linked to its page, fields and variants, inherent
  impls, trait impls (written, blanket and auto ones), "Methods from `Deref<Target = ..>`",
  implementors of a trait (from every crate documented, as rustdoc's script adds them), the
  members an impl leaves as the trait provides them, and notes for `#[deprecated]`,
  `#[must_use]`, `unsafe` and `#[doc(cfg)]`; a type alias shows the type it stands for with
  its fields or variants ("Aliased Type");
- re-exports inlined or listed as rustdoc decides (`#[doc(inline)]`, `#[doc(no_inline)]`,
  `#[doc(hidden)]`, globs, private modules, a chain through another public `pub use`);
- docs as rustdoc renders Markdown: pulldown-cmark with rustdoc's options, headings with
  anchors, the first paragraph as the summary in lists and search, Rust examples highlighted
  with their `# ` lines hidden, and intra-doc links resolved by rust-analyzer; a link that
  resolves to nothing prints `warning: unresolved link to `...`` as rustdoc does;
- a source page for every file, linked from each item's `Source` link;
- an "All items" page per crate, the module list in the sidebar, a search box (`s` or `/`)
  over every crate documented, light and dark themes following the system's.

It prints `Documenting`, `Finished` and `Generated` as cargo does; `-v` says how long the load,
the model and the writing took.

The flags are cargo's: `-p`, `--workspace`, `--exclude`, `--no-deps`,
`--document-private-items`, `--open`, `--features`, `--all-features`, `--no-default-features`,
`--target`, `--target-dir`, `--manifest-path`, `--lib`, `--bins`, `--bin`, `--release`,
`--profile`, `-q`, `-v`. A binary is documented with its private items, as cargo does. Two
dependencies of one name share a directory; as cargo's `remove_duplicate_doc` does, the newest
version is documented, and none that shares a name with a package asked for.

## How

Three passes over one loaded database, each parallel over the crates:

1. `places` walks every module tree as rustdoc does and gives each item its page: where it is
   declared, or where a public re-export inlines it, under the crate that places it.
2. `collect` builds the model of each crate (`model.rs`): declarations as code with links,
   docs with links resolved, members, impls. Nothing in the model is HTML.
3. `html` writes the pages, source pages, sidebars and search index from the model alone.

Which provided methods an impl shows follows rustc: a method whose where-clauses on the
trait's parameters cannot hold for the impl (`rev` of an iterator that is not a
`DoubleEndedIterator`) is left out, by a port of rustc's `is_impossible_associated_item` into
rust-analyzer's solver (`hir_ty::traits::is_impossible_associated_item`). Blanket impls apply
as rustdoc's `synthesize_blanket_impls` asks (`hir_ty::traits::impl_may_apply`).

### The load is one-shot, in the command's process

`cargo blaze doc` loads the workspace in its own process, as the daemon's checker loads it
(`crate::load` in `crates/ra-jit`, from `crates/ra-jit/src/cargo/own/doc.rs`), writes the pages
and exits. It does not ask the daemon: a database for documentation needs every dependency's
items and bodies' signatures, with `cfg(doc)` set, and a daemon would hold that only for `doc`
to read once; the one-shot load needs no daemon running and leaves nothing behind.

### What still runs cargo

The pages are blaze's, and no rustdoc or rustc runs to write them. The load the command shares
with the checker still runs cargo in two places, and this command adds no new one:

- build scripts and proc macros: the load builds and runs them through cargo
  (`ProjectWorkspace::run_build_scripts`) so a crate that `include!`s its `OUT_DIR` or derives
  through a proc macro reads as it compiles. That is most of a first run's wall time.
- the sysroot: unless the toolchain is known, rust-analyzer asks `rustc --print sysroot` and
  cargo where the standard library's source is (`RustLibSource::Discover`).

Both go when the load stops needing them ([no-rustc.md](no-rustc.md)).

## Differences from rustdoc

- Smart punctuation is on, as in rustdoc's `main_body_opts` (`--` becomes an en dash, quotes
  curl); the brief asked for it off.
- A link to an item another crate places points straight at that crate's page. rustdoc points
  at the path the item is defined at and writes a redirect page there; blaze writes no
  redirect pages, and names the defined path in the link's title as rustdoc does.
- Implementors from crates downstream of a trait's crate are written into the trait's page,
  where rustdoc adds them with a script (`implementors/*.js`); a reader sees the same list.
- The stylesheet and script are blaze's own (`crates/blaze-doc/static`), with rustdoc's class
  names and colors and system fonts; the theme follows the system, with no settings page.

## How complete it is

`harness/docdiff.py <workspace>` runs the pinned nightly's `cargo doc` and `cargo blaze doc` with
the same flags and compares, for each crate both documented: the item pages, the members and
impls anchored on pages both have (`method.new`, `impl-Debug-for-Foo`), the intra-doc links
rustdoc resolved (by their titles), and the `unresolved link` warnings. On the bench apps (each
with its dependencies; 2026-10-26, dev-aws-8):

RESULTS

What blaze still misses or adds, and why:

GAPS

## Not done

- Doctests are `cargo blaze test --doc`'s, not this command's.
- `--message-format json`, `--examples`, `--tests` and `--benches` are not read.
- No settings page, no `help.html`, no per-item "Copy item path" menu, no
  `--extern-html-root-url`: links into the standard library point at doc.rust-lang.org.

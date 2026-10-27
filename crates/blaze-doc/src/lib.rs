//! HTML documentation written from rust-analyzer's HIR, the way `cargo doc` and rustdoc write
//! it, with no rustdoc.
//!
//! Three passes over a loaded database, each parallel over the crates: [`places`] walks every
//! module tree as rustdoc does and gives each item its page; [`collect`] builds the [`model`] of
//! each crate (declarations, docs with intra-doc links resolved, members, impls); [`html`]
//! writes the pages, source pages, sidebars and search index from the model alone.

mod attrs;
mod cfg;
mod collect;
mod highlight;
pub mod html;
mod links;
mod markdown;
pub mod model;
mod places;
mod sig;
mod source;
#[cfg(test)]
mod tests;

use hir::Crate;
use ide_db::RootDatabase;
use rustc_hash::FxHashSet;

pub use crate::places::Documented;

/// A crate the user asked to document.
#[derive(Debug, Clone, Copy)]
pub struct Asked {
    pub krate: Crate,
    /// Document items that are not public too: `--document-private-items`, or a binary.
    pub private: bool,
}

/// The crates `cargo doc` documents for `asked`: those, and unless `no_deps` every library
/// they depend on, directly or not, but none of the sysroot's. Two crates of one name keep the
/// one asked for, else the newest version.
pub fn crates_to_document(db: &RootDatabase, asked: &[Asked], no_deps: bool) -> Vec<Documented> {
    let mut out: Vec<Documented> = vec![];
    let mut names = FxHashSet::default();
    let mut seen = FxHashSet::default();
    for it in asked {
        let name = crate_name(db, it.krate);
        if names.insert(name.clone()) {
            seen.insert(it.krate);
            out.push(Documented { krate: it.krate, name, private: it.private, warn: true });
        }
    }
    if no_deps {
        return out;
    }
    // Two dependencies sharing a name share a directory too: cargo documents the newest
    // (`remove_duplicate_doc`), and none that shares a name with a crate asked for.
    let mut deps: Vec<(String, Crate)> = vec![];
    let mut stack: Vec<Crate> = asked.iter().map(|it| it.krate).collect();
    while let Some(krate) = stack.pop() {
        for dep in krate.dependencies(db) {
            let dep = dep.krate;
            if !seen.insert(dep) || is_sysroot(db, dep) {
                continue;
            }
            let name = crate_name(db, dep);
            if !names.contains(&name) {
                match deps.iter_mut().find(|(it, _)| *it == name) {
                    Some(kept) if version_key(db, dep) > version_key(db, kept.1) => kept.1 = dep,
                    Some(_) => {}
                    None => deps.push((name, dep)),
                }
            }
            stack.push(dep);
        }
    }
    out.extend(deps.into_iter().map(|(name, krate)| Documented {
        krate,
        name,
        private: false,
        warn: false,
    }));
    out
}

/// A crate's version as semver orders it, near enough: the numbers, then a release above
/// any of its pre-releases.
fn version_key(db: &RootDatabase, krate: Crate) -> (Vec<u64>, bool, String) {
    let version = krate.version(db).unwrap_or_default();
    let version = version.split('+').next().unwrap_or_default();
    let (release, pre) = version.split_once('-').unwrap_or((version, ""));
    let numbers = release.split('.').map(|it| it.parse().unwrap_or(0)).collect();
    (numbers, pre.is_empty(), pre.to_owned())
}

/// The name code uses for a crate: its library name, `-` as `_`.
pub fn crate_name(db: &RootDatabase, krate: Crate) -> String {
    krate
        .display_name(db)
        .map(|it| it.crate_name().to_string())
        .unwrap_or_else(|| "unknown".to_owned())
}

fn is_sysroot(db: &RootDatabase, krate: Crate) -> bool {
    matches!(krate.origin(db), ide_db::base_db::CrateOrigin::Lang(_))
}

/// Sets `cfg(doc)` on the crates documented, as rustdoc does for the crate it documents.
pub fn set_cfg_doc(db: &mut RootDatabase, crates: &[Documented]) {
    use ide_db::base_db::salsa::Setter as _;
    for documented in crates {
        let krate: ide_db::base_db::Crate = documented.krate.into();
        let mut cfg = krate.cfg_options(db).clone();
        cfg.insert_atom(intern::Symbol::intern("doc"));
        krate.set_cfg_options(db).to(cfg);
    }
}

/// The documentation of `crates`, ready to write.
pub fn model(db: &RootDatabase, crates: Vec<Documented>) -> model::Model {
    let places = places::place(db, crates);
    collect::build(db, &places)
}

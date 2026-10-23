//! Frozen dependency artifacts (`docs/glancer.md`, item 1; #40): the def map of each crate
//! whose source cannot change, written to the store once and read back instead of resolving
//! it again.
//!
//! A crate is frozen when its source is the toolchain's (the standard library) or a
//! registry's or git checkout's, and every crate it depends on is frozen too. Its key is a
//! Merkle hash: where the source is below the toolchain, registry or checkout, its edition,
//! kind, crate attributes, cfg, environment, and the names and keys of its dependencies; the store adds the toolchain
//! and ra-jit's own build.
//!
//! `RA_JIT_FROZEN` says what is done with what the store has: by default it is served;
//! `check` resolves as well and fails the build on any difference, `verify` writes what was
//! read and fails on any difference, `off` neither reads nor writes.

use ide_db::base_db::Crate;
use rustc_hash::FxHashMap;
use sha2::{Digest, Sha256};

use hir_def::nameres::frozen::{self as hir_frozen, Frozen, Mode};

/// What a build did with the frozen def maps of the crates it resolved.
#[derive(Default)]
pub(crate) struct Served {
    /// The key of each crate that can be frozen.
    keys: FxHashMap<Crate, [u8; 32]>,
    /// The crates whose def map came from the store.
    served: Vec<Crate>,
    /// The crates whose def map the store has but which read a file that is not loaded.
    unreadable: usize,
    mode: Option<Mode>,
}

fn mode() -> Option<Mode> {
    match std::env::var("RA_JIT_FROZEN").as_deref() {
        Ok("off" | "0") => None,
        Ok("check") => Some(Mode::Check),
        Ok("verify") => Some(Mode::Verify),
        _ => Some(Mode::Serve),
    }
}

/// Where a crate's source is when it is one that cannot change: the part of the path from
/// the toolchain's source, the registry's or a git checkout's on.
fn immutable(path: &str) -> Option<&str> {
    ["/lib/rustlib/src/rust/", "/registry/src/", "/git/checkouts/"]
        .iter()
        .find_map(|marker| path.find(marker).map(|at| &path[at + marker.len()..]))
}

/// The key of `krate`, when it and every crate it depends on cannot change.
fn key(
    db: &ide_db::RootDatabase,
    krate: Crate,
    keys: &mut FxHashMap<Crate, Option<[u8; 32]>>,
) -> Option<[u8; 32]> {
    if let Some(&it) = keys.get(&krate) {
        return it;
    }
    // A cycle, which only dev-dependencies make, is not frozen.
    keys.insert(krate, None);
    let found = key_of(db, krate, keys);
    keys.insert(krate, found);
    found
}

fn key_of(
    db: &ide_db::RootDatabase,
    krate: Crate,
    keys: &mut FxHashMap<Crate, Option<[u8; 32]>>,
) -> Option<[u8; 32]> {
    let root = crate::crate_root_file(db, krate.into())?;
    let root = root.to_str()?;
    let source = immutable(root)?;
    // The directory the crate's paths are under, which its environment names.
    let package = &root[..root.len() - source.len()];
    let data = krate.data(db);
    let mut hasher = Sha256::new();
    let mut field = |bytes: &[u8]| {
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
    };
    field(b"ra-jit frozen def map 1");
    field(source.as_bytes());
    field(format!("{:?} {:?} {:?}", data.edition, data.target_kind, data.crate_attrs).as_bytes());
    let mut cfgs: Vec<String> = krate.cfg_options(db).into_iter().map(|it| it.to_string()).collect();
    cfgs.sort_unstable();
    field(cfgs.join("\n").as_bytes());
    let mut env: Vec<String> = krate
        .env(db)
        .iter()
        .map(|(name, value)| format!("{name}={}", value.replace(package, "<source>/")))
        .collect();
    env.sort_unstable();
    field(env.join("\n").as_bytes());
    let deps: Vec<_> = data.dependencies.iter().map(|it| (it.crate_id, it.name.to_string())).collect();
    for (dep, name) in deps {
        let dep_key = key(db, dep, keys)?;
        field(name.as_bytes());
        field(&dep_key);
    }
    Some(hasher.finalize().into())
}

/// Has name resolution read the def maps the store has of `crates` and their dependencies.
pub(crate) fn serve(db: &mut ide_db::RootDatabase, crates: &[Crate]) -> Served {
    let Some(mode) = mode() else { return Served::default() };
    let mut all: FxHashMap<Crate, Option<[u8; 32]>> = FxHashMap::default();
    for &krate in crates {
        for dep in krate.transitive_deps(db) {
            key(db, dep, &mut all);
        }
    }
    let keys: FxHashMap<Crate, [u8; 32]> =
        all.into_iter().filter_map(|(krate, key)| Some((krate, key?))).collect();
    let mut served = Served { keys, mode: Some(mode), ..Served::default() };
    let Some(store) = ra_jit_store::Store::global() else { return served };
    let mut maps = FxHashMap::default();
    for (&krate, key) in &served.keys {
        let Some(bytes) = store.get(key) else { continue };
        if hir_frozen::unreadable(db, krate, &bytes).is_some() {
            served.unreadable += 1;
            continue;
        }
        maps.insert(krate, Frozen { bytes: bytes.into(), mode });
        served.served.push(krate);
    }
    hir_frozen::set(db, maps);
    served
}

/// Writes to the store the def maps of the crates that can be frozen and were resolved, and
/// fails on a difference between a frozen def map and a live one.
pub(crate) fn store(db: &ide_db::RootDatabase, served: &Served) -> anyhow::Result<()> {
    let differences = hir_frozen::differences();
    if !differences.is_empty() {
        anyhow::bail!("frozen def maps differ:\n{}", differences.join("\n"));
    }
    let Some(store) = served.mode.and(ra_jit_store::Store::global()) else { return Ok(()) };
    let mut stored = 0;
    let mut refused: FxHashMap<String, usize> = FxHashMap::default();
    for (&krate, key) in &served.keys {
        if served.served.contains(&krate) || store.has(key) {
            continue;
        }
        match hir::attach_db(db, || hir_frozen::encode(db, krate)) {
            Ok(bytes) => {
                if stored == 0 {
                    store.settled();
                }
                store.insert(key, &bytes);
                stored += 1;
            }
            Err(why) => *refused.entry(why).or_default() += 1,
        }
    }
    if stored > 0 {
        store.flush();
    }
    let mut said = format!(
        "frozen def maps: {} of {} crates from the store, {stored} written",
        served.served.len(),
        served.keys.len()
    );
    if served.unreadable > 0 {
        said += &format!(", {} naming a file not loaded", served.unreadable);
    }
    if !refused.is_empty() {
        let mut refused: Vec<_> = refused.into_iter().collect();
        refused.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let why: Vec<String> = refused.iter().map(|(why, n)| format!("{n} with {why}")).collect();
        said += &format!(", not frozen: {}", why.join(", "));
    }
    eprintln!("{said}");
    Ok(())
}

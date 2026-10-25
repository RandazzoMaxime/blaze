//! Fetches every registry package of a `Cargo.lock` into a `CARGO_HOME`, as `cargo fetch` would
//! with that lock file, for `harness/fetch.py` to compare with cargo's.
//!
//!     cargo run -p blaze-registry --example fetch -- <Cargo.lock> <CARGO_HOME> [--store <dir>] [--offline]

use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

use anyhow::{Context as _, bail};
use blaze_registry::{Context, Package, Registry, SourceId, download::From, http::HyperHttp};
use rustc_hash::FxHashMap;
use semver::Version;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let (Some(lock), Some(home)) = (args.next(), args.next()) else {
        bail!("usage: fetch <Cargo.lock> <CARGO_HOME> [--store <dir>] [--offline]");
    };
    let mut context = Context::new(PathBuf::from(home), HyperHttp::shared()?);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--offline" => context.net.offline = true,
            "--store" => {
                let dir = PathBuf::from(args.next().context("--store needs a directory")?);
                ra_jit_store::compiles_against("blaze-registry");
                let store = ra_jit_store::Store::global_or(Some(&dir)).context("the store")?;
                context.store = Some(Arc::new(StoreRef(store)));
            }
            other => bail!("unknown argument `{other}`"),
        }
    }
    let context = Arc::new(context);
    let text = std::fs::read_to_string(&lock).with_context(|| format!("reading {lock}"))?;
    let lock: toml::Table = toml::from_str(&text)?;
    let metadata = lock.get("metadata").and_then(|it| it.as_table());
    let mut by_source: FxHashMap<String, Vec<(Package, bool)>> = FxHashMap::default();
    for package in lock.get("package").and_then(|it| it.as_array()).into_iter().flatten() {
        let field = |name: &str| package.get(name).and_then(|it| it.as_str());
        let (Some(name), Some(version)) = (field("name"), field("version")) else { continue };
        let Some(source) = field("source") else { continue };
        if !source.starts_with("registry+") && !source.starts_with("sparse+") {
            continue;
        }
        let v1 = format!("checksum {name} {version} ({source})");
        let checksum = field("checksum")
            .or_else(|| metadata.and_then(|it| it.get(&v1)).and_then(|it| it.as_str()));
        let package = Package {
            name: name.to_owned(),
            version: Version::parse(version)?,
            checksum: checksum.unwrap_or_default().to_owned(),
        };
        by_source.entry(source.to_owned()).or_default().push((package, checksum.is_some()));
    }
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for (source, packages) in by_source {
        let (id, _) = SourceId::from_url(&source)?;
        // cargo's own `[source.crates-io]`: the git index's packages come from the sparse one.
        let id = if id.is_crates_io() { SourceId::crates_io() } else { id };
        let registry = Registry::remote(id, context.clone())?;
        let mut wanted = Vec::with_capacity(packages.len());
        for (mut package, locked) in packages {
            if !locked {
                package.checksum = registry.checksum(&package.name, &package.version)?;
            }
            wanted.push(package);
        }
        for (_, from) in registry.fetch(&wanted)? {
            let from = match from {
                From::Unpacked => "unpacked",
                From::Store => "store",
                From::CargoCache => "cargo-cache",
                From::Network => "network",
            };
            *counts.entry(from).or_default() += 1;
        }
    }
    println!("{}", serde_json::to_string(&counts)?);
    if let Some(store) = ra_jit_store::Store::global() {
        store.flush();
    }
    Ok(())
}

struct StoreRef(&'static ra_jit_store::Store);

impl blaze_registry::Blobs for StoreRef {
    fn get(&self, sha256: &[u8; 32]) -> Option<Vec<u8>> {
        self.0.get(sha256)
    }
    fn has(&self, sha256: &[u8; 32]) -> bool {
        self.0.has(sha256)
    }
    fn put(&self, sha256: &[u8; 32], bytes: &[u8]) {
        self.0.insert(sha256, bytes);
    }
}

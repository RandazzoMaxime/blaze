//! Reads the JSON of `cargo metadata --format-version 1` on stdin and prints the `resolve` that
//! blaze-resolve makes of its packages: the members resolved with their default features, as
//! `cargo metadata` resolves them, against a registry of the packages listed. `harness/
//! resolve_graph.py` compares it with cargo's.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use blaze_resolve::metadata::{MetadataPackages, resolve_graph};
use blaze_resolve::resolver::{
    self, CliFeatures, CompileKind, ResolveOpts, ResolveSettings, VersionPreferences,
    features::RequestedFeatures,
};
use blaze_resolve::{
    DepKind, Dependency, InternedString, MemRegistry, PackageId, Platform, ResolveVersion,
    SourceId, Summary,
};
use serde_json::Value;

fn str_of<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key].as_str().ok_or_else(|| anyhow!("no `{key}` in {value}"))
}

fn package_source(package: &Value) -> Result<SourceId> {
    match package["source"].as_str() {
        Some(source) => SourceId::from_url(source),
        None => SourceId::for_path(
            Path::new(str_of(package, "manifest_path")?).parent().context("a manifest's dir")?,
        ),
    }
}

fn dependency(dep: &Value) -> Result<Dependency> {
    let source = match (dep["source"].as_str(), dep["path"].as_str()) {
        (Some(source), _) => SourceId::from_url(source)?,
        (None, Some(path)) => SourceId::for_path(Path::new(path))?,
        (None, None) => SourceId::crates_io(),
    };
    let mut d = Dependency::parse(str_of(dep, "name")?, Some(str_of(dep, "req")?), source)?;
    d.set_kind(match dep["kind"].as_str() {
        None => DepKind::Normal,
        Some("dev") => DepKind::Development,
        Some("build") => DepKind::Build,
        Some(kind) => return Err(anyhow!("dependency kind `{kind}`")),
    });
    if let Some(rename) = dep["rename"].as_str() {
        d.set_explicit_name_in_toml(rename);
    }
    d.set_optional(dep["optional"].as_bool().unwrap_or(false));
    d.set_default_features(dep["uses_default_features"].as_bool().unwrap_or(true));
    let features: Vec<&str> =
        dep["features"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    d.set_features(features);
    if let Some(target) = dep["target"].as_str() {
        d.set_platform(Some(target.parse::<Platform>()?));
    }
    Ok(d)
}

struct Packages {
    lib_names: BTreeMap<PackageId, InternedString>,
}

impl MetadataPackages for Packages {
    fn package_id(&self, id: PackageId) -> PackageId {
        id
    }

    fn lib_crate_name(&self, id: PackageId) -> Option<InternedString> {
        self.lib_names.get(&id).copied()
    }

    fn platform_activated(&self, _platform: &Platform, _kind: CompileKind) -> bool {
        true
    }
}

const LIB_KINDS: &[&str] = &["lib", "rlib", "dylib", "cdylib", "staticlib", "proc-macro"];

fn main() -> Result<()> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    let metadata: Value = serde_json::from_str(&input)?;

    let mut registry = MemRegistry::new();
    let mut by_spec = BTreeMap::new();
    let mut lib_names = BTreeMap::new();
    for package in metadata["packages"].as_array().context("no packages")? {
        let id = PackageId::try_new(
            str_of(package, "name")?,
            str_of(package, "version")?,
            package_source(package)?,
        )?;
        let deps = package["dependencies"]
            .as_array()
            .into_iter()
            .flatten()
            .map(dependency)
            .collect::<Result<Vec<_>>>()?;
        let features: BTreeMap<InternedString, Vec<InternedString>> = package["features"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(name, values)| {
                let values = values
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(InternedString::new)
                    .collect();
                (InternedString::new(name), values)
            })
            .collect();
        let summary = Summary::new(id, deps, &features, package["links"].as_str(), None)?;
        let lib = package["targets"].as_array().into_iter().flatten().find(|target| {
            target["kind"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|k| k.as_str().is_some_and(|k| LIB_KINDS.contains(&k)))
        });
        if let Some(lib) = lib {
            lib_names.insert(id, InternedString::new(&str_of(lib, "name")?.replace('-', "_")));
        }
        by_spec.insert(str_of(package, "id")?.to_owned(), summary.clone());
        registry.add(blaze_resolve::IndexSummary::Candidate(summary));
    }

    let member = |spec: &Value| -> Result<Summary> {
        let spec = spec.as_str().context("a member's id")?;
        by_spec.get(spec).cloned().ok_or_else(|| anyhow!("no package {spec}"))
    };
    let members = metadata["workspace_members"]
        .as_array()
        .context("no workspace_members")?
        .iter()
        .map(member)
        .collect::<Result<Vec<_>>>()?;
    let roots: Vec<(Summary, ResolveOpts)> = members
        .iter()
        .map(|m| {
            let features = RequestedFeatures::CliFeatures(CliFeatures::new_all(false));
            (m.clone(), ResolveOpts::new(true, features))
        })
        .collect();
    let resolve = resolver::resolve(
        &roots,
        &[],
        &registry,
        &VersionPreferences::default(),
        ResolveVersion::default(),
        &ResolveSettings::default(),
    )?;

    let member_ids: Vec<PackageId> = members.iter().map(|m| m.package_id()).collect();
    let root = match &metadata["resolve"]["root"] {
        Value::Null => None,
        spec => Some(member(spec)?.package_id()),
    };
    let graph =
        resolve_graph(&resolve, &member_ids, root, &Packages { lib_names }, &[CompileKind::Host])?;
    writeln!(std::io::stdout(), "{}", serde_json::to_string(&graph)?)?;
    Ok(())
}

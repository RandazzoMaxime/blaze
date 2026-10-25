//! The profile of every unit of a unit graph cargo printed (`--unit-graph`), recomputed with
//! blaze-manifest's `Profiles`: what each unit is built for is derived from the graph's edges
//! as cargo's `unit_dependencies` derives it, and the strip and debuginfo adjustments of cargo's
//! `rebuild_unit_graph_shared` are applied over the graph's dependencies.

use std::fmt;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, anyhow, bail};
use blaze_manifest::config::CompileKind;
use blaze_manifest::diagnostic::report;
use blaze_manifest::metadata::root_manifest;
use blaze_manifest::profiles::{Profile, ProfilePackage, Profiles, UnitFor, requested_profile};
use blaze_manifest::source::SourceId;
use blaze_manifest::{Config, Context, Workspace};
use cargo_util_schemas::core::{PackageIdSpec, SourceKind};
use rustc_hash::FxHashMap;
use serde_json::Value;

/// What the profiles of `cargo <args> --unit-graph` were made from.
pub(crate) struct Output {
    pub(crate) code: u8,
    /// The profile of each unit of the graph, a JSON array.
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

/// The cargo command the graph is of, as far as profiles go.
struct Command {
    /// `build`, `check`, `test`, `bench`.
    name: String,
    requested_profile: String,
    /// Whether `--target` was given, which keeps host units apart from target units.
    target_given: bool,
    config: Vec<String>,
    manifest_path: Option<PathBuf>,
}

fn parse_args(args: &[String]) -> anyhow::Result<Command> {
    let mut args = args.iter().cloned();
    let Some(name) = args.next() else { bail!("no cargo command given") };
    let mut command = Command {
        requested_profile: String::new(),
        target_given: false,
        config: Vec::new(),
        manifest_path: None,
        name,
    };
    let mut release = false;
    let mut profile = None;
    while let Some(arg) = args.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n.to_owned(), Some(v.to_owned())),
            _ => (arg.clone(), None),
        };
        let mut value = || {
            inline
                .clone()
                .or_else(|| args.next())
                .ok_or_else(|| anyhow!("a value is required for '{flag} <VALUE>'"))
        };
        match flag.as_str() {
            "--release" | "-r" => release = true,
            "--profile" => profile = Some(value()?),
            "--target" => {
                value()?;
                command.target_given = true;
            }
            "--config" => command.config.push(value()?),
            "--manifest-path" => command.manifest_path = Some(PathBuf::from(value()?)),
            "-p" | "--package" | "--exclude" | "--features" | "-F" | "--bin" | "--example"
            | "--test" | "--bench" | "-j" | "--jobs" => {
                value()?;
            }
            _ => {}
        }
    }
    let default = match command.name.as_str() {
        "test" | "bench" => command.name.as_str(),
        _ => "dev",
    };
    command.requested_profile = match (command.name.as_str(), profile.as_deref()) {
        ("check", Some("test")) => "test".to_owned(),
        _ => requested_profile(default, release, profile.as_deref())?,
    };
    Ok(command)
}

/// `cargo <args>`'s profiles from cwd, the unit graph cargo printed for it in `graph`.
pub(crate) fn main(
    cwd: &Path,
    env: impl IntoIterator<Item = (String, String)>,
    host: &str,
    args: &[String],
    graph: &str,
) -> Output {
    let mut config = match Config::load(cwd, env, &[]) {
        Ok(config) => config,
        Err(err) => {
            return Output { code: 101, stdout: String::new(), stderr: report(&err, cwd) + "\n" };
        }
    };
    let mut stderr = String::new();
    let result = parse_args(args).and_then(|command| {
        let started = config.start(&command.name, &command.config);
        for warning in config.take_warnings() {
            let _ = writeln!(stderr, "warning: {warning}");
        }
        started?;
        let ctx = Context::new(config);
        let result = run(&ctx, host, &command, graph, &mut stderr);
        for warning in ctx.config.take_warnings() {
            let _ = writeln!(stderr, "warning: {warning}");
        }
        result
    });
    match result {
        Ok(stdout) => Output { code: 0, stdout, stderr },
        Err(err) => {
            let _ = writeln!(stderr, "{}", report(&err, cwd));
            Output { code: 101, stdout: String::new(), stderr }
        }
    }
}

fn run(
    ctx: &Context,
    host: &str,
    command: &Command,
    graph: &str,
    stderr: &mut String,
) -> anyhow::Result<String> {
    let root = root_manifest(command.manifest_path.as_deref(), ctx)?;
    let ws = Workspace::new(&root, ctx)?;
    for (path, warning) in ws.warnings() {
        let rel = path.strip_prefix(ctx.config.cwd()).unwrap_or(path);
        let _ = writeln!(stderr, "warning: {}: {warning}", rel.display());
    }
    let profiles = Profiles::new(
        ws.profiles(),
        ws.unstable_features(),
        &ctx.config,
        &command.requested_profile,
        host,
    )?;
    let graph: Value = serde_json::from_str(graph).context("cargo's unit graph")?;
    let profiles = profiles_of(&ws, &profiles, command, &graph, stderr)?;
    Ok(serde_json::to_string_pretty(&profiles)? + "\n")
}

struct Pkg {
    name: String,
    version: semver::Version,
    source_id: SourceId,
}

impl fmt::Display for Pkg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} v{}", self.name, self.version)?;
        match &self.source_id.kind {
            SourceKind::Path => write!(f, " ({})", self.source_id.url.path()),
            _ if self.source_id.url.as_str() == blaze_manifest::source::CRATES_IO_INDEX => Ok(()),
            _ => write!(f, " ({})", self.source_id.url),
        }
    }
}

impl ProfilePackage for Pkg {
    fn name(&self) -> &str {
        &self.name
    }
    fn version(&self) -> &semver::Version {
        &self.version
    }
    fn source_id(&self) -> &SourceId {
        &self.source_id
    }
}

fn package(pkg_id: &str) -> anyhow::Result<Pkg> {
    let spec = PackageIdSpec::parse(pkg_id).with_context(|| format!("package id `{pkg_id}`"))?;
    let version = spec.version().ok_or_else(|| anyhow!("package id `{pkg_id}` has no version"))?;
    let url = spec.url().ok_or_else(|| anyhow!("package id `{pkg_id}` has no source"))?;
    let kind = spec.kind().ok_or_else(|| anyhow!("package id `{pkg_id}` has no source kind"))?;
    let source_id = SourceId { kind: kind.clone(), url: url.clone(), registry_key: None };
    Ok(Pkg { name: spec.name().to_owned(), version, source_id })
}

struct Unit {
    pkg: Pkg,
    is_member: bool,
    custom_build: bool,
    proc_macro: bool,
    mode: String,
    kind: CompileKind,
    /// The unit as cargo interns it, less its profile: what a probe of cargo's compares.
    identity: String,
    deps: Vec<usize>,
}

impl Unit {
    fn for_host(&self) -> bool {
        self.custom_build || self.proc_macro
    }
    fn is_any_test(&self) -> bool {
        matches!(self.mode.as_str(), "test" | "doctest")
    }
}

/// A way cargo reached a unit: what it was built for, its profile before the graph was
/// shared, and whether its kind is the host's own (`CompileKind::Host`).
struct Visit {
    unit: usize,
    edge_for_host: bool,
    kind_host: bool,
    profile: Profile,
    edges: Vec<usize>,
}

/// The profiles of `graph`'s units, in its order, and the warnings of the package overrides
/// that match none of its packages.
fn profiles_of(
    ws: &Workspace<'_>,
    profiles: &Profiles,
    command: &Command,
    graph: &Value,
    stderr: &mut String,
) -> anyhow::Result<Vec<Profile>> {
    let units = graph["units"].as_array().ok_or_else(|| anyhow!("no `units` in the unit graph"))?;
    let mut parsed = Vec::new();
    for unit in units {
        let pkg = package(unit["pkg_id"].as_str().unwrap_or_default())?;
        let is_member = matches!(pkg.source_id.kind, SourceKind::Path)
            && pkg.source_id.url.to_file_path().is_ok_and(|p| ws.is_member(&p.join("Cargo.toml")));
        let kinds: Vec<&str> = unit["target"]["kind"]
            .as_array()
            .map(|k| k.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let kind = match unit["platform"].as_str() {
            Some(t) => CompileKind::Target(t.to_owned()),
            None => CompileKind::Host,
        };
        let identity = serde_json::to_string(&[
            &unit["pkg_id"],
            &unit["target"],
            &unit["mode"],
            &unit["features"],
        ])?;
        parsed.push(Unit {
            pkg,
            is_member,
            custom_build: kinds.contains(&"custom-build"),
            proc_macro: kinds.contains(&"proc-macro"),
            mode: unit["mode"].as_str().unwrap_or_default().to_owned(),
            kind,
            identity,
            deps: unit["dependencies"]
                .as_array()
                .map(|d| d.iter().filter_map(|d| d["index"].as_u64()).map(|i| i as usize).collect())
                .unwrap_or_default(),
        });
    }
    let mut packages: Vec<&dyn ProfilePackage> = Vec::new();
    let mut seen = rustc_hash::FxHashSet::default();
    for (unit, value) in parsed.iter().zip(units) {
        if seen.insert(value["pkg_id"].as_str()) {
            packages.push(&unit.pkg);
        }
    }
    let mut warnings = Vec::new();
    profiles.validate_packages(ws.profiles(), &packages, &mut warnings)?;
    for warning in warnings {
        let _ = writeln!(stderr, "warning: {warning}");
    }
    let mut walk =
        Walk { units: &parsed, profiles, visits: Vec::new(), by_key: FxHashMap::default() };
    let roots = graph["roots"].as_array().ok_or_else(|| anyhow!("no `roots` in the unit graph"))?;
    let is_test_command = matches!(command.name.as_str(), "test" | "bench");
    for root in roots {
        let i = root.as_u64().ok_or_else(|| anyhow!("a root is not an index"))? as usize;
        let unit = parsed.get(i).ok_or_else(|| anyhow!("root {i} is not a unit"))?;
        let profile_for = if unit.is_any_test() || is_test_command {
            UnitFor::new_test()
        } else if unit.for_host() {
            UnitFor::new_compiler()
        } else {
            UnitFor::new_normal()
        };
        let deps_for = if unit.is_any_test() || is_test_command {
            if unit.proc_macro { UnitFor::new_host_test() } else { UnitFor::new_test() }
        } else if unit.custom_build {
            UnitFor::new_host(false)
        } else if unit.proc_macro {
            UnitFor::new_host(true)
        } else if unit.for_host() {
            UnitFor::new_compiler()
        } else {
            UnitFor::new_normal()
        };
        let profile = walk.profile(i, profile_for);
        walk.visit(i, deps_for, false, unit.for_host(), profile)?;
    }
    let to_host = !command.target_given;
    let mut finals: Vec<Option<Profile>> = vec![None; walk.visits.len()];
    for v in 0..walk.visits.len() {
        walk.finalize(v, to_host, &mut finals);
    }
    let mut out: Vec<Option<Profile>> = vec![None; parsed.len()];
    for (v, visit) in walk.visits.iter().enumerate() {
        let profile = finals[v].take().unwrap();
        match &out[visit.unit] {
            Some(prev) if serde_json::to_string(prev)? != serde_json::to_string(&profile)? => {
                bail!("unit {} gets two profiles: {prev:?} and {profile:?}", visit.unit)
            }
            _ => out[visit.unit] = Some(profile),
        }
    }
    out.into_iter()
        .enumerate()
        .map(|(i, v)| v.ok_or_else(|| anyhow!("unit {i} is reached from no root")))
        .collect()
}

struct Walk<'a> {
    units: &'a [Unit],
    profiles: &'a Profiles,
    visits: Vec<Visit>,
    by_key: FxHashMap<(usize, String), usize>,
}

impl Walk<'_> {
    fn profile(&self, i: usize, unit_for: UnitFor) -> Profile {
        let unit = &self.units[i];
        let is_local = matches!(unit.pkg.source_id.kind, SourceKind::Path);
        let kind = if unit_for.is_for_host() { CompileKind::Host } else { unit.kind.clone() };
        self.profiles.get_profile(&unit.pkg, unit.is_member, is_local, unit_for, &kind)
    }

    fn visit(
        &mut self,
        i: usize,
        unit_for: UnitFor,
        edge_for_host: bool,
        kind_host: bool,
        profile: Profile,
    ) -> anyhow::Result<usize> {
        let key = (i, format!("{unit_for:?} {edge_for_host} {kind_host} {profile:?}"));
        if let Some(&v) = self.by_key.get(&key) {
            return Ok(v);
        }
        let v = self.visits.len();
        self.visits.push(Visit {
            unit: i,
            edge_for_host,
            kind_host,
            profile: profile.clone(),
            edges: Vec::new(),
        });
        self.by_key.insert(key, v);
        let unit = &self.units[i];
        let mut edges = Vec::new();
        for &d in &unit.deps {
            let dep = &self.units[d];
            let (dep_for, dep_profile, dep_kind_host) = if dep.mode == "run-custom-build" {
                if unit.mode == "run-custom-build" {
                    // The run of a `links` dependency's script, walked from its own package.
                    continue;
                }
                let dep_for = unit_for.for_custom_build();
                (dep_for, self.profiles.get_profile_run_custom_build(&profile), kind_host)
            } else if unit.mode == "run-custom-build" {
                let dep_for = unit_for.for_custom_build();
                (dep_for, self.profile(d, dep_for), true)
            } else {
                let dep_for =
                    unit_for.with_dependency(unit.custom_build, dep.for_host(), dep.proc_macro);
                (dep_for, self.profile(d, dep_for), kind_host || dep.for_host())
            };
            edges.push(self.visit(
                d,
                dep_for,
                dep_for.is_for_host(),
                dep_kind_host,
                dep_profile,
            )?);
        }
        self.visits[v].edges = edges;
        Ok(v)
    }

    fn finalize(&self, v: usize, to_host: bool, finals: &mut Vec<Option<Profile>>) {
        if finals[v].is_some() {
            return;
        }
        let edges = self.visits[v].edges.clone();
        for &e in &edges {
            self.finalize(e, to_host, finals);
        }
        let visit = &self.visits[v];
        let mut profile = visit.profile.clone();
        if profile.strip.is_deferred()
            && !profile.debuginfo.is_turned_on()
            && edges.iter().all(|&e| !finals[e].as_ref().unwrap().debuginfo.is_turned_on())
        {
            profile.strip = profile.strip.strip_debuginfo();
        }
        if visit.edge_for_host && to_host && profile.debuginfo.is_deferred() {
            let canonical = profile.debuginfo.finalize();
            let mut probe = profile.clone();
            probe.debuginfo = canonical;
            let identity = &self.units[visit.unit].identity;
            let shared = self.visits.iter().any(|w| {
                !w.kind_host && self.units[w.unit].identity == *identity && w.profile == probe
            });
            profile.debuginfo = if shared { canonical } else { canonical.weaken() };
        }
        finals[v] = Some(profile);
    }
}

//! `units build|check|test|bench [FLAGS]`: the unit graph blaze makes for the cargo command of
//! those flags, as `cargo <command> --unit-graph -Zunstable-options` prints it, from the
//! current directory. `harness/resolve.py units` compares the two. The host's triple and cfg
//! are rustc's (`RUSTC`, else `rustc` on the PATH), as cargo asks them.

#![allow(
    clippy::print_stderr,
    clippy::print_stdout,
    clippy::disallowed_methods,
    reason = "a command line program that asks rustc where it is run"
)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;

use anyhow::{Context as _, bail};
use blaze_units::{Filter, Intent, Platformed};

fn rustc_says(args: &[&str]) -> anyhow::Result<String> {
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned());
    let output =
        Command::new(&rustc).args(args).output().with_context(|| format!("running {rustc}"))?;
    anyhow::ensure!(output.status.success(), "`{rustc} {}` failed", args.join(" "));
    Ok(String::from_utf8(output.stdout)?)
}

fn platformed(triple: &str) -> anyhow::Result<Platformed> {
    let printed = rustc_says(&["--print", "cfg", "--target", triple])?;
    let cfg = printed
        .lines()
        .filter(|it| !it.is_empty())
        .map(|it| blaze_resolve::Cfg::from_str(it).with_context(|| format!("the cfg {it}")))
        .collect::<anyhow::Result<_>>()?;
    Ok(Platformed { triple: triple.to_owned(), cfg })
}

#[derive(Default)]
struct Flags {
    release: bool,
    profile: Option<String>,
    target: Option<String>,
    packages: Vec<String>,
    workspace: bool,
    excludes: Vec<String>,
    features: Vec<String>,
    all_features: bool,
    no_default_features: bool,
    manifest_path: Option<PathBuf>,
    config: Vec<String>,
    lib: bool,
    bins: Vec<String>,
    all_bins: bool,
    tests: Vec<String>,
    all_tests: bool,
    examples: Vec<String>,
    all_examples: bool,
    benches: Vec<String>,
    all_benches: bool,
    all_targets: bool,
}

fn parse(args: &[String]) -> anyhow::Result<(String, Flags)> {
    let mut args = args.iter().cloned();
    let command = args.next().context("no command given")?;
    let mut flags = Flags::default();
    while let Some(arg) = args.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((name, value)) if name.starts_with("--") => {
                (name.to_owned(), Some(value.to_owned()))
            }
            _ => (arg.clone(), None),
        };
        let mut value = || {
            inline.clone().or_else(|| args.next()).with_context(|| format!("a value for {flag}"))
        };
        match flag.as_str() {
            "--release" | "-r" => flags.release = true,
            "--profile" => flags.profile = Some(value()?),
            "--target" => flags.target = Some(value()?),
            "-p" | "--package" => flags.packages.push(value()?),
            "--workspace" | "--all" => flags.workspace = true,
            "--exclude" => flags.excludes.push(value()?),
            "-F" | "--features" => flags.features.push(value()?),
            "--all-features" => flags.all_features = true,
            "--no-default-features" => flags.no_default_features = true,
            "--manifest-path" => flags.manifest_path = Some(PathBuf::from(value()?)),
            "--config" => flags.config.push(value()?),
            "--lib" => flags.lib = true,
            "--bin" => flags.bins.push(value()?),
            "--bins" => flags.all_bins = true,
            "--test" => flags.tests.push(value()?),
            "--tests" => flags.all_tests = true,
            "--example" => flags.examples.push(value()?),
            "--examples" => flags.all_examples = true,
            "--bench" => flags.benches.push(value()?),
            "--benches" => flags.all_benches = true,
            "--all-targets" => flags.all_targets = true,
            "--no-run" | "--unit-graph" | "-Zunstable-options" => {}
            other => bail!("units does not read the flag {other}"),
        }
    }
    Ok((command, flags))
}

fn main() {
    // As cargo ends when it refuses a build: its message, status 101.
    if let Err(err) = run() {
        eprintln!("error: {err:#}");
        std::process::exit(101);
    }
}

fn run() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (command, flags) = parse(&args)?;
    let intent = match (command.as_str(), flags.profile.as_deref()) {
        ("build", _) => Intent::Build,
        ("check", Some("test")) => Intent::Check { test: true },
        ("check", _) => Intent::Check { test: false },
        ("test", _) => Intent::Test,
        ("bench", _) => Intent::Bench,
        (other, _) => bail!("units makes the graph of build, check, test or bench, not {other}"),
    };
    let filter = Filter::from_flags(
        flags.lib,
        flags.bins,
        flags.all_bins,
        flags.tests,
        flags.all_tests,
        flags.examples,
        flags.all_examples,
        flags.benches,
        flags.all_benches,
        flags.all_targets,
    );
    let told = rustc_says(&["-vV"])?;
    let host =
        told.lines().find_map(|it| it.strip_prefix("host: ")).context("rustc names no host")?;
    let host = platformed(host)?;
    let target = flags.target.as_deref().map(platformed).transpose()?;
    let cwd = std::env::current_dir()?;
    let asked = blaze_metadata::Asked {
        cwd: &cwd,
        env: std::env::vars().collect(),
        manifest_path: flags.manifest_path.as_deref().map(Path::new),
        config: &flags.config,
        features: &flags.features,
        all_features: flags.all_features,
        no_default_features: flags.no_default_features,
        build: Some(blaze_metadata::Build {
            packages: &flags.packages,
            workspace: flags.workspace,
            excludes: &flags.excludes,
            dev_units: filter.need_dev_deps(intent),
        }),
        platform: Some(blaze_metadata::TargetPlatform {
            triple: &target.as_ref().unwrap_or(&host).triple,
            cfg: &target.as_ref().unwrap_or(&host).cfg,
        }),
        offline: false,
        store: None,
    };
    let units = blaze_metadata::Units {
        intent,
        filter: &filter,
        release: flags.release,
        profile: flags.profile.as_deref(),
        host: &host,
        target: target.as_ref(),
    };
    let graph = blaze_metadata::units(&asked, &units)?;
    println!("{}", serde_json::to_string(&graph)?);
    Ok(())
}

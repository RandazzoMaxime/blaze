//! `cargo blaze doc`: the HTML documentation of the packages asked for and of what they depend
//! on, written by blaze from rust-analyzer's HIR (`crates/blaze-doc`) where cargo doc writes
//! rustdoc's: `target/doc/<crate>/index.html`. No rustdoc runs. The workspace is loaded once in
//! this process, as the daemon's checker loads it, and the pages are written in parallel: a
//! one-shot load needs no daemon, and a daemon would hold a database only `doc` reads.

use std::{
    path::{Path, PathBuf},
    time::Instant,
};

use anyhow::Context as _;
use blaze_doc::{Asked, Documented};
use cargo_metadata::{Metadata, Package, TargetKind};
use hir::Crate;
use ide_db::RootDatabase;
use rustc_hash::FxHashMap;

use crate::{
    Args,
    cargo::{metadata, say},
    plan::Selection,
};

/// The flags of `cargo doc` that take their value as the next argument.
const VALUED: &[&str] = &[
    "-p",
    "--package",
    "--exclude",
    "-F",
    "--features",
    "--target",
    "--manifest-path",
    "--bin",
    "--profile",
    "--config",
    "-Z",
    "-j",
    "--jobs",
    "--color",
    "--message-format",
    "--target-dir",
    "--lockfile-path",
];

/// Flags `cargo doc` takes that change nothing blaze writes.
const IGNORED: &[&str] =
    &["-j", "--jobs", "--color", "--locked", "--frozen", "--offline", "--keep-going", "--ignore-rust-version"];

#[derive(Default, Debug)]
struct Flags {
    packages: Vec<String>,
    workspace: bool,
    excludes: Vec<String>,
    no_deps: bool,
    private: bool,
    open: bool,
    features: Vec<String>,
    all_features: bool,
    no_default_features: bool,
    target: Option<String>,
    manifest_path: Option<PathBuf>,
    lib: bool,
    bins: bool,
    bin: Vec<String>,
    profile: Option<String>,
    config: Vec<String>,
    unstable: Vec<String>,
    quiet: bool,
    verbose: bool,
}

impl Flags {
    fn parse(args: &[String]) -> Result<Flags, String> {
        let mut flags = Flags::default();
        let mut words = args.iter();
        while let Some(word) = words.next() {
            let (flag, joined) = match word.split_once('=') {
                Some((flag, value)) if flag.starts_with("--") => (flag, Some(value.to_owned())),
                _ if !word.starts_with("--") && word.len() > 2 && VALUED.contains(&&word[..2]) => {
                    (&word[..2], Some(word[2..].to_owned()))
                }
                _ => (word.as_str(), None),
            };
            let value = match joined {
                None if VALUED.contains(&flag) => {
                    Some(words.next().cloned().ok_or(format!("`{flag}` needs a value"))?)
                }
                joined => joined,
            };
            let value = || value.clone().unwrap_or_default();
            match flag {
                "-p" | "--package" => flags.packages.push(value()),
                "--workspace" | "--all" => flags.workspace = true,
                "--exclude" => flags.excludes.push(value()),
                "--no-deps" => flags.no_deps = true,
                "--document-private-items" => flags.private = true,
                "--open" => flags.open = true,
                "-F" | "--features" => flags
                    .features
                    .extend(value().split([',', ' ']).filter(|it| !it.is_empty()).map(str::to_owned)),
                "--all-features" => flags.all_features = true,
                "--no-default-features" => flags.no_default_features = true,
                "--target" => flags.target = Some(value()),
                "--manifest-path" => flags.manifest_path = Some(PathBuf::from(value())),
                "--lib" => flags.lib = true,
                "--bins" => flags.bins = true,
                "--bin" => flags.bin.push(value()),
                "-r" | "--release" => flags.profile = Some("release".to_owned()),
                "--profile" => flags.profile = Some(value()),
                "--config" => flags.config.push(value()),
                "-Z" => flags.unstable.push(value()),
                "-q" | "--quiet" => flags.quiet = true,
                "-v" | "-vv" | "--verbose" => flags.verbose = true,
                "--target-dir" | "--message-format" | "--lockfile-path" => {
                    return Err(format!("`cargo blaze doc` does not take `{flag}` yet"));
                }
                _ if IGNORED.contains(&flag) => {}
                _ => return Err(format!("unexpected argument `{word}` for `cargo blaze doc`")),
            }
        }
        Ok(flags)
    }

    fn selection(&self, asked_in: Option<PathBuf>) -> Selection {
        Selection {
            profile: self.profile.clone().unwrap_or_else(|| "dev".to_owned()),
            features: self.features.clone(),
            no_default_features: self.no_default_features,
            all_features: self.all_features,
            packages: self.packages.clone(),
            workspace: self.workspace,
            excludes: self.excludes.clone(),
            unstable: self.unstable.clone(),
            target: self.target.clone().or_else(|| {
                std::env::var("CARGO_BUILD_TARGET").ok().filter(|it| !it.is_empty())
            }),
            dev_deps: false,
            config: self.config.clone(),
            asked_in,
        }
    }
}

/// `cargo blaze doc [flags]`: the status cargo doc would end with.
pub(crate) fn command(args: &[String]) -> anyhow::Result<i32> {
    let started = Instant::now();
    let flags = match Flags::parse(args) {
        Ok(flags) => flags,
        Err(why) => {
            say::error(&why);
            return Ok(1);
        }
    };
    let cwd = std::env::current_dir()?;
    let metadata = metadata::no_deps(flags.manifest_path.as_deref(), &flags.config);
    let packages = match selected(&flags, &metadata, &cwd) {
        Ok(packages) => packages,
        Err(why) => {
            say::error(&why);
            return Ok(101);
        }
    };
    let root = metadata.workspace_root.as_std_path().to_owned();
    let project = match &packages[..] {
        [one] => package_dir(one),
        _ => root.clone(),
    };
    let asked_in = flags.manifest_path.as_ref().map(|it| cwd.join(it));
    let selection = flags.selection(asked_in);
    let mut args = vec![project.display().to_string(), "--own-target".to_owned()];
    args.extend(selection.flags());
    if let Some(asked_in) = &selection.asked_in {
        args.extend(["--asked-in".to_owned(), asked_in.display().to_string()]);
    }
    args.extend(["--crate", "doc", "--out", "none"].map(str::to_owned));
    let args = Args::parse(args.into_iter())?;
    crate::process_wide();
    let cargo_config = crate::cargo_config(&args);
    let (mut db, vfs, _checkouts) = crate::load(
        &[project.as_path()],
        &cargo_config,
        crate::load_config(),
        selection.clone(),
        None,
        None,
    )?;
    let loaded = started.elapsed();
    let asked = match asked_crates(&db, &vfs, &packages, &flags) {
        Ok(asked) => asked,
        Err(why) => {
            say::error(&why);
            return Ok(101);
        }
    };
    let crates = hir::attach_db(&db, || blaze_doc::crates_to_document(&db, &asked, flags.no_deps));
    blaze_doc::set_cfg_doc(&mut db, &crates);
    for documented in &crates {
        say::status(flags.quiet, "Documenting", &describe(&db, &vfs, documented));
    }
    let first = crates.first().map(|it| it.name.clone()).context("nothing to document")?;
    let asked_names: Vec<String> =
        crates.iter().filter(|it| it.warn).map(|it| it.name.clone()).collect();
    let model = hir::attach_db(&db, || blaze_doc::model(&db, crates));
    let modelled = started.elapsed();
    let target_dir = metadata.target_directory.as_std_path().to_owned();
    let out = match &selection.target {
        Some(triple) => target_dir.join(triple).join("doc"),
        None => target_dir.join("doc"),
    };
    let written = blaze_doc::html::write(&model, &out)
        .with_context(|| format!("writing the documentation in {}", out.display()))?;
    warn(&model, &cwd);
    if flags.verbose {
        say::note(&format!(
            "loaded in {:.2}s, modelled in {:.2}s, wrote {} pages and {} source pages in {:.2}s",
            loaded.as_secs_f64(),
            (modelled - loaded).as_secs_f64(),
            written.pages,
            written.source_pages,
            (started.elapsed() - modelled).as_secs_f64(),
        ));
    }
    let profile = selection.profile.as_str();
    let shape = if profile == "release" { "optimized" } else { "unoptimized + debuginfo" };
    let took = started.elapsed().as_secs_f64();
    say::status(flags.quiet, "Finished", &format!("`{profile}` profile [{shape}] target(s) in {took:.2}s"));
    let page = out.join(&first).join("index.html");
    let shown = page.strip_prefix(&cwd).unwrap_or(&page).display().to_string();
    match asked_names.len() {
        0 | 1 => say::status(flags.quiet, "Generated", &shown),
        n => {
            let others = n - 1;
            let files = if others == 1 { "file" } else { "files" };
            say::status(flags.quiet, "Generated", &format!("{shown} and {others} other {files}"));
        }
    }
    if flags.open {
        say::status(flags.quiet, "Opening", &shown);
        open(&page);
    }
    Ok(0)
}

/// The packages `cargo doc` documents with `flags`: those `-p` names, every member with
/// `--workspace`, else the one of the directory, or the default members at a virtual root.
fn selected<'a>(flags: &Flags, metadata: &'a Metadata, cwd: &Path) -> Result<Vec<&'a Package>, String> {
    let members = metadata.workspace_packages();
    if !flags.packages.is_empty() {
        let mut out = vec![];
        for named in &flags.packages {
            let name = named.split('@').next().unwrap_or(named);
            let package = members
                .iter()
                .find(|it| it.name.as_str() == name)
                .ok_or(format!("package ID specification `{named}` did not match any packages"))?;
            out.push(*package);
        }
        return Ok(out);
    }
    if flags.workspace {
        let excluded = |it: &&&Package| flags.excludes.iter().any(|name| name == it.name.as_str());
        return Ok(members.iter().filter(|it| !excluded(it)).copied().collect());
    }
    let start = match &flags.manifest_path {
        Some(manifest) => cwd.join(manifest).parent().map(Path::to_owned).unwrap_or_default(),
        None => cwd.to_owned(),
    };
    let here = start.ancestors().find_map(|dir| members.iter().find(|it| package_dir(it) == dir));
    if let Some(package) = here {
        return Ok(vec![*package]);
    }
    let defaults: Vec<&Package> = metadata.workspace_default_packages();
    if defaults.is_empty() {
        return Err("the workspace has no package to document".to_owned());
    }
    Ok(defaults)
}

fn package_dir(package: &Package) -> PathBuf {
    package.manifest_path.parent().map(|it| it.as_std_path().to_owned()).unwrap_or_default()
}

/// The crates of `packages` to document: each one's library, and its binaries unless `--lib`
/// alone is asked, private items and all as cargo documents a binary. A binary named as its
/// package's library is left out, as cargo leaves it.
fn asked_crates(
    db: &RootDatabase,
    vfs: &vfs::Vfs,
    packages: &[&Package],
    flags: &Flags,
) -> Result<Vec<Asked>, String> {
    let mut by_root: FxHashMap<PathBuf, Crate> = FxHashMap::default();
    for krate in Crate::all(db) {
        let path = vfs.file_path(krate.root_file(db));
        if let Some(path) = path.as_path() {
            by_root.entry(PathBuf::from(path.as_str())).or_insert(krate);
        }
    }
    let picks_targets = flags.lib || flags.bins || !flags.bin.is_empty();
    let mut out = vec![];
    for package in packages {
        let lib = package.targets.iter().find(|it| {
            it.kind.iter().any(|kind| matches!(kind, TargetKind::Lib | TargetKind::RLib | TargetKind::ProcMacro | TargetKind::DyLib))
        });
        let lib_name = lib.map(|it| it.name.replace('-', "_"));
        if let Some(lib) = lib
            && (!picks_targets || flags.lib)
        {
            let krate = by_root
                .get(lib.src_path.as_std_path())
                .ok_or(format!("the library of {} was not loaded", package.name))?;
            out.push(Asked { krate: *krate, private: flags.private });
        }
        for bin in package.targets.iter().filter(|it| it.kind.contains(&TargetKind::Bin)) {
            let wanted = match picks_targets {
                false => true,
                true => flags.bins || flags.bin.iter().any(|name| *name == bin.name),
            };
            if !wanted || lib_name.as_deref() == Some(&bin.name.replace('-', "_")) {
                continue;
            }
            let Some(krate) = by_root.get(bin.src_path.as_std_path()) else { continue };
            out.push(Asked { krate: *krate, private: true });
        }
    }
    if out.is_empty() {
        return Err("no library or binary to document in the packages asked for".to_owned());
    }
    Ok(out)
}

/// `serde v1.0.219` as cargo names a package it documents, with the directory of one that is
/// not from a registry or a git checkout.
fn describe(db: &RootDatabase, vfs: &vfs::Vfs, documented: &Documented) -> String {
    use ide_db::base_db::CrateOrigin;
    let krate = documented.krate;
    let name = match krate.origin(db) {
        CrateOrigin::Local { name: Some(name), .. } | CrateOrigin::Library { name, .. } => {
            name.as_str().to_owned()
        }
        _ => documented.name.clone(),
    };
    let version = krate.version(db).map(|it| format!(" v{it}")).unwrap_or_default();
    let root = vfs.file_path(krate.root_file(db));
    let dir = root
        .as_path()
        .map(|it| Path::new(it.as_str()))
        .and_then(|it| it.ancestors().skip(1).find(|dir| dir.join("Cargo.toml").is_file()));
    let fetched = crate::cargo::switch::cargo_home().map(|home| [home.join("registry"), home.join("git")]);
    match dir {
        Some(dir) if !fetched.iter().flatten().any(|it| dir.starts_with(it)) => {
            format!("{name}{version} ({})", dir.display())
        }
        _ => format!("{name}{version}"),
    }
}

/// rustdoc's warnings, with the count cargo says after them.
fn warn(model: &blaze_doc::model::Model, cwd: &Path) {
    let mut counts: Vec<(String, usize)> = vec![];
    for warning in &model.warnings {
        say::warning(&warning.message);
        if let Some(at) = &warning.at {
            let at = Path::new(at).strip_prefix(cwd).map(|it| it.display().to_string());
            let at = at.unwrap_or_else(|_| warning.at.clone().unwrap_or_default());
            eprintln!("  --> {at}");
        }
        eprintln!();
        match counts.iter_mut().find(|(name, _)| *name == warning.krate) {
            Some((_, count)) => *count += 1,
            None => counts.push((warning.krate.clone(), 1)),
        }
    }
    for (name, count) in counts {
        let plural = if count == 1 { "warning" } else { "warnings" };
        say::warning(&format!("`{name}` (lib doc) generated {count} {plural}"));
    }
}

/// Opens `page` as cargo doc --open does: with `$BROWSER`, else the system's opener.
fn open(page: &Path) {
    let browser = std::env::var_os("BROWSER").filter(|it| !it.is_empty());
    let opener = match &browser {
        Some(browser) => browser.clone(),
        None if cfg!(target_os = "macos") => "open".into(),
        None if cfg!(windows) => "explorer".into(),
        None => "xdg-open".into(),
    };
    if let Err(why) = std::process::Command::new(&opener).arg(page).status() {
        say::error(&format!("could not open {} with {}: {why}", page.display(), opener.to_string_lossy()));
    }
}

//! The cold build by blaze (docs/no-rustc.md, step K, #91): with `RA_JIT_WHOLE_BUILD=1`,
//! `cargo blaze build` and `run` from nothing build each program of the build whole, every
//! crate from source over the shipped standard library and linked by blaze (step G), and put
//! it where cargo would. No `cargo build` runs, and what blaze cannot compile that the program
//! reaches fails the build, with what it is.

use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Instant,
};

use anyhow::Context as _;
use cargo_metadata::{Metadata, Package};

use crate::plan::Plan;

use super::{
    FAILED,
    cli::{Invocation, Subcommand},
    run, say,
    state::{as_built, configured_env, library_path, place, run_env, sysroot_libraries},
};

/// The setting that has a build from nothing be blaze's whole build.
const VARIABLE: &str = "RA_JIT_WHOLE_BUILD";

/// Whether the build `asked` for, from nothing, is blaze's whole build: `build` or `run`, of
/// the programs a build picks by default or one named with `--bin` or `--example`, for one
/// target at most.
pub(super) fn asked_for(asked: &Invocation) -> bool {
    let on = std::env::var_os(VARIABLE).is_some_and(|it| !it.is_empty() && it != "0");
    let flags = &asked.flags;
    let named = flags.bin.is_some() || flags.example.is_some();
    on && matches!(asked.subcommand, Subcommand::Build | Subcommand::Run)
        && (!flags.selects_targets || named)
        && !flags.cargos_plan
        && flags.targets.len() <= 1
}

/// A program of the build, as cargo plans it.
struct Program {
    /// The name of the executable.
    name: String,
    /// The root file of its crate.
    root: PathBuf,
    package: Package,
    /// Where cargo puts it.
    executable: PathBuf,
}

/// Builds the programs of `asked` whole and runs the one `run` asks for. `None` when the build
/// has no program to link, which is then built as without the setting.
pub(super) fn build(asked: &Invocation) -> anyhow::Result<Option<i32>> {
    let started = Instant::now();
    let cwd = std::env::current_dir()?;
    let metadata = super::state::metadata(asked);
    let programs = planned(asked, &cwd, &metadata)?;
    if programs.is_empty() {
        return Ok(None);
    }
    let dir = metadata.target_directory.as_std_path().join("ra-jit/whole");
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    for program in &programs {
        let package = &program.package;
        let at = super::state::package_dir(package);
        say::status(
            asked.flags.quiet,
            "Compiling",
            &format!("{} v{} ({})", package.name, package.version, at.display()),
        );
        if let Err(why) = whole(asked, &cwd, program, &dir) {
            say::error(&why);
            return Ok(Some(FAILED));
        }
    }
    let took = started.elapsed().as_secs_f64();
    let profile = asked.profile();
    let finished = format!("`{profile}` profile [ra-jit] target(s) in {took:.2}s");
    say::status(asked.flags.quiet, "Finished", &finished);
    if asked.subcommand != Subcommand::Run {
        return Ok(Some(0));
    }
    ran(asked, &cwd, &programs).map(Some)
}

/// Builds `program` whole into `dir` and puts it where cargo would. `Err` says why not.
fn whole(asked: &Invocation, cwd: &Path, program: &Program, dir: &Path) -> Result<(), String> {
    let built = dir.join(&program.name);
    let log = dir.join(format!("{}.log", program.name));
    let asked_in = asked.flags.manifest_path.as_ref().map_or(cwd.to_owned(), |it| cwd.join(it));
    let mut blaze = Command::new(std::env::current_exe().map_err(|it| it.to_string())?);
    blaze.arg(super::state::package_dir(&program.package)).arg("--own-target");
    blaze.args(asked.selection().flags()).arg("--crate").arg(&program.root);
    blaze.arg("--asked-in").arg(asked_in);
    blaze.arg("--out").arg(built.with_extension("o")).arg("--exe").arg(&built);
    // What blaze says goes to the log as it says it, so a long build can be followed there.
    let writing =
        std::fs::File::create(&log).map_err(|it| format!("writing {}: {it}", log.display()))?;
    blaze.current_dir(cwd).stdin(Stdio::null()).stdout(Stdio::null()).stderr(writing);
    let output = blaze.output().map_err(|it| format!("blaze did not start: {it}"))?;
    let said = std::fs::read(&log).map_err(|it| format!("reading {}: {it}", log.display()))?;
    let said = String::from_utf8_lossy(&said);
    let shown = log.strip_prefix(cwd).unwrap_or(&log).display().to_string();
    // A function of the program's own crates blaze cannot compile is left out of the object,
    // and the link then fails on what it would have defined.
    let unsupported: Vec<String> = said
        .lines()
        .filter_map(|it| it.trim().strip_prefix("left to rustc: "))
        .map(without_hashes)
        .collect();
    if !output.status.success() && !unsupported.is_empty() {
        return Err(format!(
            "`{}` has functions blaze could not compile, so it is not built:\n  {}\n(what blaze said is in {shown})",
            program.name,
            unsupported.join("\n  ")
        ));
    }
    if !output.status.success() {
        let last = said.lines().rev().find(|it| !it.trim().is_empty()).unwrap_or("");
        let last = last.strip_prefix("Error: ").unwrap_or(last);
        return Err(format!(
            "could not build `{}`: {last} (what blaze said is in {shown})",
            program.name
        ));
    }
    let mut left = said.lines().skip_while(|it| !is_linked(it));
    match left.next() {
        Some(linked) if !linked.ends_with(", 0 functions in it ra-jit could not compile") => {
            // The link names the first few of them, each on a line of its own.
            let reached: Vec<String> = left
                .map_while(|it| it.strip_prefix("  "))
                .map(|it| it.trim().trim_start_matches(hir_ty_codegen::STAND_IN_SAYS))
                .map(without_hashes)
                .collect();
            return Err(format!(
                "`{}` can reach functions blaze could not compile, so it is not built:\n  {}\n(what blaze said is in {shown})",
                program.name,
                reached.join("\n  ")
            ));
        }
        Some(_) => {}
        None => {
            return Err(format!(
                "blaze did not link `{}` (what it said is in {shown})",
                program.name
            ));
        }
    }
    if let Some(parent) = program.executable.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|it| format!("creating {}: {it}", parent.display()))?;
    }
    place(&built, &program.executable)
        .map_err(|it| format!("installing {}: {it}", program.executable.display()))
}

/// `name` with the `[0123456789abcdef]` after each crate's name, and the `ra-jit.<hash>.` of
/// a symbol, taken out.
fn without_hashes(name: &str) -> String {
    let is_hash = |it: &str| it.len() == 16 && it.bytes().all(|it| it.is_ascii_hexdigit());
    let mut name = name.to_owned();
    while let Some(at) = name.find("ra-jit.")
        && let Some(hash) = name.get(at + 7..at + 23)
        && is_hash(hash)
        && name[at + 23..].starts_with('.')
    {
        name.replace_range(at..at + 24, "");
    }
    let mut out = String::with_capacity(name.len());
    let mut rest = name.as_str();
    while let Some(at) = rest.find('[') {
        let hash = rest[at + 1..].split_once(']').map(|(it, _)| it);
        out.push_str(&rest[..at]);
        match hash {
            Some(hash) if is_hash(hash) => {
                rest = &rest[at + hash.len() + 2..];
            }
            _ => {
                out.push('[');
                rest = &rest[at + 1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn is_linked(line: &str) -> bool {
    line.starts_with("linked ") && line.ends_with(" functions in it ra-jit could not compile")
}

/// Runs the program `asked` names, as `cargo run` does.
fn ran(asked: &Invocation, cwd: &Path, programs: &[Program]) -> anyhow::Result<i32> {
    let named = asked.flags.bin.as_ref().or(asked.flags.example.as_ref());
    let mut chosen: Vec<&Program> =
        programs.iter().filter(|it| named.is_none_or(|name| it.name == *name)).collect();
    if chosen.len() > 1 {
        let default = |it: &&Program| it.package.default_run.as_deref() == Some(it.name.as_str());
        chosen.retain(default);
    }
    let [program] = chosen[..] else {
        let names: Vec<&str> = programs.iter().map(|it| it.name.as_str()).collect();
        say::error(&format!(
            "`cargo run` could not determine which binary to run. Use the `--bin` option to specify a binary, \
             or the `default-run` manifest key.\navailable binaries: {}",
            names.join(", ")
        ));
        return Ok(FAILED);
    };
    let shown =
        program.executable.strip_prefix(cwd).unwrap_or(&program.executable).display().to_string();
    let args = asked.program_args().map(|it| format!(" {it}")).collect::<String>();
    say::status(asked.flags.quiet, "Running", &format!("`{shown}{args}`"));
    let mut command = Command::new(&program.executable);
    for it in configured_env(cwd) {
        if it.force || std::env::var_os(&it.name).is_none() {
            command.env(&it.name, &it.value);
        }
    }
    command.args(asked.program_args()).envs(run_env(&program.package, &[]));
    command.envs(library_path(&program.executable, sysroot_libraries().as_deref()));
    as_built(&mut command)?;
    let status = command.status().with_context(|| format!("could not execute {shown}"))?;
    Ok(run::exit_code(status))
}

/// The programs `asked` builds, as cargo's unit graph has them: each executable root.
fn planned(asked: &Invocation, cwd: &Path, metadata: &Metadata) -> anyhow::Result<Vec<Program>> {
    let manifest = asked.flags.manifest_path.as_ref().map(|it| cwd.join(it));
    let dir = manifest.as_deref().and_then(Path::parent).unwrap_or(cwd);
    let mut targets = Vec::new();
    if let Some(bin) = &asked.flags.bin {
        targets.extend(["--bin".to_owned(), bin.clone()]);
    }
    if let Some(example) = &asked.flags.example {
        targets.extend(["--example".to_owned(), example.clone()]);
    }
    let selection = asked.selection();
    let planned = Plan::programs(dir, manifest.as_deref(), &selection, &targets)?;
    // cargo's directory of a profile: `dev`'s is `debug`.
    let profile = match selection.profile.as_str() {
        "dev" => "debug",
        other => other,
    };
    let target = metadata.target_directory.as_std_path();
    let mut programs = Vec::new();
    for program in planned {
        let Some(package) = metadata.packages.iter().find(|it| it.id.repr == program.package)
        else {
            anyhow::bail!(
                "cargo's unit graph names {}, which `cargo metadata` does not",
                program.package
            );
        };
        let mut dir = target.to_owned();
        if let Some(triple) = &program.platform {
            dir.push(triple);
        }
        dir.push(profile);
        if program.example {
            dir.push("examples");
        }
        let name = format!("{}{}", program.name, std::env::consts::EXE_SUFFIX);
        programs.push(Program {
            executable: dir.join(&name),
            name: program.name,
            root: program.root,
            package: package.clone(),
        });
    }
    Ok(programs)
}

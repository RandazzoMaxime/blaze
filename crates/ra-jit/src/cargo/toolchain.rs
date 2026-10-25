//! The Rust release `cargo blaze` builds with: the one blaze compiles, whatever toolchain the
//! project names. blaze lays out types, names symbols and links as that release does, so cargo,
//! rustc and the daemon all run under it. Rust builds today what an older release built, so a
//! project pinned to an older stable builds the same; one that needs its own toolchain (a
//! nightly's features, a newer release's) fails with rustc's error and a line saying which
//! release built it, and never goes to cargo for it ([#269]).
//!
//! [#269]: https://github.com/indexable-inc/blaze/issues/269

use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
};

/// The release blaze compiles.
pub(crate) const OURS: &str = include_str!("../../../../harness/toolchain");

/// The project's own toolchain, as a failure names it, when blaze builds with its release in
/// its place.
static IN_PLACE_OF: OnceLock<(String, &'static str)> = OnceLock::new();

/// Has the cargo, rustc and daemon of this command be blaze's release: names its toolchain in
/// `RUSTUP_TOOLCHAIN`, which rustup's proxies take over a toolchain file or an override, and
/// its cargo in `CARGO`. `Err` is why the command can't be built, said for the user to act on.
/// Before any thread starts: it sets the environment.
pub(crate) fn settle() -> Result<(), String> {
    let ours = OURS.trim();
    let other = super::state::other_toolchain_allowed();
    let rustc = super::state::rustc_path();
    let version = version_of(Path::new(&rustc));
    let version = version.as_deref().unwrap_or("of no release it says");
    if other {
        return with_library(&sysroot_of(Path::new(&rustc)), version);
    }
    if version == ours {
        let sysroot = sysroot_of(Path::new(&rustc));
        with_library(&sysroot, ours)?;
        // One name for it however it was named (`1.99.0`, `1.99.0-<host>`, a path), since the
        // name is part of which build this is.
        let home = rustup_home().and_then(|it| dunce::canonicalize(it.join("toolchains")).ok());
        let dir = dunce::canonicalize(&sysroot).ok();
        let name = dir.filter(|dir| home.is_some_and(|home| dir.parent() == Some(&*home)));
        if std::env::var_os("RUSTUP_TOOLCHAIN").is_some()
            && let Some(name) = name.as_ref().and_then(|it| it.file_name())
        {
            // SAFETY: before any thread is started.
            unsafe { std::env::set_var("RUSTUP_TOOLCHAIN", name) };
        }
        return Ok(());
    }
    let named = named_by();
    if std::env::var_os("RUSTC").is_some() {
        return Err(format!(
            "`RUSTC` names rustc {version}, and blaze builds with Rust {ours}, the release it \
             compiles: without `RUSTC` it builds with {ours} (`BLAZE_OTHER_TOOLCHAIN=1` builds \
             with {version} anyway, laying its types out as {ours} does)"
        ));
    }
    let Some(home) = rustup_home() else {
        return Err(format!(
            "blaze builds with Rust {ours}, the release it compiles, and this rustc is {version}, \
             with no rustup to switch: put Rust {ours} with its `rust-src` first on `PATH` \
             (`BLAZE_OTHER_TOOLCHAIN=1` builds with {version} anyway, laying its types out as \
             {ours} does)"
        ));
    };
    let Some(dir) = installed(&home, ours) else {
        return Err(format!(
            "blaze builds with Rust {ours}, the release it compiles, which is not installed: \
             `rustup toolchain install {ours} --component rust-src` installs it"
        ));
    };
    with_library(&dir, ours)?;
    let name = dir.file_name().and_then(|it| it.to_str()).unwrap_or(ours).to_owned();
    let cargo = dir.join("bin").join(format!("cargo{}", std::env::consts::EXE_SUFFIX));
    // SAFETY: before any thread is started.
    unsafe {
        std::env::set_var("RUSTUP_TOOLCHAIN", name);
        std::env::set_var("CARGO", cargo);
    }
    let _ = IN_PLACE_OF.set((version.to_owned(), named));
    Ok(())
}

/// What a build that failed says after its errors: which release built it, when it was not
/// the project's own, since the failure may be what one has and the other has not.
pub(crate) fn failed_note() -> Option<String> {
    let (version, named) = IN_PLACE_OF.get()?;
    Some(format!(
        "blaze built this with Rust {}, the release it compiles, in place of {version}, {named}; \
         an error {version} would not give comes from that difference",
        OURS.trim()
    ))
}

pub(crate) fn note_failed() {
    if let Some(note) = failed_note() {
        super::say::note(&note);
    }
}

/// The release `rustc` says it is, as `1.99.0` or `1.101.0-nightly`.
fn version_of(rustc: &Path) -> Option<String> {
    said(rustc, "--version")?.split_whitespace().nth(1).map(str::to_owned)
}

fn sysroot_of(rustc: &Path) -> PathBuf {
    PathBuf::from(said(rustc, "--print=sysroot").unwrap_or_default().trim())
}

fn said(rustc: &Path, flag: &str) -> Option<String> {
    let output = crate::answers::answer(Command::new(rustc).arg(flag), &[]).ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// What chose the project's toolchain, as rustup tells the programs its proxies start.
fn named_by() -> &'static str {
    match std::env::var("RUSTUP_TOOLCHAIN_SOURCE").as_deref() {
        Ok("toolchain-file") => "which the project's rust-toolchain file names",
        Ok("path-override" | "override") => "which `rustup override` names for the project",
        Ok("cli") => "which `+toolchain` names",
        Ok("env") => "which `RUSTUP_TOOLCHAIN` names",
        Ok("default") => "rustup's default toolchain",
        _ => "the toolchain cargo was started with",
    }
}

fn rustup_home() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("RUSTUP_HOME") {
        return Some(PathBuf::from(home));
    }
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })?;
    Some(PathBuf::from(home).join(".rustup")).filter(|it| it.is_dir())
}

/// The directory rustup installed `release` for this host in: `1.99.0-aarch64-apple-darwin`.
fn installed(home: &Path, release: &str) -> Option<PathBuf> {
    let prefix = format!("{release}-");
    let cargo = format!("cargo{}", std::env::consts::EXE_SUFFIX);
    let mut found: Vec<PathBuf> = std::fs::read_dir(home.join("toolchains"))
        .ok()?
        .flatten()
        .filter(|it| it.file_name().to_str().is_some_and(|name| name.starts_with(&prefix)))
        .map(|it| it.path())
        .filter(|it| it.join("bin").join(&cargo).is_file())
        .collect();
    found.sort();
    let host = |dir: &PathBuf| {
        let name = dir.file_name().and_then(|it| it.to_str()).unwrap_or_default();
        let os = match std::env::consts::OS {
            "macos" => "apple-darwin",
            os => os,
        };
        name[prefix.len()..].starts_with(std::env::consts::ARCH) && name.contains(os)
    };
    found.iter().find(|it| host(it)).or(found.first()).cloned()
}

/// `Ok` when the toolchain at `sysroot` has the source of its standard library, which the
/// daemon compiles.
fn with_library(sysroot: &Path, release: &str) -> Result<(), String> {
    if sysroot.join("lib/rustlib/src/rust/library").is_dir() {
        return Ok(());
    }
    Err(format!(
        "Rust {release} has no source of its standard library, which blaze compiles: \
         `rustup component add rust-src --toolchain {release}` adds it"
    ))
}

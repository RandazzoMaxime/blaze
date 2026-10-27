//! What a build script's run is declared to be made of, Nix's way (Andrew, 2026-10-27): its
//! package's source, its `OUT_DIR`, the toolchain closure (clang, lld, a libc sysroot,
//! pkg-config and a shell's tools, pinned from nixpkgs) and the system libraries its crate is
//! declared to need ([`libs_for`]), and nothing else. On Linux the run sees only that, in a
//! mount namespace; what it is made of is then all its run is a function of, so the key of
//! the run is of that ([`package_identity`], the closure's hashes) and not of what it was seen
//! reading.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use anyhow::Context as _;
use sha2::{Digest as _, Sha256};

/// The table of system libraries by crate, shipped with blaze.
const LIBS: &str = include_str!("libs.toml");

/// What every build script is given of the closure, whatever its crate: the C and C++
/// compilers and their linker, the target's libc and kernel headers, pkg-config, and the
/// tools a `configure` or a `Makefile` runs. By nixpkgs attribute and output, as
/// `closure/pins.txt` names them.
pub(crate) const TOOLCHAIN: [&str; 14] = [
    "clang.out",
    "lld.out",
    "glibc.out",
    "glibc.dev",
    "linuxHeaders.out",
    "pkg-config.out",
    "bash.out",
    "coreutils.out",
    "gnumake.out",
    "gnused.out",
    "gawk.out",
    "gnugrep.out",
    "diffutils.out",
    "findutils.out",
];

/// An entry of the table: a crate's libraries, for the versions `versions` allows.
#[derive(serde_derive::Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    versions: Option<String>,
    libs: Vec<String>,
}

/// The system libraries the build script of the crate `name` at `version` is given: the
/// workspace's own entry (`build-script-libs`, `overrides`) when it has one, else the shipped
/// table's for that version, else none. `Err` is a table that says it wrongly.
pub(crate) fn libs_for(
    name: &str,
    version: &str,
    overrides: &BTreeMap<String, Vec<String>>,
) -> anyhow::Result<Vec<String>> {
    if let Some(libs) = overrides.get(name) {
        return Ok(libs.clone());
    }
    let table: BTreeMap<String, Entry> = toml::from_str(LIBS).context("blaze's libs.toml")?;
    let Some(entry) = table.get(name) else { return Ok(Vec::new()) };
    let allowed = match &entry.versions {
        None => true,
        Some(range) => {
            let range = semver::VersionReq::parse(range)
                .with_context(|| format!("libs.toml: `{name}`'s versions `{range}`"))?;
            semver::Version::parse(version).is_ok_and(|it| range.matches(&it))
        }
    };
    Ok(match allowed {
        true => entry.libs.clone(),
        false => Vec::new(),
    })
}

/// What the source of the package in `dir` is, for the key of a run of its script: a
/// registry's package by its index, name, version and the digest of its `.crate` (what
/// `Cargo.lock`'s checksum is of); a git dependency's by its checkout, which cargo names by
/// the commit and never changes; any other by every file below `dir` but those in `hidden`,
/// which its run does not see. `cargo_home` is where cargo keeps the first two.
pub(crate) fn package_identity(
    dir: &Path,
    cargo_home: Option<&Path>,
    hidden: &[PathBuf],
) -> anyhow::Result<String> {
    if let Some(home) = cargo_home {
        if let Ok(rest) = dir.strip_prefix(home.join("registry").join("src")) {
            let mut parts = rest.components();
            let index = parts.next().context("a registry's package outside an index")?;
            let package = parts.next().context("a registry's index with no package")?;
            let (index, package) = (index.as_os_str(), package.as_os_str());
            let file = home.join("registry").join("cache").join(index);
            let file = file.join(format!("{}.crate", package.to_string_lossy()));
            let bytes = std::fs::read(&file)
                .with_context(|| format!("the package {} came from", file.display()))?;
            return Ok(format!(
                "registry {} {} {}",
                index.to_string_lossy(),
                package.to_string_lossy(),
                hex(&Sha256::digest(&bytes))
            ));
        }
        if let Ok(rest) = dir.strip_prefix(home.join("git").join("checkouts")) {
            return Ok(format!("git {}", rest.display()));
        }
    }
    let mut digest = Sha256::new();
    tree_digest(dir, dir, hidden, &mut digest)?;
    Ok(format!("tree {}", hex(&digest.finalize())))
}

/// Adds each file below `dir` but those in `hidden` to `digest`, in order: its path below
/// `root`, its kind and whether it is executable, and what it has or where it leads.
fn tree_digest(
    root: &Path,
    dir: &Path,
    hidden: &[PathBuf],
    digest: &mut Sha256,
) -> anyhow::Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .with_context(|| format!("listing {}", dir.display()))?
        .collect::<Result<_, _>>()
        .with_context(|| format!("listing {}", dir.display()))?;
    entries.sort_by_key(|it| it.file_name());
    for entry in entries {
        let path = entry.path();
        if hidden.iter().any(|it| *it == path) {
            continue;
        }
        let kind = entry.file_type()?;
        let name = path.strip_prefix(root).unwrap_or(&path).as_os_str().as_encoded_bytes();
        let mut field = |bytes: &[u8]| {
            digest.update((bytes.len() as u64).to_le_bytes());
            digest.update(bytes);
        };
        field(name);
        if kind.is_symlink() {
            field(b"link");
            field(std::fs::read_link(&path)?.as_os_str().as_encoded_bytes());
        } else if kind.is_dir() {
            field(b"dir");
            tree_digest(root, &path, hidden, digest)?;
        } else {
            let metadata = entry.metadata()?;
            field(if super::script::is_executable(&metadata) { b"exe" } else { b"file" });
            let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
            field(&Sha256::digest(&bytes));
        }
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|it| format!("{it:02x}")).collect()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn the_table_gives_a_crate_its_libraries_and_the_workspace_wins() {
        let none = BTreeMap::new();
        let openssl = libs_for("openssl-sys", "0.9.109", &none).unwrap();
        assert_eq!(openssl, ["openssl.out", "openssl.dev"]);
        assert!(libs_for("serde", "1.0.228", &none).unwrap().is_empty());
        let ours = BTreeMap::from([("openssl-sys".to_owned(), vec!["libressl.dev".to_owned()])]);
        assert_eq!(libs_for("openssl-sys", "0.9.109", &ours).unwrap(), ["libressl.dev"]);
    }

    #[test]
    fn every_entry_of_the_table_reads() {
        let table: BTreeMap<String, Entry> = toml::from_str(LIBS).unwrap();
        for (name, entry) in &table {
            assert!(!entry.libs.is_empty(), "{name}");
            if let Some(range) = &entry.versions {
                semver::VersionReq::parse(range).unwrap();
            }
            for lib in &entry.libs {
                assert!(lib.contains('.'), "{name}: `{lib}` names its output");
            }
        }
    }

    #[test]
    fn a_package_is_its_crate_its_commit_or_its_files() {
        let root = std::env::temp_dir().join(format!("ra-jit-hermetic-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&root);
        let home = root.join("home");
        let src = home.join("registry/src/index.crates.io-1949cf8c6b5b557f/zlib-sys-1.2.3");
        std::fs::create_dir_all(&src).unwrap();
        let cache = home.join("registry/cache/index.crates.io-1949cf8c6b5b557f");
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::write(cache.join("zlib-sys-1.2.3.crate"), b"the crate").unwrap();
        let registry = package_identity(&src, Some(&home), &[]).unwrap();
        assert!(registry.starts_with("registry index.crates.io-"), "{registry}");
        assert!(registry.contains(" zlib-sys-1.2.3 "), "{registry}");
        let git = home.join("git/checkouts/zed-abc/0a1c519/crates/webrtc-sys");
        std::fs::create_dir_all(&git).unwrap();
        assert_eq!(
            package_identity(&git, Some(&home), &[]).unwrap(),
            "git zed-abc/0a1c519/crates/webrtc-sys"
        );
        let ws = root.join("ws");
        std::fs::create_dir_all(ws.join("src")).unwrap();
        std::fs::create_dir_all(ws.join("target/debug")).unwrap();
        std::fs::write(ws.join("build.rs"), b"fn main() {}").unwrap();
        std::fs::write(ws.join("src/lib.rs"), b"").unwrap();
        let hidden = [ws.join("target")];
        let first = package_identity(&ws, Some(&home), &hidden).unwrap();
        std::fs::write(ws.join("target/debug/x"), b"built").unwrap();
        assert_eq!(package_identity(&ws, Some(&home), &hidden).unwrap(), first);
        std::fs::write(ws.join("src/lib.rs"), b"// edited").unwrap();
        assert_ne!(package_identity(&ws, Some(&home), &hidden).unwrap(), first);
        _ = std::fs::remove_dir_all(&root);
    }
}

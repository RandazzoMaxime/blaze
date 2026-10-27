//! The `llvm` feature links the one LLVM blaze pins (`llvm.json` beside this script: LLVM
//! 23.1.4, built by .github/scripts/llvm.py and fetched by its sha256) and no other,
//! statically: a binary built with the feature needs no libLLVM where it runs.
//!
//! `LLVM_SYS_231_PREFIX` names the prefix. Its `blaze-llvm.json`, written by the build, must name
//! the pinned commit and version, and its `llvm-config` must say that version: neither a
//! distribution's LLVM nor Homebrew's is a source, and a prefix that is not the pinned one stops
//! the build with one message. llvm-sys is built with `no-llvm-linking`, so this script does the
//! linking; on Windows llvm-sys links the same prefix itself (`llvm-sys`'s static mode).
//!
//! On Linux the C++ runtime LLVM is written against, libstdc++, is linked in statically too:
//! a release binary then asks a system for nothing of C++. On macOS it is the system's libc++.

use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
};

const PREFIX: &str = "LLVM_SYS_231_PREFIX";
const MARKER: &str = "blaze-llvm.json";

fn main() {
    println!("cargo::rerun-if-env-changed={PREFIX}");
    println!("cargo::rerun-if-changed=llvm.json");
    if env::var_os("CARGO_FEATURE_LLVM").is_none() {
        return;
    }
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let pin = read_json(&manifest.join("llvm.json"));
    let (version, commit) = (field(&pin, "version"), field(&pin, "commit"));

    let prefix = env::var(PREFIX).unwrap_or_default();
    let refuse = |why: String| {
        println!(
            "cargo::error=the `llvm` feature builds against the LLVM blaze pins ({version}, \
             {} at {}): {why}. Enter `nix develop`, or fetch it with \
             `python3 .github/scripts/llvm.py fetch --target <triple> --dir <dir>` and set \
             {PREFIX} to that directory",
            field(&pin, "repo"),
            &commit[..12],
        );
    };
    if prefix.is_empty() {
        return refuse(format!("{PREFIX} is unset"));
    }
    let prefix = PathBuf::from(prefix);
    let marker = prefix.join(MARKER);
    println!("cargo::rerun-if-changed={}", marker.display());
    if !marker.exists() {
        return refuse(format!("`{}` has no {MARKER}, so it is not a build of it", prefix.display()));
    }
    let built = read_json(&marker);
    if field(&built, "commit") != commit || field(&built, "version") != version {
        return refuse(format!(
            "`{}` is LLVM {} at {}",
            prefix.display(),
            field(&built, "version"),
            field(&built, "commit")
        ));
    }
    let exe = if env::var_os("CARGO_CFG_WINDOWS").is_some() { ".exe" } else { "" };
    let config = prefix.join("bin").join(format!("llvm-config{exe}"));
    let said = ask(&config, &["--version"]);
    if said.trim() != version {
        return refuse(format!("`{}` says {}", config.display(), said.trim()));
    }
    // llvm-sys links it on Windows.
    if env::var_os("CARGO_CFG_WINDOWS").is_some() {
        return;
    }

    println!("cargo::rustc-link-search=native={}", ask(&config, &["--libdir"]).trim());
    // Every library, in the order llvm-config gives (each before what it needs); the linker takes
    // from each only what something calls. `-bundle`: the archives are named to the final link,
    // not copied into this crate's rlib.
    for file in ask(&config, &["--libnames", "--link-static"]).split_whitespace() {
        let name = file
            .strip_prefix("lib")
            .and_then(|n| n.strip_suffix(".a"))
            .unwrap_or_else(|| panic!("{file} is not a static library"));
        println!("cargo::rustc-link-lib=static:-bundle={name}");
    }
    for flag in ask(&config, &["--system-libs", "--link-static"]).split_whitespace() {
        let name =
            flag.strip_prefix("-l").unwrap_or_else(|| panic!("{flag} is not a library flag"));
        println!("cargo::rustc-link-lib=dylib={name}");
    }
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo::rustc-link-lib=dylib=c++");
    } else {
        let archive = cxx_archive();
        let dir = archive.parent().expect("libstdc++.a is in a directory");
        println!("cargo::rustc-link-search=native={}", dir.display());
        println!("cargo::rustc-link-lib=static:-bundle=stdc++");
    }
}

/// libstdc++.a of the C++ compiler cargo's `cc` would use (`CXX`, else `c++`).
fn cxx_archive() -> PathBuf {
    println!("cargo::rerun-if-env-changed=CXX");
    let cxx = env::var("CXX").unwrap_or_else(|_| "c++".to_owned());
    let found = ask(Path::new(&cxx), &["-print-file-name=libstdc++.a"]);
    let path = PathBuf::from(found.trim());
    assert!(
        path.is_absolute() && path.exists(),
        "`{cxx} -print-file-name=libstdc++.a` found no libstdc++.a ({}): the `llvm` feature links \
         libstdc++ statically, so install the C++ compiler's static library (Debian and Ubuntu: \
         libstdc++-<version>-dev)",
        path.display()
    );
    path
}

fn ask(program: &Path, args: &[&str]) -> String {
    let out = Command::new(program)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("{} {args:?}: {e}", program.display()));
    assert!(
        out.status.success(),
        "{} {args:?}: {}",
        program.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("UTF-8 output")
}

/// The JSON object in `path`, kept as text: a build script with no dependencies reads the few
/// string fields it needs with [`field`].
fn read_json(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The string value of the top-level `"key": "value"` in `json` (none of the values it is asked
/// for holds a quote or a backslash).
fn field(json: &str, key: &str) -> String {
    let at = json
        .find(&format!("\"{key}\""))
        .unwrap_or_else(|| panic!("no \"{key}\" in {json}"));
    let rest = &json[at + key.len() + 2..];
    let rest = rest.trim_start().strip_prefix(':').expect("a `:` after the key").trim_start();
    let rest = rest.strip_prefix('"').unwrap_or_else(|| panic!("\"{key}\" is not a string"));
    rest[..rest.find('"').expect("a closing quote")].to_owned()
}

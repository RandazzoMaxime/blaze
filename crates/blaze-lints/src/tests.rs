//! The lints run over crates written out as rust-analyzer's fixtures write them, their
//! diagnostics compared with what clippy writes of the same source.

use ide_db::{RootDatabase, base_db::SourceDatabase as _};
use rustc_hash::FxHashMap;
use test_fixture::WithFixture as _;

use crate::{
    check::{Setup, check},
    conf::Conf,
    emit::{self, Sources, Style},
    level::CommandLine,
};

/// What `std` is to the fixtures: the macros the lints look for, as `std` writes them.
pub(crate) const STD: &str = r#"
//- /std/lib.rs crate:std
pub mod prelude {
    pub mod v1 {
        pub use crate::{assert_eq, assert_ne, debug_assert_eq, test, vec};
        pub use core::prelude::v1::*;
    }
    pub mod rust_2015 { pub use super::v1::*; }
    pub mod rust_2018 { pub use super::v1::*; }
    pub mod rust_2021 { pub use super::v1::*; }
    pub mod rust_2024 { pub use super::v1::*; }
}
#[macro_export]
macro_rules! assert_eq {
    ($left:expr, $right:expr $(,)?) => {
        match (&$left, &$right) {
            (left_val, right_val) => {
                if !(*left_val == *right_val) {
                    loop {}
                }
            }
        }
    };
}
#[macro_export]
macro_rules! assert_ne {
    ($left:expr, $right:expr $(,)?) => {
        match (&$left, &$right) {
            (left_val, right_val) => {
                if *left_val == *right_val {
                    loop {}
                }
            }
        }
    };
}
#[macro_export]
macro_rules! debug_assert_eq {
    ($($arg:tt)*) => {
        if true {
            $crate::assert_eq!($($arg)*);
        }
    };
}
#[rustc_builtin_macro]
pub macro test($item:item) {}
#[macro_export]
macro_rules! vec {
    ($($x:expr),* $(,)?) => {
        [$($x),*]
    };
}
"#;

/// The diagnostics clippy's lints have in the crate `main` of `fixture`, rustc's flags `flags`
/// given, each as rustc writes it.
pub(crate) fn lint(fixture: &str, flags: &str) -> String {
    let db = RootDatabase::with_files(fixture);
    hir::attach_db(&db, || {
        let krate = hir::Crate::all(&db)
            .into_iter()
            .find(|it| it.display_name(&db).is_some_and(|it| it.to_string() == "main"))
            .expect("the fixture has a crate `main`");
        let mut names: FxHashMap<ide_db::FileId, String> = FxHashMap::default();
        let mut texts = Vec::new();
        for krate in hir::Crate::all(&db) {
            for module in krate.modules(&db) {
                let Some(file) = module.definition_source_file_id(&db).file_id() else { continue };
                let file = file.file_id(&db);
                let root = db.file_source_root(file).source_root_id(&db);
                let root = db.source_root(root).source_root(&db);
                let Some(path) = root.path_for_file(&file) else { continue };
                let name = path.to_string().trim_start_matches('/').to_owned();
                if names.insert(file, name.clone()).is_none() {
                    texts.push((name, db.file_text(file).text(&db).to_string()));
                }
            }
        }
        let flags: Vec<String> = flags.split_whitespace().map(str::to_owned).collect();
        let setup = Setup {
            command_line: CommandLine::parse(&flags).expect("the flags read"),
            conf: Conf::default(),
        };
        let name_of = |file: ide_db::FileId| names.get(&file).cloned();
        let found = check(&db, krate, &setup, &name_of);
        let mut sources = Sources::of(texts);
        found
            .iter()
            .map(|it| emit::written(&it.diagnostic, &mut sources, Style::default()))
            .collect()
    })
}

#[test]
fn lints_what_an_attribute_macro_is_handed() {
    let fixture = format!(
        r#"
//- minicore: copy, unary_ops, bool_impl
//- /src/lib.rs crate:main deps:std cfg:test
#[cfg(test)]
mod tests {{
    #[test]
    fn asserts() {{
        assert_eq!(true, 1 > 0);
    }}
}}
{STD}"#
    );
    expect_test::expect![[r#""#]].assert_eq(&lint(&fixture, ""));
}

#[test]
fn bool_assert_comparison() {
    let fixture = format!(
        r#"
//- minicore: copy, unary_ops, bool_impl
//- /src/lib.rs crate:main deps:std
fn f() {{
    assert_eq!(true, 1 > 0);
    assert_ne!(1 > 0, false);
}}
{STD}"#
    );
    expect_test::expect![[r#""#]].assert_eq(&lint(&fixture, ""));
}

//! The model and pages of a fixture workspace with every item kind, re-exports, intra-doc links
//! (one unresolved), a dependency and a proc macro.

use hir::Crate;
use ide_db::RootDatabase;
use test_fixture::WithFixture;

use crate::{
    Asked,
    model::{CrateDoc, Item, Kind, Model, Target},
};

const FIXTURE: &str = r#"
//- proc_macros: identity, derive_identity, mirror
//- minicore: sized, copy, clone, derive, option, iterator, send, sync, unpin, non_null, cell, phantom_data, deref, deref_mut, slice, from
//- /lib.rs crate:fixture@0.3.1,https://example.com/fixture.git deps:dep
//! The fixture crate. See [`Thing`], [`shapes::Square`] and [`missing_item`].
//!
//! # Usage
//!
//! ```
//! let thing = fixture::Thing::new(1);
//! # let hidden = 2;
//! ```

pub mod shapes;

pub use dep::Helper;
pub use dep::nested::*;
pub use proc_macros::DeriveIdentity;
#[doc(no_inline)]
pub use dep::Outside;

mod private {
    /// Reachable only through a re-export.
    pub struct Inner;
}
pub use private::Inner as Shown;
pub use shapes::Corner;

#[doc(hidden)]
pub struct Secret;

/// A thing. Make one with [Self::new]; see [`Kind::A`], [`free()`], [dep::Helper], [dep::Deep],
/// [`Option`] and [the shape trait](Shape).
#[derive(Clone)]
pub struct Thing<T: Clone, const N: usize> {
    /// The value.
    pub value: T,
    hidden: [u8; N],
}

impl<T: Clone, const N: usize> Thing<T, N> {
    /// Makes one.
    pub fn new(value: T) -> Self {
        loop {}
    }

    pub(crate) fn private_helper(&self) {}
}

/// Kinds of things.
#[derive(Clone)]
pub enum Kind {
    /// The first.
    A,
    B(u32),
    C {
        /// A field of a variant.
        x: i32,
    },
}

/// Either one.
pub union Bits {
    pub int: u32,
    pub float: f32,
}

/// A shape.
pub trait Shape: Clone {
    /// What it makes.
    type Out;
    const SIDES: usize;
    /// Its area.
    fn area(&self) -> u32;
    fn name(&self) -> &'static str {
        "shape"
    }
}

impl Shape for Kind {
    type Out = u8;
    const SIDES: usize = 3;
    fn area(&self) -> u32 {
        0
    }
}

/// Free. Unresolved: [Nope].
pub fn free<'a, I>(input: &'a str, items: I) -> impl Iterator<Item = u8> + 'a
where
    I: Iterator<Item = u32> + Clone,
{
    loop {}
}

pub unsafe extern "C" fn ffi(ptr: *const u8, len: usize) -> Option<&'static [u8]> {
    loop {}
}

/// An alias.
pub type Alias<T> = Option<T>;

/// An alias with its argument.
pub type Maybe = Option<shapes::Square>;

/// The most.
pub const MAX: u32 = 10;

pub static mut COUNT: u32 = 0;

/// Holds a raw pointer, so is neither `Send` nor `Sync`.
pub struct Raw(*const u8);

/// Derefs to a `Chain`, and through it to bytes.
pub struct Bytes([u8; 4]);
impl core::ops::Deref for Bytes {
    type Target = Chain;
    fn deref(&self) -> &Chain {
        loop {}
    }
}

/// A link in a chain of derefs.
pub struct Chain;
impl Chain {
    /// Reads.
    pub fn read(&self) {}
    /// Writes, which needs `DerefMut`.
    pub fn write(&mut self) {}
    /// Consumes, which needs `Chain: Copy`.
    pub fn take(self) {}
    /// Makes one, with no receiver.
    pub fn new() -> Chain {
        Chain
    }
}
impl core::ops::Deref for Chain {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        loop {}
    }
}

pub struct NotSend;
impl !Send for NotSend {}
/// Neither `Send`, from its field, nor `Sync`, from its parameter's.
pub struct Wraps<T>(NotSend, T);

/// Makes a thing.
#[macro_export]
macro_rules! make {
    ($value:expr) => { $value };
    () => {};
}

#[deprecated(since = "1.0", note = "use `free`")]
pub fn old() {}

#[must_use]
pub fn important() -> u32 {
    0
}

/// Yields what it holds.
pub trait Each {
    type Item;
    /// Only where the items are `Copy`.
    fn copied(self)
    where
        Self::Item: Copy,
        Self: Sized,
    {
    }
}
/// Yields squares, which are not `Copy`.
pub struct Squares;
impl Each for Squares {
    type Item = shapes::Square;
}
/// Yields helpers, not `Copy` either, though their crate could make them so.
pub struct Helpers;
impl Each for Helpers {
    type Item = dep::Helper;
}

//- /shapes.rs
/// A square.
pub struct Square;

impl Square {
    /// Its side.
    pub const SIDE: u32 = 1;
}

mod corner {
    /// Documented in `shapes`, which re-exports it publicly.
    pub struct Corner;
}
pub use corner::Corner;

//- /dep.rs crate:dep@1.2.0,https://example.com/dep.git
/// Helps.
pub struct Helper;

pub struct Outside;

pub mod nested {
    /// Found through a glob.
    pub fn glob_fn() {}
}

mod deep {
    /// Declared in a private module, documented at the root.
    pub struct Deep;
}
pub use deep::Deep;
"#;

fn load() -> (RootDatabase, Model) {
    let mut db = RootDatabase::with_files(FIXTURE);
    let fixture = Crate::all(&db)
        .into_iter()
        .find(|it| crate::crate_name(&db, *it) == "fixture")
        .expect("the fixture crate");
    let crates = crate::crates_to_document(&db, &[Asked { krate: fixture, private: false }], false);
    crate::set_cfg_doc(&mut db, &crates);
    let model = crate::model(&db, crates);
    (db, model)
}

fn krate<'a>(model: &'a Model, name: &str) -> &'a CrateDoc {
    model.krate(name).unwrap_or_else(|| panic!("crate {name} documented"))
}

/// The item at `path`, written from the crate's name.
fn item<'a>(krate: &'a CrateDoc, path: &str) -> &'a Item {
    let inner = path.strip_prefix(krate.name.as_str()).unwrap_or(path);
    let inner = inner.strip_prefix("::").unwrap_or(inner);
    krate.find(inner).unwrap_or_else(|| {
        let all: Vec<_> = krate.items.iter().map(|it| it.full_path()).collect();
        panic!("{path} documented; the crate has {all:?}")
    })
}

fn link<'a>(item: &'a Item, text: &str) -> &'a Target {
    item.docs
        .links
        .get(text)
        .unwrap_or_else(|| panic!("link {text} resolved; resolved are {:?}", item.docs.links))
}

fn page(target: &Target) -> String {
    match target {
        Target::Item(it) => match &it.anchor {
            Some(anchor) => format!("{}#{anchor}", it.page()),
            None => it.page(),
        },
        Target::Url(url) => url.clone(),
    }
}

#[test]
fn documents_the_crate_and_its_dependencies() {
    let (_db, model) = load();
    let names: Vec<_> = model.crates.iter().map(|it| it.name.as_str()).collect();
    assert_eq!(names[0], "fixture");
    assert!(names.contains(&"dep"), "{names:?}");
    assert!(names.contains(&"proc_macros"), "{names:?}");
    assert!(!names.contains(&"core"), "{names:?}");
    assert_eq!(krate(&model, "fixture").version.as_deref(), Some("0.3.1"));
}

#[test]
fn every_item_kind_has_its_page() {
    let (_db, model) = load();
    let fixture = krate(&model, "fixture");
    for (path, kind) in [
        ("fixture", Kind::Module),
        ("fixture::shapes", Kind::Module),
        ("fixture::shapes::Square", Kind::Struct),
        ("fixture::Thing", Kind::Struct),
        ("fixture::Kind", Kind::Enum),
        ("fixture::Bits", Kind::Union),
        ("fixture::Shape", Kind::Trait),
        ("fixture::free", Kind::Function),
        ("fixture::ffi", Kind::Function),
        ("fixture::Alias", Kind::TypeAlias),
        ("fixture::MAX", Kind::Constant),
        ("fixture::COUNT", Kind::Static),
        ("fixture::make", Kind::Macro),
        ("fixture::old", Kind::Function),
        ("fixture::important", Kind::Function),
    ] {
        assert_eq!(item(fixture, path).kind, kind, "{path}");
    }
    assert!(fixture.find("Secret").is_none(), "doc(hidden) items have no page");
}

#[test]
fn reexports_inline_as_rustdoc_does() {
    let (_db, model) = load();
    let fixture = krate(&model, "fixture");
    assert_eq!(item(fixture, "fixture::Helper").kind, Kind::Struct, "a foreign item is inlined");
    assert_eq!(item(fixture, "fixture::glob_fn").kind, Kind::Function, "a foreign glob is inlined");
    assert_eq!(item(fixture, "fixture::DeriveIdentity").kind, Kind::ProcDerive);
    assert_eq!(
        item(fixture, "fixture::Shown").kind,
        Kind::Struct,
        "an unreachable item is inlined"
    );
    assert!(fixture.find("Outside").is_none(), "doc(no_inline) stays a line");
    let root = fixture.root();
    let lines: Vec<String> = root.reexports.iter().map(|it| it.decl.plain()).collect();
    assert!(lines.iter().any(|it| it == "pub use dep::Outside;"), "{lines:?}");
    assert!(item(fixture, "fixture::Helper").docs.markdown.contains("Helps."));
    // `shapes` re-exports it publicly, so the root's `use` of that re-export stays a line.
    assert_eq!(item(fixture, "fixture::shapes::Corner").kind, Kind::Struct);
    assert!(fixture.find("Corner").is_none(), "re-exported through a public re-export: {lines:?}");
    assert!(lines.iter().any(|it| it == "pub use fixture::shapes::Corner;"), "{lines:?}");
}

#[test]
fn aliases_show_the_type_they_stand_for() {
    let (_db, model) = load();
    let fixture = krate(&model, "fixture");
    let alias = item(fixture, "fixture::Alias");
    let aliased = alias.aliased.as_ref().expect("an alias of an enum shows it").plain();
    assert!(aliased.starts_with("pub enum Alias<T> {"), "{aliased}");
    let variants: Vec<String> = alias.variants.iter().map(|it| it.decl.plain()).collect();
    assert_eq!(variants, ["None", "Some(T)"]);
    let maybe = item(fixture, "fixture::Maybe");
    let aliased = maybe.aliased.as_ref().expect("an alias of an enum shows it").plain();
    assert!(aliased.contains("Some(Square)"), "the alias's argument for the parameter: {aliased}");
    let variants: Vec<String> = maybe.variants.iter().map(|it| it.decl.plain()).collect();
    assert_eq!(variants, ["None", "Some(Square)"]);
}

#[test]
fn signatures_come_from_the_hir() {
    let (_db, model) = load();
    let fixture = krate(&model, "fixture");
    assert_eq!(
        item(fixture, "fixture::free").decl.plain(),
        "pub fn free<'a, I>(input: &'a str, items: I) -> impl Iterator<Item = u8> + 'a\nwhere\n    I: Iterator<Item = u32> + Clone,"
    );
    assert_eq!(
        item(fixture, "fixture::ffi").decl.plain(),
        "pub unsafe extern \"C\" fn ffi(ptr: *const u8, len: usize) -> Option<&'static [u8]>"
    );
    let thing = item(fixture, "fixture::Thing").decl.plain();
    assert!(thing.starts_with("pub struct Thing<T: Clone, const N: usize> {"), "{thing}");
    assert!(thing.contains("pub value: T,"), "{thing}");
    assert!(thing.contains("/* private fields */"), "{thing}");
    assert_eq!(item(fixture, "fixture::Alias").decl.plain(), "pub type Alias<T> = Option<T>;");
    assert_eq!(item(fixture, "fixture::MAX").decl.plain(), "pub const MAX: u32 = 10;");
    assert_eq!(item(fixture, "fixture::COUNT").decl.plain(), "pub static mut COUNT: u32");
    let make = item(fixture, "fixture::make").decl.plain();
    assert!(make.starts_with("macro_rules! make {"), "{make}");
    assert!(make.contains("($value:expr) => { ... };"), "{make}");
    let shape = item(fixture, "fixture::Shape").decl.plain();
    assert!(shape.starts_with("pub trait Shape: Clone {"), "{shape}");
    assert!(shape.contains("fn area(&self) -> u32;"), "{shape}");
    assert!(shape.contains("fn name(&self) -> &'static str { ... }"), "{shape}");
}

#[test]
fn signature_names_link_to_their_pages() {
    let (_db, model) = load();
    let fixture = krate(&model, "fixture");
    let ffi = item(fixture, "fixture::ffi");
    let targets: Vec<String> = ffi.decl.links().map(|(_, to)| page(to)).collect();
    assert!(targets.iter().any(|it| it == "core/option/enum.Option.html"), "{targets:?}");
    assert!(targets.iter().any(|it| it == "std/primitive.u8.html"), "{targets:?}");
}

#[test]
fn members_and_impls() {
    let (_db, model) = load();
    let fixture = krate(&model, "fixture");
    let thing = item(fixture, "fixture::Thing");
    assert_eq!(thing.fields.iter().map(|it| it.name.as_str()).collect::<Vec<_>>(), ["value"]);
    assert!(thing.hidden_fields);
    let inherent: Vec<&str> = thing
        .impls
        .inherent
        .iter()
        .flat_map(|it| it.items.iter().map(|it| it.name.as_str()))
        .collect();
    assert_eq!(inherent, ["new"], "a pub(crate) method is not shown");
    let traits: Vec<String> = thing.impls.traits.iter().map(|it| it.header.plain()).collect();
    assert!(traits.iter().any(|it| it.contains("Clone for Thing<T, N>")), "{traits:?}");
    let auto: Vec<String> = thing.impls.auto.iter().map(|it| it.header.plain()).collect();
    assert!(
        auto.contains(
            &"impl<T, const N: usize> Send for Thing<T, N>\nwhere\n    T: Send,".to_owned()
        ),
        "{auto:?}"
    );
    assert!(
        auto.contains(
            &"impl<T, const N: usize> Sync for Thing<T, N>\nwhere\n    T: Sync,".to_owned()
        ),
        "{auto:?}"
    );
    let blanket: Vec<String> = thing.impls.blanket.iter().map(|it| it.header.plain()).collect();
    assert!(blanket.contains(&"impl<T> From<T> for T".to_owned()), "{blanket:?}");
    assert!(blanket.iter().any(|it| it.contains("Into<U> for T")), "{blanket:?}");
    assert!(!blanket.iter().any(|it| it.contains("IntoIterator")), "{blanket:?}");
    let raw = item(fixture, "fixture::Raw");
    let auto: Vec<String> = raw.impls.auto.iter().map(|it| it.header.plain()).collect();
    assert!(auto.contains(&"impl !Send for Raw".to_owned()), "{auto:?}");
    assert!(auto.contains(&"impl !Sync for Raw".to_owned()), "{auto:?}");
    let wraps = item(fixture, "fixture::Wraps");
    let auto: Vec<String> = wraps.impls.auto.iter().map(|it| it.header.plain()).collect();
    assert!(auto.contains(&"impl<T> !Send for Wraps<T>".to_owned()), "{auto:?}");
    assert!(
        auto.contains(&"impl<T> Sync for Wraps<T>\nwhere\n    T: Sync,".to_owned()),
        "{auto:?}"
    );

    let kind = item(fixture, "fixture::Kind");
    let variants: Vec<&str> = kind.variants.iter().map(|it| it.name.as_str()).collect();
    assert_eq!(variants, ["A", "B", "C"]);
    assert_eq!(kind.variants[2].fields[0].name, "x");
    let shape_impl = kind
        .impls
        .traits
        .iter()
        .find(|it| it.trait_name.as_deref() == Some("Shape"))
        .expect("impl Shape");
    let names: Vec<&str> = shape_impl.items.iter().map(|it| it.name.as_str()).collect();
    for name in ["Out", "SIDES", "area", "name"] {
        assert!(names.contains(&name), "{name} in {names:?}");
    }
    assert!(shape_impl.items.iter().find(|it| it.name == "name").unwrap().provided);

    let shape = item(fixture, "fixture::Shape");
    let implementors: Vec<String> =
        shape.impls.implementors.iter().map(|it| it.header.plain()).collect();
    assert_eq!(implementors, ["impl Shape for Kind"]);
    let members: Vec<&str> = shape.trait_items.iter().map(|it| it.name.as_str()).collect();
    assert_eq!(members, ["Out", "SIDES", "area", "name"]);

    let square = item(fixture, "fixture::shapes::Square");
    assert_eq!(square.impls.inherent[0].items[0].name, "SIDE");
}

#[test]
fn provided_methods_that_cannot_apply_are_left_out() {
    let (_db, model) = load();
    let fixture = krate(&model, "fixture");
    let members = |path: &str| -> Vec<String> {
        let each = item(fixture, path)
            .impls
            .traits
            .iter()
            .find(|it| it.trait_name.as_deref() == Some("Each"));
        each.expect("impl Each").items.iter().map(|it| it.name.clone()).collect()
    };
    assert_eq!(members("fixture::Squares"), ["Item"], "a square of this crate is not Copy");
    assert_eq!(members("fixture::Helpers"), ["Item", "copied"], "dep may make a helper Copy");
}

#[test]
fn notes() {
    let (_db, model) = load();
    let fixture = krate(&model, "fixture");
    let old = item(fixture, "fixture::old").notes.deprecated.clone().expect("deprecated");
    assert_eq!(old.since.as_deref(), Some("1.0"));
    assert_eq!(old.note.as_deref(), Some("use `free`"));
    assert!(item(fixture, "fixture::important").notes.must_use.is_some());
    assert!(item(fixture, "fixture::ffi").notes.is_unsafe);
}

#[test]
fn intra_doc_links_resolve() {
    let (_db, model) = load();
    let fixture = krate(&model, "fixture");
    let root = fixture.root();
    assert_eq!(page(link(root, "`Thing`")), "fixture/struct.Thing.html");
    assert_eq!(page(link(root, "`shapes::Square`")), "fixture/shapes/struct.Square.html");
    let thing = item(fixture, "fixture::Thing");
    assert_eq!(page(link(thing, "Self::new")), "fixture/struct.Thing.html#method.new");
    assert_eq!(page(link(thing, "`Kind::A`")), "fixture/enum.Kind.html#variant.A");
    assert_eq!(page(link(thing, "`free()`")), "fixture/fn.free.html");
    // An item the crate inlined links to its page there, another crate's to its own.
    assert_eq!(page(link(thing, "dep::Helper")), "fixture/struct.Helper.html");
    assert_eq!(page(link(thing, "dep::Deep")), "dep/struct.Deep.html");
    assert_eq!(page(link(thing, "`Option`")), "core/option/enum.Option.html");
    assert_eq!(page(link(thing, "Shape")), "fixture/trait.Shape.html");
}

#[test]
fn unresolved_links_warn() {
    let (_db, model) = load();
    let messages: Vec<&str> = model.warnings.iter().map(|it| it.message.as_str()).collect();
    assert!(
        messages.iter().any(|it| it.contains("unresolved link to `missing_item`")),
        "{messages:?}"
    );
    assert!(messages.iter().any(|it| it.contains("unresolved link to `Nope`")), "{messages:?}");
    assert!(model.warnings.iter().all(|it| it.krate == "fixture"), "dependencies do not warn");
}

#[test]
fn writes_pages_sources_and_search() {
    let (_db, model) = load();
    let out = std::env::temp_dir().join(format!("blaze-doc-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    let written = crate::html::write(&model, &out).expect("written");
    assert!(written.pages > 20, "{written:?}");
    assert!(written.source_pages >= 3, "{written:?}");
    for path in [
        "fixture/index.html",
        "fixture/struct.Thing.html",
        "fixture/trait.Shape.html",
        "fixture/macro.make.html",
        "fixture/shapes/index.html",
        "fixture/shapes/struct.Square.html",
        "fixture/sidebar-items.js",
        "dep/struct.Helper.html",
        "src/fixture/lib.rs.html",
        "search-index/fixture.js",
        "crates.js",
        "fixture/all.html",
    ] {
        assert!(out.join(path).is_file(), "{path} written");
    }
    let thing = std::fs::read_to_string(out.join("fixture/struct.Thing.html")).unwrap();
    assert!(thing.contains("id=\"method.new\""), "a member anchor");
    assert!(
        thing.contains("href=\"../fixture/struct.Helper.html\" title=\"struct fixture::Helper\""),
        "an inlined item's page"
    );
    assert!(
        thing.contains("href=\"../dep/struct.Deep.html\" title=\"struct dep::deep::Deep\""),
        "a dependency's page, titled where the item is declared"
    );
    assert!(thing.contains("https://doc.rust-lang.org/nightly/core/option/enum.Option.html"));
    let index = text(&std::fs::read_to_string(out.join("fixture/index.html")).unwrap());
    assert!(index.contains("let thing = fixture::Thing::new(1);"), "the example is there: {index}");
    assert!(!index.contains("let hidden"), "hidden example lines are not");
    let _ = std::fs::remove_dir_all(&out);
}

/// The text of an HTML page, its tags dropped.
fn text(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

#[test]
fn impl_ids_follow_rustdoc() {
    let id = |text: &str| {
        let mut code = crate::model::Code::default();
        code.text(text);
        crate::collect::impl_anchor(&code)
    };
    assert_eq!(
        id("impl<'a, 'h, A: Debug> Debug for FindIter<'a, 'h, A>"),
        "impl-Debug-for-FindIter%3C'a,+'h,+A%3E"
    );
    assert_eq!(id("impl<T> From<T> for T"), "impl-From%3CT%3E-for-T");
    assert_eq!(id("impl<T> !Send for Wraps<T>"), "impl-Send-for-Wraps%3CT%3E");
    assert_eq!(id("impl<F: Fn() -> u8> Run<F>"), "impl-Run%3CF%3E");
    assert_eq!(
        id("impl<'de: 'a, 'a> Deserialize<'de> for &'a [u8]"),
        "impl-Deserialize%3C'de%3E-for-%26%5Bu8%5D"
    );
    assert_eq!(
        id("impl From<Error> for Box<dyn Error + Send>"),
        "impl-From%3CError%3E-for-Box%3Cdyn+Error+%2B+Send%3E"
    );
    assert_eq!(
        id("impl<'a> Iterator for Iter<'a>\nwhere\n    'a: 'static,"),
        "impl-Iterator-for-Iter%3C'a%3E"
    );
}

#[test]
fn deref_methods_follow_the_chain() {
    let (_db, model) = load();
    let fixture = krate(&model, "fixture");
    let bytes = item(fixture, "fixture::Bytes");
    let sections: Vec<(String, Vec<&str>)> = bytes
        .impls
        .deref
        .iter()
        .map(|it| (it.target.plain(), it.members.iter().map(|m| m.name.as_str()).collect()))
        .collect();
    assert_eq!(sections, [("Chain".to_owned(), vec!["read"]), ("[u8]".to_owned(), vec!["len"])]);
    assert_eq!(bytes.impls.deref[1].anchor, "deref-methods-%5Bu8%5D");
}

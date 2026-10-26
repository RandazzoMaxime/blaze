//! Where each item is documented: the module tree of every crate documented, walked as rustdoc
//! walks it, with `pub use` either inlined (the item gets a page where it is re-exported) or
//! left as a line under "Re-exports", and the one page every other page links to for an item.
//!
//! The rules are rustdoc's (`clean::clean_use_statement` and `visit_ast` in librustdoc at rust
//! 12433c307b5a497388ffe2f4e49dbf1ab8c71fa0): a re-export of another crate's item is inlined
//! unless `#[doc(no_inline)]`; one of this crate's is inlined when `#[doc(inline)]` asks or the
//! item cannot be reached where it is declared; a glob from a module that cannot be reached is
//! inlined item by item; `#[doc(hidden)]` drops the item or the re-export.

use std::sync::Mutex;

use hir::{
    AsAssocItem, AssocItem, AssocItemContainer, Crate, HasAttrs, HasVisibility, Macro, MacroKind,
    Module, ModuleDef, Visibility,
};
use hir_def::{
    item_scope::{ImportOrExternCrate, ImportOrGlob},
    per_ns::PerNs,
};
use ide_db::{RootDatabase, base_db::LangCrateOrigin};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::model::{ItemRef, Kind, Target};

/// A crate whose documentation is written.
#[derive(Debug, Clone)]
pub struct Documented {
    pub krate: Crate,
    /// The name code uses, the directory under `doc/`.
    pub name: String,
    /// Items not public are documented too.
    pub private: bool,
    /// Warnings about its docs are shown: one the user asked for rather than a dependency, whose
    /// lints cargo caps.
    pub warn: bool,
}

/// One item where a module page lists it.
#[derive(Debug, Clone)]
pub(crate) struct Placed {
    pub(crate) def: ModuleDef,
    pub(crate) kind: Kind,
    pub(crate) name: String,
    /// The modules from the crate root down to the one listing it.
    pub(crate) modules: Vec<String>,
    /// A module's items, as indices into the crate's placements.
    pub(crate) children: Vec<usize>,
    /// A module's `pub use` lines that are not inlined.
    pub(crate) reexports: Vec<Line>,
    /// The `use` that inlined the item, whose docs rustdoc puts before the item's own.
    pub(crate) via: Option<hir_def::UseId>,
}

/// A `pub use` or `pub extern crate` shown as itself.
#[derive(Debug, Clone)]
pub(crate) enum Line {
    Use { name: String, target: ModuleDef, glob: bool, use_id: hir_def::UseId },
    ExternCrate { name: String, krate: Crate },
}

pub(crate) struct Places {
    pub(crate) crates: Vec<Documented>,
    /// Each crate's placements; `[0]` is its root module.
    pub(crate) placed: Vec<Vec<Placed>>,
    canonical: FxHashMap<ModuleDef, (usize, usize)>,
    /// The path of an item of `std`, `core` or `alloc` on doc.rust-lang.org, found once.
    sysroot: Mutex<FxHashMap<ModuleDef, Option<ItemRef>>>,
}

impl Places {
    /// The page an item placed in crate `krate` at `at` is on.
    pub(crate) fn placed_ref(&self, krate: usize, at: usize) -> ItemRef {
        let placed = &self.placed[krate][at];
        ItemRef {
            krate: self.crates[krate].name.clone(),
            modules: placed.modules.clone(),
            kind: placed.kind,
            name: placed.name.clone(),
            anchor: None,
            descr: None,
        }
    }

    /// Where the documentation of `def` is, for a link to it: the page in the docs written, or
    /// on doc.rust-lang.org for `std` and the crates under it, or nowhere.
    pub(crate) fn target(&self, db: &RootDatabase, def: ModuleDef) -> Option<Target> {
        match def {
            ModuleDef::BuiltinType(builtin) => Some(Target::Item(primitive(builtin.name().as_str()))),
            ModuleDef::EnumVariant(variant) => {
                let mut parent = self.item_ref(db, ModuleDef::Adt(variant.parent_enum(db).into()))?;
                parent.anchor = Some(format!("variant.{}", variant.name(db).as_str()));
                parent.descr = Some("variant");
                Some(Target::Item(parent))
            }
            ModuleDef::Function(it) => match it.as_assoc_item(db) {
                Some(assoc) => self.assoc_target(db, assoc),
                None => self.item_ref(db, def).map(Target::Item),
            },
            ModuleDef::Const(it) => match it.as_assoc_item(db) {
                Some(assoc) => self.assoc_target(db, assoc),
                None => self.item_ref(db, def).map(Target::Item),
            },
            ModuleDef::TypeAlias(it) => match it.as_assoc_item(db) {
                Some(assoc) => self.assoc_target(db, assoc),
                None => self.item_ref(db, def).map(Target::Item),
            },
            _ => self.item_ref(db, def).map(Target::Item),
        }
    }

    pub(crate) fn field_target(&self, db: &RootDatabase, field: hir::Field) -> Option<Target> {
        let name = field.name(db);
        let mut parent = match field.parent_def(db) {
            hir::Variant::Struct(it) => self.item_ref(db, ModuleDef::Adt(it.into()))?,
            hir::Variant::Union(it) => self.item_ref(db, ModuleDef::Adt(it.into()))?,
            hir::Variant::EnumVariant(variant) => {
                let mut parent =
                    self.item_ref(db, ModuleDef::Adt(variant.parent_enum(db).into()))?;
                parent.anchor = Some(format!(
                    "variant.{}.field.{}",
                    variant.name(db).as_str(),
                    name.as_str()
                ));
                parent.descr = Some("field");
                return Some(Target::Item(parent));
            }
        };
        parent.anchor = Some(format!("structfield.{}", name.as_str()));
        parent.descr = Some("field");
        Some(Target::Item(parent))
    }

    pub(crate) fn assoc_target(&self, db: &RootDatabase, assoc: AssocItem) -> Option<Target> {
        let (mut parent, in_trait) = match assoc.container(db) {
            AssocItemContainer::Trait(it) => (self.item_ref(db, ModuleDef::Trait(it))?, true),
            AssocItemContainer::Impl(it) => {
                let ty = it.self_ty(db);
                let parent = if let Some(adt) = ty.as_adt() {
                    self.item_ref(db, ModuleDef::Adt(adt))?
                } else if let Some(builtin) = ty.as_builtin() {
                    primitive(builtin.name().as_str())
                } else {
                    return None;
                };
                (parent, false)
            }
        };
        let name = assoc.name(db)?;
        let anchor = match assoc {
            AssocItem::Function(f) if in_trait && !f.has_body(db) => "tymethod",
            AssocItem::Function(_) => "method",
            AssocItem::Const(_) => "associatedconstant",
            AssocItem::TypeAlias(_) => "associatedtype",
        };
        parent.anchor = Some(format!("{anchor}.{}", name.as_str()));
        parent.descr = Some(match assoc {
            AssocItem::Function(f) if f.has_self_param(db) => "method",
            AssocItem::Function(_) => "associated function",
            AssocItem::Const(_) => "associated constant",
            AssocItem::TypeAlias(_) => "associated type",
        });
        Some(Target::Item(parent))
    }

    pub(crate) fn item_ref(&self, db: &RootDatabase, def: ModuleDef) -> Option<ItemRef> {
        if let Some(&(krate, at)) = self.canonical.get(&def) {
            return Some(self.placed_ref(krate, at));
        }
        let krate = krate_of(db, def)?;
        let name = lang_crate_name(db, krate)?;
        if let Some(found) = self.sysroot.lock().unwrap().get(&def) {
            return found.clone();
        }
        let found = sysroot_ref(db, def, name);
        self.sysroot.lock().unwrap().insert(def, found.clone());
        found
    }
}

pub(crate) fn primitive(name: &str) -> ItemRef {
    ItemRef {
        krate: "std".to_owned(),
        modules: vec![],
        kind: Kind::Primitive,
        name: name.to_owned(),
        anchor: None,
        descr: None,
    }
}

/// The name of a crate doc.rust-lang.org documents, as its URLs have it.
pub(crate) fn lang_crate_name(db: &RootDatabase, krate: Crate) -> Option<&'static str> {
    match krate.origin(db) {
        ide_db::base_db::CrateOrigin::Lang(lang) => match lang {
            LangCrateOrigin::Core => Some("core"),
            LangCrateOrigin::Alloc => Some("alloc"),
            LangCrateOrigin::Std => Some("std"),
            LangCrateOrigin::ProcMacro => Some("proc_macro"),
            LangCrateOrigin::Test => Some("test"),
            LangCrateOrigin::Dependency | LangCrateOrigin::Other => None,
        },
        _ => None,
    }
}

/// The page of an item of a sysroot crate where it is defined, as rustdoc links it: under a
/// private module that is a page doc.rust-lang.org redirects to where it is exported. `None`
/// for an item the crate does not export, or only hidden.
fn sysroot_ref(db: &RootDatabase, def: ModuleDef, krate_name: &str) -> Option<ItemRef> {
    let kind = kind_of(db, def)?;
    if let ModuleDef::Module(it) = def
        && it.is_crate_root(db)
    {
        let krate = krate_name.to_owned();
        return Some(ItemRef { krate, modules: vec![], kind, name: String::new(), anchor: None, descr: None });
    }
    let name = def.name(db)?.as_str().to_owned();
    // The module whose scope declares it; a module's parent for a module.
    let module = def.module(db)?;
    let mut modules = vec![];
    let mut reachable = def.visibility(db) == Visibility::Public && !def.attrs(db).is_doc_hidden();
    let mut at = Some(module);
    while let Some(m) = at {
        let Some(parent) = m.parent(db) else { break };
        if m.visibility(db) != Visibility::Public || m.attrs(db).is_doc_hidden() {
            reachable = false;
        }
        modules.push(m.name(db)?.as_str().to_owned());
        at = Some(parent);
    }
    modules.reverse();
    if !reachable && def.attrs(db).is_doc_hidden() {
        return None;
    }
    if !reachable {
        let root = module.krate(db).root_module(db);
        let config = hir::FindPathConfig {
            prefer_no_std: false,
            prefer_prelude: false,
            prefer_absolute: false,
            allow_unstable: true,
        };
        let path = root.find_path(db, def, config)?;
        let segments = path.segments();
        let (_, parents) = segments.split_last()?;
        let mut at = root;
        for segment in parents {
            if segment.as_str() == "prelude" {
                return None;
            }
            let Some(ModuleDef::Module(next)) = at.scope(db, None).into_iter().find_map(|(name, def)| match def {
                hir::ScopeDef::ModuleDef(def) if name == *segment => Some(def),
                _ => None,
            }) else {
                return None;
            };
            if next.attrs(db).is_doc_hidden() {
                return None;
            }
            at = next;
        }
    }
    Some(ItemRef { krate: krate_name.to_owned(), modules, kind, name, anchor: None, descr: None })
}

/// The kind of page rustdoc gives an item, or none for an item that has no page of its own.
pub(crate) fn kind_of(db: &RootDatabase, def: ModuleDef) -> Option<Kind> {
    Some(match def {
        ModuleDef::Module(_) => Kind::Module,
        ModuleDef::Function(_) => Kind::Function,
        ModuleDef::Adt(hir::Adt::Struct(_)) => Kind::Struct,
        ModuleDef::Adt(hir::Adt::Enum(_)) => Kind::Enum,
        ModuleDef::Adt(hir::Adt::Union(_)) => Kind::Union,
        ModuleDef::Const(_) => Kind::Constant,
        ModuleDef::Static(_) => Kind::Static,
        ModuleDef::Trait(it) => {
            if trait_is_alias(db, it) {
                Kind::TraitAlias
            } else {
                Kind::Trait
            }
        }
        ModuleDef::TypeAlias(_) => Kind::TypeAlias,
        ModuleDef::Macro(it) => macro_kind(db, it)?,
        ModuleDef::BuiltinType(_) => Kind::Primitive,
        ModuleDef::EnumVariant(_) => return None,
    })
}

pub(crate) fn trait_is_alias(db: &RootDatabase, it: hir::Trait) -> bool {
    let id: hir_def::TraitId = it.into();
    hir_def::signatures::TraitSignature::of(db, id)
        .flags
        .contains(hir_def::signatures::TraitFlags::ALIAS)
}

pub(crate) fn macro_kind(db: &RootDatabase, it: Macro) -> Option<Kind> {
    Some(match it.kind(db) {
        MacroKind::Declarative | MacroKind::ProcMacro => Kind::Macro,
        MacroKind::Derive => Kind::ProcDerive,
        MacroKind::Attr => Kind::ProcAttribute,
        MacroKind::DeclarativeBuiltIn | MacroKind::DeriveBuiltIn | MacroKind::AttrBuiltIn => {
            return None;
        }
    })
}

/// Places every item of every crate, in parallel over the crates.
pub(crate) fn place(db: &RootDatabase, crates: Vec<Documented>) -> Places {
    use rayon::prelude::*;
    let placed: Vec<Vec<Placed>> = crates
        .par_iter()
        .map_with(db.clone(), |db, documented| {
            hir::attach_db(db, || Walker::new(db, documented).walk())
        })
        .collect();
    let mut canonical: FxHashMap<ModuleDef, ((u8, usize), (usize, usize))> = FxHashMap::default();
    for (krate, list) in placed.iter().enumerate() {
        for (at, placed) in list.iter().enumerate() {
            let own = krate_of(db, placed.def) == Some(crates[krate].krate);
            let rank = match (own, placed.via.is_none()) {
                (true, true) => 0,
                (true, false) => 1,
                (false, _) => 2,
            };
            let key = (rank, placed.modules.len());
            canonical
                .entry(placed.def)
                .and_modify(|best| {
                    if key < best.0 {
                        *best = (key, (krate, at));
                    }
                })
                .or_insert((key, (krate, at)));
        }
    }
    Places {
        crates,
        placed,
        canonical: canonical.into_iter().map(|(def, (_, at))| (def, at)).collect(),
        sysroot: Mutex::default(),
    }
}

struct Walker<'a> {
    db: &'a RootDatabase,
    krate: Crate,
    private: bool,
    out: Vec<Placed>,
    /// Modules being walked, so a module that re-exports its parent does not loop.
    walking: FxHashSet<Module>,
}

impl<'a> Walker<'a> {
    fn new(db: &'a RootDatabase, documented: &Documented) -> Self {
        Walker {
            db,
            krate: documented.krate,
            private: documented.private,
            out: vec![],
            walking: FxHashSet::default(),
        }
    }

    fn walk(mut self) -> Vec<Placed> {
        let root = self.krate.root_module(self.db);
        self.out.push(Placed {
            def: ModuleDef::Module(root),
            kind: Kind::Module,
            name: String::new(),
            modules: vec![],
            children: vec![],
            reexports: vec![],
            via: None,
        });
        self.module(root, 0, vec![], true);
        self.out
    }

    /// Whether an item declared in this crate is shown: public or documenting private items,
    /// and not `#[doc(hidden)]`.
    fn shown(&self, def: ModuleDef) -> bool {
        if def.attrs(self.db).is_doc_hidden() {
            return false;
        }
        self.private || def.visibility(self.db) == Visibility::Public
    }

    /// Whether `def`, declared in this crate, is documented where it is declared: shown, and in
    /// modules all shown down from the root.
    fn reachable_as_declared(&self, def: ModuleDef) -> bool {
        let db = self.db;
        if !self.shown(def) {
            return false;
        }
        if let ModuleDef::Macro(it) = def
            && is_macro_export(db, it)
        {
            return true;
        }
        // The module whose scope declares it; none for the crate root.
        let Some(mut module) = def.module(db) else { return true };
        loop {
            let Some(parent) = module.parent(db) else { return true };
            if !self.shown(ModuleDef::Module(module)) {
                return false;
            }
            module = parent;
        }
    }

    /// Lists the items of `module` under the placement `at`. `local` is false inside a module
    /// of another crate that is inlined, whose public items all are.
    fn module(&mut self, module: Module, at: usize, path: Vec<String>, local: bool) {
        if !self.walking.insert(module) {
            return;
        }
        let db = self.db;
        let id: hir_def::ModuleId = module.into();
        let scope = id.def_map(db).module(db, id).scope(db);
        let mut seen_globs = FxHashSet::default();
        let entries: Vec<(hir::Name, PerNs)> =
            scope.entries().map(|(name, per_ns)| (name.clone(), per_ns)).collect();
        for (name, per_ns) in entries {
            let mut defs: Vec<(ModuleDef, Option<ImportOrExternCrate>, bool)> = vec![];
            if let Some(item) = per_ns.types {
                defs.push((item.def.into(), item.import, item.vis == hir_def::visibility::Visibility::Public));
            }
            if let Some(item) = per_ns.values {
                defs.push((
                    item.def.into(),
                    item.import.map(|it| match it {
                        ImportOrGlob::Glob(it) => ImportOrExternCrate::Glob(it),
                        ImportOrGlob::Import(it) => ImportOrExternCrate::Import(it),
                    }),
                    item.vis == hir_def::visibility::Visibility::Public,
                ));
            }
            if let Some(item) = per_ns.macros {
                defs.push((
                    ModuleDef::Macro(item.def.into()),
                    item.import,
                    item.vis == hir_def::visibility::Visibility::Public,
                ));
            }
            let mut done: FxHashSet<ModuleDef> = FxHashSet::default();
            for (def, import, public) in defs {
                if !done.insert(def) {
                    continue;
                }
                self.entry(module, at, &path, local, &name, def, import, public, &mut seen_globs);
            }
        }
        self.walking.remove(&module);
    }

    #[allow(clippy::too_many_arguments)]
    fn entry(
        &mut self,
        module: Module,
        at: usize,
        path: &[String],
        local: bool,
        name: &hir::Name,
        def: ModuleDef,
        import: Option<ImportOrExternCrate>,
        public: bool,
        seen_globs: &mut FxHashSet<hir_def::item_scope::GlobId>,
    ) {
        let db = self.db;
        if matches!(def, ModuleDef::BuiltinType(_) | ModuleDef::EnumVariant(_)) {
            return;
        }
        if let ModuleDef::Function(it) = def
            && it.as_proc_macro(db).is_some()
        {
            return;
        }
        let Some(kind) = kind_of(db, def) else { return };
        let def_krate = krate_of(db, def);
        let Some(import) = import else {
            // Declared here (or a `#[macro_export]` macro, which the crate root has).
            let declared_here = match def {
                ModuleDef::Macro(it) => {
                    it.module(db) == module || (module.is_crate_root(db) && is_macro_export(db, it))
                }
                _ => def.module(db) == Some(module),
            };
            if !declared_here {
                return;
            }
            let shown = if local {
                self.shown(def)
            } else {
                public && !def.attrs(db).is_doc_hidden()
            };
            if shown {
                self.put(at, path, def, kind, name, None, local);
            }
            return;
        };
        // rustdoc shows no `use` that is not public, private items documented or not.
        if !public {
            return;
        }
        match import {
            ImportOrExternCrate::ExternCrate(extern_crate) => {
                let decl: hir::ExternCrateDecl = extern_crate.into();
                if decl.attrs(db).is_doc_hidden() {
                    return;
                }
                if let ModuleDef::Module(m) = def
                    && m.is_crate_root(db)
                {
                    self.out[at].reexports.push(Line::ExternCrate {
                        name: name.as_str().to_owned(),
                        krate: m.krate(db),
                    });
                }
            }
            ImportOrExternCrate::Import(import) => {
                let use_id = import.use_;
                let use_attrs = UseAttrs::of(db, use_id);
                if use_attrs.hidden || def.attrs(db).is_doc_hidden() {
                    return;
                }
                let foreign = def_krate != Some(self.krate);
                let inline = if !local {
                    true
                } else if use_attrs.no_inline {
                    false
                } else if use_attrs.inline {
                    true
                } else if foreign {
                    !matches!(def, ModuleDef::Module(m) if m.is_crate_root(db))
                } else {
                    !self.reachable_as_declared(def)
                };
                if inline {
                    if foreign && !local_or_public(db, def) {
                        return;
                    }
                    self.put(at, path, def, kind, name, Some(use_id), local && !foreign);
                } else {
                    self.out[at].reexports.push(Line::Use {
                        name: name.as_str().to_owned(),
                        target: def,
                        glob: false,
                        use_id,
                    });
                }
            }
            ImportOrExternCrate::Glob(glob) => {
                let use_id = glob.use_;
                let use_attrs = UseAttrs::of(db, use_id);
                if use_attrs.hidden || def.attrs(db).is_doc_hidden() {
                    return;
                }
                let foreign = def_krate != Some(self.krate);
                let inline = !local
                    || use_attrs.inline
                    || (!use_attrs.no_inline && (foreign || !self.reachable_as_declared(def)));
                if inline {
                    if foreign && !local_or_public(db, def) {
                        return;
                    }
                    self.put(at, path, def, kind, name, Some(use_id), local && !foreign);
                } else if seen_globs.insert(glob) {
                    let target = def.module(db).map(ModuleDef::Module).unwrap_or(def);
                    self.out[at].reexports.push(Line::Use {
                        name: "*".to_owned(),
                        target,
                        glob: true,
                        use_id,
                    });
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn put(
        &mut self,
        at: usize,
        path: &[String],
        def: ModuleDef,
        kind: Kind,
        name: &hir::Name,
        via: Option<hir_def::UseId>,
        local: bool,
    ) {
        let index = self.out.len();
        let name = name.as_str().to_owned();
        self.out.push(Placed {
            def,
            kind,
            name: name.clone(),
            modules: path.to_vec(),
            children: vec![],
            reexports: vec![],
            via,
        });
        self.out[at].children.push(index);
        if let ModuleDef::Module(module) = def {
            let mut inner = path.to_vec();
            inner.push(name);
            self.module(module, index, inner, local);
        }
    }
}

/// An item of another crate that rustdoc can inline: public there, not hidden.
fn local_or_public(db: &RootDatabase, def: ModuleDef) -> bool {
    def.visibility(db) == Visibility::Public && !def.attrs(db).is_doc_hidden()
}

pub(crate) fn is_macro_export(db: &RootDatabase, it: Macro) -> bool {
    let id: hir_def::MacroId = it.into();
    hir_def::attrs::AttrFlags::query(db, id.into())
        .contains(hir_def::attrs::AttrFlags::IS_MACRO_EXPORT)
}

/// What the attributes of a `use` item say about inlining.
#[derive(Default)]
struct UseAttrs {
    hidden: bool,
    inline: bool,
    no_inline: bool,
}

impl UseAttrs {
    fn of(db: &RootDatabase, id: hir_def::UseId) -> UseAttrs {
        use hir_def::{Lookup, src::HasSource};
        use syntax::ast::HasAttrs as _;
        let flags = hir_def::attrs::AttrFlags::query(db, id.into());
        let mut out =
            UseAttrs { hidden: flags.contains(hir_def::attrs::AttrFlags::IS_DOC_HIDDEN), ..UseAttrs::default() };
        let source = id.lookup(db).source(db);
        for attr in source.value.attrs() {
            for word in crate::attrs::doc_words(&attr) {
                match word.as_str() {
                    "inline" => out.inline = true,
                    "no_inline" => out.no_inline = true,
                    _ => {}
                }
            }
        }
        out
    }
}

/// The crates a documented crate's docs may link to and list impls from: itself and every crate
/// it depends on, directly or not.
pub(crate) fn upstream(db: &RootDatabase, krate: Crate) -> FxHashSet<Crate> {
    let mut seen = FxHashSet::default();
    let mut stack = vec![krate];
    while let Some(it) = stack.pop() {
        if seen.insert(it) {
            stack.extend(it.dependencies(db).into_iter().map(|dep| dep.krate));
        }
    }
    seen
}

/// The crates linked into `krate`: its dependencies, transitively. Of the sysroot's crates,
/// which rust-analyzer makes dependencies of every crate, `core` is linked; `std` and `alloc`
/// unless the crate is `#![no_std]`, `proc_macro` for a crate of procedural macros, and any of
/// them the crate names with `extern crate`.
pub(crate) fn linked(db: &RootDatabase, krate: Crate) -> FxHashSet<Crate> {
    let mut seen = FxHashSet::default();
    let mut stack = vec![krate];
    while let Some(it) = stack.pop() {
        if !seen.insert(it) {
            continue;
        }
        let def_map = hir_def::nameres::crate_def_map(db, it.into());
        let named: Vec<Crate> =
            it.root_module(db).extern_crate_decls(db).into_iter().filter_map(|it| it.resolved_crate(db)).collect();
        for dep in it.dependencies(db) {
            let linked = match lang_crate_name(db, dep.krate) {
                Some("std" | "alloc") => !def_map.is_no_std(),
                Some("proc_macro") => def_map.is_proc_macro(),
                Some("test") => false,
                _ => true,
            };
            if linked || named.contains(&dep.krate) {
                stack.push(dep.krate);
            }
        }
    }
    seen
}

/// The crate of what a type is: an ADT's, a `dyn Trait`'s trait's; `None` for a builtin type,
/// a reference or anything else whose docs are the standard library's.
pub(crate) fn type_home(db: &RootDatabase, ty: &hir::Type<'_>) -> Option<Crate> {
    if let Some(adt) = ty.as_adt() {
        return krate_of(db, ModuleDef::Adt(adt));
    }
    ty.as_dyn_trait().and_then(|it| krate_of(db, ModuleDef::Trait(it)))
}

/// The crate an item belongs to.
pub(crate) fn krate_of(db: &RootDatabase, def: ModuleDef) -> Option<Crate> {
    match def {
        ModuleDef::BuiltinType(_) => None,
        ModuleDef::Module(it) => Some(it.krate(db)),
        _ => def.module(db).map(|it| it.krate(db)),
    }
}

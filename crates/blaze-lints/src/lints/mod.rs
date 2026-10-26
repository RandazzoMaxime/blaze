//! The lints, each a port of clippy's (its file says which), and what runs them on a node.

mod bool_assert_comparison;
mod empty_enums;
mod map_unwrap_or;
mod needless_borrows_for_generic_args;
mod nonstandard_macro_braces;
mod redundant_static_lifetimes;
mod utils;

use rustc_hash::FxHashSet;
use syntax::{AstNode, SyntaxNode, ast};

use crate::{
    Level,
    check::Cx,
    level::{Levels, Lint},
    registry,
};

/// The lints implemented, and whether each is one of rustc's early lints, which rustc runs
/// over the crate before the late ones.
const IMPLEMENTED: &[(&str, bool)] = &[
    ("bool_assert_comparison", false),
    ("empty_enums", false),
    ("map_unwrap_or", false),
    ("needless_borrows_for_generic_args", false),
    ("nonstandard_macro_braces", true),
    ("redundant_static_lifetimes", true),
];

pub(crate) fn implemented(lint: &str) -> bool {
    IMPLEMENTED.iter().any(|(it, _)| *it == lint)
}

pub(crate) fn is_early(lint: &str) -> bool {
    IMPLEMENTED.iter().any(|(it, early)| *it == lint && *early)
}

/// Every lint implemented, by name.
pub fn names() -> impl Iterator<Item = &'static str> {
    IMPLEMENTED.iter().map(|(it, _)| *it)
}

/// The lints that may be at a level other than `allow` somewhere in a crate whose files have
/// `texts`: those the command line or their group does not allow, and those the crate names
/// in an attribute.
pub(crate) fn on(levels: &Levels, texts: &[&str]) -> FxHashSet<&'static str> {
    let mut on = FxHashSet::default();
    for (name, _) in IMPLEMENTED {
        let Some(info) = registry::lint(name) else { continue };
        let named = |text: &str| {
            text.contains(name) || text.contains(info.group.name()) || text.contains("clippy::all")
        };
        if levels.of(Lint::Clippy(info), &[]).level != Level::Allow
            || texts.iter().any(|it| named(it))
        {
            on.insert(*name);
        }
    }
    on
}

/// Whether the walk goes into what derives expand to: only lints that run there need it.
pub(crate) fn in_derives(cx: &Cx<'_, '_, '_>) -> bool {
    cx.on("empty_enums")
}

/// Whether the walk goes into what macro calls expand to, for the code handed to them: only
/// lints that run on code a macro can be handed need it.
pub(crate) fn in_calls(cx: &Cx<'_, '_, '_>) -> bool {
    IMPLEMENTED.iter().any(|(name, _)| *name != "empty_enums" && cx.on(name))
}

/// Runs the lints of `node`'s kind on it.
pub(crate) fn check_node(cx: &mut Cx<'_, '_, '_>, node: &SyntaxNode) {
    if cx.expansion.is_some() {
        if let Some(it) = ast::Enum::cast(node.clone()) {
            empty_enums::check(cx, &it);
        }
        return;
    }
    if let Some(it) = ast::Enum::cast(node.clone()) {
        empty_enums::check(cx, &it);
    } else if let Some(it) = ast::Const::cast(node.clone()) {
        redundant_static_lifetimes::check_const(cx, &it);
    } else if let Some(it) = ast::Static::cast(node.clone()) {
        redundant_static_lifetimes::check_static(cx, &it);
    } else if let Some(it) = ast::MacroCall::cast(node.clone()) {
        nonstandard_macro_braces::check_call(cx, &it);
        bool_assert_comparison::check(cx, &it);
    } else if let Some(it) = ast::MacroRules::cast(node.clone()) {
        nonstandard_macro_braces::check_rules(cx, &it);
    }
    if let Some(it) = ast::Expr::cast(node.clone()) {
        map_unwrap_or::check(cx, &it);
        needless_borrows_for_generic_args::check(cx, &it);
    }
}

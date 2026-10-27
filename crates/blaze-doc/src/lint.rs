//! Whether rustdoc reports a broken intra-doc link at a place: the nearest `allow`, `expect`,
//! `warn`, `deny` or `forbid` naming `rustdoc::broken_intra_doc_links`, `rustdoc::all` or
//! `warnings`, on the items around the link, then on the modules it is in, each `cfg_attr` as
//! the crate's cfg says. Levels given on the command line (`RUSTDOCFLAGS`) are not read.

use hir::{CfgExpr, CfgOptions, HirFileId, InFile, Semantics};
use ide_db::RootDatabase;
use syntax::{
    AstNode, NodeOrToken, TextRange,
    ast::{self, HasAttrs},
};

/// Whether a broken link at `at`, in `krate`, is reported.
pub(crate) fn broken_links_reported(
    db: &RootDatabase,
    krate: hir::Crate,
    at: InFile<TextRange>,
) -> bool {
    level(db, krate, at).unwrap_or(true)
}

fn level(db: &RootDatabase, krate: hir::Crate, at: InFile<TextRange>) -> Option<bool> {
    let cfg = krate.cfg(db);
    let sema = Semantics::new(db);
    let root = at.file_id.parse_or_expand(db);
    if !root.text_range().contains_range(at.value) {
        return None;
    }
    let start = match root.covering_element(at.value) {
        NodeOrToken::Node(it) => it,
        NodeOrToken::Token(it) => it.parent()?,
    };
    let mut node = InFile::new(at.file_id, start);
    loop {
        for it in node.value.ancestors() {
            if let Some(it) = ast::AnyHasAttrs::cast(it)
                && let Some(reported) = level_on(&it, cfg)
            {
                return Some(reported);
            }
        }
        // Up from a macro's output to its call, from a module's file to its `mod` item.
        node = match node.file_id {
            HirFileId::MacroFile(_) => node.file_id.call_node(db)?,
            HirFileId::FileId(file) => {
                let module = sema.file_to_module_def(file.file_id(db))?;
                module.declaration_source(db)?.map(|it| it.syntax().clone())
            }
        };
    }
}

/// The level the attributes of `node` give the lint, the last one that names it.
fn level_on(node: &ast::AnyHasAttrs, cfg: &CfgOptions) -> Option<bool> {
    let mut out = None;
    for attr in node.attrs() {
        let Some(meta) = attr.meta() else { continue };
        for meta in active(meta, cfg) {
            let Some((name, tt)) = meta.as_simple_call() else { continue };
            let reported = match name.as_str() {
                "allow" | "expect" => false,
                "warn" | "deny" | "forbid" => true,
                _ => continue,
            };
            if names_the_lint(&tt) {
                out = Some(reported);
            }
        }
    }
    out
}

/// `meta`, or what a `cfg_attr` whose condition holds gives.
fn active(meta: ast::Meta, cfg: &CfgOptions) -> Vec<ast::Meta> {
    match meta {
        ast::Meta::CfgAttrMeta(it) => {
            let holds = it
                .cfg_predicate()
                .is_some_and(|pred| cfg.check(&CfgExpr::parse_from_ast(pred)) != Some(false));
            if !holds {
                return vec![];
            }
            it.metas().flat_map(|meta| active(meta, cfg)).collect()
        }
        _ => vec![meta],
    }
}

fn names_the_lint(tt: &ast::TokenTree) -> bool {
    let text: String = tt.syntax().text().to_string().chars().filter(|c| !c.is_whitespace()).collect();
    text.trim_start_matches('(').trim_end_matches(')').split(',').any(|lint| {
        matches!(
            lint,
            "rustdoc::broken_intra_doc_links" | "broken_intra_doc_links" | "rustdoc::all" | "warnings"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::names_the_lint;
    use syntax::{AstNode, Edition, SourceFile, ast};

    #[test]
    fn lint_names() {
        let tt = |attr: &str| {
            let file = SourceFile::parse(&format!("{attr} fn f() {{}}"), Edition::CURRENT).tree();
            file.syntax().descendants().find_map(ast::TokenTree::cast).unwrap()
        };
        assert!(names_the_lint(&tt("#[allow(rustdoc::broken_intra_doc_links)]")));
        assert!(names_the_lint(&tt("#[allow(dead_code, rustdoc :: all)]")));
        assert!(names_the_lint(&tt("#[deny(warnings)]")));
        assert!(!names_the_lint(&tt("#[allow(rustdoc::private_intra_doc_links)]")));
    }
}

//! What several lints ask of the code, as clippy's `clippy_utils` answers it.

use hir::PathResolution;
use ide_db::famous_defs::FamousDefs;
use syntax::{
    AstNode, SyntaxNode, TextRange,
    ast::{self, HasArgList as _},
};

use crate::check::Cx;

/// A method call: `receiver.name(args)`.
pub(crate) struct MethodCall {
    pub(crate) name: String,
    /// The method's name, where it is written.
    pub(crate) name_ref: ast::NameRef,
    pub(crate) receiver: ast::Expr,
    pub(crate) args: Vec<ast::Expr>,
}

/// clippy's `method_call`: `expr` as a method call whose receiver and arguments are all
/// written in the file, none of them made by a macro.
pub(crate) fn method_call(cx: &Cx<'_, '_, '_>, expr: &ast::Expr) -> Option<MethodCall> {
    let ast::Expr::MethodCallExpr(call) = expr else { return None };
    let receiver = call.receiver()?;
    let args: Vec<ast::Expr> = call.arg_list()?.args().collect();
    if !cx.is_written(receiver.syntax()) || !args.iter().all(|it| cx.is_written(it.syntax())) {
        return None;
    }
    let name_ref = call.name_ref()?;
    Some(MethodCall { name: name_ref.text().to_string(), name_ref, receiver, args })
}

/// `Option` or `Result`, when `ty`, its references peeled, is one of them.
pub(crate) fn option_or_result(cx: &Cx<'_, '_, '_>, ty: &hir::Type<'_>) -> Option<&'static str> {
    let adt = ty.strip_references().as_adt()?;
    let famous = FamousDefs(cx.sema(), cx.krate());
    let option = famous.core_option_Option().map(hir::Adt::Enum);
    let result = famous.core_result_Result().map(hir::Adt::Enum);
    match () {
        _ if option == Some(adt) => Some("Option"),
        _ if result == Some(adt) => Some("Result"),
        _ => None,
    }
}

/// clippy's `is_none_expr`: `expr` is `None`, `Option`'s.
pub(crate) fn is_none(cx: &Cx<'_, '_, '_>, expr: &ast::Expr) -> bool {
    let ast::Expr::PathExpr(path) = expr else { return false };
    let Some(path) = path.path() else { return false };
    let Some(PathResolution::Def(hir::ModuleDef::EnumVariant(variant))) = cx.sema().resolve_path(&path)
    else {
        return false;
    };
    let famous = FamousDefs(cx.sema(), cx.krate());
    famous.core_option_Option() == Some(variant.parent_enum(cx.db()))
        && variant.name(cx.db()).as_str() == "None"
}

/// `expr` is the literal `value`.
pub(crate) fn is_bool_literal(expr: &ast::Expr, value: bool) -> bool {
    let ast::Expr::Literal(literal) = expr else { return false };
    matches!(literal.kind(), ast::LiteralKind::Bool(it) if it == value)
}

/// The locals the paths of `node` name.
pub(crate) fn locals_in<'db>(cx: &Cx<'_, '_, 'db>, node: &SyntaxNode) -> Vec<hir::Local<'db>> {
    let mut found = Vec::new();
    for path in node.descendants().filter_map(ast::PathExpr::cast) {
        if let Some(local) = local_of(cx, &path)
            && !found.contains(&local)
        {
            found.push(local);
        }
    }
    found
}

pub(crate) fn local_of<'db>(cx: &Cx<'_, '_, 'db>, path: &ast::PathExpr) -> Option<hir::Local<'db>> {
    match cx.sema().resolve_path(&path.path()?)? {
        PathResolution::Local(local) => Some(local),
        _ => None,
    }
}

/// The body `node` is in, as rustc's `hir_enclosing_body_owner` finds it: its closure, its
/// function, or the initializer of its constant or static.
pub(crate) fn enclosing_body(node: &SyntaxNode) -> Option<SyntaxNode> {
    node.ancestors().skip(1).find_map(|it| {
        if let Some(closure) = ast::ClosureExpr::cast(it.clone()) {
            return closure.body().map(|it| it.syntax().clone());
        }
        if let Some(function) = ast::Fn::cast(it.clone()) {
            return function.body().map(|it| it.syntax().clone());
        }
        if let Some(konst) = ast::Const::cast(it.clone()) {
            return konst.body().map(|it| it.syntax().clone());
        }
        ast::Static::cast(it).and_then(|it| it.body()).map(|it| it.syntax().clone())
    })
}

/// The range from the end of `a` to the end of `b`, as the file has them.
pub(crate) fn between_ends(cx: &Cx<'_, '_, '_>, a: &SyntaxNode, b: &SyntaxNode) -> Option<TextRange> {
    let (a, b) = (cx.range(a)?, cx.range(b)?);
    (a.end() <= b.end()).then(|| TextRange::new(a.end(), b.end()))
}

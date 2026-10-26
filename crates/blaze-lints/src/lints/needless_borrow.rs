//! `needless_borrow`: `&x` where `x` is already a reference and the compiler dereferences the
//! borrow away again.
//!
//! Follows clippy's `clippy_lints/src/dereference.rs` (the `DerefedBorrow` state) at
//! rust-lang/rust 12433c307b5a497388ffe2f4e49dbf1ab8c71fa0 (MIT or Apache-2.0), for a borrow
//! used as a call's or method's argument, a field's value, a `let`'s initializer or what a
//! `return` gives. Not ported: borrows that are a receiver, a callee or have a field read
//! (where clippy asks whether auto-borrow would pick the same method), chains of `&` and `*`,
//! mutable reborrows, and `ref` patterns.

use hir::{Adjust, AutoBorrow, Mutability};
use syntax::{AstNode, ast};

use crate::{check::Cx, diag::Applicability, sugg::has_enclosing_paren};

const LINT: &str = "needless_borrow";

pub(super) fn check(cx: &mut Cx<'_, '_, '_>, expr: &ast::Expr) {
    if !cx.on(LINT) || cx.expansion.is_some() {
        return;
    }
    let ast::Expr::RefExpr(borrow) = expr else { return };
    if borrow.raw_token().is_some() {
        return;
    }
    let Some(sub) = borrow.expr() else { return };
    if !cx.is_written(sub.syntax()) || is_ref_op(&strip_parens(&sub)) {
        return;
    }
    // The node the borrow is used at, past the parentheses around it.
    let mut used = expr.syntax().clone();
    while let Some(parent) = used.parent()
        && ast::ParenExpr::can_cast(parent.kind())
    {
        used = parent;
    }
    let Some(parent) = used.parent() else { return };
    let in_call = ast::ArgList::can_cast(parent.kind());
    let is_use = in_call
        || ast::RecordExprField::can_cast(parent.kind())
        || ast::LetStmt::can_cast(parent.kind())
        || ast::ReturnExpr::can_cast(parent.kind());
    if !is_use {
        return;
    }
    let Some(adjustments) = ast::Expr::cast(used.clone()).and_then(|it| cx.sema().expr_adjustments(&it)) else {
        return;
    };
    let mut iter = adjustments.iter();
    let mut deref_count = 0usize;
    let next = loop {
        match iter.next() {
            Some(adjust) if !matches!(adjust.kind, Adjust::Deref(_)) => break Some(adjust),
            Some(adjust) if !adjust.target.is_reference() => {
                deref_count += 1;
                break iter.next();
            }
            Some(_) => deref_count += 1,
            None => break None,
        }
    };
    if next.is_some_and(|it| matches!(it.kind, Adjust::Borrow(AutoBorrow::Ref(Mutability::Mut)))) {
        return;
    }
    if deref_count < 2 {
        return;
    }
    // `&*p` of a raw pointer stays, as rustc's `dangerous_implicit_autorefs` wants it.
    if let ast::Expr::PrefixExpr(prefix) = strip_parens(&sub)
        && prefix.op_kind() == Some(ast::UnaryOp::Deref)
        && prefix.expr().and_then(|it| cx.adjusted_ty(&it)).is_some_and(|it| it.is_raw_ptr())
    {
        return;
    }
    let (Some(range), Some(sub_range)) = (cx.range(expr.syntax()), cx.range(sub.syntax())) else { return };
    let snip = cx.snippet(sub_range);
    // A method call binds tighter than an operator: a call's arguments are each whole.
    let in_method_call = in_call && parent.parent().is_some_and(|it| ast::MethodCallExpr::can_cast(it.kind()));
    let needs_paren = in_method_call
        && matches!(
            sub,
            ast::Expr::PrefixExpr(_) | ast::Expr::BinExpr(_) | ast::Expr::CastExpr(_) | ast::Expr::RangeExpr(_)
        );
    let sugg = if needs_paren && !has_enclosing_paren(&snip) { format!("({snip})") } else { snip };
    let span = cx.span(range);
    cx.lint(
        LINT,
        expr.syntax(),
        span.clone(),
        "this expression creates a reference which is immediately dereferenced by the compiler",
    )
    .suggestion(span, "change this to", sugg, Applicability::MachineApplicable);
}

/// clippy's `try_parse_ref_op`: a borrow or a dereference.
fn is_ref_op(expr: &ast::Expr) -> bool {
    match expr {
        ast::Expr::RefExpr(it) => it.raw_token().is_none(),
        ast::Expr::PrefixExpr(it) => it.op_kind() == Some(ast::UnaryOp::Deref),
        _ => false,
    }
}

fn strip_parens(expr: &ast::Expr) -> ast::Expr {
    let mut expr = expr.clone();
    while let ast::Expr::ParenExpr(paren) = &expr
        && let Some(inner) = paren.expr()
    {
        expr = inner;
    }
    expr
}

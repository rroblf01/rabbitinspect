use rustpython_ast::*;

use crate::analyze::{byte_to_line_col, text_size_to_usize, Finding, Fix};
use std::collections::HashSet;

pub fn check_unused_variables(
    _source: &str,
    _line_starts: &[usize],
    stmts: &[Stmt],
) -> Vec<Finding> {
    let mut findings = Vec::new();
    check_scope_unused(stmts, &mut findings, "<module>");
    findings
}

fn check_scope_unused(stmts: &[Stmt], findings: &mut Vec<Finding>, scope_name: &str) {
    let mut assigned: HashSet<String> = HashSet::new();
    let mut used: HashSet<String> = HashSet::new();
    collect_names_in_scope(stmts, &mut assigned, &mut used, findings);

    for name in assigned.difference(&used) {
        if name.starts_with('_') {
            continue;
        }
        if name == scope_name {
            continue;
        }
        findings.push(Finding {
            line: 0,
            col: 0,
            end_line: 0,
            end_col: 0,
            code: "RAB001".to_string(),
            message: format!(
                "Variable '{}' in '{}' is assigned but never used",
                name, scope_name
            ),
            fix: None,
        });
    }
}

fn collect_names_in_scope(
    stmts: &[Stmt],
    assigned: &mut HashSet<String>,
    used: &mut HashSet<String>,
    findings: &mut Vec<Finding>,
) {
    for stmt in stmts {
        match stmt {
            Stmt::FunctionDef(f) => {
                let mut fn_assigned: HashSet<String> = HashSet::new();
                let mut fn_used: HashSet<String> = HashSet::new();
                for arg in &f.args.args {
                    fn_assigned.insert(arg.def.arg.to_string());
                }
                for arg in &f.args.posonlyargs {
                    fn_assigned.insert(arg.def.arg.to_string());
                }
                for arg in &f.args.kwonlyargs {
                    fn_assigned.insert(arg.def.arg.to_string());
                }
                if let Some(vararg) = &f.args.vararg {
                    fn_assigned.insert(vararg.arg.to_string());
                }
                if let Some(kwarg) = &f.args.kwarg {
                    fn_assigned.insert(kwarg.arg.to_string());
                }
                collect_names_in_scope(&f.body, &mut fn_assigned, &mut fn_used, findings);
                for name in fn_assigned.difference(&fn_used) {
                    if name.starts_with('_') {
                        continue;
                    }
                    findings.push(Finding {
                        line: 0,
                        col: 0,
                        end_line: 0,
                        end_col: 0,
                        code: "RAB001".to_string(),
                        message: format!(
                            "Parameter '{}' in function '{}' is never used",
                            name, f.name
                        ),
                        fix: None,
                    });
                }
            }
            Stmt::AsyncFunctionDef(f) => {
                let mut fn_assigned: HashSet<String> = HashSet::new();
                let mut fn_used: HashSet<String> = HashSet::new();
                for arg in &f.args.args {
                    fn_assigned.insert(arg.def.arg.to_string());
                }
                for arg in &f.args.posonlyargs {
                    fn_assigned.insert(arg.def.arg.to_string());
                }
                for arg in &f.args.kwonlyargs {
                    fn_assigned.insert(arg.def.arg.to_string());
                }
                if let Some(vararg) = &f.args.vararg {
                    fn_assigned.insert(vararg.arg.to_string());
                }
                if let Some(kwarg) = &f.args.kwarg {
                    fn_assigned.insert(kwarg.arg.to_string());
                }
                collect_names_in_scope(&f.body, &mut fn_assigned, &mut fn_used, findings);
                for name in fn_assigned.difference(&fn_used) {
                    if name.starts_with('_') {
                        continue;
                    }
                    findings.push(Finding {
                        line: 0,
                        col: 0,
                        end_line: 0,
                        end_col: 0,
                        code: "RAB001".to_string(),
                        message: format!(
                            "Parameter '{}' in async function '{}' is never used",
                            name, f.name
                        ),
                        fix: None,
                    });
                }
            }
            Stmt::Assign(a) => {
                collect_names_from_targets(&a.targets, assigned);
                walk_expr_names(&a.value, used, assigned);
            }
            Stmt::AnnAssign(a) => {
                collect_name_from_expr(&a.target, assigned, true);
                if let Some(val) = &a.value {
                    walk_expr_names(val, used, assigned);
                }
            }
            Stmt::AugAssign(a) => {
                collect_name_from_expr(&a.target, assigned, true);
                collect_name_from_expr(&a.target, used, false);
                walk_expr_names(&a.value, used, assigned);
            }
            Stmt::For(f) => {
                collect_name_from_expr(&f.target, assigned, true);
                walk_expr_names(&f.iter, used, assigned);
                collect_names_in_scope(&f.body, assigned, used, findings);
                collect_names_in_scope(&f.orelse, assigned, used, findings);
            }
            Stmt::AsyncFor(f) => {
                collect_name_from_expr(&f.target, assigned, true);
                walk_expr_names(&f.iter, used, assigned);
                collect_names_in_scope(&f.body, assigned, used, findings);
                collect_names_in_scope(&f.orelse, assigned, used, findings);
            }
            Stmt::With(w) => {
                for item in &w.items {
                    if let Some(optional_vars) = &item.optional_vars {
                        collect_name_from_expr(optional_vars, assigned, true);
                    }
                    walk_expr_names(&item.context_expr, used, assigned);
                }
                collect_names_in_scope(&w.body, assigned, used, findings);
            }
            Stmt::AsyncWith(w) => {
                for item in &w.items {
                    if let Some(optional_vars) = &item.optional_vars {
                        collect_name_from_expr(optional_vars, assigned, true);
                    }
                    walk_expr_names(&item.context_expr, used, assigned);
                }
                collect_names_in_scope(&w.body, assigned, used, findings);
            }
            Stmt::If(i) => {
                walk_expr_names(&i.test, used, assigned);
                collect_names_in_scope(&i.body, assigned, used, findings);
                collect_names_in_scope(&i.orelse, assigned, used, findings);
            }
            Stmt::While(w) => {
                walk_expr_names(&w.test, used, assigned);
                collect_names_in_scope(&w.body, assigned, used, findings);
                collect_names_in_scope(&w.orelse, assigned, used, findings);
            }
            Stmt::Try(t) => {
                collect_names_in_scope(&t.body, assigned, used, findings);
                for handler in &t.handlers {
                    let ExceptHandler::ExceptHandler(h) = handler;
                    if let Some(name) = &h.name {
                        assigned.insert(name.to_string());
                    }
                    collect_names_in_scope(&h.body, assigned, used, findings);
                }
                collect_names_in_scope(&t.orelse, assigned, used, findings);
                collect_names_in_scope(&t.finalbody, assigned, used, findings);
            }
            Stmt::ClassDef(c) => {
                collect_names_in_scope(&c.body, assigned, used, findings);
            }
            Stmt::Expr(e) => {
                walk_expr_names(&e.value, used, assigned);
            }
            Stmt::Match(m) => {
                walk_expr_names(&m.subject, used, assigned);
                for case in &m.cases {
                    collect_names_in_scope(&case.body, assigned, used, findings);
                }
            }
            Stmt::Import(i) => {
                for alias in &i.names {
                    let name = alias
                        .asname
                        .clone()
                        .unwrap_or_else(|| alias.name.clone());
                    let short = name.split('.').next().unwrap_or(&name).to_string();
                    assigned.insert(short);
                }
            }
            Stmt::ImportFrom(i) => {
                for alias in &i.names {
                    let name = alias
                        .asname
                        .clone()
                        .unwrap_or_else(|| alias.name.clone());
                    assigned.insert(name.to_string());
                }
            }
            Stmt::Global(g) => {
                for name in &g.names {
                    used.insert(name.to_string());
                }
            }
            Stmt::Nonlocal(g) => {
                for name in &g.names {
                    used.insert(name.to_string());
                }
            }
            Stmt::Return(r) => {
                if let Some(val) = &r.value {
                    walk_expr_names(val, used, assigned);
                }
            }
            Stmt::Raise(r) => {
                if let Some(val) = &r.exc {
                    walk_expr_names(val, used, assigned);
                }
            }
            Stmt::Delete(d) => {
                for target in &d.targets {
                    walk_expr_names(target, used, assigned);
                }
            }
            Stmt::Assert(a) => {
                walk_expr_names(&a.test, used, assigned);
                if let Some(msg) = &a.msg {
                    walk_expr_names(msg, used, assigned);
                }
            }
            _ => {}
        }
    }
}

fn collect_names_from_targets(targets: &[Expr], assigned: &mut HashSet<String>) {
    for target in targets {
        collect_name_from_expr(target, assigned, true);
    }
}

fn collect_name_from_expr(expr: &Expr, names: &mut HashSet<String>, is_store: bool) {
    match expr {
        Expr::Name(n) => {
            if (is_store && n.ctx == ExprContext::Store)
                || (!is_store && n.ctx == ExprContext::Load)
            {
                names.insert(n.id.to_string());
            }
        }
        Expr::Tuple(t) => {
            for elt in &t.elts {
                collect_name_from_expr(elt, names, is_store);
            }
        }
        Expr::List(l) => {
            for elt in &l.elts {
                collect_name_from_expr(elt, names, is_store);
            }
        }
        Expr::Starred(s) => {
            collect_name_from_expr(&s.value, names, is_store);
        }
        _ => {}
    }
}

fn walk_expr_names(
    expr: &Expr,
    used: &mut HashSet<String>,
    assigned: &mut HashSet<String>,
) {
    match expr {
        Expr::Name(n) => {
            if n.ctx == ExprContext::Load {
                used.insert(n.id.to_string());
            }
        }
        Expr::Call(c) => {
            walk_expr_names(&c.func, used, assigned);
            for arg in &c.args {
                walk_expr_names(arg, used, assigned);
            }
            for kw in &c.keywords {
                walk_expr_names(&kw.value, used, assigned);
            }
        }
        Expr::Attribute(a) => walk_expr_names(&a.value, used, assigned),
        Expr::BinOp(b) => {
            walk_expr_names(&b.left, used, assigned);
            walk_expr_names(&b.right, used, assigned);
        }
        Expr::UnaryOp(u) => walk_expr_names(&u.operand, used, assigned),
        Expr::BoolOp(b) => {
            for val in &b.values {
                walk_expr_names(val, used, assigned);
            }
        }
        Expr::Compare(c) => {
            walk_expr_names(&c.left, used, assigned);
            for comp in &c.comparators {
                walk_expr_names(comp, used, assigned);
            }
        }
        Expr::Subscript(s) => {
            walk_expr_names(&s.value, used, assigned);
            walk_expr_names(&s.slice, used, assigned);
        }
        Expr::List(l) => {
            for elt in &l.elts {
                walk_expr_names(elt, used, assigned);
            }
        }
        Expr::Tuple(t) => {
            for elt in &t.elts {
                walk_expr_names(elt, used, assigned);
            }
        }
        Expr::Dict(d) => {
            for key in &d.keys {
                if let Some(k) = key {
                    walk_expr_names(k, used, assigned);
                }
            }
            for val in &d.values {
                walk_expr_names(val, used, assigned);
            }
        }
        Expr::Set(s) => {
            for elt in &s.elts {
                walk_expr_names(elt, used, assigned);
            }
        }
        Expr::ListComp(lc) => {
            walk_expr_names(&lc.elt, used, assigned);
            for gen in &lc.generators {
                collect_name_from_expr(&gen.target, assigned, true);
                walk_expr_names(&gen.iter, used, assigned);
                for cond in &gen.ifs {
                    walk_expr_names(cond, used, assigned);
                }
            }
        }
        Expr::SetComp(sc) => {
            walk_expr_names(&sc.elt, used, assigned);
            for gen in &sc.generators {
                collect_name_from_expr(&gen.target, assigned, true);
                walk_expr_names(&gen.iter, used, assigned);
                for cond in &gen.ifs {
                    walk_expr_names(cond, used, assigned);
                }
            }
        }
        Expr::DictComp(dc) => {
            walk_expr_names(&dc.key, used, assigned);
            walk_expr_names(&dc.value, used, assigned);
            for gen in &dc.generators {
                collect_name_from_expr(&gen.target, assigned, true);
                walk_expr_names(&gen.iter, used, assigned);
                for cond in &gen.ifs {
                    walk_expr_names(cond, used, assigned);
                }
            }
        }
        Expr::GeneratorExp(ge) => {
            walk_expr_names(&ge.elt, used, assigned);
            for gen in &ge.generators {
                collect_name_from_expr(&gen.target, assigned, true);
                walk_expr_names(&gen.iter, used, assigned);
                for cond in &gen.ifs {
                    walk_expr_names(cond, used, assigned);
                }
            }
        }
        Expr::Lambda(l) => {
            let mut lam_assigned: HashSet<String> = HashSet::new();
            let mut lam_used: HashSet<String> = HashSet::new();
            for arg in &l.args.args {
                lam_assigned.insert(arg.def.arg.to_string());
            }
            for arg in &l.args.posonlyargs {
                lam_assigned.insert(arg.def.arg.to_string());
            }
            for arg in &l.args.kwonlyargs {
                lam_assigned.insert(arg.def.arg.to_string());
            }
            walk_expr_names(&l.body, &mut lam_used, &mut lam_assigned);
            for name in lam_assigned.difference(&lam_used) {
                if !name.starts_with('_') {
                    used.insert(name.clone());
                }
            }
            for name in &lam_used {
                if !lam_assigned.contains(name) {
                    used.insert(name.clone());
                }
            }
        }
        Expr::IfExp(if_exp) => {
            walk_expr_names(&if_exp.test, used, assigned);
            walk_expr_names(&if_exp.body, used, assigned);
            walk_expr_names(&if_exp.orelse, used, assigned);
        }
        Expr::Starred(s) => walk_expr_names(&s.value, used, assigned),
        Expr::NamedExpr(ne) => {
            collect_name_from_expr(&ne.target, assigned, true);
            walk_expr_names(&ne.value, used, assigned);
        }
        Expr::Await(a) => walk_expr_names(&a.value, used, assigned),
        Expr::Yield(y) => {
            if let Some(val) = &y.value {
                walk_expr_names(val, used, assigned);
            }
        }
        Expr::YieldFrom(yf) => walk_expr_names(&yf.value, used, assigned),
        Expr::Slice(s) => {
            if let Some(lower) = &s.lower {
                walk_expr_names(lower, used, assigned);
            }
            if let Some(upper) = &s.upper {
                walk_expr_names(upper, used, assigned);
            }
            if let Some(step) = &s.step {
                walk_expr_names(step, used, assigned);
            }
        }
        _ => {}
    }
}

// -- RAB002: None comparison --

pub fn check_none_comparison(
    source: &str,
    line_starts: &[usize],
    stmts: &[Stmt],
) -> Vec<Finding> {
    let mut findings = Vec::new();
    for stmt in stmts {
        walk_none_comp(stmt, source, line_starts, &mut findings);
    }
    findings
}

fn walk_none_comp<'a>(
    stmt: &'a Stmt,
    source: &str,
    line_starts: &[usize],
    findings: &mut Vec<Finding>,
) {
    match stmt {
        Stmt::FunctionDef(f) => {
            for s in &f.body {
                walk_none_comp(s, source, line_starts, findings);
            }
        }
        Stmt::AsyncFunctionDef(f) => {
            for s in &f.body {
                walk_none_comp(s, source, line_starts, findings);
            }
        }
        Stmt::ClassDef(c) => {
            for s in &c.body {
                walk_none_comp(s, source, line_starts, findings);
            }
        }
        Stmt::If(i) => {
            check_expr_none_comp(&i.test, source, line_starts, findings);
            for s in &i.body {
                walk_none_comp(s, source, line_starts, findings);
            }
            for s in &i.orelse {
                walk_none_comp(s, source, line_starts, findings);
            }
        }
        Stmt::While(w) => {
            check_expr_none_comp(&w.test, source, line_starts, findings);
            for s in &w.body {
                walk_none_comp(s, source, line_starts, findings);
            }
            for s in &w.orelse {
                walk_none_comp(s, source, line_starts, findings);
            }
        }
        Stmt::For(f) => {
            check_expr_none_comp(&f.iter, source, line_starts, findings);
            for s in &f.body {
                walk_none_comp(s, source, line_starts, findings);
            }
            for s in &f.orelse {
                walk_none_comp(s, source, line_starts, findings);
            }
        }
        Stmt::AsyncFor(f) => {
            check_expr_none_comp(&f.iter, source, line_starts, findings);
            for s in &f.body {
                walk_none_comp(s, source, line_starts, findings);
            }
            for s in &f.orelse {
                walk_none_comp(s, source, line_starts, findings);
            }
        }
        Stmt::With(w) => {
            for item in &w.items {
                check_expr_none_comp(&item.context_expr, source, line_starts, findings);
            }
            for s in &w.body {
                walk_none_comp(s, source, line_starts, findings);
            }
        }
        Stmt::AsyncWith(w) => {
            for item in &w.items {
                check_expr_none_comp(&item.context_expr, source, line_starts, findings);
            }
            for s in &w.body {
                walk_none_comp(s, source, line_starts, findings);
            }
        }
        Stmt::Try(t) => {
            for s in &t.body {
                walk_none_comp(s, source, line_starts, findings);
            }
            for handler in &t.handlers {
                let ExceptHandler::ExceptHandler(h) = handler;
                for s in &h.body {
                    walk_none_comp(s, source, line_starts, findings);
                }
            }
            for s in &t.orelse {
                walk_none_comp(s, source, line_starts, findings);
            }
            for s in &t.finalbody {
                walk_none_comp(s, source, line_starts, findings);
            }
        }
        Stmt::Expr(e) => {
            check_expr_none_comp(&e.value, source, line_starts, findings);
        }
        Stmt::Return(r) => {
            if let Some(val) = &r.value {
                check_expr_none_comp(val, source, line_starts, findings);
            }
        }
        Stmt::Assign(a) => {
            check_expr_none_comp(&a.value, source, line_starts, findings);
        }
        Stmt::AnnAssign(a) => {
            if let Some(val) = &a.value {
                check_expr_none_comp(val, source, line_starts, findings);
            }
        }
        Stmt::AugAssign(a) => {
            check_expr_none_comp(&a.value, source, line_starts, findings);
        }
        Stmt::Match(m) => {
            for case in &m.cases {
                for s in &case.body {
                    walk_none_comp(s, source, line_starts, findings);
                }
            }
        }
        _ => {}
    }
}

fn check_expr_none_comp(
    expr: &Expr,
    source: &str,
    line_starts: &[usize],
    findings: &mut Vec<Finding>,
) {
    match expr {
        Expr::Compare(c) => {
            for (i, op) in c.ops.iter().enumerate() {
                if i >= c.comparators.len() {
                    continue;
                }
                let (is_eq, is_ne) = match op {
                    CmpOp::Eq => (true, false),
                    CmpOp::NotEq => (false, true),
                    _ => (false, false),
                };
                if !is_eq && !is_ne {
                    continue;
                }
                if !is_none_expr(&c.comparators[i]) {
                    continue;
                }

                let range = c.range();
                let (line, col) = byte_to_line_col(text_size_to_usize(range.start()), line_starts);
                let (end_line, end_col) =
                    byte_to_line_col(text_size_to_usize(range.end()), line_starts);

                let operator_text = if is_eq { "==" } else { "!=" };
                let replacement_op = if is_eq { "is" } else { "is not" };

                let left_end = text_size_to_usize(c.left.end());
                let fix_end = text_size_to_usize(range.end());

                let replacement = format!(" {} None", replacement_op);

                findings.push(Finding {
                    line,
                    col,
                    end_line,
                    end_col,
                    code: "RAB002".to_string(),
                    message: format!(
                        "Use '{}' instead of '{}' for None comparison",
                        replacement_op, operator_text
                    ),
                    fix: Some(Fix {
                        start: left_end,
                        end: fix_end,
                        replacement,
                    }),
                });
            }
            check_expr_none_comp(&c.left, source, line_starts, findings);
            for comp in &c.comparators {
                check_expr_none_comp(comp, source, line_starts, findings);
            }
        }
        Expr::BoolOp(b) => {
            for val in &b.values {
                check_expr_none_comp(val, source, line_starts, findings);
            }
        }
        Expr::BinOp(b) => {
            check_expr_none_comp(&b.left, source, line_starts, findings);
            check_expr_none_comp(&b.right, source, line_starts, findings);
        }
        Expr::UnaryOp(u) => check_expr_none_comp(&u.operand, source, line_starts, findings),
        Expr::Call(c) => {
            check_expr_none_comp(&c.func, source, line_starts, findings);
            for arg in &c.args {
                check_expr_none_comp(arg, source, line_starts, findings);
            }
            for kw in &c.keywords {
                check_expr_none_comp(&kw.value, source, line_starts, findings);
            }
        }
        Expr::IfExp(i) => {
            check_expr_none_comp(&i.test, source, line_starts, findings);
            check_expr_none_comp(&i.body, source, line_starts, findings);
            check_expr_none_comp(&i.orelse, source, line_starts, findings);
        }
        Expr::Subscript(s) => {
            check_expr_none_comp(&s.value, source, line_starts, findings);
            check_expr_none_comp(&s.slice, source, line_starts, findings);
        }
        Expr::Attribute(a) => check_expr_none_comp(&a.value, source, line_starts, findings),
        Expr::NamedExpr(ne) => {
            check_expr_none_comp(&ne.value, source, line_starts, findings);
        }
        Expr::Await(a) => check_expr_none_comp(&a.value, source, line_starts, findings),
        Expr::Lambda(l) => check_expr_none_comp(&l.body, source, line_starts, findings),
        Expr::List(l) => {
            for elt in &l.elts {
                check_expr_none_comp(elt, source, line_starts, findings);
            }
        }
        Expr::Tuple(t) => {
            for elt in &t.elts {
                check_expr_none_comp(elt, source, line_starts, findings);
            }
        }
        Expr::Dict(d) => {
            for key in &d.keys {
                if let Some(k) = key {
                    check_expr_none_comp(k, source, line_starts, findings);
                }
            }
            for val in &d.values {
                check_expr_none_comp(val, source, line_starts, findings);
            }
        }
        _ => {}
    }
}

fn is_none_expr(expr: &Expr) -> bool {
    matches!(expr, Expr::Constant(c) if matches!(&c.value, Constant::None))
}

// -- RAB003: len(x) == 0 --

pub fn check_len_zero(
    source: &str,
    line_starts: &[usize],
    stmts: &[Stmt],
) -> Vec<Finding> {
    let mut findings = Vec::new();
    for stmt in stmts {
        walk_len_zero(stmt, source, line_starts, &mut findings);
    }
    findings
}

fn walk_len_zero(
    stmt: &Stmt,
    source: &str,
    line_starts: &[usize],
    findings: &mut Vec<Finding>,
) {
    match stmt {
        Stmt::FunctionDef(f) => {
            for s in &f.body {
                walk_len_zero(s, source, line_starts, findings);
            }
        }
        Stmt::AsyncFunctionDef(f) => {
            for s in &f.body {
                walk_len_zero(s, source, line_starts, findings);
            }
        }
        Stmt::ClassDef(c) => {
            for s in &c.body {
                walk_len_zero(s, source, line_starts, findings);
            }
        }
        Stmt::If(i) => {
            check_expr_len_zero(&i.test, source, line_starts, findings);
            for s in &i.body {
                walk_len_zero(s, source, line_starts, findings);
            }
            for s in &i.orelse {
                walk_len_zero(s, source, line_starts, findings);
            }
        }
        Stmt::While(w) => {
            check_expr_len_zero(&w.test, source, line_starts, findings);
            for s in &w.body {
                walk_len_zero(s, source, line_starts, findings);
            }
            for s in &w.orelse {
                walk_len_zero(s, source, line_starts, findings);
            }
        }
        Stmt::For(f) => {
            for s in &f.body {
                walk_len_zero(s, source, line_starts, findings);
            }
            for s in &f.orelse {
                walk_len_zero(s, source, line_starts, findings);
            }
        }
        Stmt::AsyncFor(f) => {
            for s in &f.body {
                walk_len_zero(s, source, line_starts, findings);
            }
            for s in &f.orelse {
                walk_len_zero(s, source, line_starts, findings);
            }
        }
        Stmt::With(w) => {
            for s in &w.body {
                walk_len_zero(s, source, line_starts, findings);
            }
        }
        Stmt::AsyncWith(w) => {
            for s in &w.body {
                walk_len_zero(s, source, line_starts, findings);
            }
        }
        Stmt::Try(t) => {
            for s in &t.body {
                walk_len_zero(s, source, line_starts, findings);
            }
            for handler in &t.handlers {
                let ExceptHandler::ExceptHandler(h) = handler;
                for s in &h.body {
                    walk_len_zero(s, source, line_starts, findings);
                }
            }
            for s in &t.orelse {
                walk_len_zero(s, source, line_starts, findings);
            }
            for s in &t.finalbody {
                walk_len_zero(s, source, line_starts, findings);
            }
        }
        Stmt::Expr(e) => {
            check_expr_len_zero(&e.value, source, line_starts, findings);
        }
        Stmt::Return(r) => {
            if let Some(val) = &r.value {
                check_expr_len_zero(val, source, line_starts, findings);
            }
        }
        Stmt::Assign(a) => {
            check_expr_len_zero(&a.value, source, line_starts, findings);
        }
        Stmt::AnnAssign(a) => {
            if let Some(val) = &a.value {
                check_expr_len_zero(val, source, line_starts, findings);
            }
        }
        Stmt::AugAssign(a) => {
            check_expr_len_zero(&a.value, source, line_starts, findings);
        }
        Stmt::Match(m) => {
            for case in &m.cases {
                for s in &case.body {
                    walk_len_zero(s, source, line_starts, findings);
                }
            }
        }
        _ => {}
    }
}

fn is_len_call(expr: &Expr) -> bool {
    matches!(
        expr,
        Expr::Call(c) if matches!(&*c.func, Expr::Name(n) if n.id.as_str() == "len")
            && c.args.len() == 1
    )
}

fn is_zero_int(expr: &Expr) -> bool {
    match expr {
        Expr::Constant(c) => {
            if let Constant::Int(i) = &c.value {
                i == &rustpython_ast::bigint::BigInt::from(0u64)
            } else {
                false
            }
        }
        _ => false,
    }
}

fn get_len_arg(source: &str, len_call: &Expr) -> String {
    if let Expr::Call(c) = len_call {
        if !c.args.is_empty() {
            let arg_range = c.args[0].range();
            let start = text_size_to_usize(arg_range.start());
            let end = text_size_to_usize(arg_range.end());
            return source[start..end].to_string();
        }
    }
    String::new()
}

fn check_expr_len_zero(
    expr: &Expr,
    source: &str,
    line_starts: &[usize],
    findings: &mut Vec<Finding>,
) {
    if let Expr::Compare(c) = expr {
        if c.ops.len() != 1 || c.comparators.len() != 1 {
            return;
        }
        if !is_len_call(&c.left) || !is_zero_int(&c.comparators[0]) {
            return;
        }

        let range = c.range();
        let (line, col) = byte_to_line_col(text_size_to_usize(range.start()), line_starts);
        let (end_line, end_col) =
            byte_to_line_col(text_size_to_usize(range.end()), line_starts);

        let arg_src = get_len_arg(source, &c.left);
        let (message, replacement) = match &c.ops[0] {
            CmpOp::Eq => (
                "Use 'not x' instead of 'len(x) == 0' for emptiness check",
                format!("not {}", arg_src),
            ),
            CmpOp::NotEq => (
                "Use 'x' instead of 'len(x) != 0' for non-emptiness check",
                arg_src.clone(),
            ),
            CmpOp::Gt => (
                "Use 'x' instead of 'len(x) > 0' for non-emptiness check",
                arg_src.clone(),
            ),
            CmpOp::Lt => (
                "Use 'not x' instead of 'len(x) < 1' for emptiness check",
                format!("not {}", arg_src),
            ),
            CmpOp::LtE => (
                "Use 'not x' instead of 'len(x) <= 0' for emptiness check",
                format!("not {}", arg_src),
            ),
            CmpOp::GtE => (
                "Use 'x' instead of 'len(x) >= 1' for non-emptiness check",
                arg_src.clone(),
            ),
            _ => return,
        };

        findings.push(Finding {
            line,
            col,
            end_line,
            end_col,
            code: "RAB003".to_string(),
            message: message.to_string(),
            fix: Some(Fix {
                start: text_size_to_usize(range.start()),
                end: text_size_to_usize(range.end()),
                replacement,
            }),
        });
    }
}

// -- RAB004: List comprehension to generator --

pub fn check_list_to_generator(
    source: &str,
    line_starts: &[usize],
    stmts: &[Stmt],
) -> Vec<Finding> {
    let mut findings = Vec::new();
    for stmt in stmts {
        walk_list_gen(stmt, source, line_starts, &mut findings);
    }
    findings
}

fn walk_list_gen(
    stmt: &Stmt,
    source: &str,
    line_starts: &[usize],
    findings: &mut Vec<Finding>,
) {
    match stmt {
        Stmt::FunctionDef(f) => {
            for s in &f.body {
                walk_list_gen(s, source, line_starts, findings);
            }
        }
        Stmt::AsyncFunctionDef(f) => {
            for s in &f.body {
                walk_list_gen(s, source, line_starts, findings);
            }
        }
        Stmt::ClassDef(c) => {
            for s in &c.body {
                walk_list_gen(s, source, line_starts, findings);
            }
        }
        Stmt::If(i) => {
            check_expr_list_gen(&i.test, source, line_starts, findings);
            for s in &i.body {
                walk_list_gen(s, source, line_starts, findings);
            }
            for s in &i.orelse {
                walk_list_gen(s, source, line_starts, findings);
            }
        }
        Stmt::While(w) => {
            check_expr_list_gen(&w.test, source, line_starts, findings);
            for s in &w.body {
                walk_list_gen(s, source, line_starts, findings);
            }
            for s in &w.orelse {
                walk_list_gen(s, source, line_starts, findings);
            }
        }
        Stmt::For(f) => {
            check_expr_list_gen(&f.iter, source, line_starts, findings);
            for s in &f.body {
                walk_list_gen(s, source, line_starts, findings);
            }
            for s in &f.orelse {
                walk_list_gen(s, source, line_starts, findings);
            }
        }
        Stmt::AsyncFor(f) => {
            check_expr_list_gen(&f.iter, source, line_starts, findings);
            for s in &f.body {
                walk_list_gen(s, source, line_starts, findings);
            }
            for s in &f.orelse {
                walk_list_gen(s, source, line_starts, findings);
            }
        }
        Stmt::With(w) => {
            for s in &w.body {
                walk_list_gen(s, source, line_starts, findings);
            }
        }
        Stmt::AsyncWith(w) => {
            for s in &w.body {
                walk_list_gen(s, source, line_starts, findings);
            }
        }
        Stmt::Try(t) => {
            for s in &t.body {
                walk_list_gen(s, source, line_starts, findings);
            }
            for handler in &t.handlers {
                let ExceptHandler::ExceptHandler(h) = handler;
                for s in &h.body {
                    walk_list_gen(s, source, line_starts, findings);
                }
            }
            for s in &t.orelse {
                walk_list_gen(s, source, line_starts, findings);
            }
            for s in &t.finalbody {
                walk_list_gen(s, source, line_starts, findings);
            }
        }
        Stmt::Expr(e) => {
            check_expr_list_gen(&e.value, source, line_starts, findings);
        }
        Stmt::Return(r) => {
            if let Some(val) = &r.value {
                check_expr_list_gen(val, source, line_starts, findings);
            }
        }
        Stmt::Assign(a) => {
            check_expr_list_gen(&a.value, source, line_starts, findings);
        }
        Stmt::AnnAssign(a) => {
            if let Some(val) = &a.value {
                check_expr_list_gen(val, source, line_starts, findings);
            }
        }
        Stmt::AugAssign(a) => {
            check_expr_list_gen(&a.value, source, line_starts, findings);
        }
        Stmt::Match(m) => {
            for case in &m.cases {
                for s in &case.body {
                    walk_list_gen(s, source, line_starts, findings);
                }
            }
        }
        _ => {}
    }
}

fn check_expr_list_gen(
    expr: &Expr,
    source: &str,
    line_starts: &[usize],
    findings: &mut Vec<Finding>,
) {
    match expr {
        Expr::Call(c) => {
            let func_name = get_call_func_name(&c.func);
            let is_iter_func = matches!(
                func_name.as_deref(),
                Some("any" | "all" | "sum" | "min" | "max")
            );

            if is_iter_func {
                for arg in &c.args {
                    if let Expr::ListComp(lc) = arg {
                        let lc_range = lc.range();
                        let (line, col) =
                            byte_to_line_col(text_size_to_usize(lc_range.start()), line_starts);
                        let (end_line, end_col) =
                            byte_to_line_col(text_size_to_usize(lc_range.end()), line_starts);

                        let arg_name = func_name.as_deref().unwrap_or("");

                        let start = text_size_to_usize(lc_range.start());
                        let end = text_size_to_usize(lc_range.end());

                        // Replace the entire list comp with generator expression
                        let lc_text = &source[start..end];
                        let gen_text = format!("({})", &lc_text[1..lc_text.len() - 1]);

                        findings.push(Finding {
                            line,
                            col,
                            end_line,
                            end_col,
                            code: "RAB004".to_string(),
                            message: format!(
                                "Use a generator expression instead of a list comprehension in '{}()'",
                                arg_name
                            ),
                            fix: Some(Fix {
                                start,
                                end,
                                replacement: gen_text,
                            }),
                        });
                    }
                }
            }

            check_expr_list_gen(&c.func, source, line_starts, findings);
            for arg in &c.args {
                check_expr_list_gen(arg, source, line_starts, findings);
            }
            for kw in &c.keywords {
                check_expr_list_gen(&kw.value, source, line_starts, findings);
            }
        }
        Expr::BoolOp(b) => {
            for val in &b.values {
                check_expr_list_gen(val, source, line_starts, findings);
            }
        }
        Expr::BinOp(b) => {
            check_expr_list_gen(&b.left, source, line_starts, findings);
            check_expr_list_gen(&b.right, source, line_starts, findings);
        }
        Expr::UnaryOp(u) => check_expr_list_gen(&u.operand, source, line_starts, findings),
        Expr::Compare(c) => {
            check_expr_list_gen(&c.left, source, line_starts, findings);
            for comp in &c.comparators {
                check_expr_list_gen(comp, source, line_starts, findings);
            }
        }
        Expr::IfExp(i) => {
            check_expr_list_gen(&i.test, source, line_starts, findings);
            check_expr_list_gen(&i.body, source, line_starts, findings);
            check_expr_list_gen(&i.orelse, source, line_starts, findings);
        }
        Expr::Subscript(s) => {
            check_expr_list_gen(&s.value, source, line_starts, findings);
            check_expr_list_gen(&s.slice, source, line_starts, findings);
        }
        Expr::Attribute(a) => check_expr_list_gen(&a.value, source, line_starts, findings),
        Expr::List(l) => {
            for elt in &l.elts {
                check_expr_list_gen(elt, source, line_starts, findings);
            }
        }
        Expr::Tuple(t) => {
            for elt in &t.elts {
                check_expr_list_gen(elt, source, line_starts, findings);
            }
        }
        Expr::Dict(d) => {
            for key in &d.keys {
                if let Some(k) = key {
                    check_expr_list_gen(k, source, line_starts, findings);
                }
            }
            for val in &d.values {
                check_expr_list_gen(val, source, line_starts, findings);
            }
        }
        Expr::Starred(s) => check_expr_list_gen(&s.value, source, line_starts, findings),
        Expr::NamedExpr(ne) => {
            check_expr_list_gen(&ne.value, source, line_starts, findings);
        }
        Expr::Await(a) => check_expr_list_gen(&a.value, source, line_starts, findings),
        Expr::Lambda(l) => check_expr_list_gen(&l.body, source, line_starts, findings),
        _ => {}
    }
}

fn get_call_func_name(func: &Expr) -> Option<String> {
    match func {
        Expr::Name(n) => Some(n.id.to_string()),
        Expr::Attribute(a) => Some(a.attr.to_string()),
        _ => None,
    }
}

// -- RAB005: for k in d.keys() --

pub fn check_dict_keys_loop(
    _source: &str,
    line_starts: &[usize],
    stmts: &[Stmt],
) -> Vec<Finding> {
    let mut findings = Vec::new();
    for stmt in stmts {
        walk_dict_keys(stmt, _source, line_starts, &mut findings);
    }
    findings
}

fn walk_dict_keys(
    stmt: &Stmt,
    source: &str,
    line_starts: &[usize],
    findings: &mut Vec<Finding>,
) {
    match stmt {
        Stmt::FunctionDef(f) => {
            for s in &f.body {
                walk_dict_keys(s, source, line_starts, findings);
            }
        }
        Stmt::AsyncFunctionDef(f) => {
            for s in &f.body {
                walk_dict_keys(s, source, line_starts, findings);
            }
        }
        Stmt::ClassDef(c) => {
            for s in &c.body {
                walk_dict_keys(s, source, line_starts, findings);
            }
        }
        Stmt::For(f) => {
            check_for_dict_keys(&f.iter, source, line_starts, findings);
            for s in &f.body {
                walk_dict_keys(s, source, line_starts, findings);
            }
            for s in &f.orelse {
                walk_dict_keys(s, source, line_starts, findings);
            }
        }
        Stmt::AsyncFor(f) => {
            check_for_dict_keys(&f.iter, source, line_starts, findings);
            for s in &f.body {
                walk_dict_keys(s, source, line_starts, findings);
            }
            for s in &f.orelse {
                walk_dict_keys(s, source, line_starts, findings);
            }
        }
        Stmt::If(i) => {
            for s in &i.body {
                walk_dict_keys(s, source, line_starts, findings);
            }
            for s in &i.orelse {
                walk_dict_keys(s, source, line_starts, findings);
            }
        }
        Stmt::While(w) => {
            for s in &w.body {
                walk_dict_keys(s, source, line_starts, findings);
            }
            for s in &w.orelse {
                walk_dict_keys(s, source, line_starts, findings);
            }
        }
        Stmt::With(w) => {
            for s in &w.body {
                walk_dict_keys(s, source, line_starts, findings);
            }
        }
        Stmt::AsyncWith(w) => {
            for s in &w.body {
                walk_dict_keys(s, source, line_starts, findings);
            }
        }
        Stmt::Try(t) => {
            for s in &t.body {
                walk_dict_keys(s, source, line_starts, findings);
            }
            for handler in &t.handlers {
                let ExceptHandler::ExceptHandler(h) = handler;
                for s in &h.body {
                    walk_dict_keys(s, source, line_starts, findings);
                }
            }
            for s in &t.orelse {
                walk_dict_keys(s, source, line_starts, findings);
            }
            for s in &t.finalbody {
                walk_dict_keys(s, source, line_starts, findings);
            }
        }
        Stmt::Match(m) => {
            for case in &m.cases {
                for s in &case.body {
                    walk_dict_keys(s, source, line_starts, findings);
                }
            }
        }
        _ => {}
    }
}

fn check_for_dict_keys(
    iter: &Expr,
    source: &str,
    line_starts: &[usize],
    findings: &mut Vec<Finding>,
) {
    if let Expr::Call(c) = iter {
        if is_keys_call(c) {
            let range = c.range();
            let (line, col) =
                byte_to_line_col(text_size_to_usize(range.start()), line_starts);
            let (end_line, end_col) =
                byte_to_line_col(text_size_to_usize(range.end()), line_starts);

            findings.push(Finding {
                line,
                col,
                end_line,
                end_col,
                code: "RAB005".to_string(),
                message: "Use 'for k in d:' instead of 'for k in d.keys()'".to_string(),
                fix: None,
            });
        }
    }
}

fn is_keys_call(c: &ExprCall) -> bool {
    matches!(
        &*c.func,
        Expr::Attribute(a) if a.attr.as_str() == "keys" && c.args.is_empty()
    )
}

// -- RAB006: type(x) == T --

pub fn check_type_comparison(
    source: &str,
    line_starts: &[usize],
    stmts: &[Stmt],
) -> Vec<Finding> {
    let mut findings = Vec::new();
    for stmt in stmts {
        walk_type_comp(stmt, source, line_starts, &mut findings);
    }
    findings
}

fn walk_type_comp(
    stmt: &Stmt,
    source: &str,
    line_starts: &[usize],
    findings: &mut Vec<Finding>,
) {
    match stmt {
        Stmt::FunctionDef(f) => {
            for s in &f.body {
                walk_type_comp(s, source, line_starts, findings);
            }
        }
        Stmt::AsyncFunctionDef(f) => {
            for s in &f.body {
                walk_type_comp(s, source, line_starts, findings);
            }
        }
        Stmt::ClassDef(c) => {
            for s in &c.body {
                walk_type_comp(s, source, line_starts, findings);
            }
        }
        Stmt::If(i) => {
            check_expr_type_comp(&i.test, source, line_starts, findings);
            for s in &i.body {
                walk_type_comp(s, source, line_starts, findings);
            }
            for s in &i.orelse {
                walk_type_comp(s, source, line_starts, findings);
            }
        }
        Stmt::While(w) => {
            check_expr_type_comp(&w.test, source, line_starts, findings);
            for s in &w.body {
                walk_type_comp(s, source, line_starts, findings);
            }
            for s in &w.orelse {
                walk_type_comp(s, source, line_starts, findings);
            }
        }
        Stmt::For(f) => {
            for s in &f.body {
                walk_type_comp(s, source, line_starts, findings);
            }
            for s in &f.orelse {
                walk_type_comp(s, source, line_starts, findings);
            }
        }
        Stmt::AsyncFor(f) => {
            for s in &f.body {
                walk_type_comp(s, source, line_starts, findings);
            }
            for s in &f.orelse {
                walk_type_comp(s, source, line_starts, findings);
            }
        }
        Stmt::With(w) => {
            for s in &w.body {
                walk_type_comp(s, source, line_starts, findings);
            }
        }
        Stmt::AsyncWith(w) => {
            for s in &w.body {
                walk_type_comp(s, source, line_starts, findings);
            }
        }
        Stmt::Try(t) => {
            for s in &t.body {
                walk_type_comp(s, source, line_starts, findings);
            }
            for handler in &t.handlers {
                let ExceptHandler::ExceptHandler(h) = handler;
                for s in &h.body {
                    walk_type_comp(s, source, line_starts, findings);
                }
            }
            for s in &t.orelse {
                walk_type_comp(s, source, line_starts, findings);
            }
            for s in &t.finalbody {
                walk_type_comp(s, source, line_starts, findings);
            }
        }
        Stmt::Expr(e) => {
            check_expr_type_comp(&e.value, source, line_starts, findings);
        }
        Stmt::Return(r) => {
            if let Some(val) = &r.value {
                check_expr_type_comp(val, source, line_starts, findings);
            }
        }
        Stmt::Assign(a) => {
            check_expr_type_comp(&a.value, source, line_starts, findings);
        }
        Stmt::AnnAssign(a) => {
            if let Some(val) = &a.value {
                check_expr_type_comp(val, source, line_starts, findings);
            }
        }
        Stmt::AugAssign(a) => {
            check_expr_type_comp(&a.value, source, line_starts, findings);
        }
        Stmt::Match(m) => {
            for case in &m.cases {
                for s in &case.body {
                    walk_type_comp(s, source, line_starts, findings);
                }
            }
        }
        _ => {}
    }
}

fn check_expr_type_comp(
    expr: &Expr,
    source: &str,
    line_starts: &[usize],
    findings: &mut Vec<Finding>,
) {
    match expr {
        Expr::Compare(c) => {
            if c.ops.len() == 1 && c.comparators.len() == 1 {
                let is_type_call = is_type_call(&c.left);
                let is_eq_or_is = matches!(&c.ops[0], CmpOp::Eq | CmpOp::Is);
                if is_type_call && is_eq_or_is {
                    let range = c.range();
                    let (line, col) =
                        byte_to_line_col(text_size_to_usize(range.start()), line_starts);
                    let (end_line, end_col) =
                        byte_to_line_col(text_size_to_usize(range.end()), line_starts);

                    let type_arg = get_type_call_arg(source, &c.left);
                    let type_name = expr_to_source(source, &c.comparators[0]);

                    findings.push(Finding {
                        line,
                        col,
                        end_line,
                        end_col,
                        code: "RAB006".to_string(),
                        message: format!(
                            "Use 'isinstance({}, {})' instead of 'type(...) == ...'",
                            type_arg, type_name
                        ),
                        fix: Some(Fix {
                            start: text_size_to_usize(range.start()),
                            end: text_size_to_usize(range.end()),
                            replacement: format!("isinstance({}, {})", type_arg, type_name),
                        }),
                    });
                }
            }

            check_expr_type_comp(&c.left, source, line_starts, findings);
            for comp in &c.comparators {
                check_expr_type_comp(comp, source, line_starts, findings);
            }
        }
        Expr::BoolOp(b) => {
            for val in &b.values {
                check_expr_type_comp(val, source, line_starts, findings);
            }
        }
        Expr::BinOp(b) => {
            check_expr_type_comp(&b.left, source, line_starts, findings);
            check_expr_type_comp(&b.right, source, line_starts, findings);
        }
        Expr::UnaryOp(u) => check_expr_type_comp(&u.operand, source, line_starts, findings),
        Expr::Call(c) => {
            check_expr_type_comp(&c.func, source, line_starts, findings);
            for arg in &c.args {
                check_expr_type_comp(arg, source, line_starts, findings);
            }
            for kw in &c.keywords {
                check_expr_type_comp(&kw.value, source, line_starts, findings);
            }
        }
        Expr::IfExp(i) => {
            check_expr_type_comp(&i.test, source, line_starts, findings);
            check_expr_type_comp(&i.body, source, line_starts, findings);
            check_expr_type_comp(&i.orelse, source, line_starts, findings);
        }
        Expr::Subscript(s) => {
            check_expr_type_comp(&s.value, source, line_starts, findings);
            check_expr_type_comp(&s.slice, source, line_starts, findings);
        }
        Expr::Attribute(a) => check_expr_type_comp(&a.value, source, line_starts, findings),
        Expr::NamedExpr(ne) => {
            check_expr_type_comp(&ne.value, source, line_starts, findings);
        }
        Expr::Await(a) => check_expr_type_comp(&a.value, source, line_starts, findings),
        Expr::Lambda(l) => check_expr_type_comp(&l.body, source, line_starts, findings),
        _ => {}
    }
}

fn is_type_call(expr: &Expr) -> bool {
    matches!(
        expr,
        Expr::Call(c) if matches!(&*c.func, Expr::Name(n) if n.id.as_str() == "type")
            && c.args.len() == 1
    )
}

fn get_type_call_arg(source: &str, expr: &Expr) -> String {
    if let Expr::Call(c) = expr {
        if !c.args.is_empty() {
            let arg_range = c.args[0].range();
            let start = text_size_to_usize(arg_range.start());
            let end = text_size_to_usize(arg_range.end());
            return source[start..end].to_string();
        }
    }
    String::new()
}

fn expr_to_source(source: &str, expr: &Expr) -> String {
    let range = expr.range();
    let start = text_size_to_usize(range.start());
    let end = text_size_to_usize(range.end());
    source[start..end].to_string()
}

// -- RAB007: Unnecessary else --

pub fn check_unnecessary_else(
    source: &str,
    line_starts: &[usize],
    stmts: &[Stmt],
) -> Vec<Finding> {
    let mut findings = Vec::new();
    for stmt in stmts {
        walk_unnecessary_else(stmt, source, line_starts, &mut findings);
    }
    findings
}

fn walk_unnecessary_else(
    stmt: &Stmt,
    source: &str,
    line_starts: &[usize],
    findings: &mut Vec<Finding>,
) {
    match stmt {
        Stmt::FunctionDef(f) => {
            for s in &f.body {
                walk_unnecessary_else(s, source, line_starts, findings);
            }
        }
        Stmt::AsyncFunctionDef(f) => {
            for s in &f.body {
                walk_unnecessary_else(s, source, line_starts, findings);
            }
        }
        Stmt::ClassDef(c) => {
            for s in &c.body {
                walk_unnecessary_else(s, source, line_starts, findings);
            }
        }
        Stmt::If(i) => {
            if has_unnecessary_else(i) {
                let range = i.range();
                let (line, col) =
                    byte_to_line_col(text_size_to_usize(range.start()), line_starts);
                let (end_line, end_col) =
                    byte_to_line_col(text_size_to_usize(range.end()), line_starts);

                findings.push(Finding {
                    line,
                    col,
                    end_line,
                    end_col,
                    code: "RAB007".to_string(),
                    message: "Unnecessary 'else' after return/raise/break/continue".to_string(),
                    fix: None,
                });
            }

            for s in &i.body {
                walk_unnecessary_else(s, source, line_starts, findings);
            }
            for s in &i.orelse {
                walk_unnecessary_else(s, source, line_starts, findings);
            }
        }
        Stmt::For(f) => {
            for s in &f.body {
                walk_unnecessary_else(s, source, line_starts, findings);
            }
            for s in &f.orelse {
                walk_unnecessary_else(s, source, line_starts, findings);
            }
        }
        Stmt::AsyncFor(f) => {
            for s in &f.body {
                walk_unnecessary_else(s, source, line_starts, findings);
            }
            for s in &f.orelse {
                walk_unnecessary_else(s, source, line_starts, findings);
            }
        }
        Stmt::While(w) => {
            for s in &w.body {
                walk_unnecessary_else(s, source, line_starts, findings);
            }
            for s in &w.orelse {
                walk_unnecessary_else(s, source, line_starts, findings);
            }
        }
        Stmt::With(w) => {
            for s in &w.body {
                walk_unnecessary_else(s, source, line_starts, findings);
            }
        }
        Stmt::AsyncWith(w) => {
            for s in &w.body {
                walk_unnecessary_else(s, source, line_starts, findings);
            }
        }
        Stmt::Try(t) => {
            for s in &t.body {
                walk_unnecessary_else(s, source, line_starts, findings);
            }
            for handler in &t.handlers {
                let ExceptHandler::ExceptHandler(h) = handler;
                for s in &h.body {
                    walk_unnecessary_else(s, source, line_starts, findings);
                }
            }
            for s in &t.orelse {
                walk_unnecessary_else(s, source, line_starts, findings);
            }
            for s in &t.finalbody {
                walk_unnecessary_else(s, source, line_starts, findings);
            }
        }
        Stmt::Match(m) => {
            for case in &m.cases {
                for s in &case.body {
                    walk_unnecessary_else(s, source, line_starts, findings);
                }
            }
        }
        _ => {}
    }
}

fn has_unnecessary_else(if_stmt: &StmtIf) -> bool {
    if if_stmt.orelse.is_empty() {
        return false;
    }
    if let Some(last) = if_stmt.body.last() {
        matches!(
            last,
            Stmt::Return(_) | Stmt::Raise(_) | Stmt::Break(_) | Stmt::Continue(_)
        )
    } else {
        false
    }
}

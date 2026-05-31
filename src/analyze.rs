use rustpython_ast::*;
use rustpython_parser::{ast, Parse};

pub const CHECK_CODES: &[&str] = &[
    "RAB001", "RAB002", "RAB003", "RAB004", "RAB005", "RAB006", "RAB007",
    "RAB008", "RAB009", "RAB010", "RAB015", "RAB023",
    "RAB029", "RAB030", "RAB032", "RAB034", "RAB101",
];

#[derive(Debug, Clone)]
pub struct Fix {
    pub start: usize,
    pub end: usize,
    pub replacement: String,
}

#[derive(Debug, Clone)]
pub struct Finding {
    pub line: usize,
    pub col: usize,
    pub end_line: usize,
    pub end_col: usize,
    pub code: String,
    pub message: String,
    pub fix: Option<Fix>,
}

pub fn byte_to_line_col(byte_offset: usize, line_starts: &[usize]) -> (usize, usize) {
    let line = match line_starts.binary_search(&byte_offset) {
        Ok(i) => i,
        Err(i) => i.saturating_sub(1),
    };
    let col = byte_offset - line_starts[line];
    (line, col)
}

pub fn text_size_to_usize(ts: TextSize) -> usize {
    u32::from(ts) as usize
}

pub fn pos_to_line_col(start: TextSize, line_starts: &[usize]) -> (usize, usize) {
    let byte = text_size_to_usize(start);
    byte_to_line_col(byte, line_starts)
}

fn compute_line_starts(source: &str) -> Vec<usize> {
    std::iter::once(0)
        .chain(source.chars().enumerate().filter_map(|(i, c)| {
            if c == '\n' {
                Some(i + 1)
            } else {
                None
            }
        }))
        .collect()
}

pub trait Checker {
    fn enter_scope(&mut self) {}
    fn exit_scope(&mut self, _findings: &mut Vec<Finding>) {}
    fn visit_stmt(
        &mut self,
        _stmt: &Stmt,
        _source: &str,
        _line_starts: &[usize],
        _findings: &mut Vec<Finding>,
    ) {
    }
    fn visit_expr(
        &mut self,
        _expr: &Expr,
        _source: &str,
        _line_starts: &[usize],
        _findings: &mut Vec<Finding>,
    ) {
    }
}

pub fn analyze_source(source: &str) -> Vec<Finding> {
    let Ok(module_stmts) = ast::Suite::parse(source, "<source>") else {
        return vec![];
    };

    let line_starts = compute_line_starts(source);
    let mut findings = Vec::new();

    let mut unused_vars = crate::checks::UnusedVarsChecker::new();
    let mut none_comp = crate::checks::NoneComparisonChecker;
    let mut len_zero = crate::checks::LenZeroChecker;
    let mut list_gen = crate::checks::ListGenChecker;
    let mut dict_keys = crate::checks::DictKeysChecker;
    let mut type_comp = crate::checks::TypeComparisonChecker;
    let mut unnecess_else = crate::checks::UnnecessaryElseChecker;
    let mut enumerate = crate::checks::EnumerateChecker;
    let mut str_concat = crate::checks::StrConcatChecker;
    let mut list_set = crate::checks::ListSetChecker;
    let mut dict_keys_in = crate::checks::DictKeysInChecker;
    let mut redundant_call = crate::checks::RedundantCallChecker;
    let mut bool_comp = crate::checks::BoolComparisonChecker;
    let mut bool_ret = crate::checks::BoolReturnChecker;
    let mut bool_call = crate::checks::BoolCallChecker;
    let mut assert_const = crate::checks::AssertConstantChecker;
    let mut complexity = crate::checks::ComplexityChecker::new();

    let checkers: &mut [&mut dyn Checker] = &mut [
        &mut unused_vars,
        &mut none_comp,
        &mut len_zero,
        &mut list_gen,
        &mut dict_keys,
        &mut type_comp,
        &mut unnecess_else,
        &mut enumerate,
        &mut str_concat,
        &mut list_set,
        &mut dict_keys_in,
        &mut redundant_call,
        &mut bool_comp,
        &mut bool_ret,
        &mut bool_call,
        &mut assert_const,
        &mut complexity,
    ];

    for c in checkers.iter_mut() {
        c.enter_scope();
    }
    walk_stmts(&module_stmts, source, &line_starts, checkers, &mut findings);
    for c in checkers.iter_mut() {
        c.exit_scope(&mut findings);
    }

    findings.sort_by_key(|f| (f.line, f.col));
    findings
}

fn walk_stmts(
    stmts: &[Stmt],
    source: &str,
    line_starts: &[usize],
    checkers: &mut [&mut dyn Checker],
    findings: &mut Vec<Finding>,
) {
    for stmt in stmts {
        for c in checkers.iter_mut() {
            c.visit_stmt(stmt, source, line_starts, findings);
        }
        match stmt {
            Stmt::FunctionDef(f) => {
                for expr in &f.decorator_list {
                    walk_expr(expr, source, line_starts, checkers, findings);
                }
                walk_expr_opt(f.returns.as_deref(), source, line_starts, checkers, findings);
                for c in checkers.iter_mut() {
                    c.enter_scope();
                }
                walk_stmts(&f.body, source, line_starts, checkers, findings);
                for c in checkers.iter_mut() {
                    c.exit_scope(findings);
                }
            }
            Stmt::AsyncFunctionDef(f) => {
                for expr in &f.decorator_list {
                    walk_expr(expr, source, line_starts, checkers, findings);
                }
                walk_expr_opt(f.returns.as_deref(), source, line_starts, checkers, findings);
                for c in checkers.iter_mut() {
                    c.enter_scope();
                }
                walk_stmts(&f.body, source, line_starts, checkers, findings);
                for c in checkers.iter_mut() {
                    c.exit_scope(findings);
                }
            }
            Stmt::ClassDef(cd) => {
                for expr in &cd.decorator_list {
                    walk_expr(expr, source, line_starts, checkers, findings);
                }
                for base in &cd.bases {
                    walk_expr(base, source, line_starts, checkers, findings);
                }
                for kw in &cd.keywords {
                    walk_expr(&kw.value, source, line_starts, checkers, findings);
                }
                for c in checkers.iter_mut() {
                    c.enter_scope();
                }
                walk_stmts(&cd.body, source, line_starts, checkers, findings);
                for c in checkers.iter_mut() {
                    c.exit_scope(findings);
                }
            }
            Stmt::Return(r) => {
                walk_expr_opt(r.value.as_deref(), source, line_starts, checkers, findings);
            }
            Stmt::Delete(d) => {
                for target in &d.targets {
                    walk_expr(target, source, line_starts, checkers, findings);
                }
            }
            Stmt::Assign(a) => {
                for target in &a.targets {
                    walk_expr(target, source, line_starts, checkers, findings);
                }
                walk_expr(&a.value, source, line_starts, checkers, findings);
            }
            Stmt::AugAssign(a) => {
                walk_expr(&a.target, source, line_starts, checkers, findings);
                walk_expr(&a.value, source, line_starts, checkers, findings);
            }
            Stmt::AnnAssign(a) => {
                walk_expr(&a.target, source, line_starts, checkers, findings);
                walk_expr_opt(a.value.as_deref(), source, line_starts, checkers, findings);
            }
            Stmt::For(f) => {
                walk_expr(&f.target, source, line_starts, checkers, findings);
                walk_expr(&f.iter, source, line_starts, checkers, findings);
                walk_stmts(&f.body, source, line_starts, checkers, findings);
                walk_stmts(&f.orelse, source, line_starts, checkers, findings);
            }
            Stmt::AsyncFor(f) => {
                walk_expr(&f.target, source, line_starts, checkers, findings);
                walk_expr(&f.iter, source, line_starts, checkers, findings);
                walk_stmts(&f.body, source, line_starts, checkers, findings);
                walk_stmts(&f.orelse, source, line_starts, checkers, findings);
            }
            Stmt::While(w) => {
                walk_expr(&w.test, source, line_starts, checkers, findings);
                walk_stmts(&w.body, source, line_starts, checkers, findings);
                walk_stmts(&w.orelse, source, line_starts, checkers, findings);
            }
            Stmt::If(i) => {
                walk_expr(&i.test, source, line_starts, checkers, findings);
                walk_stmts(&i.body, source, line_starts, checkers, findings);
                walk_stmts(&i.orelse, source, line_starts, checkers, findings);
            }
            Stmt::With(w) => {
                for item in &w.items {
                    walk_expr(&item.context_expr, source, line_starts, checkers, findings);
                    walk_expr_opt(item.optional_vars.as_deref(), source, line_starts, checkers, findings);
                }
                walk_stmts(&w.body, source, line_starts, checkers, findings);
            }
            Stmt::AsyncWith(w) => {
                for item in &w.items {
                    walk_expr(&item.context_expr, source, line_starts, checkers, findings);
                    walk_expr_opt(item.optional_vars.as_deref(), source, line_starts, checkers, findings);
                }
                walk_stmts(&w.body, source, line_starts, checkers, findings);
            }
            Stmt::Raise(r) => {
                walk_expr_opt(r.exc.as_deref(), source, line_starts, checkers, findings);
                walk_expr_opt(r.cause.as_deref(), source, line_starts, checkers, findings);
            }
            Stmt::Try(t) => {
                walk_stmts(&t.body, source, line_starts, checkers, findings);
                for handler in &t.handlers {
                    let ExceptHandler::ExceptHandler(h) = handler;
                    walk_stmts(&h.body, source, line_starts, checkers, findings);
                }
                walk_stmts(&t.orelse, source, line_starts, checkers, findings);
                walk_stmts(&t.finalbody, source, line_starts, checkers, findings);
            }
            Stmt::Assert(a) => {
                walk_expr(&a.test, source, line_starts, checkers, findings);
                walk_expr_opt(a.msg.as_deref(), source, line_starts, checkers, findings);
            }
            Stmt::Import(_) | Stmt::ImportFrom(_) => {}
            Stmt::Global(_) | Stmt::Nonlocal(_) => {}
            Stmt::Expr(e) => {
                walk_expr(&e.value, source, line_starts, checkers, findings);
            }
            Stmt::Match(m) => {
                walk_expr(&m.subject, source, line_starts, checkers, findings);
                for case in &m.cases {
                    walk_stmts(&case.body, source, line_starts, checkers, findings);
                }
            }
            Stmt::TryStar(t) => {
                walk_stmts(&t.body, source, line_starts, checkers, findings);
                for handler in &t.handlers {
                    let ExceptHandler::ExceptHandler(h) = handler;
                    walk_stmts(&h.body, source, line_starts, checkers, findings);
                }
                walk_stmts(&t.orelse, source, line_starts, checkers, findings);
                walk_stmts(&t.finalbody, source, line_starts, checkers, findings);
            }
            Stmt::Pass(_) | Stmt::Break(_) | Stmt::Continue(_) | Stmt::TypeAlias(_) => {}
        }
    }
}

fn walk_expr(
    expr: &Expr,
    source: &str,
    line_starts: &[usize],
    checkers: &mut [&mut dyn Checker],
    findings: &mut Vec<Finding>,
) {
    for c in checkers.iter_mut() {
        c.visit_expr(expr, source, line_starts, findings);
    }
    match expr {
        Expr::BoolOp(b) => {
            for val in &b.values {
                walk_expr(val, source, line_starts, checkers, findings);
            }
        }
        Expr::NamedExpr(ne) => {
            walk_expr(&ne.target, source, line_starts, checkers, findings);
            walk_expr(&ne.value, source, line_starts, checkers, findings);
        }
        Expr::BinOp(b) => {
            walk_expr(&b.left, source, line_starts, checkers, findings);
            walk_expr(&b.right, source, line_starts, checkers, findings);
        }
        Expr::UnaryOp(u) => {
            walk_expr(&u.operand, source, line_starts, checkers, findings);
        }
        Expr::Lambda(l) => {
            walk_expr(&l.body, source, line_starts, checkers, findings);
        }
        Expr::IfExp(if_exp) => {
            walk_expr(&if_exp.test, source, line_starts, checkers, findings);
            walk_expr(&if_exp.body, source, line_starts, checkers, findings);
            walk_expr(&if_exp.orelse, source, line_starts, checkers, findings);
        }
        Expr::Dict(d) => {
            for key in &d.keys {
                if let Some(k) = key {
                    walk_expr(k, source, line_starts, checkers, findings);
                }
            }
            for val in &d.values {
                walk_expr(val, source, line_starts, checkers, findings);
            }
        }
        Expr::Set(s) => {
            for elt in &s.elts {
                walk_expr(elt, source, line_starts, checkers, findings);
            }
        }
        Expr::ListComp(lc) => {
            walk_expr(&lc.elt, source, line_starts, checkers, findings);
            for gen in &lc.generators {
                walk_expr(&gen.target, source, line_starts, checkers, findings);
                walk_expr(&gen.iter, source, line_starts, checkers, findings);
                for cond in &gen.ifs {
                    walk_expr(cond, source, line_starts, checkers, findings);
                }
            }
        }
        Expr::SetComp(sc) => {
            walk_expr(&sc.elt, source, line_starts, checkers, findings);
            for gen in &sc.generators {
                walk_expr(&gen.target, source, line_starts, checkers, findings);
                walk_expr(&gen.iter, source, line_starts, checkers, findings);
                for cond in &gen.ifs {
                    walk_expr(cond, source, line_starts, checkers, findings);
                }
            }
        }
        Expr::DictComp(dc) => {
            walk_expr(&dc.key, source, line_starts, checkers, findings);
            walk_expr(&dc.value, source, line_starts, checkers, findings);
            for gen in &dc.generators {
                walk_expr(&gen.target, source, line_starts, checkers, findings);
                walk_expr(&gen.iter, source, line_starts, checkers, findings);
                for cond in &gen.ifs {
                    walk_expr(cond, source, line_starts, checkers, findings);
                }
            }
        }
        Expr::GeneratorExp(ge) => {
            walk_expr(&ge.elt, source, line_starts, checkers, findings);
            for gen in &ge.generators {
                walk_expr(&gen.target, source, line_starts, checkers, findings);
                walk_expr(&gen.iter, source, line_starts, checkers, findings);
                for cond in &gen.ifs {
                    walk_expr(cond, source, line_starts, checkers, findings);
                }
            }
        }
        Expr::Await(a) => {
            walk_expr(&a.value, source, line_starts, checkers, findings);
        }
        Expr::Yield(y) => {
            walk_expr_opt(y.value.as_deref(), source, line_starts, checkers, findings);
        }
        Expr::YieldFrom(yf) => {
            walk_expr(&yf.value, source, line_starts, checkers, findings);
        }
        Expr::Compare(c) => {
            walk_expr(&c.left, source, line_starts, checkers, findings);
            for comp in &c.comparators {
                walk_expr(comp, source, line_starts, checkers, findings);
            }
        }
        Expr::Call(c) => {
            walk_expr(&c.func, source, line_starts, checkers, findings);
            for arg in &c.args {
                walk_expr(arg, source, line_starts, checkers, findings);
            }
            for kw in &c.keywords {
                walk_expr(&kw.value, source, line_starts, checkers, findings);
            }
        }
        Expr::Constant(_) => {}
        Expr::Attribute(a) => {
            walk_expr(&a.value, source, line_starts, checkers, findings);
        }
        Expr::Subscript(s) => {
            walk_expr(&s.value, source, line_starts, checkers, findings);
            walk_expr(&s.slice, source, line_starts, checkers, findings);
        }
        Expr::Starred(s) => {
            walk_expr(&s.value, source, line_starts, checkers, findings);
        }
        Expr::Name(_) => {}
        Expr::List(l) => {
            for elt in &l.elts {
                walk_expr(elt, source, line_starts, checkers, findings);
            }
        }
        Expr::Tuple(t) => {
            for elt in &t.elts {
                walk_expr(elt, source, line_starts, checkers, findings);
            }
        }
        Expr::Slice(s) => {
            walk_expr_opt(s.lower.as_deref(), source, line_starts, checkers, findings);
            walk_expr_opt(s.upper.as_deref(), source, line_starts, checkers, findings);
            walk_expr_opt(s.step.as_deref(), source, line_starts, checkers, findings);
        }
        _ => {}
    }
}

fn walk_expr_opt(
    expr: Option<&Expr>,
    source: &str,
    line_starts: &[usize],
    checkers: &mut [&mut dyn Checker],
    findings: &mut Vec<Finding>,
) {
    if let Some(e) = expr {
        walk_expr(e, source, line_starts, checkers, findings);
    }
}

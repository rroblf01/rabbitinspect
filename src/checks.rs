use crate::analyze::{byte_to_line_col, count_fn_args, iter_fn_args, text_size_to_usize, Checker, Finding, Fix};
use rustc_hash::FxHashSet;
use rustpython_ast::*;

// ── RAB001: Unused variables ──────────────────────────────────────────────

pub struct UnusedVarsChecker {
    assigned: Vec<(String, usize, usize)>,
    used: Vec<String>,
    scope_stack: Vec<(Vec<(String, usize, usize)>, Vec<String>)>,
}

impl UnusedVarsChecker {
    pub fn new() -> Self {
        Self {
            assigned: Vec::new(),
            used: Vec::new(),
            scope_stack: Vec::new(),
        }
    }

    fn collect_names_from_target(&mut self, expr: &Expr, line_starts: &[usize]) {
        match expr {
            Expr::Name(n) => {
                let (line, col) = byte_to_line_col(text_size_to_usize(n.range().start()), line_starts);
                self.assigned.push((n.id.to_string(), line, col));
            }
            Expr::Tuple(t) => {
                for elt in &t.elts {
                    self.collect_names_from_target(elt, line_starts);
                }
            }
            Expr::List(l) => {
                for elt in &l.elts {
                    self.collect_names_from_target(elt, line_starts);
                }
            }
            Expr::Starred(s) => {
                self.collect_names_from_target(&s.value, line_starts);
            }
            _ => {}
        }
    }

    fn add_arg(&mut self, name: &str) {
        self.assigned.push((name.to_string(), 0, 0));
    }
}

impl Checker for UnusedVarsChecker {
    fn enter_scope(&mut self) {
        self.scope_stack.push((
            std::mem::take(&mut self.assigned),
            std::mem::take(&mut self.used),
        ));
    }

    fn exit_scope(&mut self, findings: &mut Vec<Finding>) {
        let used_set: FxHashSet<&str> = self.used.iter().map(|s| s.as_str()).collect();
        for (name, line, col) in &self.assigned {
            if name.starts_with('_') {
                continue;
            }
            if !used_set.contains(name.as_str()) {
                findings.push(Finding {
                    line: *line,
                    col: *col,
                    end_line: 0,
                    end_col: 0,
                    code: "RAB001".to_string(),
                    message: format!("Variable '{}' is assigned but never used", name),
                    fix: None,
                });
            }
        }
        if let Some((parent_assigned, parent_used)) = self.scope_stack.pop() {
            self.assigned = parent_assigned;
            self.used = parent_used;
        }
    }

    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, line_starts: &[usize], _findings: &mut Vec<Finding>) {
        match stmt {
            Stmt::FunctionDef(f) => {
                for arg in iter_fn_args(&f.args) {
                    self.add_arg(&arg.def.arg);
                }
                if let Some(vararg) = &f.args.vararg {
                    self.add_arg(&vararg.arg);
                }
                if let Some(kwarg) = &f.args.kwarg {
                    self.add_arg(&kwarg.arg);
                }
            }
            Stmt::AsyncFunctionDef(f) => {
                for arg in iter_fn_args(&f.args) {
                    self.add_arg(&arg.def.arg);
                }
                if let Some(vararg) = &f.args.vararg {
                    self.add_arg(&vararg.arg);
                }
                if let Some(kwarg) = &f.args.kwarg {
                    self.add_arg(&kwarg.arg);
                }
            }
            Stmt::Assign(a) => {
                for target in &a.targets {
                    self.collect_names_from_target(target, line_starts);
                }
            }
            Stmt::AnnAssign(a) => {
                self.collect_names_from_target(&a.target, line_starts);
            }
            Stmt::AugAssign(a) => {
                self.collect_names_from_target(&a.target, line_starts);
            }
            Stmt::For(f) => {
                self.collect_names_from_target(&f.target, line_starts);
            }
            Stmt::AsyncFor(f) => {
                self.collect_names_from_target(&f.target, line_starts);
            }
            Stmt::With(w) => {
                for item in &w.items {
                    if let Some(optional_vars) = &item.optional_vars {
                        self.collect_names_from_target(optional_vars, line_starts);
                    }
                }
            }
            Stmt::AsyncWith(w) => {
                for item in &w.items {
                    if let Some(optional_vars) = &item.optional_vars {
                        self.collect_names_from_target(optional_vars, line_starts);
                    }
                }
            }
            Stmt::Import(i) => {
                for alias in &i.names {
                    let name = alias
                        .asname
                        .clone()
                        .unwrap_or_else(|| alias.name.clone());
                    let short = name.split('.').next().unwrap_or(&name).to_string();
                    // Use the alias range for position, or just add with 0
                    self.assigned.push((short, 0, 0));
                }
            }
            Stmt::ImportFrom(i) => {
                for alias in &i.names {
                    let name = alias
                        .asname
                        .clone()
                        .unwrap_or_else(|| alias.name.clone());
                    self.assigned.push((name.to_string(), 0, 0));
                }
            }
            Stmt::Try(t) => {
                for handler in &t.handlers {
                    let ExceptHandler::ExceptHandler(h) = handler;
                    if let Some(name) = &h.name {
                        self.assigned.push((name.to_string(), 0, 0));
                    }
                }
            }
            _ => {}
        }
    }

    fn visit_expr(&mut self, expr: &Expr, _source: &str, _line_starts: &[usize], _findings: &mut Vec<Finding>) {
        if let Expr::Name(n) = expr {
            if n.ctx == ExprContext::Load {
                self.used.push(n.id.to_string());
            }
        }
    }
}

// ── RAB002: None comparison ───────────────────────────────────────────────

pub struct NoneComparisonChecker;

impl Checker for NoneComparisonChecker {
    fn visit_expr(&mut self, expr: &Expr, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        if let Expr::Compare(c) = expr {
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
                let rhs = &c.comparators[i];
                if !matches!(rhs, Expr::Constant(cc) if matches!(&cc.value, Constant::None)) {
                    continue;
                }

                let range = c.range();
                let (line, col) = byte_to_line_col(text_size_to_usize(range.start()), line_starts);
                let (end_line, end_col) = byte_to_line_col(text_size_to_usize(range.end()), line_starts);

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
        }
    }
}

// ── RAB003: len(x) == 0 ───────────────────────────────────────────────────

pub struct LenZeroChecker;

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

impl Checker for LenZeroChecker {
    fn visit_expr(&mut self, expr: &Expr, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        if let Expr::Compare(c) = expr {
            if c.ops.len() != 1 || c.comparators.len() != 1 {
                return;
            }
            if !is_len_call(&c.left) || !is_zero_int(&c.comparators[0]) {
                return;
            }

            let range = c.range();
            let (line, col) = byte_to_line_col(text_size_to_usize(range.start()), line_starts);
            let (end_line, end_col) = byte_to_line_col(text_size_to_usize(range.end()), line_starts);

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
}

// ── RAB004: List comprehension in any/all/sum/min/max ─────────────────────

pub struct ListGenChecker;

fn get_call_func_name(func: &Expr) -> Option<String> {
    match func {
        Expr::Name(n) => Some(n.id.to_string()),
        Expr::Attribute(a) => Some(a.attr.to_string()),
        _ => None,
    }
}

impl Checker for ListGenChecker {
    fn visit_expr(&mut self, expr: &Expr, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        if let Expr::Call(c) = expr {
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

                        let start = text_size_to_usize(lc_range.start());
                        let end = text_size_to_usize(lc_range.end());
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
                                func_name.as_deref().unwrap_or("")
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
        }
    }
}

// ── RAB005: for k in d.keys() ─────────────────────────────────────────────

pub struct DictKeysChecker;

impl Checker for DictKeysChecker {
    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let iter = match stmt {
            Stmt::For(f) => &f.iter,
            Stmt::AsyncFor(f) => &f.iter,
            _ => return,
        };

        if let Expr::Call(c) = &**iter {
            if let Expr::Attribute(a) = &*c.func {
                if a.attr.as_str() == "keys" && c.args.is_empty() {
                    let range = c.range();
                    let (line, col) = byte_to_line_col(text_size_to_usize(range.start()), line_starts);
                    let (end_line, end_col) = byte_to_line_col(text_size_to_usize(range.end()), line_starts);
                    let dot_start = text_size_to_usize(a.value.end());
                    let call_end = text_size_to_usize(c.range().end());

                    findings.push(Finding {
                        line,
                        col,
                        end_line,
                        end_col,
                        code: "RAB005".to_string(),
                        message: "Use 'for k in d:' instead of 'for k in d.keys()'".to_string(),
                        fix: Some(Fix {
                            start: dot_start,
                            end: call_end,
                            replacement: String::new(),
                        }),
                    });
                }
            }
        }
    }
}

// ── RAB006: type(x) == T ──────────────────────────────────────────────────

pub struct TypeComparisonChecker;

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

impl Checker for TypeComparisonChecker {
    fn visit_expr(&mut self, expr: &Expr, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        if let Expr::Compare(c) = expr {
            if c.ops.len() == 1 && c.comparators.len() == 1 {
                let is_type = is_type_call(&c.left);
                let is_eq_or_is = matches!(&c.ops[0], CmpOp::Eq | CmpOp::Is);
                if is_type && is_eq_or_is {
                    let range = c.range();
                    let (line, col) = byte_to_line_col(text_size_to_usize(range.start()), line_starts);
                    let (end_line, end_col) = byte_to_line_col(text_size_to_usize(range.end()), line_starts);

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
        }
    }
}

// ── RAB007: Unnecessary else after return/raise/break/continue ────────────

pub struct UnnecessaryElseChecker;

impl Checker for UnnecessaryElseChecker {
    fn visit_stmt(&mut self, stmt: &Stmt, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        if let Stmt::If(i) = stmt {
            if !i.orelse.is_empty() {
                if let Some(last) = i.body.last() {
                    let is_terminal = matches!(
                        last,
                        Stmt::Return(_) | Stmt::Raise(_) | Stmt::Break(_) | Stmt::Continue(_)
                    );
                    if is_terminal {
                        // Skip if orelse is an elif chain (not a plain else)
                        if i.orelse.len() == 1 && matches!(&i.orelse[0], Stmt::If(_)) {
                            return;
                        }
                        // Skip if RAB030 will handle this (orelse is single boolean return)
                        if i.orelse.len() == 1 {
                            if let Stmt::Return(r) = &i.orelse[0] {
                                if let Some(val) = &r.value {
                                    if matches!(&**val, Expr::Constant(cc) if matches!(&cc.value, Constant::Bool(_))) {
                                        return;
                                    }
                                }
                            }
                        }
                        let range = i.range();
                        let (line, col) = byte_to_line_col(text_size_to_usize(range.start()), line_starts);
                        let (end_line, end_col) = byte_to_line_col(text_size_to_usize(range.end()), line_starts);

                        let body_end = text_size_to_usize(last.end());
                        let if_end = text_size_to_usize(i.range().end());
                        let search_region = &source[body_end..if_end];

                        let else_pos = search_region.find("else:");
                        let else_start = else_pos.map(|p| body_end + p).unwrap_or(if_end);

                        let replacement = if let Some(ep) = else_pos {
                            let else_text = &source[body_end + ep..if_end];
                            let mut lines: Vec<&str> = else_text.split('\n').collect();
                            if !lines.is_empty() {
                                lines.remove(0);
                            }
                            let unindented: Vec<String> = lines
                                .iter()
                                .map(|line| {
                                    if line.starts_with("    ") {
                                        line[4..].to_string()
                                    } else {
                                        line.to_string()
                                    }
                                })
                                .collect();
                            unindented.join("\n")
                        } else {
                            String::new()
                        };

                        findings.push(Finding {
                            line,
                            col,
                            end_line,
                            end_col,
                            code: "RAB007".to_string(),
                            message: "Unnecessary 'else' after return/raise/break/continue".to_string(),
                            fix: Some(Fix {
                                start: else_start,
                                end: if_end,
                                replacement,
                            }),
                        });
                    }
                }
            }
        }
    }
}

// ── RAB008: for i in range(len(x)) → enumerate ───────────────────────────

pub struct EnumerateChecker;

impl Checker for EnumerateChecker {
    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let (iter, _) = match stmt {
            Stmt::For(f) => (&f.iter, "for"),
            Stmt::AsyncFor(f) => (&f.iter, "async for"),
            _ => return,
        };

        if let Expr::Call(c) = &**iter {
            let is_range_len = matches!(
                &*c.func,
                Expr::Name(n) if n.id.as_str() == "range"
                    && c.args.len() == 1
                    && matches!(&c.args[0], Expr::Call(len_call) if len_call.args.len() == 1
                        && matches!(&*len_call.func, Expr::Name(ln) if ln.id.as_str() == "len"))
            );
            if is_range_len {
                let range = c.range();
                let (line, col) = byte_to_line_col(text_size_to_usize(range.start()), line_starts);
                let (end_line, end_col) = byte_to_line_col(text_size_to_usize(range.end()), line_starts);

                findings.push(Finding {
                    line,
                    col,
                    end_line,
                    end_col,
                    code: "RAB008".to_string(),
                    message: "Use 'enumerate()' instead of 'range(len())' for indexed iteration".to_string(),
                    fix: None,
                });
            }
        }
    }
}

// ── RAB009: String concatenation in loop ──────────────────────────────────

pub struct StrConcatChecker;

impl Checker for StrConcatChecker {
    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        if let Stmt::AugAssign(a) = stmt {
            if !matches!(a.op, Operator::Add) {
                return;
            }
            let is_str = matches!(&*a.value, Expr::Call(c) if matches!(&*c.func, Expr::Name(n) if n.id.as_str() == "str"));
            let is_chr_add = matches!(&*a.value, Expr::Name(_));
            if is_str || is_chr_add {
                let range = a.range();
                let (line, col) = byte_to_line_col(text_size_to_usize(range.start()), line_starts);
                let (end_line, end_col) = byte_to_line_col(text_size_to_usize(range.end()), line_starts);

                findings.push(Finding {
                    line,
                    col,
                    end_line,
                    end_col,
                    code: "RAB009".to_string(),
                    message: "Use 'str.join()' instead of string concatenation in a loop".to_string(),
                    fix: None,
                });
            }
        }
    }
}

// ── RAB010: set(list(x)) → set(x) ─────────────────────────────────────────

pub struct ListSetChecker;

impl Checker for ListSetChecker {
    fn visit_expr(&mut self, expr: &Expr, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        if let Expr::Call(c) = expr {
            let is_set = matches!(&*c.func, Expr::Name(n) if n.id.as_str() == "set");
            if !is_set || c.args.is_empty() {
                return;
            }
            if let Expr::Call(inner) = &c.args[0] {
                let is_list = matches!(&*inner.func, Expr::Name(n) if n.id.as_str() == "list");
                if is_list && inner.args.len() == 1 {
                    let range = c.range();
                    let (line, col) = byte_to_line_col(text_size_to_usize(range.start()), line_starts);
                    let (end_line, end_col) = byte_to_line_col(text_size_to_usize(range.end()), line_starts);

                    findings.push(Finding {
                        line,
                        col,
                        end_line,
                        end_col,
                        code: "RAB010".to_string(),
                        message: "Unnecessary 'list()' call inside 'set()', use 'set(...)' directly".to_string(),
                        fix: None,
                    });
                }
            }
        }
    }
}

// ── RAB015: k in d.keys() → k in d ────────────────────────────────────────

pub struct DictKeysInChecker;

impl Checker for DictKeysInChecker {
    fn visit_expr(&mut self, expr: &Expr, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        if let Expr::Compare(c) = expr {
            if c.ops.len() == 1 && matches!(c.ops[0], CmpOp::In | CmpOp::NotIn) && c.comparators.len() == 1 {
                if let Expr::Call(kc) = &c.comparators[0] {
                    if let Expr::Attribute(a) = &*kc.func {
                        if a.attr.as_str() == "keys" && kc.args.is_empty() {
                            let range = c.range();
                            let (line, col) = byte_to_line_col(text_size_to_usize(range.start()), line_starts);
                            let (end_line, end_col) = byte_to_line_col(text_size_to_usize(range.end()), line_starts);
                            let dot_start = text_size_to_usize(a.value.end());
                            let call_end = text_size_to_usize(kc.range().end());

                            findings.push(Finding {
                                line,
                                col,
                                end_line,
                                end_col,
                                code: "RAB015".to_string(),
                                message: "Use 'k in d' instead of 'k in d.keys()'".to_string(),
                                fix: Some(Fix {
                                    start: dot_start,
                                    end: call_end,
                                    replacement: String::new(),
                                }),
                            });
                        }
                    }
                }
            }
        }
    }
}

// ── RAB023: Redundant `.call()` method ────────────────────────────────────

pub struct RedundantCallChecker;

impl Checker for RedundantCallChecker {
    fn visit_expr(&mut self, expr: &Expr, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        if let Expr::Call(c) = expr {
            if matches!(&*c.func, Expr::Attribute(a) if a.attr.as_str() == "call" && c.args.is_empty()) {
                let range = c.range();
                let (line, col) = byte_to_line_col(text_size_to_usize(range.start()), line_starts);
                let (end_line, end_col) = byte_to_line_col(text_size_to_usize(range.end()), line_starts);

                findings.push(Finding {
                    line,
                    col,
                    end_line,
                    end_col,
                    code: "RAB023".to_string(),
                    message: "Redundant '.call()' call, call the object directly".to_string(),
                    fix: None,
                });
            }
        }
    }
}

// ── RAB029: x == True / x == False → x / not x ───────────────────────────

pub struct BoolComparisonChecker;

impl Checker for BoolComparisonChecker {
    fn visit_expr(&mut self, expr: &Expr, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        if let Expr::Compare(c) = expr {
            if c.ops.len() != 1 || c.comparators.len() != 1 {
                return;
            }
            let is_bool = match &c.comparators[0] {
                Expr::Constant(cc) => matches!(&cc.value, Constant::Bool(_)),
                _ => false,
            };
            if !is_bool {
                return;
            }
            let (is_eq, _is_ne) = match &c.ops[0] {
                CmpOp::Eq => (true, false),
                CmpOp::NotEq => (false, true),
                _ => return,
            };
            let is_true = match &c.comparators[0] {
                Expr::Constant(cc) => matches!(&cc.value, Constant::Bool(true)),
                _ => false,
            };
            let range = c.range();
            let start = text_size_to_usize(range.start());
            let end = text_size_to_usize(range.end());
            let (line, col) = byte_to_line_col(start, line_starts);
            let (end_line, end_col) = byte_to_line_col(end, line_starts);

            let left_src = expr_to_source(source, &c.left);
            let (msg, replacement) = match (is_eq, is_true) {
                (true, true) => ("Redundant equality with True", left_src.clone()),
                (true, false) => ("Use 'not x' instead of 'x == False'", format!("not {}", left_src)),
                (false, true) => ("Use 'not x' instead of 'x != True'", format!("not {}", left_src)),
                (false, false) => ("Redundant inequality with False", left_src),
            };

            findings.push(Finding {
                line,
                col,
                end_line,
                end_col,
                code: "RAB029".to_string(),
                message: msg.to_string(),
                fix: Some(Fix { start, end, replacement }),
            });
        }
    }
}

// ── RAB030: if cond: return True else: return False → return cond ────────

pub struct BoolReturnChecker;

impl Checker for BoolReturnChecker {
    fn visit_stmt(&mut self, stmt: &Stmt, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        if let Stmt::If(i) = stmt {
            if i.orelse.is_empty() || i.body.len() != 1 || i.orelse.len() != 1 {
                return;
            }
            let then_ret = match &i.body[0] {
                Stmt::Return(r) => r.value.as_deref(),
                _ => return,
            };
            let else_ret = match &i.orelse[0] {
                Stmt::Return(r) => r.value.as_deref(),
                _ => return,
            };
            let (then_true, then_false) = match then_ret {
                Some(Expr::Constant(c)) => (matches!(&c.value, Constant::Bool(true)), matches!(&c.value, Constant::Bool(false))),
                _ => return,
            };
            let (else_true, else_false) = match else_ret {
                Some(Expr::Constant(c)) => (matches!(&c.value, Constant::Bool(true)), matches!(&c.value, Constant::Bool(false))),
                _ => return,
            };
            if !((then_true && else_false) || (then_false && else_true)) {
                return;
            }
            let range = i.range();
            let start = text_size_to_usize(range.start());
            let end = text_size_to_usize(range.end());
            let (line, col) = byte_to_line_col(start, line_starts);
            let (end_line, end_col) = byte_to_line_col(end, line_starts);

            let cond_src = expr_to_source(source, &i.test);
            let negate = then_false && else_true;
            let replacement = if negate {
                let needs_parens = cond_src.contains(' ') || cond_src.contains('(');
                if needs_parens {
                    format!("return not ({})", cond_src)
                } else {
                    format!("return not {}", cond_src)
                }
            } else {
                format!("return {}", cond_src)
            };

            findings.push(Finding {
                line,
                col,
                end_line,
                end_col,
                code: "RAB030".to_string(),
                message: "Unnecessary if/else returning boolean literals, use direct return".to_string(),
                fix: Some(Fix { start, end, replacement }),
            });
        }
    }
}

// ── RAB032: bool(x) → x ──────────────────────────────────────────────────

pub struct BoolCallChecker;

impl Checker for BoolCallChecker {
    fn visit_expr(&mut self, expr: &Expr, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        if let Expr::Call(c) = expr {
            if !matches!(&*c.func, Expr::Name(n) if n.id.as_str() == "bool") {
                return;
            }
            if c.args.len() != 1 {
                return;
            }
            let range = c.range();
            let start = text_size_to_usize(range.start());
            let end = text_size_to_usize(range.end());
            let (line, col) = byte_to_line_col(start, line_starts);
            let (end_line, end_col) = byte_to_line_col(end, line_starts);

            let arg_src = expr_to_source(source, &c.args[0]);

            findings.push(Finding {
                line,
                col,
                end_line,
                end_col,
                code: "RAB032".to_string(),
                message: "Unnecessary 'bool()' call, use the value directly".to_string(),
                fix: Some(Fix { start, end, replacement: arg_src }),
            });
        }
    }
}

// ── RAB034: assert True / assert False ────────────────────────────────────

pub struct AssertConstantChecker;

impl Checker for AssertConstantChecker {
    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        if let Stmt::Assert(a) = stmt {
            let is_true = matches!(&*a.test, Expr::Constant(c) if matches!(&c.value, Constant::Bool(true)));
            let is_false = matches!(&*a.test, Expr::Constant(c) if matches!(&c.value, Constant::Bool(false)));
            if !is_true && !is_false {
                return;
            }
            let range = a.range();
            let start = text_size_to_usize(range.start());
            let end = text_size_to_usize(range.end());
            let (line, col) = byte_to_line_col(start, line_starts);
            let (end_line, end_col) = byte_to_line_col(end, line_starts);

            let (msg, replacement) = if is_true {
                ("Assertion with 'True' is always a no-op, remove it".to_string(), String::new())
            } else {
                (
                    "Assertion with 'False' will always fail, use 'raise AssertionError' or remove it".to_string(),
                    "raise AssertionError".to_string(),
                )
            };

            findings.push(Finding {
                line,
                col,
                end_line,
                end_col,
                code: "RAB034".to_string(),
                message: msg,
                fix: Some(Fix { start, end, replacement }),
            });
        }
    }
}

// ── RAB101: Cyclomatic complexity ─────────────────────────────────────────

const COMPLEXITY_THRESHOLD: u32 = 10;

pub struct ComplexityChecker {
    complexity: u32,
    scope_stack: Vec<u32>,
}

impl ComplexityChecker {
    pub fn new() -> Self {
        Self {
            complexity: 1,
            scope_stack: Vec::new(),
        }
    }
}

impl Checker for ComplexityChecker {
    fn enter_scope(&mut self) {
        self.scope_stack.push(std::mem::replace(&mut self.complexity, 1));
    }

    fn exit_scope(&mut self, findings: &mut Vec<Finding>) {
        if self.complexity > COMPLEXITY_THRESHOLD {
            findings.push(Finding {
                line: 0,
                col: 0,
                end_line: 0,
                end_col: 0,
                code: "RAB101".to_string(),
                message: format!(
                    "Cyclomatic complexity is {} (threshold: {}), consider simplifying",
                    self.complexity, COMPLEXITY_THRESHOLD
                ),
                fix: None,
            });
        }
        if let Some(parent) = self.scope_stack.pop() {
            self.complexity = parent;
        }
    }

    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, _line_starts: &[usize], _findings: &mut Vec<Finding>) {
        match stmt {
            Stmt::If(i) => {
                self.complexity += 1;
                for elif in &i.orelse {
                    if let Stmt::If(_) = elif {
                        self.complexity += 1;
                    }
                }
            }
            Stmt::For(_) | Stmt::AsyncFor(_) | Stmt::While(_) => {
                self.complexity += 1;
            }
            Stmt::Try(t) => {
                let handlers = t.handlers.len() as u32;
                self.complexity += handlers;
            }
            Stmt::Match(m) => {
                self.complexity += m.cases.len() as u32;
            }
            Stmt::Assert(_) => {
                self.complexity += 1;
            }
            Stmt::With(_) | Stmt::AsyncWith(_) => {
                self.complexity += 1;
            }
            _ => {}
        }
    }

    fn visit_expr(&mut self, expr: &Expr, _source: &str, _line_starts: &[usize], _findings: &mut Vec<Finding>) {
        if let Expr::BoolOp(b) = expr {
            if b.values.len() > 1 {
                self.complexity += b.values.len() as u32 - 1;
            }
        }
    }
}

// ── RAB011: Mutable default argument ─────────────────────────────────────

pub struct MutableDefaultChecker;

impl Checker for MutableDefaultChecker {
    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let args = match stmt {
            Stmt::FunctionDef(f) => &f.args,
            Stmt::AsyncFunctionDef(f) => &f.args,
            _ => return,
        };
        for arg in iter_fn_args(args) {
            if let Some(default) = &arg.default {
                if matches!(default.as_ref(), Expr::List(_) | Expr::Dict(_) | Expr::Set(_)) {
                    let range = default.range();
                    let (line, col) = byte_to_line_col(text_size_to_usize(range.start()), line_starts);
                    let (end_line, end_col) = byte_to_line_col(text_size_to_usize(range.end()), line_starts);
                    findings.push(Finding {
                        line, col, end_line, end_col,
                        code: "RAB011".to_string(),
                        message: "Mutable default argument, use 'None' instead and initialize inside the function".to_string(),
                        fix: None,
                    });
                }
            }
        }
    }
}

// ── RAB012: Bare except ──────────────────────────────────────────────────

pub struct BareExceptChecker;

impl Checker for BareExceptChecker {
    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        if let Stmt::Try(t) = stmt {
            for handler in &t.handlers {
                let ExceptHandler::ExceptHandler(h) = handler;
                if h.type_.is_none() && h.name.is_none() {
                    let range = handler.range();
                    let (line, col) = byte_to_line_col(text_size_to_usize(range.start()), line_starts);
                    let (end_line, end_col) = byte_to_line_col(text_size_to_usize(range.end()), line_starts);
                    findings.push(Finding {
                        line, col, end_line, end_col,
                        code: "RAB012".to_string(),
                        message: "Bare 'except:' catches all exceptions including SystemExit/KeyboardInterrupt, specify exception type".to_string(),
                        fix: None,
                    });
                }
            }
        }
    }
}

// ── RAB013: Bare except with pass ────────────────────────────────────────

pub struct BareExceptPassChecker;

impl Checker for BareExceptPassChecker {
    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        if let Stmt::Try(t) = stmt {
            for handler in &t.handlers {
                let ExceptHandler::ExceptHandler(h) = handler;
                if h.type_.is_none() && h.name.is_none() && h.body.len() == 1 {
                    if let Stmt::Pass(_) = &h.body[0] {
                        let range = handler.range();
                        let (line, col) = byte_to_line_col(text_size_to_usize(range.start()), line_starts);
                        let (end_line, end_col) = byte_to_line_col(text_size_to_usize(range.end()), line_starts);
                        findings.push(Finding {
                            line, col, end_line, end_col,
                            code: "RAB013".to_string(),
                            message: "Bare 'except: pass' silently swallows all exceptions, at minimum log the error".to_string(),
                            fix: None,
                        });
                    }
                }
            }
        }
    }
}

// ── RAB014: class Foo(object) ────────────────────────────────────────────

pub struct ClassObjectChecker;

impl Checker for ClassObjectChecker {
    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        if let Stmt::ClassDef(c) = stmt {
            for base in &c.bases {
                if matches!(base, Expr::Name(n) if n.id.as_str() == "object") {
                    let range = base.range();
                    let (line, col) = byte_to_line_col(text_size_to_usize(range.start()), line_starts);
                    let (end_line, end_col) = byte_to_line_col(text_size_to_usize(range.end()), line_starts);
                    findings.push(Finding {
                        line, col, end_line, end_col,
                        code: "RAB014".to_string(),
                        message: "Redundant 'object' base class in Python 3, use 'class Foo:' directly".to_string(),
                        fix: None,
                    });
                }
            }
        }
    }
}

// ── RAB016: .format() instead of f-string ────────────────────────────────

pub struct FormatCallChecker;

impl Checker for FormatCallChecker {
    fn visit_expr(&mut self, expr: &Expr, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        if let Expr::Call(c) = expr {
            if let Expr::Attribute(a) = &*c.func {
                if a.attr.as_str() == "format" && !c.args.is_empty() {
                    let range = c.range();
                    let (line, col) = byte_to_line_col(text_size_to_usize(range.start()), line_starts);
                    let (end_line, end_col) = byte_to_line_col(text_size_to_usize(range.end()), line_starts);
                    findings.push(Finding {
                        line, col, end_line, end_col,
                        code: "RAB016".to_string(),
                        message: "Use f-string instead of '.format()' for better readability and performance".to_string(),
                        fix: None,
                    });
                }
            }
        }
    }
}

// ── RAB017: Function too long ────────────────────────────────────────────

const MAX_FUNCTION_STMTS: u32 = 30;

pub struct FunctionLengthChecker {
    count: u32,
    scope_stack: Vec<u32>,
}

impl FunctionLengthChecker {
    pub fn new() -> Self {
        Self { count: 0, scope_stack: Vec::new() }
    }
}

impl Checker for FunctionLengthChecker {
    fn enter_scope(&mut self) {
        self.scope_stack.push(std::mem::replace(&mut self.count, 0));
    }

    fn exit_scope(&mut self, findings: &mut Vec<Finding>) {
        // Only report inside function scopes, not at module level
        if self.count > MAX_FUNCTION_STMTS && self.scope_stack.len() > 1 {
            findings.push(Finding {
                line: 0, col: 0, end_line: 0, end_col: 0,
                code: "RAB017".to_string(),
                message: format!(
                    "Function contains {} statements (threshold: {}), consider refactoring",
                    self.count, MAX_FUNCTION_STMTS
                ),
                fix: None,
            });
        }
        if let Some(parent) = self.scope_stack.pop() {
            self.count = parent;
        }
    }

    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, _line_starts: &[usize], _findings: &mut Vec<Finding>) {
        match stmt {
            Stmt::FunctionDef(_) | Stmt::AsyncFunctionDef(_) | Stmt::ClassDef(_) => {}
            _ => self.count += 1,
        }
    }
}

// ── RAB018: Too many parameters ──────────────────────────────────────────

pub struct TooManyParamsChecker;

impl Checker for TooManyParamsChecker {
    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, _line_starts: &[usize], findings: &mut Vec<Finding>) {
        let (args, name) = match stmt {
            Stmt::FunctionDef(f) => (&f.args, &f.name),
            Stmt::AsyncFunctionDef(f) => (&f.args, &f.name),
            _ => return,
        };
        let total = count_fn_args(args);
        if total > 6 {
            findings.push(Finding {
                line: 0, col: 0, end_line: 0, end_col: 0,
                code: "RAB018".to_string(),
                message: format!(
                    "Function '{}' has {} parameters (threshold: 6), consider refactoring",
                    name, total
                ),
                fix: None,
            });
        }
    }
}

// ── RAB019: os.system() → subprocess.run() ───────────────────────────────

pub struct OsSystemChecker;

impl Checker for OsSystemChecker {
    fn visit_expr(&mut self, expr: &Expr, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        if let Expr::Call(c) = expr {
            if let Expr::Attribute(a) = &*c.func {
                if a.attr.as_str() == "system" && matches!(&*a.value, Expr::Name(n) if n.id.as_str() == "os") {
                    let range = c.range();
                    let (line, col) = byte_to_line_col(text_size_to_usize(range.start()), line_starts);
                    let (end_line, end_col) = byte_to_line_col(text_size_to_usize(range.end()), line_starts);
                    findings.push(Finding {
                        line, col, end_line, end_col,
                        code: "RAB019".to_string(),
                        message: "Use 'subprocess.run()' instead of 'os.system()' for subprocess control".to_string(),
                        fix: None,
                    });
                }
            }
        }
    }
}

// ── RAB020: time.time() for benchmarking → time.perf_counter() ───────────

pub struct TimeTimeChecker;

impl Checker for TimeTimeChecker {
    fn visit_expr(&mut self, expr: &Expr, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        if let Expr::Call(c) = expr {
            if let Expr::Attribute(a) = &*c.func {
                if a.attr.as_str() == "time" && matches!(&*a.value, Expr::Name(n) if n.id.as_str() == "time") {
                    let range = c.range();
                    let (line, col) = byte_to_line_col(text_size_to_usize(range.start()), line_starts);
                    let (end_line, end_col) = byte_to_line_col(text_size_to_usize(range.end()), line_starts);
                    findings.push(Finding {
                        line, col, end_line, end_col,
                        code: "RAB020".to_string(),
                        message: "Use 'time.perf_counter()' instead of 'time.time()' for benchmarking (higher resolution)".to_string(),
                        fix: None,
                    });
                }
            }
        }
    }
}

// ── RAB022: Public function missing return type hint ─────────────────────

pub struct MissingReturnHintChecker;

impl Checker for MissingReturnHintChecker {
    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let (name, returns, range) = match stmt {
            Stmt::FunctionDef(f) if !f.name.to_string().starts_with('_') => (&f.name, &f.returns, f.range()),
            Stmt::AsyncFunctionDef(f) if !f.name.to_string().starts_with('_') => (&f.name, &f.returns, f.range()),
            _ => return,
        };
        if returns.is_none() {
            let (line, col) = byte_to_line_col(text_size_to_usize(range.start()), line_starts);
            let (end_line, end_col) = byte_to_line_col(text_size_to_usize(range.end()), line_starts);
            findings.push(Finding {
                line, col, end_line, end_col,
                code: "RAB022".to_string(),
                message: format!("Public function '{}' is missing a return type hint", name),
                fix: None,
            });
        }
    }
}

// ── RAB024: Deep comprehension nesting ───────────────────────────────────

pub struct DeepComprehensionChecker;

impl Checker for DeepComprehensionChecker {
    fn visit_expr(&mut self, expr: &Expr, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let generators = match expr {
            Expr::ListComp(lc) => &lc.generators,
            Expr::SetComp(sc) => &sc.generators,
            Expr::DictComp(dc) => &dc.generators,
            Expr::GeneratorExp(ge) => &ge.generators,
            _ => return,
        };
        if generators.len() > 2 {
            let range = expr.range();
            let (line, col) = byte_to_line_col(text_size_to_usize(range.start()), line_starts);
            let (end_line, end_col) = byte_to_line_col(text_size_to_usize(range.end()), line_starts);
            findings.push(Finding {
                line, col, end_line, end_col,
                code: "RAB024".to_string(),
                message: format!(
                    "Deep comprehension with {} nested 'for' clauses, consider refactoring with helper loops",
                    generators.len()
                ),
                fix: None,
            });
        }
    }
}

// ── RAB025: Long if-elif chain (> 3) ─────────────────────────────────────

pub struct LongIfChainChecker;

fn count_elif_chain(orelse: &[Stmt]) -> u32 {
    if orelse.len() == 1 {
        if let Stmt::If(inner) = &orelse[0] {
            return 1 + count_elif_chain(&inner.orelse);
        }
    }
    0
}

impl Checker for LongIfChainChecker {
    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        if let Stmt::If(i) = stmt {
            let chain_len = 1 + count_elif_chain(&i.orelse);
            if chain_len > 3 {
                let range = i.range();
                let (line, col) = byte_to_line_col(text_size_to_usize(range.start()), line_starts);
                let (end_line, end_col) = byte_to_line_col(text_size_to_usize(range.end()), line_starts);
                findings.push(Finding {
                    line, col, end_line, end_col,
                    code: "RAB025".to_string(),
                    message: format!(
                        "Long if-elif chain with {} branches, consider using a dict dispatch",
                        chain_len
                    ),
                    fix: None,
                });
            }
        }
    }
}

// ── RAB102: Cognitive complexity ─────────────────────────────────────────

const COGNITIVE_THRESHOLD: u32 = 15;

pub struct CognitiveComplexityChecker {
    complexity: u32,
    depth: u32,
    scope_stack: Vec<(u32, u32)>,
}

impl CognitiveComplexityChecker {
    pub fn new() -> Self {
        Self { complexity: 1, depth: 0, scope_stack: Vec::new() }
    }
}

impl Checker for CognitiveComplexityChecker {
    fn enter_scope(&mut self) {
        self.scope_stack.push((self.complexity, self.depth));
        self.complexity = 1;
        self.depth = 0;
    }

    fn exit_scope(&mut self, findings: &mut Vec<Finding>) {
        if self.complexity > COGNITIVE_THRESHOLD {
            findings.push(Finding {
                line: 0, col: 0, end_line: 0, end_col: 0,
                code: "RAB102".to_string(),
                message: format!(
                    "Cognitive complexity is {} (threshold: {}), consider simplifying",
                    self.complexity, COGNITIVE_THRESHOLD
                ),
                fix: None,
            });
        }
        if let Some((c, d)) = self.scope_stack.pop() {
            self.complexity = c;
            self.depth = d;
        }
    }

    fn enter_block(&mut self) {
        self.depth += 1;
    }

    fn exit_block(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, _line_starts: &[usize], _findings: &mut Vec<Finding>) {
        match stmt {
            Stmt::If(i) => {
                self.complexity += 1 + self.depth;
                for elif in &i.orelse {
                    if let Stmt::If(_) = elif {
                        self.complexity += 1 + self.depth;
                    }
                }
            }
            Stmt::For(_) | Stmt::AsyncFor(_) | Stmt::While(_) => {
                self.complexity += 1 + self.depth;
            }
            Stmt::Try(t) => {
                self.complexity += 1 + self.depth;
                self.complexity += self.depth * t.handlers.len() as u32;
            }
            Stmt::With(_) | Stmt::AsyncWith(_) => {
                self.complexity += 1 + self.depth;
            }
            Stmt::Match(m) => {
                self.complexity += 1 + self.depth;
                self.complexity += self.depth * m.cases.len() as u32;
            }
            Stmt::Assert(_) => {
                self.complexity += 1;
            }
            _ => {}
        }
    }

    fn visit_expr(&mut self, expr: &Expr, _source: &str, _line_starts: &[usize], _findings: &mut Vec<Finding>) {
        if let Expr::BoolOp(b) = expr {
            if b.values.len() > 1 {
                self.complexity += b.values.len() as u32 - 1;
            }
        }
    }
}

// ── RAB036: x ** 2 → x * x, math.pow(x, 2) → x * x ─────────────────────

pub struct PowerOptChecker;

impl Checker for PowerOptChecker {
    fn visit_expr(&mut self, expr: &Expr, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        match expr {
            Expr::BinOp(b) if matches!(b.op, Operator::Pow) => {
                let exp = match &*b.right {
                    Expr::Constant(c) => match &c.value {
                        Constant::Int(i) => {
                            if *i == rustpython_ast::bigint::BigInt::from(2u64) { Some(2) }
                            else if *i == rustpython_ast::bigint::BigInt::from(3u64) { Some(3) }
                            else { None }
                        }
                        _ => None,
                    },
                    _ => None,
                };
                if let Some(e) = exp {
                    let left_src = expr_to_source(source, &b.left);
                    let replacement = if e == 2 {
                        format!("{} * {}", left_src, left_src)
                    } else {
                        format!("{} * {} * {}", left_src, left_src, left_src)
                    };
                    let range = b.range();
                    let start = text_size_to_usize(range.start());
                    let end = text_size_to_usize(range.end());
                    let (line, col) = byte_to_line_col(start, line_starts);
                    let (end_line, end_col) = byte_to_line_col(end, line_starts);
                    findings.push(Finding {
                        line, col, end_line, end_col,
                        code: "RAB036".to_string(),
                        message: format!("Use '{}' instead of '{} ** {}' for performance", replacement, left_src, e),
                        fix: Some(Fix { start, end, replacement }),
                    });
                }
            }
            Expr::Call(c) => {
                if let Expr::Attribute(a) = &*c.func {
                    if a.attr.as_str() == "pow"
                        && matches!(&*a.value, Expr::Name(n) if n.id.as_str() == "math")
                        && c.args.len() == 2
                    {
                        let exp = match &c.args[1] {
                            Expr::Constant(cc) => match &cc.value {
                                Constant::Int(i) => {
                                    if *i == rustpython_ast::bigint::BigInt::from(2u64) { Some(2) }
                                    else if *i == rustpython_ast::bigint::BigInt::from(3u64) { Some(3) }
                                    else { None }
                                }
                                _ => None,
                            },
                            _ => None,
                        };
                        if let Some(e) = exp {
                            let arg_src = expr_to_source(source, &c.args[0]);
                            let replacement = if e == 2 {
                                format!("{} * {}", arg_src, arg_src)
                            } else {
                                format!("{} * {} * {}", arg_src, arg_src, arg_src)
                            };
                            let range = c.range();
                            let start = text_size_to_usize(range.start());
                            let end = text_size_to_usize(range.end());
                            let (line, col) = byte_to_line_col(start, line_starts);
                            let (end_line, end_col) = byte_to_line_col(end, line_starts);
                            findings.push(Finding {
                                line, col, end_line, end_col,
                                code: "RAB036".to_string(),
                                message: format!("Use '{} * {}' instead of 'math.pow()' for squaring", arg_src, arg_src),
                                fix: Some(Fix { start, end, replacement }),
                            });
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

// ── RAB037: map(lambda...) / filter(lambda...) → comprehension ──────────

pub struct MapLambdaChecker;

impl Checker for MapLambdaChecker {
    fn visit_expr(&mut self, expr: &Expr, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        if let Expr::Call(c) = expr {
            let func_name = get_call_func_name(&c.func);
            let is_map_filter = matches!(func_name.as_deref(), Some("map" | "filter"));
            if !is_map_filter || c.args.len() != 2 {
                return;
            }
            if let Expr::Lambda(l) = &c.args[0] {
                if count_fn_args(&l.args) != 1
                    || !l.args.kwonlyargs.is_empty()
                    || l.args.vararg.is_some()
                    || l.args.kwarg.is_some()
                {
                    return;
                }
                let arg_name = &iter_fn_args(&l.args).next().unwrap().def.arg; 
                let body_src = expr_to_source(source, &l.body);
                let iter_src = expr_to_source(source, &c.args[1]);

                let is_map = func_name.as_deref() == Some("map");
                let replacement = if is_map {
                    format!("({} for {} in {})", body_src, arg_name, iter_src)
                } else {
                    format!("({} for {} in {} if {})", arg_name, arg_name, iter_src, body_src)
                };

                let range = c.range();
                let start = text_size_to_usize(range.start());
                let end = text_size_to_usize(range.end());
                let (line, col) = byte_to_line_col(start, line_starts);
                let (end_line, end_col) = byte_to_line_col(end, line_starts);
                let func = func_name.as_deref().unwrap_or("");
                findings.push(Finding {
                    line, col, end_line, end_col,
                    code: "RAB037".to_string(),
                    message: format!("Use a generator expression instead of '{}()' with a lambda", func),
                    fix: Some(Fix { start, end, replacement }),
                });
            }
        }
    }
}

// ── RAB038: in [const, ...] / in (const, ...) → in {const, ...} ─────────

pub struct ListToSetChecker;

impl Checker for ListToSetChecker {
    fn visit_expr(&mut self, expr: &Expr, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        if let Expr::Compare(c) = expr {
            for (i, op) in c.ops.iter().enumerate() {
                if i >= c.comparators.len() { break; }
                if !matches!(op, CmpOp::In | CmpOp::NotIn) { continue; }
                let container = &c.comparators[i];
                let all_const = match container {
                    Expr::List(l) => l.elts.iter().all(|e| matches!(e, Expr::Constant(_))),
                    Expr::Tuple(t) => t.elts.iter().all(|e| matches!(e, Expr::Constant(_))),
                    _ => false,
                };
                if !all_const { continue; }

                let range = container.range();
                let start = text_size_to_usize(range.start());
                let end = text_size_to_usize(range.end());
                let container_src = &source[start..end];
                let inner = &container_src[1..container_src.len() - 1];
                let replacement = format!("{{{}}}", inner);

                let (line, col) = byte_to_line_col(start, line_starts);
                let (end_line, end_col) = byte_to_line_col(end, line_starts);
                let op_str = if matches!(op, CmpOp::In) { "in" } else { "not in" };
                let left_src = expr_to_source(source, &c.left);
                findings.push(Finding {
                    line, col, end_line, end_col,
                    code: "RAB038".to_string(),
                    message: format!("Use '{} {} {{...}}' instead of '{} {} [...]' for O(1) membership test", left_src, op_str, left_src, op_str),
                    fix: Some(Fix { start, end, replacement }),
                });
            }
        }
    }
}

// ── RAB040: @dataclass without slots=True ────────────────────────────────

pub struct DataclassSlotsChecker;

impl Checker for DataclassSlotsChecker {
    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        if let Stmt::ClassDef(cd) = stmt {
            for decorator in &cd.decorator_list {
                let (msg, fix) = match decorator {
                    Expr::Name(n) if n.id.as_str() == "dataclass" => {
                        let range = n.range();
                        let start = text_size_to_usize(range.start());
                        let end = text_size_to_usize(range.end());
                        (Some("Use '@dataclass(slots=True)' to reduce memory usage and speed up attribute access".to_string()),
                         Some(Fix { start, end, replacement: "dataclass(slots=True)".to_string() }))
                    }
                    Expr::Call(c) if matches!(&*c.func, Expr::Name(n) if n.id.as_str() == "dataclass") => {
                        if c.keywords.iter().any(|kw| {
                            kw.arg.as_deref() == Some("slots")
                                && matches!(&kw.value, Expr::Constant(cc) if matches!(&cc.value, Constant::Bool(true)))
                        }) {
                            (None, None)
                        } else {
                            let range = c.range();
                            let start = text_size_to_usize(range.start());
                            let end = text_size_to_usize(range.end());
                            (Some("Use '@dataclass(slots=True)' to reduce memory usage and speed up attribute access".to_string()),
                             Some(Fix { start, end, replacement: "dataclass(slots=True)".to_string() }))
                        }
                    }
                    _ => (None, None),
                };
                if let Some(msg) = msg {
                    let range = decorator.range();
                    let (line, col) = byte_to_line_col(text_size_to_usize(range.start()), line_starts);
                    let (end_line, end_col) = byte_to_line_col(text_size_to_usize(range.end()), line_starts);
                    findings.push(Finding {
                        line, col, end_line, end_col,
                        code: "RAB040".to_string(),
                        message: msg,
                        fix,
                    });
                }
            }
        }
    }
}

// ── RAB041: re.compile() inside function/loop ───────────────────────────

pub struct ReCompileChecker {
    depth: u32,
}

impl ReCompileChecker {
    pub fn new() -> Self { Self { depth: 0 } }
}

impl Checker for ReCompileChecker {
    fn enter_scope(&mut self) { self.depth += 1; }
    fn exit_scope(&mut self, _findings: &mut Vec<Finding>) { self.depth = self.depth.saturating_sub(1); }

    fn visit_expr(&mut self, expr: &Expr, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        if self.depth <= 1 { return; }
        if let Expr::Call(c) = expr {
            let is_compile = match &*c.func {
                Expr::Attribute(a) if a.attr.as_str() == "compile"
                    && matches!(&*a.value, Expr::Name(n) if n.id.as_str() == "re") => true,
                Expr::Name(n) if n.id.as_str() == "compile" => true,
                _ => false,
            };
            if is_compile {
                let range = c.range();
                let start = text_size_to_usize(range.start());
                let end = text_size_to_usize(range.end());
                let (line, col) = byte_to_line_col(start, line_starts);
                let (end_line, end_col) = byte_to_line_col(end, line_starts);
                findings.push(Finding {
                    line, col, end_line, end_col,
                    code: "RAB041".to_string(),
                    message: "Move 're.compile()' to module level to avoid recompiling the regex on every call".to_string(),
                    fix: None,
                });
            }
        }
    }
}

// ── RAB042: for line in f.readlines() → for line in f ───────────────────

pub struct ReadlinesChecker;

impl Checker for ReadlinesChecker {
    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        if let Stmt::For(f) = stmt {
            if let Expr::Call(c) = &*f.iter {
                if let Expr::Attribute(a) = &*c.func {
                    if a.attr.as_str() == "readlines" && c.args.is_empty() {
                        let range = f.range();
                        let (line, col) = byte_to_line_col(text_size_to_usize(range.start()), line_starts);
                        let (end_line, end_col) = byte_to_line_col(text_size_to_usize(range.end()), line_starts);
                        let dot_start = text_size_to_usize(a.value.end());
                        let call_end = text_size_to_usize(c.range().end());
                        findings.push(Finding {
                            line, col, end_line, end_col,
                            code: "RAB042".to_string(),
                            message: "Use 'for line in f:' instead of 'for line in f.readlines()' to avoid loading the entire file into memory".to_string(),
                            fix: Some(Fix { start: dot_start, end: call_end, replacement: String::new() }),
                        });
                    }
                }
            }
        }
    }
}

// ── RAB044: Optional[X] → X | None, Union[A, B] → A | B ────────────────

pub struct TypeUnionChecker;

impl TypeUnionChecker {
    fn check_expr(&self, expr: &Expr, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        if let Expr::Subscript(s) = expr {
            match &*s.value {
                Expr::Name(n) if n.id.as_str() == "Optional" => {
                    let inner_src = expr_to_source(source, &s.slice);
                    let replacement = format!("{} | None", inner_src);
                    let range = s.range();
                    let start = text_size_to_usize(range.start());
                    let end = text_size_to_usize(range.end());
                    let (line, col) = byte_to_line_col(start, line_starts);
                    let (end_line, end_col) = byte_to_line_col(end, line_starts);
                    findings.push(Finding {
                        line, col, end_line, end_col,
                        code: "RAB044".to_string(),
                        message: format!("Use '{} | None' instead of 'Optional[{}]' (Python 3.10+)", inner_src, inner_src),
                        fix: Some(Fix { start, end, replacement }),
                    });
                }
                Expr::Name(n) if n.id.as_str() == "Union" => {
                    let (replacement, inner_str) = match &*s.slice {
                        Expr::Tuple(t) => {
                            let parts: Vec<String> = t.elts.iter().map(|e| expr_to_source(source, e)).collect();
                            (parts.join(" | "), format!("[{}]", parts.join(", ")))
                        }
                        _ => {
                            let inner = expr_to_source(source, &s.slice);
                            (inner.clone(), format!("[{}]", inner))
                        }
                    };
                    let range = s.range();
                    let start = text_size_to_usize(range.start());
                    let end = text_size_to_usize(range.end());
                    let (line, col) = byte_to_line_col(start, line_starts);
                    let (end_line, end_col) = byte_to_line_col(end, line_starts);
                    findings.push(Finding {
                        line, col, end_line, end_col,
                        code: "RAB044".to_string(),
                        message: format!("Use '{}' instead of 'Union{}' (Python 3.10+)", replacement, inner_str),
                        fix: Some(Fix { start, end, replacement }),
                    });
                }
                _ => {}
            }
        }
    }
}

// ── RAB026: sorted(list(x)) → sorted(x), reversed(tuple(x)) → reversed(x) ─

pub struct SortedListChecker;

impl Checker for SortedListChecker {
    fn visit_expr(&mut self, expr: &Expr, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Expr::Call(c) = expr else { return };
        let Expr::Name(n) = &*c.func else { return };
        let func_name = n.id.as_str();
        if func_name != "sorted" && func_name != "reversed" { return; }
        let Some(first_arg) = c.args.first() else { return };
        let Expr::Call(inner) = first_arg else { return };
        let Expr::Name(inner_n) = &*inner.func else { return };
        let inner_name = inner_n.id.as_str();
        if inner_name != "list" && inner_name != "tuple" { return; }
        if inner.args.len() != 1 { return; }
        let inner_arg_src = expr_to_source(source, &inner.args[0]);
        let replacement = format!("{}({})", func_name, inner_arg_src);
        let range = c.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB026".to_string(),
            message: format!("Remove redundant {}() call: use '{}({})' instead", inner_name, func_name, inner_arg_src),
            fix: Some(Fix { start, end, replacement }),
        });
    }
}

// ── RAB031: if k in d: return d[k] → return d.get(k) ────────────────────

pub struct DictGetChecker;

impl Checker for DictGetChecker {
    fn visit_stmt(&mut self, stmt: &Stmt, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Stmt::If(i) = stmt else { return };
        if !i.orelse.is_empty() || i.body.len() != 1 { return; }
        let Expr::Compare(c) = &*i.test else { return };
        if c.ops.len() != 1 || c.comparators.len() != 1 { return; }
        if !matches!(c.ops[0], CmpOp::In) { return; }
        let dict_name = match &c.comparators[0] {
            Expr::Name(n) => n,
            _ => return,
        };
        let key_src = expr_to_source(source, &c.left);
        let dict_id = dict_name.id.as_str();
        let sub_expr = match &i.body[0] {
            Stmt::Return(r) => r.value.as_deref(),
            Stmt::Assign(a) => Some(&*a.value),
            _ => None,
        };
        let Some(sub_expr) = sub_expr else { return };
        let Expr::Subscript(s) = sub_expr else { return };
        let Expr::Name(n) = &*s.value else { return };
        if n.id.as_str() != dict_id { return; }
        if expr_to_source(source, &s.slice) != key_src { return; }
        let sub_range = sub_expr.range();
        let start = text_size_to_usize(sub_range.start());
        let end = text_size_to_usize(sub_range.end());
        let replacement = format!("{}.get({})", dict_id, key_src);
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB031".to_string(),
            message: format!("Use '{}.get({})' instead of '{}[{}]' to avoid a double dict lookup", dict_id, key_src, dict_id, key_src),
            fix: Some(Fix { start, end, replacement }),
        });
    }
}

// ── RAB035: x[:] → x.copy() for lists ────────────────────────────────────

pub struct SliceCopyChecker;

impl Checker for SliceCopyChecker {
    fn visit_expr(&mut self, expr: &Expr, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Expr::Subscript(s) = expr else { return };
        if !matches!(&*s.value, Expr::Name(_)) { return; }
        let Expr::Slice(sl) = &*s.slice else { return };
        if sl.lower.is_some() || sl.upper.is_some() || sl.step.is_some() { return; }
        let obj_src = expr_to_source(source, &s.value);
        let value_end = text_size_to_usize(s.value.range().end());
        let slice_end = text_size_to_usize(s.range().end());
        let (line, col) = byte_to_line_col(value_end, line_starts);
        let (end_line, end_col) = byte_to_line_col(slice_end, line_starts);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB035".to_string(),
            message: format!("Use '{}.copy()' instead of '{}[:]'", obj_src, obj_src),
            fix: Some(Fix { start: value_end, end: slice_end, replacement: ".copy()".to_string() }),
        });
    }
}

// ── RAB039: List[X] → list[X], Dict[K,V] → dict[K,V] (Python 3.9+) ─────

pub struct NativeGenericChecker;

impl Checker for NativeGenericChecker {
    fn visit_expr(&mut self, expr: &Expr, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Expr::Subscript(s) = expr else { return };
        let Expr::Name(n) = &*s.value else { return };
        let new_name = match n.id.as_str() {
            "List" => "list",
            "Dict" => "dict",
            "Tuple" => "tuple",
            "Set" => "set",
            "FrozenSet" => "frozenset",
            "Type" => "type",
            _ => return,
        };
        let range = n.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB039".to_string(),
            message: format!("Use '{}[...]' instead of '{}[...]' (Python 3.9+)", new_name, n.id),
            fix: Some(Fix { start, end, replacement: new_name.to_string() }),
        });
    }
}

// ── RAB043: Manual list building instead of comprehension ────────────────

pub struct ManualListChecker {
    list_vars: Vec<String>,
}

impl ManualListChecker {
    pub fn new() -> Self { Self { list_vars: Vec::new() } }
}

impl Checker for ManualListChecker {
    fn enter_scope(&mut self) { self.list_vars.clear(); }

    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        if let Stmt::Assign(a) = stmt {
            if a.targets.len() == 1 {
                if let Expr::Name(n) = &a.targets[0] {
                    if matches!(&*a.value, Expr::List(l) if l.elts.is_empty()) {
                        let name = n.id.to_string();
                        if !self.list_vars.contains(&name) {
                            self.list_vars.push(name);
                        }
                    }
                }
            }
        }
        if let Stmt::For(f) = stmt {
            if !f.orelse.is_empty() { return; }
            if f.body.is_empty() { return; }
            let target_var = self.list_vars.iter().find(|v| {
                f.body.iter().all(|s| {
                    let Stmt::Expr(e) = s else { return false };
                    let Expr::Call(c) = &*e.value else { return false };
                    let Expr::Attribute(a) = &*c.func else { return false };
                    let Expr::Name(n) = &*a.value else { return false };
                    n.id.as_str() == *v && (a.attr.as_str() == "append" || a.attr.as_str() == "extend")
                })
            }).cloned();
            if target_var.is_none() { return; }
            let range = f.range();
            let start = text_size_to_usize(range.start());
            let end = text_size_to_usize(range.end());
            let (line, col) = byte_to_line_col(start, line_starts);
            let (end_line, end_col) = byte_to_line_col(end, line_starts);
            findings.push(Finding {
                line, col, end_line, end_col,
                code: "RAB043".to_string(),
                message: "Use a comprehension instead of a manual for loop with .append()".to_string(),
                fix: None,
            });
        }
    }
}

// ── RAB045: open() without `with` context manager ───────────────────────

pub struct OpenContextChecker;

impl Checker for OpenContextChecker {
    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let call_expr = match stmt {
            Stmt::Expr(e) => Some(e.value.as_ref()),
            Stmt::Assign(a) => Some(a.value.as_ref()),
            _ => None,
        };
        let Some(expr) = call_expr else { return };
        let Expr::Call(c) = expr else { return };
        let Expr::Name(n) = c.func.as_ref() else { return };
        if n.id.as_str() != "open" { return; }
        let range = c.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB045".to_string(),
            message: "Use 'with open(...) as f:' to ensure the file is properly closed".to_string(),
            fix: None,
        });
    }
}

// ── RAB046: sorted(x)[0] → min(x) ─────────────────────────────────────

pub struct SortedIndex0Checker;

impl Checker for SortedIndex0Checker {
    fn visit_expr(&mut self, expr: &Expr, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Expr::Subscript(s) = expr else { return };
        let is_zero = matches!(&*s.slice, Expr::Constant(cc) if matches!(&cc.value, Constant::Int(i) if *i == rustpython_ast::bigint::BigInt::from(0u64)));
        if !is_zero { return; }
        let Expr::Call(c) = &*s.value else { return };
        let Expr::Name(n) = &*c.func else { return };
        if n.id.as_str() != "sorted" { return; }
        if c.keywords.iter().any(|k| k.arg.as_deref() == Some("reverse")) { return; }

        let (line, col) = byte_to_line_col(text_size_to_usize(s.range().start()), line_starts);
        let (end_line, end_col) = byte_to_line_col(text_size_to_usize(s.range().end()), line_starts);
        let call_src = &source[text_size_to_usize(c.range().start())..text_size_to_usize(c.range().end())];
        let replacement = call_src.replacen("sorted", "min", 1);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB046".to_string(),
            message: "Use 'min(...)' instead of 'sorted(...)[0]' for O(n) minimum".to_string(),
            fix: Some(Fix {
                start: text_size_to_usize(s.range().start()),
                end: text_size_to_usize(s.range().end()),
                replacement,
            }),
        });
    }
}

// ── RAB047: sorted(x)[-1] → max(x) ────────────────────────────────────

pub struct SortedIndexNeg1Checker;

impl Checker for SortedIndexNeg1Checker {
    fn visit_expr(&mut self, expr: &Expr, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Expr::Subscript(s) = expr else { return };
        let is_neg_one = matches!(&*s.slice, Expr::UnaryOp(u) if matches!(u.op, UnaryOp::USub)
            && matches!(&*u.operand, Expr::Constant(cc) if matches!(&cc.value, Constant::Int(i) if *i == rustpython_ast::bigint::BigInt::from(1u64))));
        if !is_neg_one { return; }
        let Expr::Call(c) = &*s.value else { return };
        let Expr::Name(n) = &*c.func else { return };
        if n.id.as_str() != "sorted" { return; }
        if c.keywords.iter().any(|k| k.arg.as_deref() == Some("reverse")) { return; }

        let (line, col) = byte_to_line_col(text_size_to_usize(s.range().start()), line_starts);
        let (end_line, end_col) = byte_to_line_col(text_size_to_usize(s.range().end()), line_starts);
        let call_src = &source[text_size_to_usize(c.range().start())..text_size_to_usize(c.range().end())];
        let replacement = call_src.replacen("sorted", "max", 1);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB047".to_string(),
            message: "Use 'max(...)' instead of 'sorted(...)[-1]' for O(n) maximum".to_string(),
            fix: Some(Fix {
                start: text_size_to_usize(s.range().start()),
                end: text_size_to_usize(s.range().end()),
                replacement,
            }),
        });
    }
}

// ── RAB048: not x is None → x is not None ─────────────────────────────

pub struct NotIsNoneChecker;

impl Checker for NotIsNoneChecker {
    fn visit_expr(&mut self, expr: &Expr, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Expr::UnaryOp(u) = expr else { return };
        if !matches!(u.op, UnaryOp::Not) { return; }
        let Expr::Compare(c) = &*u.operand else { return };
        if c.ops.len() != 1 || c.comparators.len() != 1 { return; }
        if !matches!(c.ops[0], CmpOp::Is) { return; }
        if !matches!(&c.comparators[0], Expr::Constant(cc) if matches!(&cc.value, Constant::None)) { return; }

        let left_src = &source[text_size_to_usize(c.left.range().start())..text_size_to_usize(c.left.range().end())];
        let range = u.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        let replacement = format!("{} is not None", left_src);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB048".to_string(),
            message: "Use 'x is not None' instead of 'not x is None' (PEP 8)".to_string(),
            fix: Some(Fix { start, end, replacement }),
        });
    }
}

// ── RAB049: x = x + 1 → x += 1 ────────────────────────────────────────

pub struct AugmentedAssignChecker;

impl AugmentedAssignChecker {
    fn op_to_str(op: &Operator) -> Option<&'static str> {
        Some(match op {
            Operator::Add => "+",
            Operator::Sub => "-",
            Operator::Mult => "*",
            Operator::Div => "/",
            Operator::FloorDiv => "//",
            Operator::Mod => "%",
            Operator::Pow => "**",
            Operator::LShift => "<<",
            Operator::RShift => ">>",
            Operator::BitOr => "|",
            Operator::BitXor => "^",
            Operator::BitAnd => "&",
            _ => return None,
        })
    }
}

impl Checker for AugmentedAssignChecker {
    fn visit_stmt(&mut self, stmt: &Stmt, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Stmt::Assign(a) = stmt else { return };
        if a.targets.len() != 1 { return; }
        let Expr::Name(target) = &a.targets[0] else { return };
        let Expr::BinOp(b) = &*a.value else { return };
        let Expr::Name(left) = &*b.left else { return };
        if left.id != target.id { return; }
        let Some(op_str) = Self::op_to_str(&b.op) else { return };

        let right_src = &source[text_size_to_usize(b.right.range().start())..text_size_to_usize(b.right.range().end())];
        let range = a.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        let replacement = format!("{} {}= {}", target.id, op_str, right_src);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB049".to_string(),
            message: format!("Use '{}+=' instead of '{} = {}' for augmented assignment", target.id, target.id, target.id),
            fix: Some(Fix { start, end, replacement }),
        });
    }
}

// ── RAB053: x is True / x is False → x / not x ────────────────────────

pub struct IsTrueChecker;

impl Checker for IsTrueChecker {
    fn visit_expr(&mut self, expr: &Expr, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Expr::Compare(c) = expr else { return };
        if c.ops.len() != 1 || c.comparators.len() != 1 { return; }
        if !matches!(c.ops[0], CmpOp::Is) { return; }
        let is_true = matches!(&c.comparators[0], Expr::Constant(cc) if matches!(&cc.value, Constant::Bool(true)));
        let is_false = matches!(&c.comparators[0], Expr::Constant(cc) if matches!(&cc.value, Constant::Bool(false)));
        if !is_true && !is_false { return; }

        let left_src = &source[text_size_to_usize(c.left.range().start())..text_size_to_usize(c.left.range().end())];
        let range = c.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);

        let (code, message, replacement) = if is_true {
            ("RAB053".to_string(), "Use 'x' instead of 'x is True' for boolean check".to_string(), left_src.to_string())
        } else {
            ("RAB053".to_string(), "Use 'not x' instead of 'x is False' for boolean check".to_string(), format!("not {}", left_src))
        };
        findings.push(Finding {
            line, col, end_line, end_col,
            code, message,
            fix: Some(Fix { start, end, replacement }),
        });
    }
}

// ── RAB050: for i in range(len(seq)) → iterate directly ───────────────

pub struct RangeLenChecker;

impl Checker for RangeLenChecker {
    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Stmt::For(f) = stmt else { return };
        let Expr::Call(range_call) = &*f.iter else { return };
        let Expr::Name(range_name) = &*range_call.func else { return };
        if range_name.id.as_str() != "range" || range_call.args.len() != 1 { return; }
        let Expr::Call(len_call) = &range_call.args[0] else { return };
        let Expr::Name(len_name) = &*len_call.func else { return };
        if len_name.id.as_str() != "len" || len_call.args.len() != 1 { return; }

        let seq_src = expr_to_source(_source, &len_call.args[0]);
        let range = f.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB050".to_string(),
            message: format!("Iterate directly over '{}' instead of using 'range(len(...))'", seq_src),
            fix: None,
        });
    }
}

// ── RAB051: d.setdefault(k, []).append(v) → defaultdict[str, list] ────

pub struct SetdefaultChecker;

impl Checker for SetdefaultChecker {
    fn visit_expr(&mut self, expr: &Expr, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Expr::Call(append_call) = expr else { return };
        let Expr::Attribute(append_attr) = &*append_call.func else { return };
        if append_attr.attr.as_str() != "append" && append_attr.attr.as_str() != "add" { return; }
        let Expr::Call(sd_call) = &*append_attr.value else { return };
        let Expr::Attribute(sd_attr) = &*sd_call.func else { return };
        if sd_attr.attr.as_str() != "setdefault" || sd_call.args.len() != 2 { return; }
        let is_empty = |e: &Expr| -> bool {
            matches!(e, Expr::List(l) if l.elts.is_empty())
                || matches!(e, Expr::Call(c) if matches!(&*c.func, Expr::Name(n) if (n.id.as_str() == "set" || n.id.as_str() == "list") && c.args.is_empty() && c.keywords.is_empty()))
        };
        if !is_empty(&sd_call.args[1]) { return; }

        let range = append_call.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB051".to_string(),
            message: "Use 'collections.defaultdict(list)' instead of 'setdefault(..., []).append()'".to_string(),
            fix: None,
        });
    }
}

// ── RAB052: type(x) == A or type(x) == B → isinstance(x, (A, B)) ─────

pub struct TypeIsChecker;

impl Checker for TypeIsChecker {
    fn visit_expr(&mut self, expr: &Expr, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Expr::BoolOp(b) = expr else { return };
        if !matches!(b.op, BoolOp::Or) || b.values.len() < 2 { return; }

        let mut types: Vec<String> = Vec::new();
        let mut obj_src: Option<String> = None;

        for val in &b.values {
            let Expr::Compare(c) = val else { return };
            if c.ops.len() != 1 || c.comparators.len() != 1 { return; }
            if !matches!(c.ops[0], CmpOp::Eq) { return; }
            let Expr::Call(type_call) = &*c.left else { return };
            let Expr::Name(type_name) = &*type_call.func else { return };
            if type_name.id.as_str() != "type" || type_call.args.len() != 1 { return; }
            let current_obj = expr_to_source(source, &type_call.args[0]);
            match &obj_src {
                Some(s) if *s != current_obj => return,
                None => obj_src = Some(current_obj),
                _ => {}
            }
            let Expr::Name(t) = &c.comparators[0] else { return };
            types.push(t.id.to_string());
        }

        let Some(obj) = obj_src else { return };
        let range = b.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        let types_str = types.join(", ");
        let replacement = format!("isinstance({}, ({}))", obj, types_str);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB052".to_string(),
            message: format!("Use 'isinstance({}, ({}{}))' instead of multiple 'type()' checks", obj, types_str, if types.len() > 1 { "" } else { "," }),
            fix: Some(Fix { start, end, replacement }),
        });
    }
}

// ── RAB054: if not x: x = y → x = x or y ─────────────────────────────

pub struct IfNotAssignChecker;

impl Checker for IfNotAssignChecker {
    fn visit_stmt(&mut self, stmt: &Stmt, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Stmt::If(i) = stmt else { return };
        if !i.orelse.is_empty() || i.body.len() != 1 { return; }
        let Expr::UnaryOp(u) = &*i.test else { return };
        if !matches!(u.op, UnaryOp::Not) { return; }
        let cond = &*u.operand;
        let Stmt::Assign(a) = &i.body[0] else { return };
        if a.targets.len() != 1 { return; }
        let cond_src = expr_to_source(source, cond);
        let target_src = expr_to_source(source, &a.targets[0]);
        if target_src != cond_src { return; }

        let value_src = expr_to_source(source, &a.value);
        let range = i.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        let replacement = format!("{} = {} or {}", cond_src, cond_src, value_src);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB054".to_string(),
            message: format!("Use '{} = {} or {}' instead of 'if not {}: {} = {}'", cond_src, cond_src, value_src, cond_src, cond_src, value_src),
            fix: Some(Fix { start, end, replacement }),
        });
    }
}

// ── RAB055: Unused for-loop variable → _ ──────────────────────────────

fn contains_name_ref(expr: &Expr, name: &str) -> bool {
    match expr {
        Expr::Name(n) => n.id.as_str() == name,
        Expr::Call(c) => {
            contains_name_ref(&c.func, name)
                || c.args.iter().any(|a| contains_name_ref(a, name))
                || c.keywords.iter().any(|k| contains_name_ref(&k.value, name))
        }
        Expr::Attribute(a) => contains_name_ref(&a.value, name),
        Expr::Subscript(s) => contains_name_ref(&s.value, name) || contains_name_ref(&s.slice, name),
        Expr::BinOp(b) => contains_name_ref(&b.left, name) || contains_name_ref(&b.right, name),
        Expr::UnaryOp(u) => contains_name_ref(&u.operand, name),
        Expr::BoolOp(b) => b.values.iter().any(|v| contains_name_ref(v, name)),
        Expr::Compare(c) => contains_name_ref(&c.left, name) || c.comparators.iter().any(|c| contains_name_ref(c, name)),
        Expr::List(l) => l.elts.iter().any(|e| contains_name_ref(e, name)),
        Expr::Tuple(t) => t.elts.iter().any(|e| contains_name_ref(e, name)),
        Expr::Set(s) => s.elts.iter().any(|e| contains_name_ref(e, name)),
        Expr::Dict(d) => d.keys.iter().flatten().chain(d.values.iter()).any(|e| contains_name_ref(e, name)),
        Expr::IfExp(ifexp) => {
            contains_name_ref(&ifexp.test, name)
                || contains_name_ref(&ifexp.body, name)
                || contains_name_ref(&ifexp.orelse, name)
        }
        Expr::Lambda(l) => {
            iter_fn_args(&l.args).any(|arg| arg.def.annotation.as_ref().map_or(false, |a| contains_name_ref(a, name)))
                || contains_name_ref(&l.body, name)
        }
        Expr::ListComp(lc) => {
            lc.generators.iter().any(|g| {
                contains_name_ref(&g.iter, name)
                    || g.ifs.iter().any(|i| contains_name_ref(i, name))
            }) || contains_name_ref(&lc.elt, name)
        }
        Expr::SetComp(sc) => {
            sc.generators.iter().any(|g| {
                contains_name_ref(&g.iter, name)
                    || g.ifs.iter().any(|i| contains_name_ref(i, name))
            }) || contains_name_ref(&sc.elt, name)
        }
        Expr::DictComp(dc) => {
            dc.generators.iter().any(|g| {
                contains_name_ref(&g.iter, name)
                    || g.ifs.iter().any(|i| contains_name_ref(i, name))
            }) || contains_name_ref(&dc.key, name) || contains_name_ref(&dc.value, name)
        }
        Expr::GeneratorExp(ge) => {
            ge.generators.iter().any(|g| {
                contains_name_ref(&g.iter, name)
                    || g.ifs.iter().any(|i| contains_name_ref(i, name))
            }) || contains_name_ref(&ge.elt, name)
        }
        Expr::NamedExpr(n) => contains_name_ref(&n.value, name) || contains_name_ref(&n.target, name),
        Expr::Starred(s) => contains_name_ref(&s.value, name),
        _ => false,
    }
}

pub struct UnusedLoopVarChecker;

impl Checker for UnusedLoopVarChecker {
    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Stmt::For(f) = stmt else { return };
        let Expr::Name(target) = &*f.target else { return };
        if target.id.as_str() == "_" { return; }
        if f.body.is_empty() { return; }
        let used = f.body.iter().any(|s| stmt_contains_name_ref(s, &target.id));
        let used_in_orelse = f.orelse.iter().any(|s| stmt_contains_name_ref(s, &target.id));
        if used || used_in_orelse { return; }

        let range = f.target.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB055".to_string(),
            message: format!("Unused loop variable '{}', use '_' instead", target.id),
            fix: Some(Fix { start, end, replacement: "_".to_string() }),
        });
    }
}

fn stmt_contains_name_ref(stmt: &Stmt, name: &str) -> bool {
    match stmt {
        Stmt::Expr(e) => contains_name_ref(&e.value, name),
        Stmt::Assign(a) => {
            a.targets.iter().any(|t| contains_name_ref(t, name))
                || contains_name_ref(&a.value, name)
        }
        Stmt::AugAssign(a) => contains_name_ref(&a.value, name),
        Stmt::Return(r) => r.value.as_ref().map_or(false, |v| contains_name_ref(v, name)),
        Stmt::If(i) => {
            contains_name_ref(&i.test, name)
                || i.body.iter().any(|s| stmt_contains_name_ref(s, name))
                || i.orelse.iter().any(|s| stmt_contains_name_ref(s, name))
        }
        Stmt::For(f) => {
            contains_name_ref(&f.iter, name)
                || f.body.iter().any(|s| stmt_contains_name_ref(s, name))
                || f.orelse.iter().any(|s| stmt_contains_name_ref(s, name))
        }
        Stmt::While(w) => {
            contains_name_ref(&w.test, name)
                || w.body.iter().any(|s| stmt_contains_name_ref(s, name))
                || w.orelse.iter().any(|s| stmt_contains_name_ref(s, name))
        }
        Stmt::With(w) => {
            w.items.iter().any(|item| {
                contains_name_ref(&item.context_expr, name)
                    || item.optional_vars.as_ref().map_or(false, |v| contains_name_ref(v, name))
            }) || w.body.iter().any(|s| stmt_contains_name_ref(s, name))
        }
        Stmt::Try(t) => {
            t.body.iter().any(|s| stmt_contains_name_ref(s, name))
                || t.handlers.iter().any(|h| match h {
                    ExceptHandler::ExceptHandler(eh) => {
                        eh.body.iter().any(|s| stmt_contains_name_ref(s, name))
                    }
                })
                || t.orelse.iter().any(|s| stmt_contains_name_ref(s, name))
                || t.finalbody.iter().any(|s| stmt_contains_name_ref(s, name))
        }
        Stmt::FunctionDef(f) => {
            f.body.iter().any(|s| stmt_contains_name_ref(s, name))
        }
        Stmt::AsyncFunctionDef(f) => {
            f.body.iter().any(|s| stmt_contains_name_ref(s, name))
        }
        Stmt::AnnAssign(a) => a.value.as_ref().map_or(false, |v| contains_name_ref(v, name)),
        Stmt::Raise(r) => r.exc.as_ref().map_or(false, |e| contains_name_ref(e, name)),
        Stmt::Assert(a) => contains_name_ref(&a.test, name) || a.msg.as_ref().map_or(false, |m| contains_name_ref(m, name)),
        Stmt::Delete(d) => d.targets.iter().any(|t| contains_name_ref(t, name)),
        Stmt::Global(_) | Stmt::Nonlocal(_) | Stmt::Pass(_) | Stmt::Break(_) | Stmt::Continue(_) => false,
        _ => false,
    }
}

// ── RAB056: Nested with statements → single with ─────────────────────

pub struct NestedWithChecker;

impl Checker for NestedWithChecker {
    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Stmt::With(w) = stmt else { return };
        if w.body.len() != 1 { return; }
        let Stmt::With(_inner_w) = &w.body[0] else { return };

        let range = w.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB056".to_string(),
            message: "Nested 'with' statements can be combined into a single 'with A() as a, B() as b:'".to_string(),
            fix: None,
        });
    }
}

// ── RAB057: s.startswith('a') or s.startswith('b') → s.startswith(...) ─

pub struct StartswithOrChecker;

impl Checker for StartswithOrChecker {
    fn visit_expr(&mut self, expr: &Expr, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Expr::BoolOp(b) = expr else { return };
        if !matches!(b.op, BoolOp::Or) || b.values.len() < 2 { return; }

        let mut obj_src: Option<String> = None;
        let mut method: Option<String> = None;
        let mut args: Vec<String> = Vec::new();

        for val in &b.values {
            let Expr::Call(c) = val else { return };
            let Expr::Attribute(a) = &*c.func else { return };
            if a.attr.as_str() != "startswith" && a.attr.as_str() != "endswith" { return; }
            if c.args.len() != 1 { return; }
            let current_obj = expr_to_source(source, &a.value);
            match &obj_src {
                Some(s) if *s != current_obj => return,
                None => obj_src = Some(current_obj),
                _ => {}
            }
            match &method {
                Some(m) if *m != a.attr.as_str() => return,
                None => method = Some(a.attr.to_string()),
                _ => {}
            }
            args.push(expr_to_source(source, &c.args[0]));
        }

        let Some(obj) = obj_src else { return };
        let Some(meth) = method else { return };
        let range = b.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        let replacement = format!("{}.{}(({}))", obj, meth, args.join(", "));
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB057".to_string(),
            message: format!("Use '{}.{}((...))' instead of repeated '{}' calls", obj, meth, meth),
            fix: Some(Fix { start, end, replacement }),
        });
    }
}

// ── RAB058: return True if cond else False → return cond ─────────────

pub struct ReturnTernaryChecker;

impl Checker for ReturnTernaryChecker {
    fn visit_stmt(&mut self, stmt: &Stmt, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Stmt::Return(r) = stmt else { return };
        let Some(ret_val) = &r.value else { return };
        let Expr::IfExp(ifexp) = &**ret_val else { return };
        let body_true = matches!(&*ifexp.body, Expr::Constant(c) if matches!(&c.value, Constant::Bool(true)));
        let body_false = matches!(&*ifexp.body, Expr::Constant(c) if matches!(&c.value, Constant::Bool(false)));
        let orelse_true = matches!(&*ifexp.orelse, Expr::Constant(c) if matches!(&c.value, Constant::Bool(true)));
        let orelse_false = matches!(&*ifexp.orelse, Expr::Constant(c) if matches!(&c.value, Constant::Bool(false)));
        let ret_true = body_true && orelse_false;
        let ret_false = body_false && orelse_true;
        if !ret_true && !ret_false { return; }

        let cond_src = expr_to_source(source, &ifexp.test);
        let range = r.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        let (msg, replacement) = if ret_true {
            ("Use 'return <cond>' instead of 'return True if cond else False'", format!("return {}", cond_src))
        } else {
            ("Use 'return not <cond>' instead of 'return False if cond else True'", format!("return not {}", cond_src))
        };
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB058".to_string(),
            message: msg.to_string(),
            fix: Some(Fix { start, end, replacement }),
        });
    }
}

fn has_break_stmt(stmts: &[Stmt]) -> bool {
    stmts.iter().any(|s| match s {
        Stmt::Break(_) => true,
        Stmt::If(i) => has_break_stmt(&i.body) || has_break_stmt(&i.orelse),
        Stmt::Try(t) => {
            has_break_stmt(&t.body)
                || t.handlers.iter().any(|h| match h {
                    ExceptHandler::ExceptHandler(eh) => has_break_stmt(&eh.body),
                })
                || has_break_stmt(&t.orelse)
                || has_break_stmt(&t.finalbody)
        }
        Stmt::For(f) => has_break_stmt(&f.body) || has_break_stmt(&f.orelse),
        Stmt::While(ww) => has_break_stmt(&ww.body) || has_break_stmt(&ww.orelse),
        Stmt::With(ww) => has_break_stmt(&ww.body),
        _ => false,
    })
}

// ── RAB059: while True without break → possible infinite loop ────────

pub struct InfiniteWhileChecker;

impl Checker for InfiniteWhileChecker {
    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Stmt::While(w) = stmt else { return };
        let is_true = matches!(&*w.test, Expr::Constant(c) if matches!(&c.value, Constant::Bool(true)));
        if !is_true { return; }
        let has_break = has_break_stmt(&w.body);
        if has_break { return; }

        let range = w.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB059".to_string(),
            message: "'while True:' without 'break' results in an infinite loop".to_string(),
            fix: None,
        });
    }
}

// ── RAB060: sorted(x).sort() → x.sort() ──────────────────────────────

pub struct SortedSortChecker;

impl Checker for SortedSortChecker {
    fn visit_expr(&mut self, expr: &Expr, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Expr::Call(outer) = expr else { return };
        let Expr::Attribute(attr) = &*outer.func else { return };
        if attr.attr.as_str() != "sort" { return; }
        let Expr::Call(inner) = &*attr.value else { return };
        let Expr::Name(n) = &*inner.func else { return };
        if n.id.as_str() != "sorted" || inner.args.is_empty() { return; }

        let first_arg_src = expr_to_source(source, &inner.args[0]);
        let kw_srcs: Vec<String> = inner.keywords.iter().map(|kw| {
            let arg_name = kw.arg.as_deref().unwrap_or("");
            let val_src = expr_to_source(source, &kw.value);
            format!("{}={}", arg_name, val_src)
        }).collect();

        let range = outer.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        let replacement = if kw_srcs.is_empty() {
            format!("{}.sort()", first_arg_src)
        } else {
            format!("{}.sort({})", first_arg_src, kw_srcs.join(", "))
        };
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB060".to_string(),
            message: format!("Use '{}' instead of 'sorted(...).sort()'", replacement),
            fix: Some(Fix { start, end, replacement }),
        });
    }
}

// ── RAB061: from module import * ──────────────────────────────────────

pub struct WildcardImportChecker;

impl Checker for WildcardImportChecker {
    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Stmt::ImportFrom(i) = stmt else { return };
        let has_wildcard = i.names.iter().any(|alias| alias.name.as_str() == "*");
        if !has_wildcard { return; }
        let range = i.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB061".to_string(),
            message: "Wildcard import 'from module import *' pollutes the namespace, import specific names instead".to_string(),
            fix: None,
        });
    }
}

// ── RAB062: Redundant pass after docstring ────────────────────────────

pub struct RedundantPassChecker;

impl Checker for RedundantPassChecker {
    fn visit_stmt(&mut self, stmt: &Stmt, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let body: &[Stmt] = match stmt {
            Stmt::FunctionDef(f) => &f.body,
            Stmt::AsyncFunctionDef(f) => &f.body,
            Stmt::ClassDef(c) => &c.body,
            _ => return,
        };
        if body.len() < 2 { return; }
        let has_docstring = matches!(&body[0], Stmt::Expr(e) if matches!(&*e.value, Expr::Constant(c) if matches!(&c.value, Constant::Str(_))));
        if !has_docstring { return; }
        let pass_stmt = match &body[1] {
            Stmt::Pass(p) => p,
            _ => return,
        };
        let range = pass_stmt.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        let line_idx = line_starts.binary_search(&start).unwrap_or_else(|i| i.saturating_sub(1));
        let line_start = line_starts[line_idx];
        let line_end = line_starts.get(line_idx + 1).copied().unwrap_or(source.len());
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB062".to_string(),
            message: "Redundant 'pass' after docstring".to_string(),
            fix: Some(Fix { start: line_start, end: line_end, replacement: String::new() }),
        });
    }
}

// ── RAB063: x is 5 / x is "str" → x == 5 / x == "str" ───────────────

pub struct IsLiteralChecker;

impl Checker for IsLiteralChecker {
    fn visit_expr(&mut self, expr: &Expr, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Expr::Compare(c) = expr else { return };
        if c.ops.len() != 1 || c.comparators.len() != 1 { return; }
        let is_is = matches!(c.ops[0], CmpOp::Is);
        let is_is_not = matches!(c.ops[0], CmpOp::IsNot);
        if !is_is && !is_is_not { return; }
        let is_literal = match &c.comparators[0] {
            Expr::Constant(cc) => matches!(&cc.value, Constant::Int(_) | Constant::Float(_) | Constant::Str(_) | Constant::Bytes(_)),
            _ => false,
        };
        if !is_literal { return; }

        let left_src = expr_to_source(source, &c.left);
        let right_src = expr_to_source(source, &c.comparators[0]);
        let range = c.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        let op = if is_is { "==" } else { "!=" };
        let replacement = format!("{} {} {}", left_src, op, right_src);
        let msg = if is_is {
            format!("Use '{} == {}' instead of '{} is {}' (identity check with literal)", left_src, right_src, left_src, right_src)
        } else {
            format!("Use '{} != {}' instead of '{} is not {}' (identity check with literal)", left_src, right_src, left_src, right_src)
        };
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB063".to_string(),
            message: msg,
            fix: Some(Fix { start, end, replacement }),
        });
    }
}

// ── RAB064: __init__ returning non-None value ─────────────────────────

pub struct InitReturnChecker;

impl Checker for InitReturnChecker {
    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Stmt::FunctionDef(f) = stmt else { return };
        if f.name.as_str() != "__init__" { return; }
        for s in &f.body {
            let Stmt::Return(r) = s else { continue };
            let Some(val) = &r.value else { continue };
            let is_none = matches!(&**val, Expr::Constant(c) if matches!(&c.value, Constant::None));
            if is_none { continue; }
            let range = r.range();
            let start = text_size_to_usize(range.start());
            let end = text_size_to_usize(range.end());
            let (line, col) = byte_to_line_col(start, line_starts);
            let (end_line, end_col) = byte_to_line_col(end, line_starts);
            findings.push(Finding {
                line, col, end_line, end_col,
                code: "RAB064".to_string(),
                message: "'__init__' should not return a value, use bare 'return' instead".to_string(),
                fix: Some(Fix { start, end, replacement: "return".to_string() }),
            });
        }
    }
}

// ── RAB065: if True: / if False: dead code ────────────────────────────

pub struct DeadCodeChecker;

impl Checker for DeadCodeChecker {
    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Stmt::If(i) = stmt else { return };
        let is_true = matches!(&*i.test, Expr::Constant(c) if matches!(&c.value, Constant::Bool(true)));
        let is_false = matches!(&*i.test, Expr::Constant(c) if matches!(&c.value, Constant::Bool(false)));
        if !is_true && !is_false { return; }
        let range = i.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        let tag = if is_true { "True" } else { "False" };
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB065".to_string(),
            message: format!("'if {}:' is always {}, consider removing the condition", tag, if is_true { "true" } else { "false" }),
            fix: None,
        });
    }
}

// ── RAB066: Function definition inside a loop ─────────────────────────

pub struct DefInLoopChecker;

impl DefInLoopChecker {
    fn report_def(name: &str, stmt: &Stmt, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let range = stmt.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB066".to_string(),
            message: format!("Function '{}' defined inside a loop, consider moving it outside", name),
            fix: None,
        });
    }

    fn check_defs(stmts: &[Stmt], source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        for stmt in stmts {
            match stmt {
                Stmt::FunctionDef(f) => {
                    Self::report_def(f.name.as_str(), stmt, source, line_starts, findings);
                }
                Stmt::AsyncFunctionDef(f) => {
                    Self::report_def(f.name.as_str(), stmt, source, line_starts, findings);
                }
                Stmt::ClassDef(_) => {} // stop recursion
                Stmt::For(f) => {
                    Self::check_defs(&f.body, source, line_starts, findings);
                    Self::check_defs(&f.orelse, source, line_starts, findings);
                }
                Stmt::AsyncFor(f) => {
                    Self::check_defs(&f.body, source, line_starts, findings);
                    Self::check_defs(&f.orelse, source, line_starts, findings);
                }
                Stmt::While(w) => {
                    Self::check_defs(&w.body, source, line_starts, findings);
                    Self::check_defs(&w.orelse, source, line_starts, findings);
                }
                Stmt::If(i) => {
                    Self::check_defs(&i.body, source, line_starts, findings);
                    Self::check_defs(&i.orelse, source, line_starts, findings);
                }
                Stmt::With(w) => { Self::check_defs(&w.body, source, line_starts, findings); }
                Stmt::Try(t) => {
                    Self::check_defs(&t.body, source, line_starts, findings);
                    for handler in &t.handlers {
                        let ExceptHandler::ExceptHandler(eh) = handler;
                        Self::check_defs(&eh.body, source, line_starts, findings);
                    }
                    Self::check_defs(&t.orelse, source, line_starts, findings);
                    Self::check_defs(&t.finalbody, source, line_starts, findings);
                }
                _ => {}
            }
        }
    }
}

impl Checker for DefInLoopChecker {
    fn visit_stmt(&mut self, stmt: &Stmt, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        match stmt {
            Stmt::For(f) => {
                Self::check_defs(&f.body, source, line_starts, findings);
                Self::check_defs(&f.orelse, source, line_starts, findings);
            }
            Stmt::AsyncFor(f) => {
                Self::check_defs(&f.body, source, line_starts, findings);
                Self::check_defs(&f.orelse, source, line_starts, findings);
            }
            Stmt::While(w) => {
                Self::check_defs(&w.body, source, line_starts, findings);
                Self::check_defs(&w.orelse, source, line_starts, findings);
            }
            _ => {}
        }
    }
}

// ── RAB067: Shadowing built-in names ──────────────────────────────────

const BUILTINS: &[&str] = &[
    "abs", "all", "any", "ascii", "bin", "bool", "bytearray", "bytes", "callable",
    "chr", "classmethod", "compile", "complex", "delattr", "dict", "dir", "divmod",
    "enumerate", "eval", "exec", "filter", "float", "format", "frozenset", "getattr",
    "globals", "hasattr", "hash", "hex", "id", "input", "int", "isinstance",
    "issubclass", "iter", "len", "list", "locals", "map", "max", "memoryview", "min",
    "next", "object", "oct", "open", "ord", "pow", "print", "property", "range",
    "repr", "reversed", "round", "set", "setattr", "slice", "sorted", "staticmethod",
    "str", "sum", "super", "tuple", "type", "vars", "zip", "__import__",
];

pub struct BuiltinShadowChecker;

impl BuiltinShadowChecker {
    fn check_name(name: &str) -> Option<&'static str> {
        let stripped = name.strip_suffix("_").unwrap_or(name);
        BUILTINS.iter().find(|b| **b == stripped).copied()
    }
}

impl Checker for BuiltinShadowChecker {
    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let targets: Option<&[Expr]> = match stmt {
            Stmt::Assign(a) => Some(&a.targets),
            Stmt::FunctionDef(f) => {
                if let Some(builtin) = Self::check_name(f.name.as_str()) {
                    let range = stmt.range();
                    let start = text_size_to_usize(range.start());
                    let end = text_size_to_usize(range.end());
                    let (line, col) = byte_to_line_col(start, line_starts);
                    let (end_line, end_col) = byte_to_line_col(end, line_starts);
                    findings.push(Finding {
                        line, col, end_line, end_col,
                        code: "RAB067".to_string(),
                        message: format!("Function '{}' shadows built-in '{}', rename to '{}_'", f.name, builtin, builtin),
                        fix: Some(Fix { start, end, replacement: format!("def {}_", builtin) }),
                    });
                }
                return;
            }
            _ => return,
        };
        let Some(targets) = targets else { return };
        for target in targets {
            let Expr::Name(n) = target else { continue };
            let Some(builtin) = Self::check_name(n.id.as_str()) else { continue };
            let range = n.range();
            let start = text_size_to_usize(range.start());
            let end = text_size_to_usize(range.end());
            let (line, col) = byte_to_line_col(start, line_starts);
            let (end_line, end_col) = byte_to_line_col(end, line_starts);
            findings.push(Finding {
                line, col, end_line, end_col,
                code: "RAB067".to_string(),
                message: format!("Variable '{}' shadows built-in '{}', rename to '{}_'", n.id, builtin, builtin),
                fix: Some(Fix { start, end, replacement: format!("{}_", builtin) }),
            });
        }
    }
}

// ── RAB068: raise Exception() without from inside except ──────────────

pub struct RaiseWithoutFromChecker {
    in_except: bool,
}

impl RaiseWithoutFromChecker {
    pub fn new() -> Self {
        Self { in_except: false }
    }
}

impl Checker for RaiseWithoutFromChecker {
    fn enter_except(&mut self) { self.in_except = true; }
    fn exit_except(&mut self) { self.in_except = false; }

    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        if !self.in_except { return; }
        let Stmt::Raise(r) = stmt else { return };
        if r.cause.is_some() { return; }
        if r.exc.is_none() { return; }
        let range = r.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB068".to_string(),
            message: "Raise inside 'except' without 'from' may lose original traceback".to_string(),
            fix: None,
        });
    }
}

impl Checker for TypeUnionChecker {
    fn visit_stmt(&mut self, stmt: &Stmt, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let (returns, args) = match stmt {
            Stmt::FunctionDef(f) => (&f.returns, &f.args),
            Stmt::AsyncFunctionDef(f) => (&f.returns, &f.args),
            _ => {
                if let Stmt::AnnAssign(a) = stmt {
                    self.check_expr(&a.annotation, source, line_starts, findings);
                }
                return;
            }
        };
        if let Some(ret) = returns {
            self.check_expr(ret, source, line_starts, findings);
        }
        for arg in iter_fn_args(args) {
            if let Some(annotation) = &arg.def.annotation {
                self.check_expr(annotation, source, line_starts, findings);
            }
        }
    }
}

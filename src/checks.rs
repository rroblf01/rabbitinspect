use crate::analyze::{byte_to_line_col, text_size_to_usize, Checker, Finding, Fix};
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
                for arg in &f.args.posonlyargs {
                    self.add_arg(&arg.def.arg);
                }
                for arg in &f.args.args {
                    self.add_arg(&arg.def.arg);
                }
                for arg in &f.args.kwonlyargs {
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
                for arg in &f.args.posonlyargs {
                    self.add_arg(&arg.def.arg);
                }
                for arg in &f.args.args {
                    self.add_arg(&arg.def.arg);
                }
                for arg in &f.args.kwonlyargs {
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
        for arg in args.posonlyargs.iter()
            .chain(args.args.iter())
            .chain(args.kwonlyargs.iter())
        {
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
        let total = args.posonlyargs.len() + args.args.len() + args.kwonlyargs.len();
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

use crate::analyze::{byte_to_line_col, count_fn_args, iter_fn_args, text_size_to_usize, Checker, Finding, Fix};
use rustc_hash::FxHashSet;
use rustpython_ast::*;

// ── RAB001: Unused variables ──────────────────────────────────────────────

fn collect_all_names_from_expr(expr: &Expr, names: &mut Vec<String>) {
    match expr {
        Expr::List(l) => {
            for elt in &l.elts {
                if let Expr::Constant(c) = elt {
                    if let Constant::Str(s) = &c.value { names.push(s.clone()); }
                } else if let Expr::Name(n) = elt {
                    names.push(n.id.to_string());
                }
            }
        }
        Expr::Constant(c) => {
            if let Constant::Str(s) = &c.value { names.push(s.clone()); }
        }
        Expr::Name(n) => { names.push(n.id.to_string()); }
        _ => {}
    }
}

pub struct UnusedVarsChecker {
    assigned: Vec<(String, usize, usize)>,
    used: Vec<String>,
    all_names: Vec<String>,
    scope_stack: Vec<(Vec<(String, usize, usize)>, Vec<String>)>,
    dataclass_depth: usize,
    next_scope_is_class: bool,
    scope_types: Vec<bool>,
}

impl UnusedVarsChecker {
    pub fn new() -> Self {
        Self {
            assigned: Vec::new(),
            used: Vec::new(),
            all_names: Vec::new(),
            scope_stack: Vec::new(),
            dataclass_depth: 0,
            next_scope_is_class: false,
            scope_types: Vec::new(),
        }
    }
    fn in_class_body(&self) -> bool {
        self.scope_types.last().copied().unwrap_or(false)
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

    fn add_arg(&mut self, name: &str, line: usize, col: usize) {
        if name == "self" || name == "cls" { return; }
        self.assigned.push((name.to_string(), line, col));
    }

    fn is_dataclass(decorator_list: &[Expr]) -> bool {
        decorator_list.iter().any(|d| {
            if let Expr::Name(n) = d { n.id.as_str() == "dataclass" }
            else if let Expr::Call(c) = d {
                if let Expr::Name(n) = &*c.func { n.id.as_str() == "dataclass" }
                else { false }
            }
            else { false }
        })
    }
}

impl Checker for UnusedVarsChecker {
    fn enter_scope(&mut self) {
        self.scope_types.push(self.next_scope_is_class);
        self.next_scope_is_class = false;
        self.scope_stack.push((
            std::mem::take(&mut self.assigned),
            std::mem::take(&mut self.used),
        ));
    }

    fn exit_scope(&mut self, findings: &mut Vec<Finding>) {
        if self.dataclass_depth > 0 {
            self.dataclass_depth -= 1;
        }
        self.scope_types.pop();
        self.used.extend(self.all_names.drain(..));
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
        if let Some((parent_assigned, mut parent_used)) = self.scope_stack.pop() {
            parent_used.extend(std::mem::take(&mut self.used));
            self.assigned = parent_assigned;
            self.used = parent_used;
        }
    }

    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, line_starts: &[usize], _findings: &mut Vec<Finding>) {
        match stmt {
            Stmt::FunctionDef(f) => {
                for arg in iter_fn_args(&f.args) {
                    let pos = text_size_to_usize(arg.def.range().start());
                    let (line, col) = byte_to_line_col(pos, line_starts);
                    self.add_arg(&arg.def.arg, line, col);
                }
                if let Some(vararg) = &f.args.vararg {
                    let pos = text_size_to_usize(vararg.range().start());
                    let (line, col) = byte_to_line_col(pos, line_starts);
                    self.add_arg(&vararg.arg, line, col);
                }
                if let Some(kwarg) = &f.args.kwarg {
                    let pos = text_size_to_usize(kwarg.range().start());
                    let (line, col) = byte_to_line_col(pos, line_starts);
                    self.add_arg(&kwarg.arg, line, col);
                }
            }
            Stmt::AsyncFunctionDef(f) => {
                for arg in iter_fn_args(&f.args) {
                    let pos = text_size_to_usize(arg.def.range().start());
                    let (line, col) = byte_to_line_col(pos, line_starts);
                    self.add_arg(&arg.def.arg, line, col);
                }
                if let Some(vararg) = &f.args.vararg {
                    let pos = text_size_to_usize(vararg.range().start());
                    let (line, col) = byte_to_line_col(pos, line_starts);
                    self.add_arg(&vararg.arg, line, col);
                }
                if let Some(kwarg) = &f.args.kwarg {
                    let pos = text_size_to_usize(kwarg.range().start());
                    let (line, col) = byte_to_line_col(pos, line_starts);
                    self.add_arg(&kwarg.arg, line, col);
                }
            }
            Stmt::ClassDef(cd) => {
                self.next_scope_is_class = true;
                if Self::is_dataclass(&cd.decorator_list) {
                    self.dataclass_depth += 1;
                }
            }
            Stmt::Assign(a) => {
                if !self.in_class_body() {
                    if a.targets.len() == 1 {
                        if let Expr::Name(n) = &a.targets[0] {
                            if n.id.as_str() == "__all__" {
                                collect_all_names_from_expr(&a.value, &mut self.all_names);
                            }
                        }
                    }
                    for target in &a.targets {
                        self.collect_names_from_target(target, line_starts);
                    }
                }
            }
            Stmt::AnnAssign(a) => {
                if self.dataclass_depth == 0 && !self.in_class_body() {
                    self.collect_names_from_target(&a.target, line_starts);
                }
            }
            Stmt::AugAssign(a) => {
                if !self.in_class_body() {
                    if let Expr::Name(n) = &*a.target {
                        if n.id.as_str() == "__all__" {
                            collect_all_names_from_expr(&a.value, &mut self.all_names);
                        }
                    }
                    self.collect_names_from_target(&a.target, line_starts);
                    if let Expr::Name(n) = &*a.target {
                        self.used.push(n.id.to_string());
                    }
                }
            }
            Stmt::Expr(e) => {
                if let Expr::Call(c) = &*e.value {
                    if let Expr::Attribute(a) = &*c.func {
                        if let Expr::Name(n) = &*a.value {
                            if n.id.as_str() == "__all__" && (a.attr.as_str() == "append" || a.attr.as_str() == "extend") {
                                if let Some(arg) = c.args.first() {
                                    collect_all_names_from_expr(arg, &mut self.all_names);
                                }
                            }
                        }
                    }
                }
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
                    if alias.name.as_str() == "*" { continue; }
                    let name = alias
                        .asname
                        .clone()
                        .unwrap_or_else(|| alias.name.clone());
                    let short = name.split('.').next().unwrap_or(&name).to_string();
                    let pos = text_size_to_usize(alias.range().start());
                    let (line, col) = byte_to_line_col(pos, line_starts);
                    self.assigned.push((short, line, col));
                }
            }
            Stmt::ImportFrom(i) => {
                if i.module.as_deref() == Some("__future__") { return; }
                for alias in &i.names {
                    if alias.name.as_str() == "*" { continue; }
                    let name = alias
                        .asname
                        .clone()
                        .unwrap_or_else(|| alias.name.clone());
                    let pos = text_size_to_usize(alias.range().start());
                    let (line, col) = byte_to_line_col(pos, line_starts);
                    self.assigned.push((name.to_string(), line, col));
                }
            }
            Stmt::Try(t) => {
                for handler in &t.handlers {
                    let ExceptHandler::ExceptHandler(h) = handler;
                    if let Some(name) = &h.name {
                        let pos = text_size_to_usize(h.range().start());
                        let (line, col) = byte_to_line_col(pos, line_starts);
                        self.assigned.push((name.to_string(), line, col));
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
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

fn needs_parens(s: &str) -> bool {
    s.contains(' ') || s.contains("and") || s.contains("or") || s.contains("if") || s.contains("for")
}

fn not_expr(s: &str) -> String {
    if needs_parens(s) {
        format!("not ({})", s)
    } else {
        format!("not {}", s)
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

impl Checker for LenZeroChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
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
                    not_expr(&arg_src),
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
                    not_expr(&arg_src),
                ),
                CmpOp::LtE => (
                    "Use 'not x' instead of 'len(x) <= 0' for emptiness check",
                    not_expr(&arg_src),
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
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
                (true, false) => ("Use 'not x' instead of 'x == False'", not_expr(&left_src)),
                (false, true) => ("Use 'not x' instead of 'x != True'", not_expr(&left_src)),
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
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
    func_pos: Vec<(usize, usize)>,
}

impl ComplexityChecker {
    pub fn new() -> Self {
        Self {
            complexity: 1,
            scope_stack: Vec::new(),
            func_pos: Vec::new(),
        }
    }
}

impl Checker for ComplexityChecker {
    fn enter_scope(&mut self) {
        self.scope_stack.push(std::mem::replace(&mut self.complexity, 1));
    }

    fn exit_scope(&mut self, findings: &mut Vec<Finding>) {
        if self.complexity > COMPLEXITY_THRESHOLD {
            let (line, col) = self.func_pos.pop().unwrap_or((0, 0));
            findings.push(Finding {
                line, col, end_line: line, end_col: col,
                code: "RAB101".to_string(),
                message: format!(
                    "Cyclomatic complexity is {} (threshold: {}), consider simplifying",
                    self.complexity, COMPLEXITY_THRESHOLD
                ),
                fix: None,
            });
        } else {
            self.func_pos.pop();
        }
        if let Some(parent) = self.scope_stack.pop() {
            self.complexity = parent;
        }
    }

    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, line_starts: &[usize], _findings: &mut Vec<Finding>) {
        match stmt {
            Stmt::FunctionDef(f) => {
                let start = text_size_to_usize(f.range().start());
                let (line, col) = byte_to_line_col(start, line_starts);
                self.func_pos.push((line, col));
            }
            Stmt::AsyncFunctionDef(f) => {
                let start = text_size_to_usize(f.range().start());
                let (line, col) = byte_to_line_col(start, line_starts);
                self.func_pos.push((line, col));
            }
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
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
    func_pos: Vec<(usize, usize)>,
}

impl FunctionLengthChecker {
    pub fn new() -> Self {
        Self { count: 0, scope_stack: Vec::new(), func_pos: Vec::new() }
    }
}

impl Checker for FunctionLengthChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
    fn enter_scope(&mut self) {
        self.scope_stack.push(std::mem::replace(&mut self.count, 0));
    }

    fn exit_scope(&mut self, findings: &mut Vec<Finding>) {
        if self.count > MAX_FUNCTION_STMTS && self.scope_stack.len() > 1 {
            let (line, col) = self.func_pos.pop().unwrap_or((0, 0));
            findings.push(Finding {
                line, col, end_line: line, end_col: col,
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

    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, line_starts: &[usize], _findings: &mut Vec<Finding>) {
        match stmt {
            Stmt::FunctionDef(f) => {
                let start = text_size_to_usize(f.range().start());
                let (line, col) = byte_to_line_col(start, line_starts);
                self.func_pos.push((line, col));
            }
            Stmt::AsyncFunctionDef(f) => {
                let start = text_size_to_usize(f.range().start());
                let (line, col) = byte_to_line_col(start, line_starts);
                self.func_pos.push((line, col));
            }
            Stmt::ClassDef(_) => {}
            _ => self.count += 1,
        }
    }
}

// ── RAB018: Too many parameters ──────────────────────────────────────────

pub struct TooManyParamsChecker;

impl Checker for TooManyParamsChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, _line_starts: &[usize], findings: &mut Vec<Finding>) {
        let (args, name) = match stmt {
            Stmt::FunctionDef(f) => (&f.args, &f.name),
            Stmt::AsyncFunctionDef(f) => (&f.args, &f.name),
            _ => return,
        };
        let total = count_fn_args(args);
        if total > 6 {
            let range = stmt.range();
            let start = text_size_to_usize(range.start());
            let (line, col) = byte_to_line_col(start, _line_starts);
            findings.push(Finding {
                line, col, end_line: line, end_col: col,
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
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
    func_pos: Vec<(usize, usize)>,
}

impl CognitiveComplexityChecker {
    pub fn new() -> Self {
        Self {
            complexity: 1,
            depth: 0,
            scope_stack: Vec::new(),
            func_pos: Vec::new(),
        }
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
            let (line, col) = self.func_pos.pop().unwrap_or((0, 0));
            findings.push(Finding {
                line, col, end_line: line, end_col: col,
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

    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, line_starts: &[usize], _findings: &mut Vec<Finding>) {
        match stmt {
            Stmt::FunctionDef(f) => {
                let start = text_size_to_usize(f.range().start());
                let (line, col) = byte_to_line_col(start, line_starts);
                self.func_pos.push((line, col));
            }
            Stmt::AsyncFunctionDef(f) => {
                let start = text_size_to_usize(f.range().start());
                let (line, col) = byte_to_line_col(start, line_starts);
                self.func_pos.push((line, col));
            }
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
    fn visit_stmt(&mut self, stmt: &Stmt, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
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
                            let call_src = source;
                            let range = c.range();
                            let start = text_size_to_usize(range.start());
                            let end = text_size_to_usize(range.end());
                            let src_snippet = &call_src[start..end];
                            let replacement = if let Some(pos) = src_snippet.find(')') {
                                let before_paren = &src_snippet[..pos];
                                if let Some(paren) = before_paren.rfind('(') {
                                    let inner = &src_snippet[paren + 1..pos];
                                    if inner.trim().is_empty() {
                                        format!("dataclass(slots=True)")
                                    } else {
                                        format!("dataclass({}, slots=True)", inner)
                                    }
                                } else {
                                    "dataclass(slots=True)".to_string()
                                }
                            } else {
                                "dataclass(slots=True)".to_string()
                            };
                            (Some("Use '@dataclass(slots=True)' to reduce memory usage and speed up attribute access".to_string()),
                             Some(Fix { start, end, replacement }))
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
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
            ("RAB053".to_string(), "Use 'not x' instead of 'x is False' for boolean check".to_string(), not_expr(left_src))
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
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
        Expr::Await(a) => contains_name_ref(&a.value, name),
        Expr::Yield(y) => y.value.as_ref().map_or(false, |v| contains_name_ref(v, name)),
        Expr::YieldFrom(yf) => contains_name_ref(&yf.value, name),
        Expr::Slice(s) => {
            s.lower.as_deref().map_or(false, |v| contains_name_ref(v, name))
                || s.upper.as_deref().map_or(false, |v| contains_name_ref(v, name))
                || s.step.as_deref().map_or(false, |v| contains_name_ref(v, name))
        }
        _ => false,
    }
}

pub struct UnusedLoopVarChecker;

impl Checker for UnusedLoopVarChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let (target_expr, body, orelse) = match stmt {
            Stmt::For(f) => (&f.target, &f.body, &f.orelse),
            Stmt::AsyncFor(f) => (&f.target, &f.body, &f.orelse),
            _ => return,
        };
        let Expr::Name(target) = &**target_expr else { return };
        if target.id.as_str() == "_" { return; }
        if body.is_empty() { return; }
        let used = body.iter().any(|s| stmt_contains_name_ref(s, &target.id));
        let used_in_orelse = orelse.iter().any(|s| stmt_contains_name_ref(s, &target.id));
        if used || used_in_orelse { return; }

        let range = target_expr.range();
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
        Stmt::Match(m) => {
            contains_name_ref(&m.subject, name)
                || m.cases.iter().any(|case| {
                    case.body.iter().any(|s| stmt_contains_name_ref(s, name))
                })
        }
        Stmt::ClassDef(c) => {
            c.bases.iter().any(|b| contains_name_ref(b, name))
                || c.keywords.iter().any(|k| contains_name_ref(&k.value, name))
                || c.body.iter().any(|s| stmt_contains_name_ref(s, name))
        }
        Stmt::AsyncFor(f) => {
            contains_name_ref(&f.iter, name)
                || f.body.iter().any(|s| stmt_contains_name_ref(s, name))
                || f.orelse.iter().any(|s| stmt_contains_name_ref(s, name))
        }
        Stmt::Import(i) => i.names.iter().any(|alias| {
            let alias_name: &str = alias.asname.as_ref().unwrap_or(&alias.name);
            alias_name == name
        }),
        Stmt::ImportFrom(i) => i.names.iter().any(|alias| {
            let alias_name: &str = alias.asname.as_ref().unwrap_or(&alias.name);
            alias_name == name
        }),
        Stmt::TypeAlias(t) => {
            contains_name_ref(&t.name, name)
                || contains_name_ref(&t.value, name)
        }
        _ => false,
    }
}

// ── RAB056: Nested with statements → single with ─────────────────────

pub struct NestedWithChecker;

impl Checker for NestedWithChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
    fn visit_expr(&mut self, expr: &Expr, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Expr::Call(outer) = expr else { return };
        let Expr::Attribute(attr) = &*outer.func else { return };
        if attr.attr.as_str() != "sort" { return; }
        let Expr::Call(inner) = &*attr.value else { return };
        let Expr::Name(n) = &*inner.func else { return };
        if n.id.as_str() != "sorted" || inner.args.is_empty() { return; }
        if inner.keywords.iter().any(|k| k.arg.as_deref() == Some("reverse")) { return; }

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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
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
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
    fn visit_stmt(&mut self, stmt: &Stmt, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let targets: Option<&[Expr]> = match stmt {
            Stmt::Assign(a) => Some(&a.targets),
            Stmt::FunctionDef(f) => {
                if let Some(builtin) = Self::check_name(f.name.as_str()) {
                    let range = stmt.range();
                    let stmt_start = text_size_to_usize(range.start());
                    let stmt_src = &source[stmt_start..text_size_to_usize(range.end())];
                    let name_in_src = f.name.as_str();
                    // Search after the `def` keyword so decorators / earlier tokens
                    // in the statement range can't shadow the real name position.
                    let search_from = stmt_src.find("def ").map(|i| i + 4).unwrap_or(0);
                    let Some(rel) = stmt_src[search_from..].find(name_in_src) else { return };
                    let name_start = stmt_start + search_from + rel;
                    let name_end = name_start + name_in_src.len();
                    let (line, col) = byte_to_line_col(name_start, line_starts);
                    let (end_line, end_col) = byte_to_line_col(name_end, line_starts);
                    findings.push(Finding {
                        line, col, end_line, end_col,
                        code: "RAB067".to_string(),
                        message: format!("Function '{}' shadows built-in '{}', rename to '{}_'", f.name, builtin, builtin),
                        fix: Some(Fix { start: name_start, end: name_end, replacement: format!("{}_", builtin) }),
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
    except_depth: usize,
}

impl RaiseWithoutFromChecker {
    pub fn new() -> Self {
        Self { except_depth: 0 }
    }
}

impl Checker for RaiseWithoutFromChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
    fn enter_scope(&mut self) { self.except_depth = 0; }
    fn enter_except(&mut self) { self.except_depth += 1; }
    fn exit_except(&mut self) { self.except_depth = self.except_depth.saturating_sub(1); }

    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        if self.except_depth == 0 { return; }
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

// ── RAB069: dict() / list() / tuple() → literal syntax ──────────────

pub struct EmptyCollectionChecker;

impl Checker for EmptyCollectionChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
    fn visit_expr(&mut self, expr: &Expr, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Expr::Call(c) = expr else { return };
        if !c.args.is_empty() || !c.keywords.is_empty() { return; }
        let (replacement, name) = match &*c.func {
            Expr::Name(n) if n.id.as_str() == "dict" => ("{}", "dict"),
            Expr::Name(n) if n.id.as_str() == "list" => ("[]", "list"),
            Expr::Name(n) if n.id.as_str() == "tuple" => ("()", "tuple"),
            _ => return,
        };
        let range = c.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB069".to_string(),
            message: format!("Use '{}' instead of '{}()' for empty collection", replacement, name),
            fix: Some(Fix { start, end, replacement: replacement.to_string() }),
        });
    }
}

// ── RAB070: x == "" / x == [] / x == {} → not x / x ────────────────

pub struct EmptyCompareChecker;

impl Checker for EmptyCompareChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
    fn visit_expr(&mut self, expr: &Expr, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Expr::Compare(c) = expr else { return };
        if c.ops.len() != 1 || c.comparators.len() != 1 { return; }
        let (is_eq, _is_ne) = match c.ops[0] {
            CmpOp::Eq => (true, false),
            CmpOp::NotEq => (false, true),
            _ => return,
        };
        let is_empty = |e: &Expr| -> bool {
            matches!(e, Expr::Constant(cc) if matches!(&cc.value, Constant::Str(s) if s.is_empty()))
                || matches!(e, Expr::List(l) if l.elts.is_empty())
                || matches!(e, Expr::Dict(d) if d.keys.iter().flatten().count() == 0)
        };
        let left_empty = is_empty(&c.left);
        let right_empty = is_empty(&c.comparators[0]);
        if !left_empty && !right_empty { return; }
        let val = if left_empty { &c.comparators[0] } else { &c.left };
        let val_src = expr_to_source(source, val);
        let replacement = if is_eq { not_expr(&val_src) } else { val_src.clone() };
        let range = c.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB070".to_string(),
            message: format!("Use 'not {}' instead of equality with empty literal", val_src),
            fix: Some(Fix { start, end, replacement }),
        });
    }
}

// ── RAB071: list comp inside str.join() → generator ─────────────────

pub struct JoinListCompChecker;

impl Checker for JoinListCompChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
    fn visit_expr(&mut self, expr: &Expr, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Expr::Call(c) = expr else { return };
        let Expr::Attribute(a) = &*c.func else { return };
        if a.attr.as_str() != "join" || c.args.len() != 1 { return; }
        let Expr::ListComp(lc) = &c.args[0] else { return };
        let lc_range = lc.range();
        let start = text_size_to_usize(lc_range.start());
        let end = text_size_to_usize(lc_range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        let inner = &source[start + 1..end - 1];
        let replacement = format!("({})", inner);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB071".to_string(),
            message: "Use a generator expression instead of a list comprehension inside 'str.join()'".to_string(),
            fix: Some(Fix { start, end, replacement }),
        });
    }
}

// ── RAB072: except Exception: raise (dead handler) ──────────────────

pub struct DeadExceptChecker;

impl Checker for DeadExceptChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Stmt::Try(t) = stmt else { return };
        for handler in &t.handlers {
            let ExceptHandler::ExceptHandler(h) = handler;
            if h.body.len() != 1 { continue; }
            let Stmt::Raise(r) = &h.body[0] else { continue };
            if r.exc.is_some() { continue; }
            let range = h.range();
            let start = text_size_to_usize(range.start());
            let end = text_size_to_usize(range.end());
            let (line, col) = byte_to_line_col(start, line_starts);
            let (end_line, end_col) = byte_to_line_col(end, line_starts);
            findings.push(Finding {
                line, col, end_line, end_col,
                code: "RAB072".to_string(),
                message: "Except handler only re-raises, it can be removed".to_string(),
                fix: None,
            });
        }
    }
}

// ── RAB073: old-style % string formatting ────────────────────────────

/// Check if a percent-format string uses only `%s` and `%%` placeholders.
/// Such strings are typically SQL templates or gettext patterns where
/// converting to f-strings would be incorrect.
fn is_sql_or_gettext_template(fmt: &str) -> bool {
    let mut chars = fmt.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            match chars.next() {
                Some('%') => continue,     // escaped % — gettext pattern
                Some('s') => continue,     // %s placeholder — SQL/gettext
                _ => return false,         // other specifier (d, r, f, etc.)
            }
        }
    }
    true
}

pub struct PercentFormatChecker;

impl Checker for PercentFormatChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
    fn visit_expr(&mut self, expr: &Expr, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Expr::BinOp(b) = expr else { return };
        if !matches!(b.op, Operator::Mod) { return; }
        let Expr::Constant(cc) = &*b.left else { return };
        let Constant::Str(s) = &cc.value else { return };
        // Skip SQL/gettext templates that only use %s wildcards
        if is_sql_or_gettext_template(s.as_str()) { return; }
        let range = b.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB073".to_string(),
            message: "Use an f-string instead of old-style '%' string formatting".to_string(),
            fix: None,
        });
    }
}

// ── RAB074: os.path.* → pathlib.Path ─────────────────────────────────

pub struct OsPathChecker;

impl Checker for OsPathChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
    fn visit_expr(&mut self, expr: &Expr, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Expr::Call(c) = expr else { return };
        let Expr::Attribute(outer_attr) = &*c.func else { return };
        let Expr::Attribute(inner_attr) = &*outer_attr.value else { return };
        let Expr::Name(n) = &*inner_attr.value else { return };
        if n.id.as_str() != "os" || inner_attr.attr.as_str() != "path" { return; }
        let range = c.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB074".to_string(),
            message: format!("Use 'pathlib.Path' instead of 'os.path.{}()'", outer_attr.attr),
            fix: None,
        });
    }
}

// ── RAB075: isinstance(x, (A,)) → isinstance(x, A) ──────────────────

pub struct SingleTypeIsinstanceChecker;

impl Checker for SingleTypeIsinstanceChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
    fn visit_expr(&mut self, expr: &Expr, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Expr::Call(c) = expr else { return };
        let Expr::Name(n) = &*c.func else { return };
        if n.id.as_str() != "isinstance" && n.id.as_str() != "issubclass" { return; }
        if c.args.len() != 2 { return; }
        let Expr::Tuple(t) = &c.args[1] else { return };
        if t.elts.len() != 1 { return; }
        let range = c.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        let obj_src = expr_to_source(source, &c.args[0]);
        let type_src = expr_to_source(source, &t.elts[0]);
        let replacement = format!("{}({}, {})", n.id, obj_src, type_src);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB075".to_string(),
            message: format!("Use '{}({}, {})' instead of wrapping a single type in a tuple", n.id, obj_src, type_src),
            fix: Some(Fix { start, end, replacement }),
        });
    }
}

// ── RAB076: str() on string expression ───────────────────────────────

pub struct RedundantStrChecker;

impl Checker for RedundantStrChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
    fn visit_expr(&mut self, expr: &Expr, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Expr::Call(c) = expr else { return };
        let Expr::Name(n) = &*c.func else { return };
        if n.id.as_str() != "str" || c.args.len() != 1 { return; }
        let is_already_str = match &c.args[0] {
            Expr::Constant(cc) => matches!(&cc.value, Constant::Str(_)),
            Expr::Call(inner) => {
                matches!(&*inner.func, Expr::Attribute(a) if a.attr.as_str() == "join")
                    || matches!(&*inner.func, Expr::Name(fn_name) if matches!(fn_name.id.as_str(), "str" | "repr" | "ascii"))
            }
            _ => false,
        };
        if !is_already_str { return; }
        let inner_src = expr_to_source(source, &c.args[0]);
        let range = c.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB076".to_string(),
            message: format!("Redundant 'str()' call on value that is already a string, use '{}' directly", inner_src),
            fix: Some(Fix { start, end, replacement: inner_src }),
        });
    }
}

// ── RAB078: except Exception: pass ────────────────────────────────────

pub struct ExceptPassChecker;

impl Checker for ExceptPassChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Stmt::Try(t) = stmt else { return };
        for handler in &t.handlers {
            let ExceptHandler::ExceptHandler(h) = handler;
            if h.body.len() != 1 { continue; }
            let Stmt::Pass(_) = &h.body[0] else { continue };
            let range = h.range();
            let start = text_size_to_usize(range.start());
            let end = text_size_to_usize(range.end());
            let (line, col) = byte_to_line_col(start, line_starts);
            let (end_line, end_col) = byte_to_line_col(end, line_starts);
            findings.push(Finding {
                line, col, end_line, end_col,
                code: "RAB078".to_string(),
                message: "Except handler only contains 'pass', exception is silently swallowed".to_string(),
                fix: None,
            });
        }
    }
}

// ── RAB079: __del__ method defined ────────────────────────────────────

pub struct DelMethodChecker;

impl Checker for DelMethodChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Stmt::FunctionDef(f) = stmt else { return };
        if f.name.as_str() != "__del__" { return; }
        let range = f.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB079".to_string(),
            message: "__del__ method defined, use a context manager or explicit cleanup instead".to_string(),
            fix: None,
        });
    }
}

// ── RAB080: list(d.keys()) / list(d.values()) / list(d.items()) → list(d) ─

pub struct ListKeysChecker;

impl Checker for ListKeysChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
    fn visit_expr(&mut self, expr: &Expr, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Expr::Call(c) = expr else { return };
        let Expr::Name(n) = &*c.func else { return };
        if n.id.as_str() != "list" || c.args.len() != 1 { return; }
        let Expr::Call(inner) = &c.args[0] else { return };
        let Expr::Attribute(a) = &*inner.func else { return };
        if inner.args.len() != 0 || inner.keywords.len() != 0 { return; }
        if a.attr.as_str() != "keys" && a.attr.as_str() != "values" && a.attr.as_str() != "items" { return; }
        let obj_src = expr_to_source(source, &a.value);
        let range = c.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        let replacement = format!("list({})", obj_src);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB080".to_string(),
            message: format!("Use 'list({})' instead of 'list(d.{})'", obj_src, a.attr),
            fix: Some(Fix { start, end, replacement }),
        });
    }
}

// ── RAB083: Nested ternary (ternary inside ternary) ───────────────────

pub struct NestedTernaryChecker;

impl Checker for NestedTernaryChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
    fn visit_expr(&mut self, expr: &Expr, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Expr::IfExp(ifexp) = expr else { return };
        let has_nested = matches!(&*ifexp.body, Expr::IfExp(_))
            || matches!(&*ifexp.orelse, Expr::IfExp(_));
        if !has_nested { return; }
        let range = ifexp.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB083".to_string(),
            message: "Nested ternary expression harms readability, use if/elif/else instead".to_string(),
            fix: None,
        });
    }
}

// ── RAB085: reversed(sorted(x)) → sorted(x, reverse=True) ────────────

pub struct ReversedSortedChecker;

impl Checker for ReversedSortedChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
    fn visit_expr(&mut self, expr: &Expr, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Expr::Call(outer) = expr else { return };
        let Expr::Name(outer_n) = &*outer.func else { return };
        if outer_n.id.as_str() != "reversed" || outer.args.len() != 1 { return; }
        let Expr::Call(inner) = &outer.args[0] else { return };
        let Expr::Name(inner_n) = &*inner.func else { return };
        if inner_n.id.as_str() != "sorted" { return; }
        if inner.keywords.iter().any(|k| k.arg.as_deref() == Some("reverse")) { return; }

        let first_arg_src = expr_to_source(source, &inner.args[0]);
        let kw_srcs: Vec<String> = inner.keywords.iter().map(|kw| {
            let arg_name = kw.arg.as_deref().unwrap_or("");
            let val_src = expr_to_source(source, &kw.value);
            format!("{}={}", arg_name, val_src)
        }).collect();
        let all_kw = if kw_srcs.is_empty() {
            "reverse=True".to_string()
        } else {
            format!("{}, reverse=True", kw_srcs.join(", "))
        };
        let range = outer.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        let replacement = format!("sorted({}, {})", first_arg_src, all_kw);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB085".to_string(),
            message: "Use 'sorted(x, reverse=True)' instead of 'reversed(sorted(x))'".to_string(),
            fix: Some(Fix { start, end, replacement }),
        });
    }
}

// ── RAB087: dict comp over zip → dict(zip(...)) ──────────────────────

pub struct DictZipChecker;

impl Checker for DictZipChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
    fn visit_expr(&mut self, expr: &Expr, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Expr::DictComp(dc) = expr else { return };
        if dc.generators.len() != 1 { return; }
        let comp = &dc.generators[0];
        if !comp.ifs.is_empty() { return; }
        // Check iter is zip(...)
        let Expr::Call(zip_call) = &comp.iter else { return };
        let Expr::Name(zip_name) = &*zip_call.func else { return };
        if zip_name.id.as_str() != "zip" { return; }
        // Check target is tuple of two names matching key/value
        let Expr::Tuple(tup) = &comp.target else { return };
        if tup.elts.len() != 2 { return; }
        let Expr::Name(k_name) = &tup.elts[0] else { return };
        let Expr::Name(v_name) = &tup.elts[1] else { return };
        let Expr::Name(dk) = &*dc.key else { return };
        let Expr::Name(dv) = &*dc.value else { return };
        if dk.id != k_name.id || dv.id != v_name.id { return; }

        let zip_src = expr_to_source(source, &comp.iter);
        let range = dc.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        let replacement = format!("dict({})", zip_src);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB087".to_string(),
            message: "Use 'dict(zip(...))' instead of a dict comprehension".to_string(),
            fix: Some(Fix { start, end, replacement }),
        });
    }
}

// ── RAB088: while len(x) > 0 → while x ───────────────────────────────

pub struct WhileLenChecker;

impl Checker for WhileLenChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
    fn visit_stmt(&mut self, stmt: &Stmt, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Stmt::While(ww) = stmt else { return };
        let Expr::Compare(c) = &*ww.test else { return };
        if c.ops.len() != 1 || c.comparators.len() != 1 { return; }
        let _is_positive = match c.ops[0] {
            CmpOp::Gt | CmpOp::NotEq => true,
            _ => return,
        };
        let left = &*c.left;
        let right = &c.comparators[0];
        let (val, _is_zero) = if is_literal_zero(left) { (right, false) }
            else if is_literal_zero(right) { (left, true) }
            else { return };
        let Expr::Call(len_call) = val else { return };
        let Expr::Name(len_name) = &*len_call.func else { return };
        if len_name.id.as_str() != "len" || len_call.args.len() != 1 { return; }

        let iter_src = expr_to_source(source, &len_call.args[0]);
        let test_range = ww.test.range();
        let start = text_size_to_usize(test_range.start());
        let end = text_size_to_usize(test_range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        let replacement = iter_src.clone();
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB088".to_string(),
            message: format!("Use 'while {}:' instead of 'while len({}) > 0:'", iter_src, iter_src),
            fix: Some(Fix { start, end, replacement }),
        });
    }
}

fn is_literal_zero(expr: &Expr) -> bool {
    matches!(expr, Expr::Constant(c) if matches!(&c.value, Constant::Int(i) if *i == rustpython_ast::bigint::BigInt::from(0u64)))
}

// ── RAB089: copy.copy(x) → x.copy() ──────────────────────────────────

pub struct CopyCopyChecker;

impl Checker for CopyCopyChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
    fn visit_expr(&mut self, expr: &Expr, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Expr::Call(c) = expr else { return };
        let Expr::Attribute(a) = &*c.func else { return };
        if a.attr.as_str() != "copy" { return; }
        let Expr::Name(n) = &*a.value else { return };
        if n.id.as_str() != "copy" || c.args.len() != 1 { return; }
        let is_literal = matches!(&c.args[0], Expr::List(_) | Expr::Dict(_));
        if !is_literal { return; }
        let arg_src = expr_to_source(source, &c.args[0]);
        let range = c.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        let replacement = format!("{}.copy()", arg_src);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB089".to_string(),
            message: format!("Use '{}.copy()' instead of 'copy.copy({})'", arg_src, arg_src),
            fix: Some(Fix { start, end, replacement }),
        });
    }
}

// ── RAB097: Debug leftover (print, breakpoint, pdb) ─────────────────────────

pub struct DebugLeftoverChecker;

impl Checker for DebugLeftoverChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
    fn visit_expr(&mut self, expr: &Expr, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Expr::Call(c) = expr else { return };
        let is_print = matches!(&*c.func, Expr::Name(n) if n.id.as_str() == "print");
        let is_breakpoint = matches!(&*c.func, Expr::Name(n) if n.id.as_str() == "breakpoint");
        let is_pdb_set_trace = matches!(&*c.func, Expr::Attribute(a) if a.attr.as_str() == "set_trace"
            && matches!(&*a.value, Expr::Name(n) if n.id.as_str() == "pdb"));
        if !is_print && !is_breakpoint && !is_pdb_set_trace { return; }

        let range = c.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        let (code, msg) = if is_print {
            ("RAB097", "Debugging 'print()' call left in production code")
        } else if is_breakpoint {
            ("RAB097", "Debugging 'breakpoint()' call left in production code")
        } else {
            ("RAB097", "Debugging 'pdb.set_trace()' call left in production code")
        };
        findings.push(Finding {
            line, col, end_line, end_col,
            code: code.to_string(),
            message: msg.to_string(),
            fix: None,
        });
    }
}

// ── RAB098: Import inside function body ────────────────────────────────────

pub struct ImportInFunctionChecker {
    depth: u32,
}

impl ImportInFunctionChecker {
    pub fn new() -> Self { Self { depth: 0 } }
}

impl Checker for ImportInFunctionChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
    fn enter_scope(&mut self) { self.depth += 1; }
    fn exit_scope(&mut self, _findings: &mut Vec<Finding>) { self.depth = self.depth.saturating_sub(1); }

    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        if self.depth <= 1 { return; }
        // Check for imports inside the statement list (inside function/class body)
        // depth = 2 means inside a class or function (module level is depth 1)
        let is_import = matches!(stmt, Stmt::Import(_) | Stmt::ImportFrom(_));
        if !is_import { return; }
        let range = match stmt {
            Stmt::Import(i) => i.range(),
            Stmt::ImportFrom(i) => i.range(),
            _ => return,
        };
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB098".to_string(),
            message: "Import inside function/class body, move to module level".to_string(),
            fix: None,
        });
    }
}

// ── RAB099: Duplicate key in dict/set literal ─────────────────────────────

pub struct DuplicateKeyChecker;

impl DuplicateKeyChecker {
    fn constant_value(expr: &Expr) -> Option<String> {
        match expr {
            Expr::Constant(c) => match &c.value {
                Constant::Str(s) => Some(format!("'{}'", s)),
                Constant::Int(i) => Some(i.to_string()),
                Constant::Float(f) => Some(f.to_string()),
                Constant::Bool(b) => Some(b.to_string()),
                Constant::None => Some("None".to_string()),
                _ => None,
            },
            _ => None,
        }
    }

    fn check_dict(&self, keys: &[Option<Expr>], _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let mut seen: FxHashSet<String> = FxHashSet::default();
        for key in keys {
            let Some(key) = key else { continue };
            let Some(val) = Self::constant_value(key) else { continue };
            if seen.contains(&val) {
                let range = key.range();
                let start = text_size_to_usize(range.start());
                let end = text_size_to_usize(range.end());
                let (line, col) = byte_to_line_col(start, line_starts);
                let (end_line, end_col) = byte_to_line_col(end, line_starts);
                findings.push(Finding {
                    line, col, end_line, end_col,
                    code: "RAB099".to_string(),
                    message: format!("Duplicate key '{}' in dict literal", val),
                    fix: None,
                });
            } else {
                seen.insert(val);
            }
        }
    }

    fn check_set(&self, elts: &[Expr], _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let mut seen: FxHashSet<String> = FxHashSet::default();
        for elt in elts {
            let Some(val) = Self::constant_value(elt) else { continue };
            if seen.contains(&val) {
                let range = elt.range();
                let start = text_size_to_usize(range.start());
                let end = text_size_to_usize(range.end());
                let (line, col) = byte_to_line_col(start, line_starts);
                let (end_line, end_col) = byte_to_line_col(end, line_starts);
                findings.push(Finding {
                    line, col, end_line, end_col,
                    code: "RAB099".to_string(),
                    message: format!("Duplicate element '{}' in set literal", val),
                    fix: None,
                });
            } else {
                seen.insert(val);
            }
        }
    }
}

impl Checker for DuplicateKeyChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
    fn visit_expr(&mut self, expr: &Expr, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        match expr {
            Expr::Dict(d) => self.check_dict(&d.keys, source, line_starts, findings),
            Expr::Set(s) => self.check_set(&s.elts, source, line_starts, findings),
            _ => {}
        }
    }
}

// ── RAB106: Too broad except Exception ─────────────────────────────────────

pub struct BroadExceptChecker;

impl Checker for BroadExceptChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Stmt::Try(t) = stmt else { return };
        for handler in &t.handlers {
            let ExceptHandler::ExceptHandler(h) = handler;
            let is_exception = matches!(&h.type_, Some(t) if matches!(&**t, Expr::Name(n) if n.id.as_str() == "Exception"));
            if !is_exception { continue; }
            let range = handler.range();
            let start = text_size_to_usize(range.start());
            let end = text_size_to_usize(range.end());
            let (line, col) = byte_to_line_col(start, line_starts);
            let (end_line, end_col) = byte_to_line_col(end, line_starts);
            findings.push(Finding {
                line, col, end_line, end_col,
                code: "RAB106".to_string(),
                message: "Too broad 'except Exception:', catch only the exceptions you expect".to_string(),
                fix: None,
            });
        }
    }
}

// ── RAB112: Unnecessary pass in non-empty body ───────────────────────────

pub struct UnnecessaryPassChecker;

impl Checker for UnnecessaryPassChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
    fn visit_stmt(&mut self, stmt: &Stmt, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let body: &[Stmt] = match stmt {
            Stmt::FunctionDef(f) => &f.body,
            Stmt::AsyncFunctionDef(f) => &f.body,
            Stmt::ClassDef(c) => &c.body,
            Stmt::For(f) => &f.body,
            Stmt::AsyncFor(f) => &f.body,
            Stmt::While(w) => &w.body,
            Stmt::If(i) => &i.body,
            Stmt::With(w) => &w.body,
            Stmt::AsyncWith(w) => &w.body,
            Stmt::Try(t) => &t.body,
            _ => return,
        };
        if body.len() < 2 { return; }
        let non_pass_count = body.iter().filter(|s| !matches!(s, Stmt::Pass(_))).count();
        if non_pass_count == 0 { return; }
        for s in body {
            if let Stmt::Pass(p) = s {
                let range = p.range();
                let start = text_size_to_usize(range.start());
                let end = text_size_to_usize(range.end());
                let (line, col) = byte_to_line_col(start, line_starts);
                let (end_line, end_col) = byte_to_line_col(end, line_starts);
                let line_idx = line_starts.binary_search(&start).unwrap_or_else(|i| i.saturating_sub(1));
                let line_start = line_starts[line_idx];
                let line_end = line_starts.get(line_idx + 1).copied().unwrap_or(source.len());
                findings.push(Finding {
                    line, col, end_line, end_col,
                    code: "RAB112".to_string(),
                    message: "Unnecessary 'pass' in non-empty body".to_string(),
                    fix: Some(Fix { start: line_start, end: line_end, replacement: String::new() }),
                });
            }
        }
    }
}

// ── RAB090: Missing parameter type annotation (public functions) ──────────
// ── RAB091: Missing return type annotation (all functions) ────────────────

pub struct ParamTypeChecker {
    scope_func_name: Option<String>,
}

impl ParamTypeChecker {
    pub fn new() -> Self { Self { scope_func_name: None } }
}

impl Checker for ParamTypeChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
    fn enter_scope(&mut self) { self.scope_func_name = None; }

    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let (name, args, returns) = match stmt {
            Stmt::FunctionDef(f) => (f.name.as_str(), &f.args, &f.returns),
            Stmt::AsyncFunctionDef(f) => (f.name.as_str(), &f.args, &f.returns),
            _ => return,
        };
        let is_public = !name.starts_with('_');

        // RAB090: Missing parameter type annotations (public functions only)
        if is_public {
            for arg in iter_fn_args(args) {
                let pname = arg.def.arg.as_str();
                if pname == "self" || pname == "cls" { continue; }
                if arg.def.annotation.is_none() {
                    let range = arg.def.range();
                    let start = text_size_to_usize(range.start());
                    let end = text_size_to_usize(range.end());
                    let (line, col) = byte_to_line_col(start, line_starts);
                    let (end_line, end_col) = byte_to_line_col(end, line_starts);
                    findings.push(Finding {
                        line, col, end_line, end_col,
                        code: "RAB090".to_string(),
                        message: format!("Parameter '{}' of public function '{}' is missing type annotation", arg.def.arg, name),
                        fix: None,
                    });
                }
            }
        }

        // RAB091: Missing return type annotation (non-public functions only; public is RAB022)
        if !is_public && returns.is_none() {
            let range = stmt.range();
            let start = text_size_to_usize(range.start());
            let end = text_size_to_usize(range.end());
            let (line, col) = byte_to_line_col(start, line_starts);
            let (end_line, end_col) = byte_to_line_col(end, line_starts);
            findings.push(Finding {
                line, col, end_line, end_col,
                code: "RAB091".to_string(),
                message: format!("Function '{}' is missing a return type annotation", name),
                fix: None,
            });
        }
    }
}

// ── RAB092: Missing class/instance attribute type annotation ─────────────

fn is_enum_base(expr: &Expr) -> bool {
    match expr {
        Expr::Name(n) => matches!(n.id.as_str(), "Enum" | "IntEnum" | "StrEnum" | "Flag" | "IntFlag" | "ReprEnum"),
        Expr::Attribute(a) => {
            let mut cur = &*a.value;
            loop {
                match cur {
                    Expr::Attribute(inner) => cur = &*inner.value,
                    Expr::Name(n) => return n.id.as_str() == "enum",
                    _ => return false,
                }
            }
        }
        _ => false,
    }
}

/// Check if a class name is a Django-style inner config class (Meta, Options, Config, etc.)
fn is_config_inner_class(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    matches!(lower.as_str(), "meta" | "options")
}

pub struct AttrTypeChecker {
    in_class: Vec<bool>,
    next_is_class: bool,
    is_enum: Vec<bool>,
    next_is_enum: bool,
    class_name: Vec<String>,
    next_class_name: String,
}

impl AttrTypeChecker {
    pub fn new() -> Self {
        Self {
            in_class: Vec::new(), next_is_class: false,
            is_enum: Vec::new(), next_is_enum: false,
            class_name: Vec::new(), next_class_name: String::new(),
        }
    }
}

impl Checker for AttrTypeChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
    fn enter_scope(&mut self) {
        self.in_class.push(self.next_is_class);
        self.is_enum.push(self.next_is_enum);
        self.class_name.push(std::mem::take(&mut self.next_class_name));
        self.next_is_class = false;
        self.next_is_enum = false;
    }

    fn exit_scope(&mut self, _findings: &mut Vec<Finding>) {
        self.in_class.pop();
        self.is_enum.pop();
        self.class_name.pop();
    }

    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        if let Stmt::ClassDef(cd) = stmt {
            self.next_is_class = true;
            self.next_is_enum = cd.bases.iter().any(|b| is_enum_base(b))
                || cd.keywords.iter().any(|kw| kw.arg.as_deref() == Some("metaclass") && is_enum_base(&kw.value));
            self.next_class_name = cd.name.to_string();
            return;
        }
        // Only flag if the immediate scope is a class body (not a method inside a class)
        if !self.in_class.last().copied().unwrap_or(false) { return; }
        if *self.is_enum.last().unwrap_or(&false) { return; }
        // Skip inner config classes (Django Meta/Options/Config pattern)
        if let Some(name) = self.class_name.last() {
            if name.is_empty() { /* root scope */ }
            else if is_config_inner_class(name) { return; }
        }

        // Check simple assignments (not already annotated)
        if let Stmt::Assign(a) = stmt {
            if a.targets.len() == 1 {
                if let Expr::Name(n) = &a.targets[0] {
                    if !n.id.as_str().starts_with('_') {
                        let range = n.range();
                        let start = text_size_to_usize(range.start());
                        let end = text_size_to_usize(range.end());
                        let (line, col) = byte_to_line_col(start, line_starts);
                        let (end_line, end_col) = byte_to_line_col(end, line_starts);
                        findings.push(Finding {
                            line, col, end_line, end_col,
                            code: "RAB092".to_string(),
                            message: format!("Class attribute '{}' is missing type annotation", n.id),
                            fix: None,
                        });
                    }
                }
            }
        }
    }
}

// ── RAB093: Missing module-level variable type annotation ─────────────────

pub struct ModuleVarTypeChecker;

impl Checker for ModuleVarTypeChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Stmt::Assign(a) = stmt else { return };
        if a.targets.len() != 1 { return; }
        let Expr::Name(n) = &a.targets[0] else { return };
        if n.id.as_str().starts_with('_') { return; }

        // Skip assignments where the type is obvious from the value
        if matches!(
            a.value.as_ref(),
            Expr::Constant(_) | Expr::Call(_) | Expr::Name(_)
        ) {
            return;
        }

        let range = n.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB093".to_string(),
            message: format!("Module-level variable '{}' is missing type annotation", n.id),
            fix: None,
        });
    }
}

// ── RAB094: Any annotation detected ─────────────────────────────────────

pub struct AnyAnnotationChecker;

impl Checker for AnyAnnotationChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
    fn visit_expr(&mut self, expr: &Expr, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        if let Expr::Subscript(s) = expr {
            if let Expr::Name(n) = &*s.value {
                if n.id.as_str() == "Any" {
                    let range = n.range();
                    let start = text_size_to_usize(range.start());
                    let end = text_size_to_usize(range.end());
                    let (line, col) = byte_to_line_col(start, line_starts);
                    let (end_line, end_col) = byte_to_line_col(end, line_starts);
                    findings.push(Finding {
                        line, col, end_line, end_col,
                        code: "RAB094".to_string(),
                        message: "Use of 'Any' type annotation, prefer a more specific type".to_string(),
                        fix: None,
                    });
                }
            }
        }
        if let Expr::Name(n) = expr {
            if n.id.as_str() == "Any" {
                let range = n.range();
                let start = text_size_to_usize(range.start());
                let end = text_size_to_usize(range.end());
                let (line, col) = byte_to_line_col(start, line_starts);
                let (end_line, end_col) = byte_to_line_col(end, line_starts);
                findings.push(Finding {
                    line, col, end_line, end_col,
                    code: "RAB094".to_string(),
                    message: "Use of 'Any' type annotation, prefer a more specific type".to_string(),
                    fix: None,
                });
            }
        }
    }
}

// ── RAB095: Type annotation vs default value mismatch ─────────────────────

fn annotation_includes_none(ann: &Expr) -> bool {
    match ann {
        Expr::Name(n) => n.id.as_str() == "Optional",
        Expr::Constant(c) => matches!(c.value, Constant::None),
        Expr::Subscript(s) => {
            if let Expr::Name(n) = &*s.value {
                if n.id.as_str() == "Optional" { return true; }
                if n.id.as_str() == "Union" {
                    if let Expr::Tuple(t) = &*s.slice {
                        return t.elts.iter().any(|e| annotation_includes_none(e));
                    }
                }
            }
            false
        }
        Expr::BinOp(b) => {
            b.op == Operator::BitOr
                && (annotation_includes_none(&b.left) || annotation_includes_none(&b.right))
        }
        _ => false,
    }
}

pub struct TypeDefaultMismatchChecker;

impl Checker for TypeDefaultMismatchChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
    fn visit_stmt(&mut self, stmt: &Stmt, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let args = match stmt {
            Stmt::FunctionDef(f) => &f.args,
            Stmt::AsyncFunctionDef(f) => &f.args,
            _ => return,
        };
        for arg in iter_fn_args(args) {
            let Some(annotation) = &arg.def.annotation else { continue };
            let Some(default) = &arg.default else { continue };
            let is_none_default = matches!(&**default, Expr::Constant(cc) if matches!(&cc.value, Constant::None));
            let is_mutable_default = matches!(&**default, Expr::List(_) | Expr::Dict(_) | Expr::Set(_));
            if !is_none_default && !is_mutable_default { continue; }

            if is_none_default && annotation_includes_none(annotation) { continue; }

            let annotation_is_concrete = |e: &Expr| -> bool {
                match e {
                    Expr::Name(n) => !matches!(n.id.as_str(), "Any" | "Optional"),
                    Expr::Subscript(s) => matches!(&*s.value, Expr::Name(n) if !matches!(n.id.as_str(), "Optional" | "Union")),
                    _ => true,
                }
            };

            if !annotation_is_concrete(annotation) { continue; }
            let range = default.range();
            let start = text_size_to_usize(range.start());
            let end = text_size_to_usize(range.end());
            let (line, col) = byte_to_line_col(start, line_starts);
            let (end_line, end_col) = byte_to_line_col(end, line_starts);
            let default_src = &source[start..end];
            let ann_src = &source[text_size_to_usize(annotation.range().start())..text_size_to_usize(annotation.range().end())];
            findings.push(Finding {
                line, col, end_line, end_col,
                code: "RAB095".to_string(),
                message: format!("Default value '{}' may be incompatible with type annotation '{}'", default_src, ann_src),
                fix: None,
            });
        }
    }
}

// ── RAB100: Redundant elif after return/raise/break/continue ──────────────

pub struct RedundantElifChecker;

impl Checker for RedundantElifChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Stmt::If(i) = stmt else { return };
        if i.orelse.is_empty() { return; }
        // Check if body ends with a terminal statement
        let Some(last) = i.body.last() else { return };
        let is_terminal = matches!(last, Stmt::Return(_) | Stmt::Raise(_) | Stmt::Break(_) | Stmt::Continue(_));
        if !is_terminal { return; }
        // Check if orelse is an elif chain
        let is_elif = i.orelse.len() == 1 && matches!(&i.orelse[0], Stmt::If(_));
        if !is_elif { return; }

        let range = i.range();
        let (line, col) = byte_to_line_col(text_size_to_usize(range.start()), line_starts);
        let (end_line, end_col) = byte_to_line_col(text_size_to_usize(range.end()), line_starts);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB100".to_string(),
            message: "Redundant 'elif' after return/raise/break/continue, merge into body".to_string(),
            fix: None,
        });
    }
}

// ── RAB103: Self-comparison (x == x) ────────────────────────────────────

pub struct SelfComparisonChecker;

impl Checker for SelfComparisonChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
    fn visit_expr(&mut self, expr: &Expr, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Expr::Compare(c) = expr else { return };
        // Compare left with each comparator
        let mut all_self = true;
        for comp in &c.comparators {
            if !exprs_are_equal(&c.left, comp) {
                all_self = false;
                break;
            }
        }
        if !all_self { return; }

        let range = c.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB103".to_string(),
            message: "Self-comparison always evaluates to True/False, check the logic".to_string(),
            fix: None,
        });
    }
}

fn exprs_are_equal(a: &Expr, b: &Expr) -> bool {
    match (a, b) {
        (Expr::Name(na), Expr::Name(nb)) => na.id == nb.id,
        (Expr::Constant(ca), Expr::Constant(cb)) => {
            format!("{:?}", ca.value) == format!("{:?}", cb.value)
        }
        _ => false,
    }
}

// ── RAB104: Pass-through generator inside list()/set()/tuple() ──────────

pub struct PassThroughGenChecker;

impl Checker for PassThroughGenChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
    fn visit_expr(&mut self, expr: &Expr, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Expr::Call(c) = expr else { return };
        let func_name = match &*c.func {
            Expr::Name(n) if matches!(n.id.as_str(), "list" | "set" | "tuple" | "frozenset") => n.id.as_str(),
            _ => return,
        };
        if c.args.len() != 1 { return; }
        let Expr::GeneratorExp(ge) = &c.args[0] else { return };
        if ge.generators.len() != 1 { return; }
        let gen = &ge.generators[0];
        if !gen.ifs.is_empty() { return; }
        // Check if the generator expression is just "var for var in iter" (pass-through)
        let is_passthrough = match (&*ge.elt, &gen.target) {
            (Expr::Name(elt), Expr::Name(target)) => elt.id == target.id,
            _ => false,
        };
        if !is_passthrough { return; }

        let range = c.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        let iter_src = expr_to_source(source, &gen.iter);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB104".to_string(),
            message: format!("Use '{}({})' instead of '{}(x for x in {})'", func_name, iter_src, func_name, iter_src),
            fix: None,
        });
    }
}

// ── RAB107: TODO/FIXME/HACK comments ────────────────────────────────────

pub struct TodoCommentChecker {
    done: bool,
}

impl TodoCommentChecker {
    pub fn new() -> Self { Self { done: false } }
}

impl Checker for TodoCommentChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
    fn visit_stmt(&mut self, _stmt: &Stmt, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        if self.done { return; }
        self.done = true;
        // Scan all lines for TODO/FIXME/HACK/XXX comments
        for (i, &line_start) in line_starts.iter().enumerate() {
            let line_end = line_starts.get(i + 1).copied().unwrap_or(source.len());
            let line = &source[line_start..line_end];
            let stripped = line.trim_start();
            if !stripped.starts_with('#') { continue; }
            let upper = stripped.to_uppercase();
            let has_todo = upper.contains("TODO") || upper.contains("FIXME") || upper.contains("HACK") || upper.contains("XXX");
            if !has_todo { continue; }
            let (line_num, col) = byte_to_line_col(line_start, line_starts);
            findings.push(Finding {
                line: line_num, col, end_line: line_num, end_col: 0,
                code: "RAB107".to_string(),
                message: format!("Comment contains TODO/FIXME/HACK/XXX: '{}'", stripped.trim()),
                fix: None,
            });
        }
    }
}

// ── RAB109: Class name should be CamelCase ─────────────────────────────

pub struct ClassNameChecker;

impl Checker for ClassNameChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Stmt::ClassDef(c) = stmt else { return };
        let name = c.name.as_str();
        if name.chars().all(|c| c == '_') { return; }
        let stripped = name.trim_start_matches('_');
        let starts_upper = stripped.chars().next().map_or(false, |c| c.is_uppercase());
        let has_underscore = stripped.contains('_');
        if starts_upper && !has_underscore { return; }

        let range = c.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB109".to_string(),
            message: format!("Class name '{}' should use CamelCase convention", name),
            fix: None,
        });
    }
}

// ── RAB110: Function name should be snake_case ─────────────────────────

const UNITTEST_CONVENTIONS: &[&str] = &[
    "setUp", "tearDown", "setUpTestData", "setUpClass", "tearDownClass",
    "setUpTestCaseData",
];

pub struct FunctionNameChecker;

impl Checker for FunctionNameChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let name = match stmt {
            Stmt::FunctionDef(f) => f.name.as_str(),
            Stmt::AsyncFunctionDef(f) => f.name.as_str(),
            _ => return,
        };
        // Skip dunder methods like __init__, __str__
        if name.starts_with("__") && name.ends_with("__") { return; }
        // Skip private methods starting with _
        if name.starts_with('_') && name.len() > 1 && name.as_bytes()[1] != b'_' { return; }
        // Skip unittest convention methods (setUp, tearDown, etc.)
        if UNITTEST_CONVENTIONS.contains(&name) { return; }
        // Skip assert* methods (test assertion helpers like assertEqual, assertRaises)
        if name.starts_with("assert") && name.len() > 6
            && name.as_bytes().get(6).map_or(false, |&b| b.is_ascii_uppercase()) { return; }
        let has_upper = name.chars().any(|c| c.is_uppercase());
        let has_underscore = name.contains('_');
        if !has_upper || has_underscore { return; }

        let range = stmt.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB110".to_string(),
            message: format!("Function name '{}' should use snake_case convention", name),
            fix: None,
        });
    }
}

// ── RAB111: Module-level constant should be UPPER_CASE ──────────────────

pub struct ConstantNameChecker {
    depth: u32,
}

impl ConstantNameChecker {
    pub fn new() -> Self { Self { depth: 0 } }
}

impl Checker for ConstantNameChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
    fn enter_scope(&mut self) { self.depth += 1; }
    fn exit_scope(&mut self, _findings: &mut Vec<Finding>) { self.depth = self.depth.saturating_sub(1); }

    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        if self.depth != 1 { return; } // Module level only
        let Stmt::Assign(a) = stmt else { return };
        if a.targets.len() != 1 { return; }
        let Expr::Name(n) = &a.targets[0] else { return };
        let name = n.id.as_str();
        if name.starts_with('_') { return; }
        let is_constant_value = matches!(&*a.value, Expr::Constant(_));
        if !is_constant_value { return; }
        // Check if name is UPPER_CASE
        let is_upper = name.chars().all(|c| c.is_uppercase() || c == '_');
        if is_upper { return; }

        let range = n.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB111".to_string(),
            message: format!("Module-level constant '{}' should use UPPER_CASE naming", name),
            fix: None,
        });
    }
}

// ── RAB096: Unused import ────────────────────────────────────────────────

pub struct UnusedImportChecker {
    imports: Vec<(String, String, usize, usize)>, // (display_name, original_name, line, col)
    used: Vec<String>,
    all_names: Vec<String>,
    scope_stack: Vec<(Vec<(String, String, usize, usize)>, Vec<String>)>,
}

impl UnusedImportChecker {
    pub fn new() -> Self {
        Self {
            imports: Vec::new(),
            used: Vec::new(),
            all_names: Vec::new(),
            scope_stack: Vec::new(),
        }
    }
}

impl Checker for UnusedImportChecker {
    fn enter_scope(&mut self) {
        self.scope_stack.push((
            std::mem::take(&mut self.imports),
            std::mem::take(&mut self.used),
        ));
    }

    fn exit_scope(&mut self, findings: &mut Vec<Finding>) {
        self.used.extend(self.all_names.drain(..));
        let used_set: FxHashSet<&str> = self.used.iter().map(|s| s.as_str()).collect();
        for (name, _original, line, col) in &self.imports {
            if !used_set.contains(name.as_str()) {
                findings.push(Finding {
                    line: *line, col: *col, end_line: 0, end_col: 0,
                    code: "RAB096".to_string(),
                    message: format!("Import '{}' is unused", name),
                    fix: None,
                });
            }
        }
        if let Some((parent_imports, mut parent_used)) = self.scope_stack.pop() {
            parent_used.extend(std::mem::take(&mut self.used));
            self.imports = parent_imports;
            self.used = parent_used;
        }
    }

    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, line_starts: &[usize], _findings: &mut Vec<Finding>) {
        match stmt {
            Stmt::Import(i) => {
                for alias in &i.names {
                    if alias.name.as_str() == "*" { continue; }
                    let name = alias.asname.clone().unwrap_or_else(|| alias.name.clone());
                    let short = name.split('.').next().unwrap_or(&name).to_string();
                    let range = alias.range();
                    let (line, col) = byte_to_line_col(text_size_to_usize(range.start()), line_starts);
                    self.imports.push((short, alias.name.to_string(), line, col));
                }
            }
            Stmt::ImportFrom(i) => {
                if i.module.as_deref() == Some("__future__") { return; }
                for alias in &i.names {
                    if alias.name.as_str() == "*" { continue; }
                    let name = alias.asname.clone().unwrap_or_else(|| alias.name.clone());
                    let range = alias.range();
                    let (line, col) = byte_to_line_col(text_size_to_usize(range.start()), line_starts);
                    self.imports.push((name.to_string(), name.to_string(), line, col));
                }
            }
            Stmt::Assign(a) => {
                if a.targets.len() == 1 {
                    if let Expr::Name(n) = &a.targets[0] {
                        if n.id.as_str() == "__all__" {
                            collect_all_names_from_expr(&a.value, &mut self.all_names);
                        }
                    }
                }
            }
            Stmt::AugAssign(a) => {
                if let Expr::Name(n) = &*a.target {
                    if n.id.as_str() == "__all__" {
                        collect_all_names_from_expr(&a.value, &mut self.all_names);
                    }
                }
            }
            Stmt::Expr(e) => {
                if let Expr::Call(c) = &*e.value {
                    if let Expr::Attribute(a) = &*c.func {
                        if let Expr::Name(n) = &*a.value {
                            if n.id.as_str() == "__all__" && (a.attr.as_str() == "append" || a.attr.as_str() == "extend") {
                                if let Some(arg) = c.args.first() {
                                    collect_all_names_from_expr(arg, &mut self.all_names);
                                }
                            }
                        }
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

// ── RAB105: Inconsistent return statements ───────────────────────────────

pub struct InconsistentReturnChecker {
    has_value_return: bool,
    has_bare_return: bool,
    func_pos: (usize, usize),
    stack: Vec<(bool, bool)>,
}

impl InconsistentReturnChecker {
    pub fn new() -> Self {
        Self {
            has_value_return: false,
            has_bare_return: false,
            func_pos: (0, 0),
            stack: Vec::new(),
        }
    }
}

impl Checker for InconsistentReturnChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
    fn enter_scope(&mut self) {
        self.stack.push((self.has_value_return, self.has_bare_return));
        self.has_value_return = false;
        self.has_bare_return = false;
    }

    fn exit_scope(&mut self, findings: &mut Vec<Finding>) {
        if self.has_value_return && self.has_bare_return {
            let (line, col) = self.func_pos;
            findings.push(Finding {
                line, col, end_line: line, end_col: col,
                code: "RAB105".to_string(),
                message: "Inconsistent return statements: mix of bare 'return' and 'return <value>' in the same function".to_string(),
                fix: None,
            });
        }
        if let Some((parent_value, parent_bare)) = self.stack.pop() {
            self.has_value_return = parent_value || self.has_value_return;
            self.has_bare_return = parent_bare || self.has_bare_return;
        }
    }

    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, line_starts: &[usize], _findings: &mut Vec<Finding>) {
        if let Stmt::FunctionDef(f) = stmt {
            let start = text_size_to_usize(f.range().start());
            let (line, col) = byte_to_line_col(start, line_starts);
            self.func_pos = (line, col);
        }
        if let Stmt::AsyncFunctionDef(f) = stmt {
            let start = text_size_to_usize(f.range().start());
            let (line, col) = byte_to_line_col(start, line_starts);
            self.func_pos = (line, col);
        }
        if let Stmt::Return(r) = stmt {
            if r.value.is_some() {
                self.has_value_return = true;
            } else {
                self.has_bare_return = true;
            }
        }
    }
}

// ── RAB108: Incorrect __all__ ────────────────────────────────────────────

pub struct AllExportChecker;

impl Checker for AllExportChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Stmt::Assign(a) = stmt else { return };
        if a.targets.len() != 1 { return; }
        let Expr::Name(n) = &a.targets[0] else { return };
        if n.id.as_str() != "__all__" { return; }

        // Check that __all__ is a list/tuple of string constants
        let items: Vec<&Expr> = match &*a.value {
            Expr::List(l) => l.elts.iter().collect(),
            Expr::Tuple(t) => t.elts.iter().collect(),
            _ => return, // Not a list or tuple
        };

        for item in &items {
            let is_str = match item {
                Expr::Constant(c) => matches!(&c.value, Constant::Str(_)),
                _ => false,
            };
            if !is_str {
                let range = item.range();
                let start = text_size_to_usize(range.start());
                let end = text_size_to_usize(range.end());
                let (line, col) = byte_to_line_col(start, line_starts);
                let (end_line, end_col) = byte_to_line_col(end, line_starts);
                findings.push(Finding {
                    line, col, end_line, end_col,
                    code: "RAB108".to_string(),
                    message: "'__all__' should contain only string literals".to_string(),
                    fix: None,
                });
            }
        }
    }
}

// ── RAB113: eval/exec detected ───────────────────────────────────────────

pub struct EvalExecChecker;

impl Checker for EvalExecChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
    fn visit_expr(&mut self, expr: &Expr, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Expr::Call(c) = expr else { return };
        let name = match &*c.func {
            Expr::Name(n) if n.id.as_str() == "eval" || n.id.as_str() == "exec" => n.id.as_str(),
            _ => return,
        };
        let range = c.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB113".to_string(),
            message: format!("Use of '{}' can lead to code injection vulnerabilities, avoid it", name),
            fix: None,
        });
    }
}

// ── RAB114: pickle.load without security consideration ───────────────────

pub struct PickleLoadChecker;

impl Checker for PickleLoadChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
    fn visit_expr(&mut self, expr: &Expr, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Expr::Call(c) = expr else { return };
        let Expr::Attribute(a) = &*c.func else { return };
        let Expr::Name(n) = &*a.value else { return };
        if n.id.as_str() != "pickle" { return; }
        if a.attr.as_str() != "load" && a.attr.as_str() != "loads" { return; }
        let range = c.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB114".to_string(),
            message: "Use of 'pickle.load' on untrusted data is insecure, consider a safer serialization format".to_string(),
            fix: None,
        });
    }
}

// ── RAB115: yaml.load without Loader ─────────────────────────────────────

pub struct YamlLoadChecker;

impl Checker for YamlLoadChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
    fn visit_expr(&mut self, expr: &Expr, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Expr::Call(c) = expr else { return };
        let Expr::Attribute(a) = &*c.func else { return };
        let Expr::Name(n) = &*a.value else { return };
        if n.id.as_str() != "yaml" { return; }
        if a.attr.as_str() != "load" { return; }
        let has_loader = c.keywords.iter().any(|kw| kw.arg.as_deref() == Some("Loader"));
        if has_loader { return; }
        let range = c.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB115".to_string(),
            message: "Use 'yaml.safe_load()' or specify 'Loader=yaml.SafeLoader' to avoid arbitrary code execution".to_string(),
            fix: None,
        });
    }
}

// ── RAB118: del on except variable (Python 3.12+) ────────────────────────

pub struct DelExceptVarChecker {
    except_depth: usize,
    except_var: Option<String>,
}

impl DelExceptVarChecker {
    pub fn new() -> Self { Self { except_depth: 0, except_var: None } }
}

impl Checker for DelExceptVarChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
    fn enter_scope(&mut self) { self.except_depth = 0; self.except_var = None; }
    fn enter_except(&mut self) { self.except_depth += 1; }
    fn exit_except(&mut self) {
        self.except_depth = self.except_depth.saturating_sub(1);
        if self.except_depth == 0 { self.except_var = None; }
    }

    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        // Track exception variable names from try/except handlers
        if let Stmt::Try(t) = stmt {
            for handler in &t.handlers {
                let ExceptHandler::ExceptHandler(eh) = handler;
                if let Some(name) = &eh.name {
                    self.except_var = Some(name.to_string());
                }
            }
            return;
        }
        if self.except_depth == 0 { return; }
        let Stmt::Delete(d) = stmt else { return };
        for target in &d.targets {
            if let Expr::Name(n) = target {
                if let Some(var) = &self.except_var {
                    if n.id.as_str() == var.as_str() {
                        let range = d.range();
                        let start = text_size_to_usize(range.start());
                        let end = text_size_to_usize(range.end());
                        let (line, col) = byte_to_line_col(start, line_starts);
                        let (end_line, end_col) = byte_to_line_col(end, line_starts);
                        findings.push(Finding {
                            line, col, end_line, end_col,
                            code: "RAB118".to_string(),
                            message: format!("Deleting exception variable '{}' with 'del' clears the exception chain in Python 3.12+", var),
                            fix: None,
                        });
                    }
                }
            }
        }
    }
}

// ── RAB120: Modifying iterable during iteration ─────────────────────────

pub struct ModifyIterChecker;

impl Checker for ModifyIterChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let iter_name = match stmt {
            Stmt::For(f) => {
                if let Expr::Name(n) = &*f.iter { n.id.to_string() } else { return }
            }
            Stmt::AsyncFor(f) => {
                if let Expr::Name(n) = &*f.iter { n.id.to_string() } else { return }
            }
            _ => return,
        };
        // Check body for method calls that modify the iterable
        let body = match stmt {
            Stmt::For(f) => &f.body,
            Stmt::AsyncFor(f) => &f.body,
            _ => return,
        };
        for s in body {
            if let Stmt::Expr(e) = s {
                if let Expr::Call(c) = &*e.value {
                    if let Expr::Attribute(a) = &*c.func {
                        if let Expr::Name(n) = &*a.value {
                            if n.id.as_str() != iter_name.as_str() { continue; }
                            let is_mutator = matches!(a.attr.as_str(),
                                "remove" | "pop" | "append" | "clear" | "insert" | "__delitem__" | "__setitem__"
                            );
                            if is_mutator {
                                let range = c.range();
                                let start = text_size_to_usize(range.start());
                                let end = text_size_to_usize(range.end());
                                let (line, col) = byte_to_line_col(start, line_starts);
                                let (end_line, end_col) = byte_to_line_col(end, line_starts);
                                findings.push(Finding {
                                    line, col, end_line, end_col,
                                    code: "RAB120".to_string(),
                                    message: format!("Modifying '{}' while iterating over it can cause skipped items or runtime errors", iter_name),
                                    fix: None,
                                });
                            }
                        }
                    }
                    // Also check for del dict[key] pattern
                    if let Expr::Subscript(s) = &*c.func {
                        if let Expr::Name(n) = &*s.value {
                            if n.id.as_str() == iter_name.as_str() {
                                // Direct subscript mutation detected
                            }
                        }
                    }
                }
            }
            // Check for del dict[key] pattern
            if let Stmt::Delete(d) = s {
                for target in &d.targets {
                    if let Expr::Subscript(sub) = target {
                        if let Expr::Name(n) = &*sub.value {
                            if n.id.as_str() == iter_name.as_str() {
                                let range = d.range();
                                let start = text_size_to_usize(range.start());
                                let end = text_size_to_usize(range.end());
                                let (line, col) = byte_to_line_col(start, line_starts);
                                let (end_line, end_col) = byte_to_line_col(end, line_starts);
                                findings.push(Finding {
                                    line, col, end_line, end_col,
                                    code: "RAB120".to_string(),
                                    message: format!("Modifying '{}' while iterating over it can cause skipped items or runtime errors", iter_name),
                                    fix: None,
                                });
                            }
                        }
                    }
                }
            }
        }
    }
}

// ── RAB123: asyncio.get_event_loop (deprecated 3.12+) ────────────────────

pub struct DeprecatedAsyncioChecker;

impl Checker for DeprecatedAsyncioChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
    fn visit_expr(&mut self, expr: &Expr, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Expr::Call(c) = expr else { return };
        let Expr::Attribute(a) = &*c.func else { return };
        let Expr::Name(n) = &*a.value else { return };
        if n.id.as_str() != "asyncio" { return; }
        match a.attr.as_str() {
            "get_event_loop" | "ensure_future" => {}
            _ => return,
        }
        let range = c.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        let msg = if a.attr.as_str() == "get_event_loop" {
            "'asyncio.get_event_loop()' is deprecated in Python 3.12+, use 'asyncio.get_running_loop()' or 'asyncio.new_event_loop()'"
        } else {
            "'asyncio.ensure_future()' is deprecated, use 'asyncio.create_task()' instead"
        };
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB123".to_string(),
            message: msg.to_string(),
            fix: None,
        });
    }
}

// ── RAB124: Blocking calls inside async function ─────────────────────────

pub struct AsyncBlockingChecker {
    in_async: Vec<bool>,
    next_is_async: bool,
}

impl AsyncBlockingChecker {
    pub fn new() -> Self { Self { in_async: Vec::new(), next_is_async: false } }
}

impl Checker for AsyncBlockingChecker {
    fn enter_scope(&mut self) {
        self.in_async.push(self.next_is_async);
        self.next_is_async = false;
    }

    fn exit_scope(&mut self, _findings: &mut Vec<Finding>) { self.in_async.pop(); }

    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, _line_starts: &[usize], _findings: &mut Vec<Finding>) {
        if matches!(stmt, Stmt::AsyncFunctionDef(_)) {
            self.next_is_async = true;
        }
    }

    fn visit_expr(&mut self, expr: &Expr, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        if !self.in_async.last().copied().unwrap_or(false) { return; }
        let Expr::Call(c) = expr else { return };
        let (module, func) = match &*c.func {
            Expr::Attribute(a) => {
                let func_name = a.attr.as_str();
                let module_name = match &*a.value {
                    Expr::Name(n) => n.id.as_str(),
                    _ => return,
                };
                (module_name, func_name)
            }
            Expr::Name(n) => ("", n.id.as_str()),
            _ => return,
        };
        let is_blocking = match (module, func) {
            ("time", "sleep") => true,
            ("", "input") => true,
            ("subprocess", _) => true,
            ("requests", _) => true,
            ("urllib", _) => true,
            ("os", "system" | "popen") => true,
            _ => false,
        };
        if !is_blocking { return; }
        let range = c.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        let mut msg = format!("Blocking call '{}.{}()' inside async function, use an async alternative", module, func);
        if module == "" {
            msg = format!("Blocking call '{0}()' inside async function, use an async alternative", func);
        }
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB124".to_string(),
            message: msg,
            fix: None,
        });
    }
}

// ── RAB126: Magic numbers ───────────────────────────────────────────────

const COMMON_PORTS: &[i64] = &[22, 80, 443, 3306, 5432, 6379, 8080, 8443, 9090, 3000];
const COMMON_TIME: &[i64] = &[3600, 86400, 604800, 2592000, 31536000, 2678400, 31622400];

pub struct MagicNumberChecker {
    depth: u32,
}

impl MagicNumberChecker {
    pub fn new() -> Self { Self { depth: 0 } }
    fn is_allowed(n: &str) -> bool {
        if matches!(n, "0" | "1" | "-1" | "0.0" | "1.0" | "-1.0" | "0.5"
            | "60" | "24" | "365" | "100" | "1000" | "1024"
            | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"
            | "10" | "100.0" | "60.0") { return true; }
        if let Ok(val) = n.parse::<i64>() {
            // Years, HTTP status codes (100-599), common 3-digit ranges
            if (1000..=2099).contains(&val) { return true; }
            if (100..=599).contains(&val) { return true; }
            if COMMON_PORTS.contains(&val) { return true; }
            if COMMON_TIME.contains(&val) { return true; }
        }
        false
    }
}

impl Checker for MagicNumberChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
    fn enter_scope(&mut self) { self.depth += 1; }
    fn exit_scope(&mut self, _findings: &mut Vec<Finding>) { self.depth = self.depth.saturating_sub(1); }

    fn visit_expr(&mut self, expr: &Expr, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        if self.depth <= 1 { return; } // Skip module level
        let Expr::Constant(c) = expr else { return };
        // Cheap guards before allocating the value string.
        if !matches!(&c.value, Constant::Int(_) | Constant::Float(_)) { return; }
        // Skip numbers used in annotations/decorators (they're usually intentional)
        if findings.last().map_or(false, |f| f.code == "RAB126") { return; }
        let val_str = match &c.value {
            Constant::Int(i) => i.to_string(),
            Constant::Float(f) => f.to_string(),
            _ => unreachable!(),
        };
        if Self::is_allowed(&val_str) { return; }
        let range = c.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB126".to_string(),
            message: format!("Magic number '{}' detected, assign to a named constant instead", val_str),
            fix: None,
        });
    }
}

// ── RAB127: Unnecessary else after break/continue in loop ───────────────

pub struct LoopElseAfterBreakChecker;

impl Checker for LoopElseAfterBreakChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let (body, orelse) = match stmt {
            Stmt::For(f) => (&f.body, &f.orelse),
            Stmt::AsyncFor(f) => (&f.body, &f.orelse),
            Stmt::While(w) => (&w.body, &w.orelse),
            _ => return,
        };
        if orelse.is_empty() { return; }
        let has_break = body.iter().any(|s| has_break_in_stmt(s));
        if !has_break { return; }

        let range = stmt.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB127".to_string(),
            message: "Loop has 'break' and 'else' clause; 'else' body runs only if no break occurs, consider removing if always reached".to_string(),
            fix: None,
        });
    }
}

fn has_break_in_stmt(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Break(_) => true,
        Stmt::If(i) => i.body.iter().any(|s| has_break_in_stmt(s))
            || i.orelse.iter().any(|s| has_break_in_stmt(s)),
        Stmt::Try(t) => {
            t.body.iter().any(|s| has_break_in_stmt(s))
                || t.handlers.iter().any(|h| match h {
                    ExceptHandler::ExceptHandler(eh) => eh.body.iter().any(|s| has_break_in_stmt(s)),
                })
        }
        Stmt::With(w) => w.body.iter().any(|s| has_break_in_stmt(s)),
        Stmt::Match(m) => m.cases.iter().any(|case| case.body.iter().any(|s| has_break_in_stmt(s))),
        _ => false,
    }
}

// ── RAB128: not ... in → not in ─────────────────────────────────────────

pub struct NotInChecker;

impl Checker for NotInChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
    fn visit_expr(&mut self, expr: &Expr, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Expr::UnaryOp(u) = expr else { return };
        if !matches!(u.op, UnaryOp::Not) { return; }
        let Expr::Compare(c) = &*u.operand else { return };
        if c.ops.len() != 1 || c.comparators.len() != 1 { return; }
        if !matches!(c.ops[0], CmpOp::In) { return; }

        let range = u.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        let left_src = expr_to_source(source, &c.left);
        let right_src = expr_to_source(source, &c.comparators[0]);
        let replacement = format!("{} not in {}", left_src, right_src);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB128".to_string(),
            message: "Use 'x not in y' instead of 'not x in y' (PEP 8)".to_string(),
            fix: Some(Fix { start, end, replacement }),
        });
    }
}

// ── RAB119: __init__ without super().__init__() in subclass ──────────────

pub struct SuperInitChecker;

impl Checker for SuperInitChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
    fn enter_scope(&mut self) {} // Reset state when entering a class body

    fn visit_stmt(&mut self, stmt: &Stmt, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Stmt::ClassDef(c) = stmt else { return };
        if c.bases.is_empty() { return; } // Only check subclasses
        // Look for __init__ in the class body
        let has_init = c.body.iter().any(|s| match s {
            Stmt::FunctionDef(f) => f.name.as_str() == "__init__",
            _ => false,
        });
        if !has_init { return; }

        // Check if the __init__ calls super().__init__()
        for s in &c.body {
            let Stmt::FunctionDef(f) = s else { continue };
            if f.name.as_str() != "__init__" { continue; }
            let has_super_init = f.body.iter().any(|body_stmt| {
                contains_super_init_call(body_stmt, source)
            });
            if !has_super_init {
                let range = f.range();
                let start = text_size_to_usize(range.start());
                let end = text_size_to_usize(range.end());
                let (line, col) = byte_to_line_col(start, line_starts);
                let (end_line, end_col) = byte_to_line_col(end, line_starts);
                let base_names: Vec<String> = c.bases.iter().map(|b| expr_to_source(source, b)).collect();
                findings.push(Finding {
                    line, col, end_line, end_col,
                    code: "RAB119".to_string(),
                    message: format!("'__init__' in subclass of {} does not call 'super().__init__()'", base_names.join(", ")),
                    fix: None,
                });
            }
        }
    }
}

fn contains_super_init_call(stmt: &Stmt, source: &str) -> bool {
    match stmt {
        Stmt::Expr(e) => {
            if let Expr::Call(c) = &*e.value {
                if let Expr::Attribute(a) = &*c.func {
                    if a.attr.as_str() == "__init__" {
                        if let Expr::Call(inner) = &*a.value {
                            if let Expr::Name(n) = &*inner.func {
                                if n.id.as_str() == "super" {
                                    return true;
                                }
                            }
                        }
                    }
                }
            }
            false
        }
        Stmt::If(i) => i.body.iter().any(|s| contains_super_init_call(s, source))
            || i.orelse.iter().any(|s| contains_super_init_call(s, source)),
        Stmt::Try(t) => {
            t.body.iter().any(|s| contains_super_init_call(s, source))
                || t.handlers.iter().any(|h| match h {
                    ExceptHandler::ExceptHandler(eh) => eh.body.iter().any(|s| contains_super_init_call(s, source)),
                })
        }
        _ => false,
    }
}

impl Checker for TypeUnionChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
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

// ── RAB129: f-string in logging call ─────────────────────────────────────

pub struct LoggingFstringChecker;

const LOG_METHODS: &[&str] = &["debug", "info", "warning", "error", "critical", "exception", "log"];

fn escape_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '%' => out.push_str("%%"),
            other => out.push(other),
        }
    }
    out
}

impl Checker for LoggingFstringChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
    fn visit_expr(&mut self, expr: &Expr, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Expr::Call(c) = expr else { return };
        let Expr::Attribute(a) = &*c.func else { return };
        if !LOG_METHODS.contains(&a.attr.as_str()) { return; }
        let msg_arg = if a.attr.as_str() == "log" {
            c.args.get(1)
        } else {
            c.args.first()
        };
        let Some(msg_arg) = msg_arg else { return };
        let Expr::JoinedStr(js) = msg_arg else { return };

        let arg_range = msg_arg.range();
        let start = text_size_to_usize(arg_range.start());
        let end = text_size_to_usize(arg_range.end());

        let mut new_fmt = String::from("\"");
        let mut extra_args: Vec<String> = Vec::new();

        for val in &js.values {
            match val {
                Expr::Constant(cnst) => {
                    if let Constant::Str(s) = &cnst.value {
                        new_fmt.push_str(&escape_str(s));
                    }
                }
                Expr::FormattedValue(fv) => {
                    let spec_char = match fv.conversion {
                        ConversionFlag::Repr => 'r',
                        ConversionFlag::Ascii => 'a',
                        _ => 's',
                    };
                    let spec_text: Option<String> = fv.format_spec.as_ref().and_then(|se| {
                        match se.as_ref() {
                            Expr::Constant(sc) => {
                                if let Constant::Str(s) = &sc.value { Some(s.clone()) } else { None }
                            }
                            Expr::JoinedStr(js) => {
                                let mut t = String::new();
                                for v in &js.values {
                                    if let Expr::Constant(c) = v {
                                        if let Constant::Str(s) = &c.value {
                                            t.push_str(s);
                                        }
                                    }
                                }
                                if t.is_empty() { None } else { Some(t) }
                            }
                            _ => None,
                        }
                    });
                    if let Some(st) = &spec_text {
                        new_fmt.push('%');
                        new_fmt.push_str(st);
                    } else {
                        new_fmt.push('%');
                        new_fmt.push(spec_char);
                    }
                    let es = text_size_to_usize(fv.value.range().start());
                    let ee = text_size_to_usize(fv.value.range().end());
                    extra_args.push(source[es..ee].to_string());
                }
                _ => {}
            }
        }

        new_fmt.push('"');

        let replacement = if extra_args.is_empty() {
            new_fmt
        } else {
            new_fmt.push_str(", ");
            new_fmt.push_str(&extra_args.join(", "));
            new_fmt
        };

        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB129".to_string(),
            message: "Use lazy '%s' formatting instead of f-string in logging call".to_string(),
            fix: Some(Fix { start, end, replacement }),
        });
    }
}

// ── RAB077: assert on a non-empty tuple literal (always true) ─────────
// `assert (cond, "msg")` is always truthy because a non-empty tuple is
// truthy; the author almost certainly meant `assert cond, "msg"`.

pub struct AssertTupleChecker;

impl Checker for AssertTupleChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
    fn visit_stmt(&mut self, stmt: &Stmt, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Stmt::Assert(a) = stmt else { return };
        let Expr::Tuple(t) = &*a.test else { return };
        if t.elts.is_empty() { return; }

        let range = a.test.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);

        // Offer a fix only for the common `assert (cond, msg)` shape: exactly
        // two elements, no existing message, and a parenthesised tuple source.
        let fix = if t.elts.len() == 2 && a.msg.is_none() {
            let tsrc = &source[start..end];
            if tsrc.starts_with('(') && tsrc.ends_with(')') {
                let cond = expr_to_source(source, &t.elts[0]);
                let msg = expr_to_source(source, &t.elts[1]);
                Some(Fix { start, end, replacement: format!("{}, {}", cond, msg) })
            } else {
                None
            }
        } else {
            None
        };

        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB077".to_string(),
            message: "Assertion on a tuple literal is always true; drop the parentheses (`assert cond, msg`)".to_string(),
            fix,
        });
    }
}

// ── RAB081: return/break/continue inside a finally block ──────────────
// Control flow that leaves a `finally` swallows any in-flight exception
// and discards a pending return from the try/except body.

pub struct FinallyControlFlowChecker;

impl Checker for FinallyControlFlowChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let finalbody = match stmt {
            Stmt::Try(t) => &t.finalbody,
            Stmt::TryStar(t) => &t.finalbody,
            _ => return,
        };
        // Only the top level of the finally matters: a break/continue that
        // belongs to a loop nested inside the finally is local and fine.
        for s in finalbody {
            let kw = match s {
                Stmt::Return(_) => "return",
                Stmt::Break(_) => "break",
                Stmt::Continue(_) => "continue",
                _ => continue,
            };
            let range = s.range();
            let start = text_size_to_usize(range.start());
            let end = text_size_to_usize(range.end());
            let (line, col) = byte_to_line_col(start, line_starts);
            let (end_line, end_col) = byte_to_line_col(end, line_starts);
            findings.push(Finding {
                line, col, end_line, end_col,
                code: "RAB081".to_string(),
                message: format!("'{}' inside 'finally' swallows pending exceptions and returns", kw),
                fix: None,
            });
        }
    }
}

// ── RAB082: except BaseException is too broad ─────────────────────────
// `BaseException` also catches `SystemExit` and `KeyboardInterrupt`,
// preventing clean shutdown; catch `Exception` instead.

pub struct BaseExceptionChecker;

impl Checker for BaseExceptionChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let handlers = match stmt {
            Stmt::Try(t) => &t.handlers,
            Stmt::TryStar(t) => &t.handlers,
            _ => return,
        };
        for handler in handlers {
            let ExceptHandler::ExceptHandler(h) = handler;
            let Some(t) = &h.type_ else { continue };
            let Expr::Name(n) = &**t else { continue };
            if n.id.as_str() != "BaseException" { continue; }
            let range = t.range();
            let start = text_size_to_usize(range.start());
            let end = text_size_to_usize(range.end());
            let (line, col) = byte_to_line_col(start, line_starts);
            let (end_line, end_col) = byte_to_line_col(end, line_starts);
            findings.push(Finding {
                line, col, end_line, end_col,
                code: "RAB082".to_string(),
                message: "'except BaseException' also catches SystemExit/KeyboardInterrupt; catch 'Exception' instead".to_string(),
                fix: Some(Fix { start, end, replacement: "Exception".to_string() }),
            });
        }
    }
}

// ── RAB084: bare `raise` outside an except block ──────────────────────
// A bare `raise` re-raises the active exception; with none active it
// raises `RuntimeError: No active exception to re-raise`.

pub struct BareRaiseChecker {
    except_depth: usize,
}

impl BareRaiseChecker {
    pub fn new() -> Self { Self { except_depth: 0 } }
}

impl Checker for BareRaiseChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
    // A function/class defined inside an except handler runs in a new frame
    // with no active exception, so reset the counter at each scope boundary.
    fn enter_scope(&mut self) { self.except_depth = 0; }
    fn enter_except(&mut self) { self.except_depth += 1; }
    fn exit_except(&mut self) { self.except_depth = self.except_depth.saturating_sub(1); }

    fn visit_stmt(&mut self, stmt: &Stmt, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Stmt::Raise(r) = stmt else { return };
        if r.exc.is_some() { return; } // not a bare raise
        if self.except_depth > 0 { return; }
        let range = stmt.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB084".to_string(),
            message: "Bare 'raise' outside an 'except' block raises RuntimeError (no active exception)".to_string(),
            fix: None,
        });
    }
}

// ── RAB086: x == a or x == b ... → x in (a, b, ...) ───────────────────

pub struct EqualityOrChainChecker;

fn is_simple_ref(expr: &Expr) -> bool {
    matches!(expr, Expr::Name(_) | Expr::Attribute(_) | Expr::Subscript(_))
}

impl Checker for EqualityOrChainChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
    fn visit_expr(&mut self, expr: &Expr, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Expr::BoolOp(b) = expr else { return };
        if !matches!(b.op, BoolOp::Or) { return; }
        if b.values.len() < 2 { return; }

        let mut left_src: Option<String> = None;
        let mut rights: Vec<String> = Vec::with_capacity(b.values.len());
        for v in &b.values {
            let Expr::Compare(c) = v else { return };
            if c.ops.len() != 1 || !matches!(c.ops[0], CmpOp::Eq) { return; }
            if !is_simple_ref(&c.left) { return; }
            let ls = expr_to_source(source, &c.left);
            match &left_src {
                None => left_src = Some(ls),
                Some(prev) if *prev == ls => {}
                _ => return, // operands differ → not a single-variable chain
            }
            rights.push(expr_to_source(source, &c.comparators[0]));
        }
        let Some(left) = left_src else { return };

        let range = b.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        let replacement = format!("{} in ({})", left, rights.join(", "));
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB086".to_string(),
            message: format!("Use '{} in (...)' instead of repeated '==' with 'or'", left),
            fix: Some(Fix { start, end, replacement }),
        });
    }
}

// ── RAB130: len([... for ...]) → sum(1 for ...) ───────────────────────
// Building a throwaway list just to count it wastes time and memory.
// Restricted to list comprehensions: len() of a *set* comprehension
// counts distinct items, so `sum(1 for ...)` would not be equivalent.

pub struct LenComprehensionChecker;

impl Checker for LenComprehensionChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
    fn visit_expr(&mut self, expr: &Expr, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Expr::Call(call) = expr else { return };
        let Expr::Name(n) = &*call.func else { return };
        if n.id.as_str() != "len" || call.args.len() != 1 || !call.keywords.is_empty() { return; }
        let Expr::ListComp(lc) = &call.args[0] else { return };

        let crange = call.range();
        let start = text_size_to_usize(crange.start());
        let end = text_size_to_usize(crange.end());

        // Reconstruct the generator part: everything between the element
        // expression and the closing bracket of the comprehension.
        let lc_range = lc.range();
        let lc_end = text_size_to_usize(lc_range.end());
        let elt_end = text_size_to_usize(lc.elt.range().end());
        let rest = &source[elt_end..lc_end - 1]; // " for x in y if ..." (drops the ']')

        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB130".to_string(),
            message: "Use 'sum(1 for ...)' instead of 'len([... for ...])' to avoid building a throwaway list".to_string(),
            fix: Some(Fix { start, end, replacement: format!("sum(1{})", rest) }),
        });
    }
}

// ── RAB131: constructor([... for ...]) → constructor(... for ...) ──────
// set()/tuple()/frozenset()/sorted()/dict() accept any iterable, so the
// intermediate list from a comprehension is pure overhead.

pub struct GenExprConstructorChecker;

impl Checker for GenExprConstructorChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
    fn visit_expr(&mut self, expr: &Expr, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Expr::Call(call) = expr else { return };
        let Expr::Name(n) = &*call.func else { return };
        let func = n.id.as_str();
        if !matches!(func, "set" | "tuple" | "frozenset" | "sorted" | "dict") { return; }
        let Some(Expr::ListComp(lc)) = call.args.first() else { return };

        let lc_range = lc.range();
        let lc_start = text_size_to_usize(lc_range.start());
        let lc_end = text_size_to_usize(lc_range.end());
        let inner = &source[lc_start + 1..lc_end - 1]; // strip [ ]

        let (line, col) = byte_to_line_col(lc_start, line_starts);
        let (end_line, end_col) = byte_to_line_col(lc_end, line_starts);
        // Parenthesise so it stays valid when the call has extra args
        // (e.g. `sorted([...], key=f)` → `sorted((... ), key=f)`).
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB131".to_string(),
            message: format!("Pass a generator to {}() instead of a list comprehension to skip the intermediate list", func),
            fix: Some(Fix { start: lc_start, end: lc_end, replacement: format!("({})", inner) }),
        });
    }
}

// ── RAB132: x = x + [..] inside a loop → x.append/extend ──────────────
// Rebinding with `+` builds a new list each iteration: O(n^2) overall.

pub struct ListConcatInLoopChecker;

fn self_list_concat<'a>(a: &'a StmtAssign) -> Option<(&'a str, &'a ExprList)> {
    if a.targets.len() != 1 { return None; }
    let Expr::Name(t) = &a.targets[0] else { return None };
    let Expr::BinOp(b) = &*a.value else { return None };
    if !matches!(b.op, Operator::Add) { return None; }
    let Expr::Name(l) = &*b.left else { return None };
    if l.id.as_str() != t.id.as_str() { return None; }
    let Expr::List(list) = &*b.right else { return None };
    Some((t.id.as_str(), list))
}

impl ListConcatInLoopChecker {
    fn scan(stmts: &[Stmt], source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        for s in stmts {
            match s {
                Stmt::Assign(a) => {
                    if let Some((name, list)) = self_list_concat(a) {
                        let range = s.range();
                        let start = text_size_to_usize(range.start());
                        let end = text_size_to_usize(range.end());
                        let (line, col) = byte_to_line_col(start, line_starts);
                        let (end_line, end_col) = byte_to_line_col(end, line_starts);
                        let replacement = if list.elts.len() == 1 {
                            format!("{}.append({})", name, expr_to_source(source, &list.elts[0]))
                        } else {
                            let lr = list.range();
                            let list_src = &source[text_size_to_usize(lr.start())..text_size_to_usize(lr.end())];
                            format!("{}.extend({})", name, list_src)
                        };
                        findings.push(Finding {
                            line, col, end_line, end_col,
                            code: "RAB132".to_string(),
                            message: format!("'{0} = {0} + [...]' in a loop is O(n^2); use '{0}.append()' / '{0}.extend()'", name),
                            fix: Some(Fix { start, end, replacement }),
                        });
                    }
                }
                // Descend into conditionals/with/try, but not nested loops,
                // defs, or classes — those report (or scope) on their own.
                Stmt::If(i) => { Self::scan(&i.body, source, line_starts, findings); Self::scan(&i.orelse, source, line_starts, findings); }
                Stmt::With(w) => Self::scan(&w.body, source, line_starts, findings),
                Stmt::AsyncWith(w) => Self::scan(&w.body, source, line_starts, findings),
                Stmt::Try(t) => {
                    Self::scan(&t.body, source, line_starts, findings);
                    for h in &t.handlers { let ExceptHandler::ExceptHandler(eh) = h; Self::scan(&eh.body, source, line_starts, findings); }
                    Self::scan(&t.orelse, source, line_starts, findings);
                    Self::scan(&t.finalbody, source, line_starts, findings);
                }
                _ => {}
            }
        }
    }
}

impl Checker for ListConcatInLoopChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
    fn visit_stmt(&mut self, stmt: &Stmt, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let body = match stmt {
            Stmt::For(f) => &f.body,
            Stmt::AsyncFor(f) => &f.body,
            Stmt::While(w) => &w.body,
            _ => return,
        };
        Self::scan(body, source, line_starts, findings);
    }
}

// ── RAB133: list.pop(0) / list.insert(0, x) → collections.deque ───────
// Popping/inserting at the front of a list is O(n); a deque does it in
// O(1) via popleft()/appendleft().

pub struct DequeChecker;

impl Checker for DequeChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
    fn visit_expr(&mut self, expr: &Expr, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Expr::Call(call) = expr else { return };
        let Expr::Attribute(attr) = &*call.func else { return };
        let (method, ok) = match attr.attr.as_str() {
            "pop" => ("pop(0)", call.args.len() == 1 && is_literal_zero(&call.args[0])),
            "insert" => ("insert(0, ...)", call.args.len() >= 1 && is_literal_zero(&call.args[0])),
            _ => return,
        };
        if !ok { return; }
        let range = call.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB133".to_string(),
            message: format!("'{}' on a list is O(n); use 'collections.deque' (popleft/appendleft) for O(1)", method),
            fix: None,
        });
    }
}

// ── RAB134: sorted(x)[:k] / sorted(x)[-k:] → heapq.nsmallest/nlargest ──
// A full sort is O(n log n); a partial selection is O(n log k).

pub struct PartialSortChecker;

impl Checker for PartialSortChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Expr }
    fn visit_expr(&mut self, expr: &Expr, _source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let Expr::Subscript(sub) = expr else { return };
        let Expr::Call(call) = &*sub.value else { return };
        let Expr::Name(n) = &*call.func else { return };
        if n.id.as_str() != "sorted" { return; }
        let Expr::Slice(sl) = &*sub.slice else { return };
        if sl.step.is_some() { return; }

        let suggestion = if sl.lower.is_none() && sl.upper.is_some() {
            "heapq.nsmallest(k, x)" // sorted(x)[:k]
        } else if sl.lower.is_some() && sl.upper.is_none() {
            "heapq.nlargest(k, x)"  // sorted(x)[-k:]
        } else {
            return;
        };

        let range = sub.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB134".to_string(),
            message: format!("Slicing a full sort is O(n log n); use '{}' for an O(n log k) partial sort", suggestion),
            fix: None,
        });
    }
}

// ── RAB135: for ... in list(range(...)) → iterate range directly ──────

pub struct ListRangeLoopChecker;

impl Checker for ListRangeLoopChecker {
    fn node_kind(&self) -> crate::analyze::NodeKind { crate::analyze::NodeKind::Stmt }
    fn visit_stmt(&mut self, stmt: &Stmt, source: &str, line_starts: &[usize], findings: &mut Vec<Finding>) {
        let iter = match stmt {
            Stmt::For(f) => &f.iter,
            Stmt::AsyncFor(f) => &f.iter,
            _ => return,
        };
        let Expr::Call(outer) = &**iter else { return };
        let Expr::Name(on) = &*outer.func else { return };
        if on.id.as_str() != "list" || outer.args.len() != 1 { return; }
        let Expr::Call(inner) = &outer.args[0] else { return };
        let Expr::Name(inner_name) = &*inner.func else { return };
        if inner_name.id.as_str() != "range" { return; }

        let range = outer.range();
        let start = text_size_to_usize(range.start());
        let end = text_size_to_usize(range.end());
        let (line, col) = byte_to_line_col(start, line_starts);
        let (end_line, end_col) = byte_to_line_col(end, line_starts);
        let replacement = expr_to_source(source, &outer.args[0]);
        findings.push(Finding {
            line, col, end_line, end_col,
            code: "RAB135".to_string(),
            message: "Iterate over 'range(...)' directly instead of 'list(range(...))' to avoid materializing the list".to_string(),
            fix: Some(Fix { start, end, replacement }),
        });
    }
}

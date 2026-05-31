use rustpython_ast::*;
use rustpython_parser::{ast, Parse};

pub const CHECK_CODES: &[&str] = &[
    "RAB001", "RAB002", "RAB003", "RAB004", "RAB005", "RAB006", "RAB007",
    "RAB008", "RAB009", "RAB010", "RAB011", "RAB012", "RAB013", "RAB014",
    "RAB015", "RAB016", "RAB017", "RAB018", "RAB019", "RAB020", "RAB022",
    "RAB023", "RAB024", "RAB025", "RAB026",
    "RAB029", "RAB030", "RAB031", "RAB032",
    "RAB034", "RAB035", "RAB036", "RAB037", "RAB038", "RAB039",
    "RAB040", "RAB041", "RAB042", "RAB043", "RAB044", "RAB045",
    "RAB046", "RAB047", "RAB048", "RAB049",
    "RAB050", "RAB051", "RAB052",
    "RAB053",
    "RAB054", "RAB055", "RAB056", "RAB057", "RAB058", "RAB059", "RAB060",
    "RAB061", "RAB062", "RAB063", "RAB064", "RAB065", "RAB066", "RAB067", "RAB068",
    "RAB069", "RAB070", "RAB071", "RAB072", "RAB073", "RAB074", "RAB075",
    "RAB076", "RAB077", "RAB078", "RAB079", "RAB080", "RAB081", "RAB082",
    "RAB083", "RAB084", "RAB085", "RAB086", "RAB087", "RAB088", "RAB089",
    "RAB101", "RAB102",
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

pub fn iter_fn_args(args: &Arguments) -> impl Iterator<Item = &ArgWithDefault> {
    args.posonlyargs.iter()
        .chain(args.args.iter())
        .chain(args.kwonlyargs.iter())
}

pub fn count_fn_args(args: &Arguments) -> usize {
    args.posonlyargs.len() + args.args.len() + args.kwonlyargs.len()
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
    fn enter_block(&mut self) {}
    fn exit_block(&mut self) {}
    fn enter_except(&mut self) {}
    fn exit_except(&mut self) {}
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
    let mut mutable_default = crate::checks::MutableDefaultChecker;
    let mut bare_except = crate::checks::BareExceptChecker;
    let mut bare_except_pass = crate::checks::BareExceptPassChecker;
    let mut class_object = crate::checks::ClassObjectChecker;
    let mut format_call = crate::checks::FormatCallChecker;
    let mut func_length = crate::checks::FunctionLengthChecker::new();
    let mut too_many_params = crate::checks::TooManyParamsChecker;
    let mut os_system = crate::checks::OsSystemChecker;
    let mut time_time = crate::checks::TimeTimeChecker;
    let mut missing_hint = crate::checks::MissingReturnHintChecker;
    let mut deep_comp = crate::checks::DeepComprehensionChecker;
    let mut long_if = crate::checks::LongIfChainChecker;
    let mut cognitive = crate::checks::CognitiveComplexityChecker::new();
    let mut sorted_list = crate::checks::SortedListChecker;
    let mut dict_get = crate::checks::DictGetChecker;
    let mut slice_copy = crate::checks::SliceCopyChecker;
    let mut native_generic = crate::checks::NativeGenericChecker;
    let mut manual_list = crate::checks::ManualListChecker::new();
    let mut open_ctx = crate::checks::OpenContextChecker;
    let mut sorted_idx0 = crate::checks::SortedIndex0Checker;
    let mut sorted_idxn1 = crate::checks::SortedIndexNeg1Checker;
    let mut not_is_none = crate::checks::NotIsNoneChecker;
    let mut aug_assign = crate::checks::AugmentedAssignChecker;
    let mut is_true = crate::checks::IsTrueChecker;
    let mut range_len = crate::checks::RangeLenChecker;
    let mut setdefault = crate::checks::SetdefaultChecker;
    let mut type_is = crate::checks::TypeIsChecker;
    let mut if_not_assign = crate::checks::IfNotAssignChecker;
    let mut unused_loop_var = crate::checks::UnusedLoopVarChecker;
    let mut nested_with = crate::checks::NestedWithChecker;
    let mut startswith_or = crate::checks::StartswithOrChecker;
    let mut return_ternary = crate::checks::ReturnTernaryChecker;
    let mut inf_while = crate::checks::InfiniteWhileChecker;
    let mut sorted_sort = crate::checks::SortedSortChecker;
    let mut empty_coll = crate::checks::EmptyCollectionChecker;
    let mut empty_cmp = crate::checks::EmptyCompareChecker;
    let mut join_lc = crate::checks::JoinListCompChecker;
    let mut dead_except = crate::checks::DeadExceptChecker;
    let mut percent_fmt = crate::checks::PercentFormatChecker;
    let mut os_path = crate::checks::OsPathChecker;
    let mut single_isinstance = crate::checks::SingleTypeIsinstanceChecker;
    let mut redundant_str = crate::checks::RedundantStrChecker;
    let mut except_pass = crate::checks::ExceptPassChecker;
    let mut del_method = crate::checks::DelMethodChecker;
    let mut list_keys = crate::checks::ListKeysChecker;
    let mut nested_ternary = crate::checks::NestedTernaryChecker;
    let mut reversed_sorted = crate::checks::ReversedSortedChecker;
    let mut dict_zip = crate::checks::DictZipChecker;
    let mut while_len = crate::checks::WhileLenChecker;
    let mut copy_copy = crate::checks::CopyCopyChecker;
    let mut wildcard_import = crate::checks::WildcardImportChecker;
    let mut redundant_pass = crate::checks::RedundantPassChecker;
    let mut is_literal = crate::checks::IsLiteralChecker;
    let mut init_return = crate::checks::InitReturnChecker;
    let mut dead_code = crate::checks::DeadCodeChecker;
    let mut def_in_loop = crate::checks::DefInLoopChecker;
    let mut builtin_shadow = crate::checks::BuiltinShadowChecker;
    let mut raise_without_from = crate::checks::RaiseWithoutFromChecker::new();
    let mut power_opt = crate::checks::PowerOptChecker;
    let mut map_lambda = crate::checks::MapLambdaChecker;
    let mut list_to_set = crate::checks::ListToSetChecker;
    let mut dataclass_slots = crate::checks::DataclassSlotsChecker;
    let mut re_compile = crate::checks::ReCompileChecker::new();
    let mut readlines = crate::checks::ReadlinesChecker;
    let mut type_union = crate::checks::TypeUnionChecker;

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
        &mut mutable_default,
        &mut bare_except,
        &mut bare_except_pass,
        &mut class_object,
        &mut format_call,
        &mut func_length,
        &mut too_many_params,
        &mut os_system,
        &mut time_time,
        &mut missing_hint,
        &mut deep_comp,
        &mut long_if,
        &mut cognitive,
        &mut sorted_list,
        &mut dict_get,
        &mut slice_copy,
        &mut native_generic,
        &mut manual_list,
        &mut open_ctx,
        &mut sorted_idx0,
        &mut sorted_idxn1,
        &mut not_is_none,
        &mut aug_assign,
        &mut is_true,
        &mut range_len,
        &mut setdefault,
        &mut type_is,
        &mut if_not_assign,
        &mut unused_loop_var,
        &mut nested_with,
        &mut startswith_or,
        &mut return_ternary,
        &mut inf_while,
        &mut sorted_sort,
        &mut empty_coll,
        &mut empty_cmp,
        &mut join_lc,
        &mut dead_except,
        &mut percent_fmt,
        &mut os_path,
        &mut single_isinstance,
        &mut redundant_str,
        &mut except_pass,
        &mut del_method,
        &mut list_keys,
        &mut nested_ternary,
        &mut reversed_sorted,
        &mut dict_zip,
        &mut while_len,
        &mut copy_copy,
        &mut wildcard_import,
        &mut redundant_pass,
        &mut is_literal,
        &mut init_return,
        &mut dead_code,
        &mut def_in_loop,
        &mut builtin_shadow,
        &mut raise_without_from,
        &mut power_opt,
        &mut map_lambda,
        &mut list_to_set,
        &mut dataclass_slots,
        &mut re_compile,
        &mut readlines,
        &mut type_union,
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
                walk_expr(&a.annotation, source, line_starts, checkers, findings);
                walk_expr_opt(a.value.as_deref(), source, line_starts, checkers, findings);
            }
            Stmt::For(f) => {
                walk_expr(&f.target, source, line_starts, checkers, findings);
                walk_expr(&f.iter, source, line_starts, checkers, findings);
                for c in checkers.iter_mut() { c.enter_block(); }
                walk_stmts(&f.body, source, line_starts, checkers, findings);
                for c in checkers.iter_mut() { c.exit_block(); }
                walk_stmts(&f.orelse, source, line_starts, checkers, findings);
            }
            Stmt::AsyncFor(f) => {
                walk_expr(&f.target, source, line_starts, checkers, findings);
                walk_expr(&f.iter, source, line_starts, checkers, findings);
                for c in checkers.iter_mut() { c.enter_block(); }
                walk_stmts(&f.body, source, line_starts, checkers, findings);
                for c in checkers.iter_mut() { c.exit_block(); }
                walk_stmts(&f.orelse, source, line_starts, checkers, findings);
            }
            Stmt::While(w) => {
                walk_expr(&w.test, source, line_starts, checkers, findings);
                for c in checkers.iter_mut() { c.enter_block(); }
                walk_stmts(&w.body, source, line_starts, checkers, findings);
                for c in checkers.iter_mut() { c.exit_block(); }
                walk_stmts(&w.orelse, source, line_starts, checkers, findings);
            }
            Stmt::If(i) => {
                walk_expr(&i.test, source, line_starts, checkers, findings);
                for c in checkers.iter_mut() { c.enter_block(); }
                walk_stmts(&i.body, source, line_starts, checkers, findings);
                for c in checkers.iter_mut() { c.exit_block(); }
                walk_stmts(&i.orelse, source, line_starts, checkers, findings);
            }
            Stmt::With(w) => {
                for item in &w.items {
                    walk_expr(&item.context_expr, source, line_starts, checkers, findings);
                    walk_expr_opt(item.optional_vars.as_deref(), source, line_starts, checkers, findings);
                }
                for c in checkers.iter_mut() { c.enter_block(); }
                walk_stmts(&w.body, source, line_starts, checkers, findings);
                for c in checkers.iter_mut() { c.exit_block(); }
            }
            Stmt::AsyncWith(w) => {
                for item in &w.items {
                    walk_expr(&item.context_expr, source, line_starts, checkers, findings);
                    walk_expr_opt(item.optional_vars.as_deref(), source, line_starts, checkers, findings);
                }
                for c in checkers.iter_mut() { c.enter_block(); }
                walk_stmts(&w.body, source, line_starts, checkers, findings);
                for c in checkers.iter_mut() { c.exit_block(); }
            }
            Stmt::Raise(r) => {
                walk_expr_opt(r.exc.as_deref(), source, line_starts, checkers, findings);
                walk_expr_opt(r.cause.as_deref(), source, line_starts, checkers, findings);
            }
            Stmt::Try(t) => {
                for c in checkers.iter_mut() { c.enter_block(); }
                walk_stmts(&t.body, source, line_starts, checkers, findings);
                for c in checkers.iter_mut() { c.exit_block(); }
                for handler in &t.handlers {
                    let ExceptHandler::ExceptHandler(h) = handler;
                    for c in checkers.iter_mut() { c.enter_except(); }
                    walk_stmts(&h.body, source, line_starts, checkers, findings);
                    for c in checkers.iter_mut() { c.exit_except(); }
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
                    for c in checkers.iter_mut() { c.enter_block(); }
                    walk_stmts(&case.body, source, line_starts, checkers, findings);
                    for c in checkers.iter_mut() { c.exit_block(); }
                }
            }
            Stmt::TryStar(t) => {
                for c in checkers.iter_mut() { c.enter_block(); }
                walk_stmts(&t.body, source, line_starts, checkers, findings);
                for c in checkers.iter_mut() { c.exit_block(); }
                for handler in &t.handlers {
                    let ExceptHandler::ExceptHandler(h) = handler;
                    for c in checkers.iter_mut() { c.enter_except(); }
                    walk_stmts(&h.body, source, line_starts, checkers, findings);
                    for c in checkers.iter_mut() { c.exit_except(); }
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

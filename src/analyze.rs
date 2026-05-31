use rustpython_ast::TextSize;
use rustpython_parser::{ast, Parse};

pub const CHECK_CODES: &[&str] = &[
    "RAB001", "RAB002", "RAB003", "RAB004", "RAB005", "RAB006", "RAB007",
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

pub fn analyze_source(source: &str) -> Vec<Finding> {
    let Ok(ast) = ast::Suite::parse(source, "<source>") else {
        return vec![];
    };

    let line_starts: Vec<usize> = std::iter::once(0)
        .chain(source.chars().enumerate().filter_map(|(i, c)| {
            if c == '\n' {
                Some(i + 1)
            } else {
                None
            }
        }))
        .collect();

    let mut findings = Vec::new();

    findings.extend(crate::checks::check_unused_variables(source, &line_starts, &ast));
    findings.extend(crate::checks::check_none_comparison(source, &line_starts, &ast));
    findings.extend(crate::checks::check_len_zero(source, &line_starts, &ast));
    findings.extend(crate::checks::check_list_to_generator(source, &line_starts, &ast));
    findings.extend(crate::checks::check_dict_keys_loop(source, &line_starts, &ast));
    findings.extend(crate::checks::check_type_comparison(source, &line_starts, &ast));
    findings.extend(crate::checks::check_unnecessary_else(source, &line_starts, &ast));

    findings.sort_by_key(|f| (f.line, f.col));

    findings
}

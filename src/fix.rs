use crate::analyze::Fix;

pub fn apply_fixes(source: &str, fixes: &[&Fix]) -> String {
    let mut sorted_fixes: Vec<&Fix> = fixes.iter().copied().collect();
    sorted_fixes.sort_by(|a, b| b.start.cmp(&a.start));

    let mut result = source.to_string();
    for fix in &sorted_fixes {
        result.replace_range(fix.start..fix.end, &fix.replacement);
    }
    result
}

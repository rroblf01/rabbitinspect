use crate::analyze::Fix;

pub fn apply_fixes(source: &str, fixes: &[&Fix]) -> String {
    if fixes.is_empty() {
        return source.to_string();
    }
    let mut sorted: Vec<&Fix> = fixes.iter().copied().collect();
    sorted.sort_by(|a, b| a.start.cmp(&b.start).then_with(|| b.end.cmp(&a.end)));

    let mut result = String::with_capacity(source.len());
    let mut pos = 0usize;
    for fix in &sorted {
        // Skip any fix that overlaps an already-applied one.
        if fix.start < pos {
            continue;
        }
        result.push_str(&source[pos..fix.start]);
        result.push_str(&fix.replacement);
        pos = fix.end;
    }
    result.push_str(&source[pos..]);
    result
}

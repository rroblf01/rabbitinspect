use crate::analyze::Fix;

pub fn apply_fix(source: &str, fix: &Fix) -> String {
    let mut bytes: Vec<u8> = source.bytes().collect();
    let replacement = fix.replacement.as_bytes();
    // Drain the range and insert replacement
    bytes.splice(fix.start..fix.end, replacement.iter().copied());
    String::from_utf8(bytes).unwrap_or_else(|_| source.to_string())
}

pub fn apply_fixes(source: &str, fixes: &[&Fix]) -> String {
    // Apply fixes in reverse order (by start position) so positions remain valid
    let mut sorted_fixes: Vec<&Fix> = fixes.iter().copied().collect();
    sorted_fixes.sort_by(|a, b| b.start.cmp(&a.start));

    let mut result = source.to_string();
    for fix in &sorted_fixes {
        let mut bytes: Vec<u8> = result.bytes().collect();
        let replacement = fix.replacement.as_bytes();
        bytes.splice(fix.start..fix.end, replacement.iter().copied());
        result = String::from_utf8(bytes).unwrap_or_else(|_| result.clone());
    }
    result
}

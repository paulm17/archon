/// Generate a canonical ID in the format "{prefix}::{part1}::{part2}..."
/// Empty or None parts are skipped.
pub fn make_id(prefix: &str, parts: &[&str]) -> String {
    let joined: Vec<&str> = parts.iter().copied().filter(|p| !p.is_empty()).collect();
    format!("{}::{}", prefix, joined.join("::"))
}

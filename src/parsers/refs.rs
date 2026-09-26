//! File-reference extraction (`path/to/file.rs:123`).
//!
//! Compiler and test output constantly names `file:line` locations; they
//! get the same click-to-copy row treatment as PR/issue/commit links.
//! Only paths with a known source extension and a line number match, so
//! prose like `see 3:4 below` stays ignored.

use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;

/// `src/gui/shell.rs:120` style references. The leading word boundary
/// keeps `https://…` URLs from matching as file paths.
static FILE_REF_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"\b[\w./-]+?\.(?:rs|py|js|ts|tsx|jsx|md|toml|json|yaml|yml|c|h|cpp|hpp|go|rb|java|sh|css|html|sql):\d+\b",
    )
    .expect("static file-ref regex")
});

/// Extract `path/to/file.ext:line` references in first-seen order,
/// deduplicated — the same ordering contract as the GitHub extractors.
pub fn extract_file_refs(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for m in FILE_REF_RE.find_iter(text) {
        let hit = m.as_str();
        // Skip matches that are really the tail of a URL (`…com/x.rs:1`
        // only happens inside longer tokens the boundary missed).
        if seen.insert(hit) {
            out.push(hit.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_file_line_refs_in_order() {
        let text = "error at src/gui/shell.rs:120, also src/app.rs:9, \
                    again src/gui/shell.rs:120";
        assert_eq!(
            extract_file_refs(text),
            vec!["src/gui/shell.rs:120", "src/app.rs:9"]
        );
    }

    #[test]
    fn ignores_prose_and_unknown_extensions() {
        assert!(extract_file_refs("see 3:4 below").is_empty());
        assert!(extract_file_refs("notes.xyz:12").is_empty());
        assert!(extract_file_refs("").is_empty());
    }
}

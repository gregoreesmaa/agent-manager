//! Per-chat GitHub PR link extraction.

use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;

/// Compiled once: per-run-per-tick compilation was the hottest alloc in
/// the pump loop. Match semantics are unchanged (see [`extract_pr_links`]).
static PR_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https?://github\.com/[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+/pull/\d+")
        .expect("static PR regex")
});

/// Extract `https://github.com/<owner>/<repo>/pull/<n>` URLs (also bare
/// `owner/repo#123` references are ignored — only full PR URLs count).
/// Preserves first-seen order, deduplicated, trailing punctuation trimmed.
pub fn extract_pr_links(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for m in PR_RE.find_iter(text) {
        let url = m.as_str();
        if seen.insert(url) {
            out.push(url.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_and_dedupes_pull_urls() {
        let text = "see https://github.com/acme/app/pull/42, also \
                    https://github.com/acme/app/pull/42 and \
                    https://github.com/other/repo/pull/7.";
        assert_eq!(
            extract_pr_links(text),
            vec![
                "https://github.com/acme/app/pull/42",
                "https://github.com/other/repo/pull/7"
            ]
        );
    }

    #[test]
    fn ignores_issue_links_and_bare_refs() {
        assert!(extract_pr_links("acme/app#123 https://github.com/a/b/issues/5").is_empty());
    }

    #[test]
    fn empty_in_empty_out() {
        assert!(extract_pr_links("").is_empty());
    }
}

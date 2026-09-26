//! Per-chat GitHub PR link extraction.

use std::sync::LazyLock;

use regex::Regex;

/// PR-URL pattern, compiled once: `extract_pr_links` runs per run per tick,
/// so a per-call `Regex::new` turned every idle tick into a compile.
static PR_LINK_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https?://github\.com/[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+/pull/\d+")
        .expect("static PR link regex")
});

/// Extract `https://github.com/<owner>/<repo>/pull/<n>` URLs (also bare
/// `owner/repo#123` references are ignored — only full PR URLs count).
/// Preserves first-seen order, deduplicated, trailing punctuation trimmed.
pub fn extract_pr_links(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for m in PR_LINK_RE.find_iter(text) {
        let url = m.as_str().to_string();
        if !out.contains(&url) {
            out.push(url);
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

    #[test]
    fn repeated_calls_share_the_static_pattern() {
        // Pins the LazyLock hoist (issue #12): repeat extraction over fresh
        // inputs, including a 400-cell-wide screen line, and require stable
        // first-seen order every time.
        let wide = format!(
            "x{} https://github.com/acme/app/pull/9 y{}",
            " ".repeat(400),
            " ".repeat(400)
        );
        let first = extract_pr_links(&wide);
        assert_eq!(first, vec!["https://github.com/acme/app/pull/9"]);
        for _ in 0..10 {
            assert_eq!(extract_pr_links(&wide), first);
            assert_eq!(
                extract_pr_links("https://github.com/a/b/pull/1 https://github.com/a/b/pull/1"),
                vec!["https://github.com/a/b/pull/1"]
            );
        }
    }
}

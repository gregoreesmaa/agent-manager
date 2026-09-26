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

/// Full issue URLs share the PR row treatment (click-to-copy under the
/// run title); bare `owner/repo#123` refs stay ignored, as with PRs.
static ISSUE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https?://github\.com/[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+/issues/\d+")
        .expect("static issue regex")
});

/// Full commit URLs (`/commit/<sha>`), same row treatment.
static COMMIT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https?://github\.com/[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+/commit/[0-9a-f]{7,40}")
        .expect("static commit regex")
});

/// Extract `https://github.com/<owner>/<repo>/pull/<n>` URLs (also bare
/// `owner/repo#123` references are ignored — only full PR URLs count).
/// Preserves first-seen order, deduplicated, trailing punctuation trimmed.
pub fn extract_pr_links(text: &str) -> Vec<String> {
    collect_unique(&PR_RE, text)
}

/// Extract `https://github.com/<owner>/<repo>/issues/<n>` URLs.
/// Same ordering and dedupe contract as [`extract_pr_links`].
pub fn extract_issue_links(text: &str) -> Vec<String> {
    collect_unique(&ISSUE_RE, text)
}

/// Extract `https://github.com/<owner>/<repo>/commit/<sha>` URLs.
/// Same ordering and dedupe contract as [`extract_pr_links`].
pub fn extract_commit_links(text: &str) -> Vec<String> {
    collect_unique(&COMMIT_RE, text)
}

fn collect_unique(re: &Regex, text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for m in re.find_iter(text) {
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
    fn extracts_issue_and_commit_urls_with_pr_ordering() {
        let text = "bug https://github.com/acme/app/issues/9, \
                    fix https://github.com/acme/app/commit/abcdef1234567890, \
                    again https://github.com/acme/app/issues/9";
        assert_eq!(
            extract_issue_links(text),
            vec!["https://github.com/acme/app/issues/9"]
        );
        assert_eq!(
            extract_commit_links(text),
            vec!["https://github.com/acme/app/commit/abcdef1234567890"]
        );
        // PR extraction stays blind to the new kinds.
        assert!(extract_pr_links(text).is_empty());
        // Short SHAs are not commits.
        assert!(extract_commit_links("https://github.com/a/b/commit/abc").is_empty());
    }

    #[test]
    fn empty_in_empty_out() {
        assert!(extract_pr_links("").is_empty());
    }
}

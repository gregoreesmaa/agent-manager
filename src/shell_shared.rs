//! Shared native-shell helpers (Windows + macOS parity).
//!
//! Framework-free presentation logic the per-OS shells used to vendor
//! independently: the snapshot-to-stream feed reconciler (previously
//! ported 1:1 into Swift `ShellSupport.TerminalFeed` plus C copies in
//! `native/linux/src/feed.c` and `native/windows/src/feed.c`), the
//! roster filter match, and the relative-age label. One implementation
//! with headless tests is the cross-platform contract; the C shells bind
//! it through `am_feed_delta` / `am_roster_matches` / `am_relative_age`
//! in [`crate::ffi`], and Swift mirrors the same cases in
//! `TerminalFeedTests`.
//!
//! The Swift `TerminalFeed` implementation stays as the Swift-idiomatic
//! original (it feeds a SwiftTerm view directly); the C ports are gone,
//! so every C shell reconciles through this module.

/// ANSI reset the terminal views understand: clear screen, home cursor.
/// Matches Swift `TerminalFeed.clearScreen` and the old `AM_FEED_CLEAR`.
pub const FEED_CLEAR: &str = "\x1b[2J\x1b[H";

/// Feed text that advances a view showing `old` to also show `new`, or
/// `None` when the view is already current. Mirrors Swift
/// `TerminalFeed.delta` case for case:
///
/// - identical snapshots feed nothing;
/// - an append-only snapshot feeds just the suffix (the hot path:
///   streaming agent output and echoed typing);
/// - a scrolled snapshot feeds the new trailing lines (the overlap
///   between the old tail and the new head is already on screen);
/// - anything else (redraw, reflow after resize, cursor-addressed
///   programs) clears and replays the whole snapshot.
///
/// Newlines are normalized to CRLF: views interpret a bare LF as
/// line-feed-only, which would stair-step the output.
pub fn feed_delta(old: &str, new: &str) -> Option<String> {
    if new == old {
        return None;
    }
    if !old.is_empty() {
        if let Some(suffix) = new.strip_prefix(old) {
            if suffix.is_empty() {
                return None;
            }
            return Some(normalize_feed_newlines(suffix));
        }
    } else {
        return Some(normalize_feed_newlines(new));
    }
    let olds: Vec<&str> = old.split('\n').collect();
    let news: Vec<&str> = new.split('\n').collect();
    let overlap = largest_overlap(&olds, &news);
    if overlap > 0 {
        let mut out = String::from("\r\n");
        out.push_str(&news[overlap..].join("\r\n"));
        return Some(out);
    }
    Some(format!("{FEED_CLEAR}{}", normalize_feed_newlines(new)))
}

/// Largest k such that the last k lines of `old` equal the first k lines
/// of `new`. Quadratic in the worst case; snapshots are a few hundred
/// short lines, so this stays in the microseconds.
pub fn largest_overlap(old: &[&str], new: &[&str]) -> usize {
    let max_k = old.len().min(new.len());
    for k in (1..=max_k).rev() {
        if old[old.len() - k..] == new[..k] {
            return k;
        }
    }
    0
}

/// CRLF normalization for fed text: collapse `"\r\n"` to `"\n"` first,
/// then expand every `"\n"` to `"\r\n"` (mirrors Swift
/// `normalizeNewlines`; a CRLF pair counts as one newline).
pub fn normalize_feed_newlines(s: &str) -> String {
    s.replace("\r\n", "\n").replace('\n', "\r\n")
}

/// True when a roster row passes the sidebar filter: case-insensitive
/// substring over title, project, and id; everything passes when the
/// query is blank (whitespace-trimmed). This is the Swift/Linux match
/// semantics; the Windows shell previously filtered the raw row JSON
/// case-sensitively (which even matched JSON keys), now fixed.
pub fn roster_matches(title: &str, project: &str, id: &str, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return true;
    }
    title.to_lowercase().contains(&q)
        || project.to_lowercase().contains(&q)
        || id.to_lowercase().contains(&q)
}

/// Glanceable relative-age label for a roster row (`last_active` in unix
/// seconds): `just now` / `Nm ago` / `Nh ago` / `Nd ago`. Mirrors the
/// Swift sidebar; future times clamp to `just now`.
pub fn relative_age(now_secs: i64, then_secs: i64) -> String {
    let delta = now_secs.saturating_sub(then_secs).max(0);
    if delta < 60 {
        "just now".to_string()
    } else if delta < 3600 {
        format!("{}m ago", delta / 60)
    } else if delta < 86400 {
        format!("{}h ago", delta / 3600)
    } else {
        format!("{}d ago", delta / 86400)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_snapshots_feed_nothing() {
        assert_eq!(feed_delta("a\nb", "a\nb"), None);
        assert_eq!(feed_delta("", ""), None);
    }

    #[test]
    fn appended_output_feeds_suffix_only() {
        assert_eq!(
            feed_delta("hello", "hello world"),
            Some(" world".to_string())
        );
    }

    #[test]
    fn appended_lines_normalize_to_crlf() {
        assert_eq!(feed_delta("a", "a\nb\n"), Some("\r\nb\r\n".to_string()));
    }

    #[test]
    fn first_snapshot_feeds_whole_screen() {
        assert_eq!(feed_delta("", "ready\n$ "), Some("ready\r\n$ ".to_string()));
    }

    #[test]
    fn scrolled_snapshot_feeds_new_trailing_lines() {
        assert_eq!(feed_delta("a\nb", "b\nc"), Some("\r\nc".to_string()));
        assert_eq!(
            feed_delta("a\nb\nc", "c\nd\ne"),
            Some("\r\nd\r\ne".to_string())
        );
    }

    #[test]
    fn redraw_clears_and_replays() {
        assert_eq!(
            feed_delta("menu: [x]", "other screen"),
            Some(format!("{FEED_CLEAR}other screen"))
        );
    }

    #[test]
    fn resize_reflow_falls_back_to_redraw() {
        let feed =
            feed_delta("a very long line here", "a very\nlong line\nhere").expect("reflow replays");
        assert!(feed.starts_with(FEED_CLEAR), "feed: {feed:?}");
    }

    #[test]
    fn overlap_counts_shared_edge_lines() {
        assert_eq!(largest_overlap(&["a", "b"], &["b", "c"]), 1);
        assert_eq!(largest_overlap(&["a"], &["b"]), 0);
        assert_eq!(largest_overlap(&[], &["b"]), 0);
    }

    #[test]
    fn styled_spans_do_not_break_the_hot_path() {
        // Colors must not break append detection: appended styled output
        // still feeds just the suffix, carrying its own style state
        // (mirrors AnsiFeedTests.testStyledAppendFeedsSuffixOnly).
        assert_eq!(
            feed_delta("\x1b[0mhello", "\x1b[0mhello\x1b[0;38;2;205;0;0m red"),
            Some("\x1b[0;38;2;205;0;0m red".to_string())
        );
    }

    #[test]
    fn filter_matches_title_project_id_case_insensitively() {
        assert!(roster_matches("Shop App", "shop", "a1", ""));
        assert!(roster_matches("Shop App", "shop", "a1", "  "));
        assert!(roster_matches("Shop App", "shop", "a1", "shop"));
        assert!(roster_matches("Shop App", "shop", "a1", "SHOP"));
        assert!(roster_matches("Shop App", "shop", "a1", "a1"));
        assert!(!roster_matches("Shop App", "shop", "a1", "blog"));
        // The old Windows bug: the query must not match JSON syntax.
        assert!(!roster_matches("Shop", "shop", "a1", "\"title\""));
    }

    #[test]
    fn age_labels_stay_glanceable() {
        let now = 1_786_000_000;
        assert_eq!(relative_age(now, now), "just now");
        assert_eq!(relative_age(now, now - 59), "just now");
        assert_eq!(relative_age(now, now + 30), "just now");
        assert_eq!(relative_age(now, now - 300), "5m ago");
        assert_eq!(relative_age(now, now - 7200), "2h ago");
        assert_eq!(relative_age(now, now - 172_800), "2d ago");
    }
}

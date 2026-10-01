/* Contract tests for the core feed reconciler (`am_feed_delta`) plus the
 * shared presentation helpers (`am_roster_matches`, `am_relative_age`)
 * the WinUI shell binds through `core_bridge.h`. Mirrors
 * swift/Tests/ShellSupportTests/TerminalFeedTests.swift 1:1 so all
 * shells pin the same behavior — except the reconciler now lives in the
 * core (`src/shell_shared.rs`) instead of a vendored `feed.c`, so this
 * binary links the real staticlib and proves the C ABI edge end to end.
 * Exit 0 on success, 1 on first failure. */

#include "core_bridge.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define FEED_CLEAR "\x1b[2J\x1b[H" /* Must match the core's FEED_CLEAR. */

static int failures = 0;

#define CHECK_NULL(expr)                                                     \
    do {                                                                     \
        char *got_ = (expr);                                                 \
        if (got_ != NULL) {                                                  \
            printf("FAIL %s:%d: expected NULL, got %s\n", __func__,          \
                   __LINE__, got_);                                          \
            bridge_string_free(got_);                                        \
            failures++;                                                      \
            return;                                                          \
        }                                                                    \
    } while (0)

#define CHECK_EQ(expr, want)                                                 \
    do {                                                                     \
        char *got_ = (expr);                                                 \
        const char *want_ = (want);                                          \
        if (got_ == NULL || strcmp(got_, want_) != 0) {                      \
            printf("FAIL %s:%d: expected [%s], got [%s]\n", __func__,        \
                   __LINE__, want_, got_ ? got_ : "(null)");                 \
            bridge_string_free(got_);                                        \
            failures++;                                                      \
            return;                                                          \
        }                                                                    \
        bridge_string_free(got_);                                            \
    } while (0)

#define CHECK_TRUE(expr)                                                     \
    do {                                                                     \
        if (!(expr)) {                                                       \
            printf("FAIL %s:%d: expected true: %s\n", __func__, __LINE__,   \
                   #expr);                                                   \
            failures++;                                                      \
            return;                                                          \
        }                                                                    \
    } while (0)

static void test_identical_snapshots_feed_nothing(void) {
    CHECK_NULL(bridge_feed_delta("a\nb", "a\nb"));
    CHECK_NULL(bridge_feed_delta("", ""));
    CHECK_NULL(bridge_feed_delta(NULL, NULL));
}

static void test_appended_output_feeds_suffix_only(void) {
    CHECK_EQ(bridge_feed_delta("hello", "hello world"), " world");
}

static void test_appended_lines_normalize_to_crlf(void) {
    CHECK_EQ(bridge_feed_delta("a", "a\nb\n"), "\r\nb\r\n");
}

static void test_first_snapshot_feeds_whole_screen(void) {
    CHECK_EQ(bridge_feed_delta("", "ready\n$ "), "ready\r\n$ ");
    CHECK_EQ(bridge_feed_delta(NULL, "ready\n$ "), "ready\r\n$ ");
}

static void test_scrolled_snapshot_feeds_new_trailing_lines(void) {
    CHECK_EQ(bridge_feed_delta("a\nb", "b\nc"), "\r\nc");
    CHECK_EQ(bridge_feed_delta("a\nb\nc", "c\nd\ne"), "\r\nd\r\ne");
}

static void test_redraw_clears_and_replays(void) {
    CHECK_EQ(bridge_feed_delta("menu: [x]", "other screen"),
             FEED_CLEAR "other screen");
}

static void test_resize_reflow_falls_back_to_redraw(void) {
    char *feed = bridge_feed_delta("a very long line here",
                                   "a very\nlong line\nhere");
    int prefixed = feed != NULL &&
                   strncmp(feed, FEED_CLEAR, strlen(FEED_CLEAR)) == 0;
    if (!prefixed) {
        printf("FAIL %s:%d: expected clear-screen prefix, got [%s]\n",
               __func__, __LINE__, feed ? feed : "(null)");
        failures++;
    }
    bridge_string_free(feed);
    if (failures > 0) {
        return;
    }
}

static void test_roster_match_is_case_insensitive(void) {
    CHECK_TRUE(bridge_roster_matches("Shop App", "shop", "a1", ""));
    CHECK_TRUE(bridge_roster_matches("Shop App", "shop", "a1", "shop"));
    CHECK_TRUE(bridge_roster_matches("Shop App", "shop", "a1", "SHOP"));
    CHECK_TRUE(bridge_roster_matches("Shop App", "shop", "a1", "A1"));
    CHECK_TRUE(!bridge_roster_matches("Shop App", "shop", "a1", "blog"));
}

static void test_relative_age_labels(void) {
    CHECK_EQ(bridge_relative_age(1700000000, 1700000000), "just now");
    CHECK_EQ(bridge_relative_age(1700000300, 1700000000), "5m ago");
    CHECK_EQ(bridge_relative_age(1700007200, 1700000000), "2h ago");
    CHECK_EQ(bridge_relative_age(1700172800, 1700000000), "2d ago");
}

int main(void) {
    test_identical_snapshots_feed_nothing();
    test_appended_output_feeds_suffix_only();
    test_appended_lines_normalize_to_crlf();
    test_first_snapshot_feeds_whole_screen();
    test_scrolled_snapshot_feeds_new_trailing_lines();
    test_redraw_clears_and_replays();
    test_resize_reflow_falls_back_to_redraw();
    test_roster_match_is_case_insensitive();
    test_relative_age_labels();
    if (failures == 0) {
        printf("FEED-OK\n");
        return 0;
    }
    printf("FEED-FAIL failures=%d\n", failures);
    return 1;
}

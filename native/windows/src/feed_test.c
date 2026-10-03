/* Bridge tests for the shared snapshot→stream reconciler (dumb-shell
 * parity). The reconciler lives in the core (`staap_feed_delta`, pinned by
 * `cargo test --lib` mirroring TerminalFeedTests 1:1); this target pins
 * the bridge wrapper's NULL/empty conventions against the real
 * staticlib. Exit 0 on success, 1 on first failure. */

#include "core_bridge.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int failures = 0;

#define CHECK_NULL(expr)                                                     \
    do {                                                                     \
        char *got_ = (expr);                                                 \
        if (got_ != NULL) {                                                  \
            printf("FAIL %s:%d: expected NULL, got %s\n", __func__,          \
                   __LINE__, got_);                                          \
            free(got_);                                                      \
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
            free(got_);                                                      \
            failures++;                                                      \
            return;                                                          \
        }                                                                    \
        free(got_);                                                          \
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
    /* Clear-screen prefix + replayed snapshot (core FEED_CLEAR bytes:
     * ESC [ 2 J ESC [ H). */
    char *feed = bridge_feed_delta("menu: [x]", "other screen");
    int ok = feed != NULL && strncmp(feed, "\x1b[2J\x1b[H", 7) == 0 &&
        strcmp(feed + 7, "other screen") == 0;
    if (!ok) {
        printf("FAIL %s:%d: expected clear+replay, got [%s]\n", __func__,
               __LINE__, feed ? feed : "(null)");
        failures++;
        free(feed);
        return;
    }
    free(feed);
}

static void test_resize_reflow_falls_back_to_redraw(void) {
    char *feed = bridge_feed_delta("a very long line here",
                                   "a very\nlong line\nhere");
    int prefixed = feed != NULL && strncmp(feed, "\x1b[2J\x1b[H", 7) == 0;
    if (!prefixed) {
        printf("FAIL %s:%d: expected clear-screen prefix, got [%s]\n",
               __func__, __LINE__, feed ? feed : "(null)");
        failures++;
    }
    free(feed);
    if (failures > 0) {
        return;
    }
}

int main(void) {
    test_identical_snapshots_feed_nothing();
    test_appended_output_feeds_suffix_only();
    test_appended_lines_normalize_to_crlf();
    test_first_snapshot_feeds_whole_screen();
    test_scrolled_snapshot_feeds_new_trailing_lines();
    test_redraw_clears_and_replays();
    test_resize_reflow_falls_back_to_redraw();
    if (failures == 0) {
        printf("FEED-OK\n");
        return 0;
    }
    printf("FEED-FAIL failures=%d\n", failures);
    return 1;
}

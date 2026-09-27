/* Snapshot-to-stream reconciler for the WinUI terminal view (issue #64).
 *
 * The core owns its emulator and hands out plain-text screen snapshots
 * (`am_screen_text`); the WinUI output box shows appended text. This
 * header ports `swift/Sources/ShellSupport/TerminalFeed.swift` 1:1 (via
 * `native/linux/src/feed.h`, #63) so all shells reconcile identically:
 *
 * - identical snapshots feed nothing;
 * - an append-only snapshot feeds just the suffix (the hot path:
 *   streaming agent output and echoed typing);
 * - a scrolled snapshot feeds the new trailing lines;
 * - anything else (redraw, reflow after resize, cursor-addressed
 *   programs) clears and replays the whole snapshot.
 *
 * Newlines are normalized to CRLF: the WinUI TextBox, like VTE,
 * interprets a bare LF as line-feed-only, which would stair-step
 * the output.
 */

#ifndef AM_FEED_H
#define AM_FEED_H

/* ANSI reset VTE understands: clear screen, home cursor. */
#define AM_FEED_CLEAR "\x1b[2J\x1b[H"

/* Feed text that advances a view showing `old_text` to also show
 * `new_text`, or NULL when the view is already current. Either argument
 * may be NULL (treated as ""). The result is malloc'd; the caller frees
 * it with free(). */
char *am_feed_delta(const char *old_text, const char *new_text);

/* Largest k such that the last k lines of `old_lines[0..old_n]` equal the
 * first k lines of `new_lines[0..new_n]`. Exposed for tests. */
int am_feed_overlap(char **old_lines, int old_n, char **new_lines, int new_n);

#endif /* AM_FEED_H */

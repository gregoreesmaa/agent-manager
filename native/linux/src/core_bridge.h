/* Core C ABI bridge for the Linux GTK4 shell (issue #63).
 *
 * Thin, dependency-free wrappers over `include/agent_manager.h` at the
 * repo root. Mirrors swift/CoreBridge.swift: same owned-handle discipline
 * (core/PTY freed exactly once, strings freed with am_screen_text_free),
 * same status-code contract (Attention=0, Idle=1, Working=2; negatives
 * are null handle / out of bounds).
 */

#ifndef AM_CORE_BRIDGE_H
#define AM_CORE_BRIDGE_H

#include <stddef.h>

#include "agent_manager.h"

/* Roster status codes, matching `am_status`. */
enum {
    AM_STATUS_ATTENTION = 0,
    AM_STATUS_IDLE = 1,
    AM_STATUS_WORKING = 2,
};

/* Owned core handle. NULL on allocation failure (mirrors Core.init?). */
AmCore *bridge_core_new(void);
void bridge_core_free(AmCore *core);

/* Persist the core config. Returns 0 on success; on failure returns
 * nonzero and, when `msg_out` is non-NULL, sets it to a malloc'd copy
 * of `am_last_error` (caller frees with free()). */
int bridge_core_save(AmCore *core, char **msg_out);

/* Roster snapshot helpers. */
size_t bridge_session_count(const AmCore *core);
int bridge_status(const AmCore *core, size_t row);

/* Owned JSON of roster row `row`, or NULL (null handle / OOB / JSON
 * failure). Free with bridge_string_free(). */
char *bridge_session_json(const AmCore *core, size_t row);

/* Free a string from bridge_session_json / bridge_screen_text. */
void bridge_string_free(char *s);

/* Spawn a fresh session PTY of `cols` x `rows`. Returns NULL on failure
 * and, when `msg_out` is non-NULL, sets it to a malloc'd copy of
 * `am_last_error`. Free the handle with bridge_pty_free(). */
AmPty *bridge_spawn(const AmCore *core, unsigned cols, unsigned rows,
                    char **msg_out);
void bridge_pty_free(AmPty *pty);

/* Feed queued output into the emulator. Nonzero when the screen may
 * have changed (the shell's only repaint gate). */
int bridge_pump(AmPty *pty);

/* Forward raw bytes (already key-encoded by the shell) to the child.
 * Returns 0 on success; on failure returns nonzero with `msg_out`
 * set like bridge_core_save. */
int bridge_write(AmPty *pty, const unsigned char *data, size_t len,
                 char **msg_out);

void bridge_resize(AmPty *pty, unsigned cols, unsigned rows);

/* Owned plain-text snapshot of the emulated screen (NULL on null
 * handle). Free with bridge_string_free(). */
char *bridge_screen_text(const AmPty *pty);

/* Copy of the thread-local last-error message (never NULL;
 * caller frees with free()). */
char *bridge_last_error(void);

#endif /* AM_CORE_BRIDGE_H */

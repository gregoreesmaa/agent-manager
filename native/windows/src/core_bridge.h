/* Core C ABI bridge for the Windows WinUI shell (issue #64).
 *
 * Thin, dependency-free wrappers over `include/agent_manager.h` at the
 * repo root. Mirrors swift/CoreBridge.swift and
 * native/linux/src/core_bridge.h: same owned-handle discipline
 * (core/PTY freed exactly once, strings freed with am_screen_text_free),
 * same status-code contract (Attention=0, Idle=1, Working=2; negatives
 * are null handle / out of bounds).
 *
 * Unlike the Linux header, this one is included from C++ (the WinUI
 * shell) as well as C (the smoke binary), so the declarations carry
 * `extern "C"` under `__cplusplus`: the definitions in core_bridge.c
 * are compiled as C, and without matching linkage the C++ linker
 * would mangle the names and fail the link.
 */

#ifndef AM_CORE_BRIDGE_H
#define AM_CORE_BRIDGE_H

#include <stddef.h>

#include "agent_manager.h"

#ifdef __cplusplus
extern "C" {
#endif

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
 * `am_last_error`. Free the handle with bridge_pty_free().
 *
 * On Windows the core's PTY is ConPTY-backed (portable-pty uses the
 * native Console Pseudo-terminal API), so this is the "ConPTY shell"
 * spawn: no second console is ever created on the WinUI side. */
AmPty *bridge_spawn(const AmCore *core, unsigned cols, unsigned rows,
                    char **msg_out);
void bridge_pty_free(AmPty *pty);

/* 2D-launch spawn (folder x CLI + yolo) of `cols` x `rows`. `cli` is
 * NULL/empty (repeat the core's effective default: last-used, configured,
 * autodetected) or a harness id; `cwd` is NULL (inherit) or a path;
 * `yolo` is tri-state (1 = force on once, -1 = force off once, 0 = the
 * per-agent config default). Returns NULL on failure with `msg_out`
 * set like bridge_spawn. Free the handle with bridge_pty_free(). */
AmPty *bridge_spawn_launch(const AmCore *core, const char *cli,
                           const char *cwd, int yolo, unsigned cols,
                           unsigned rows, char **msg_out);

/* Owned JSON of the autodetected CLI catalog ([{id,program,path,
 * available}] in core order) or NULL on allocation failure. Free with
 * bridge_string_free(). Missing CLIs stay listed (available=false). */
char *bridge_clis_json(void);

/* Owned JSON string array of folder recents (MRU-first), or NULL on
 * allocation failure. Free with bridge_string_free(). */
char *bridge_recent_json(const AmCore *core);

/* Owned harness id of the effective CLI for `cli` (explicit id, or the
 * core's last-used / configured / autodetected resolution for NULL).
 * Free with bridge_string_free(). */
char *bridge_effective_cli(const AmCore *core, const char *cli);

/* Record a confirmed picker launch (last-used CLI + folder MRU), so the
 * next repeat replays it. NULL/empty `cli` keeps the previous CLI; NULL
 * `cwd` records no folder. Returns 0 on success. */
int bridge_note_launch(AmCore *core, const char *cli, const char *cwd,
                       char **msg_out);

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

/* Shared presentation helpers owned by the core (`shell_shared`,
 * exposed as `am_feed_delta` / `am_roster_matches` / `am_relative_age`
 * / `am_link_count` / `am_last_active` in `agent_manager.h`): the
 * snapshot-to-stream feed reconciler, the sidebar filter match, the
 * relative-age label, and per-row link/age getters. Owned-string results
 * free with bridge_string_free(). */
char *bridge_feed_delta(const char *old_text, const char *new_text);
int bridge_roster_matches(const char *title, const char *project,
                           const char *id, const char *query);
char *bridge_relative_age(long long now_secs, long long then_secs);
int bridge_link_count(const AmCore *core, size_t row);
long long bridge_last_active(const AmCore *core, size_t row);

/* Copy of the thread-local last-error message (never NULL;
 * caller frees with free()). */
char *bridge_last_error(void);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* AM_CORE_BRIDGE_H */

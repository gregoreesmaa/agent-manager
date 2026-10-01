/* Core C ABI bridge implementation for the Windows WinUI shell (#64).
 *
 * Thin, dependency-free wrappers over `include/agent_manager.h` at the
 * repo root. Mirrors swift/CoreBridge.swift and
 * native/linux/src/core_bridge.c: same owned-handle discipline
 * (core/PTY freed exactly once, strings freed with am_screen_text_free),
 * same status-code contract (Attention=0, Idle=1, Working=2; negatives
 * are null handle / out of bounds). Compiled as C and linked into both
 * the headless smoke binary and the WinUI app (see core_bridge.h for
 * the `extern "C"` discipline the C++ shell relies on).
 */

#include "core_bridge.h"

#include <stdint.h>
#include <stdlib.h>
#include <string.h>

AmCore *bridge_core_new(void) {
    return am_core_new();
}

void bridge_core_free(AmCore *core) {
    am_core_free(core);
}

/* Copy the core's thread-local message into a malloc'd buffer the caller
 * owns (freed with free()). Never returns NULL. */
static char *copy_last_error(void) {
    const char *msg = am_last_error();
    if (!msg) {
        msg = "unknown core error";
    }
    size_t n = strlen(msg) + 1;
    char *copy = malloc(n);
    if (copy) {
        memcpy(copy, msg, n);
    }
    return copy;
}

int bridge_core_save(AmCore *core, char **msg_out) {
    int rc = am_core_save(core);
    if (rc != 0 && msg_out) {
        *msg_out = copy_last_error();
    }
    return rc;
}

size_t bridge_session_count(const AmCore *core) {
    return am_session_count(core);
}

int bridge_status(const AmCore *core, size_t row) {
    return am_status(core, row);
}

char *bridge_session_json(const AmCore *core, size_t row) {
    return am_session_json(core, row);
}

void bridge_string_free(char *s) {
    am_screen_text_free(s);
}

AmPty *bridge_spawn(const AmCore *core, unsigned cols, unsigned rows,
                    char **msg_out) {
    AmPty *pty = NULL;
    int rc = am_spawn(core, &pty, NULL, (uint16_t)cols, (uint16_t)rows);
    if (rc != 0) {
        if (msg_out) {
            *msg_out = copy_last_error();
        }
        return NULL;
    }
    return pty;
}

void bridge_pty_free(AmPty *pty) {
    am_pty_free(pty);
}

AmPty *bridge_spawn_launch(const AmCore *core, const char *cli,
                           const char *cwd, int yolo, unsigned cols,
                           unsigned rows, char **msg_out) {
    AmPty *pty = NULL;
    int rc = am_spawn_launch(core, &pty, cli, cwd, (int32_t)yolo,
                             (uint16_t)cols, (uint16_t)rows);
    if (rc != 0) {
        if (msg_out) {
            *msg_out = copy_last_error();
        }
        return NULL;
    }
    return pty;
}

char *bridge_clis_json(void) {
    return am_clis_json();
}

char *bridge_recent_json(const AmCore *core) {
    return am_recent_json(core);
}

char *bridge_effective_cli(const AmCore *core, const char *cli) {
    return am_effective_cli(core, cli);
}

int bridge_note_launch(AmCore *core, const char *cli, const char *cwd,
                       char **msg_out) {
    int rc = am_note_launch(core, cli, cwd);
    if (rc != 0 && msg_out) {
        *msg_out = copy_last_error();
    }
    return rc;
}

int bridge_pump(AmPty *pty) {
    return am_pump(pty) ? 1 : 0;
}

int bridge_write(AmPty *pty, const unsigned char *data, size_t len,
                 char **msg_out) {
    int rc = am_write(pty, data, len);
    if (rc != 0 && msg_out) {
        *msg_out = copy_last_error();
    }
    return rc;
}

void bridge_resize(AmPty *pty, unsigned cols, unsigned rows) {
    am_resize(pty, (uint16_t)cols, (uint16_t)rows);
}

char *bridge_screen_text(const AmPty *pty) {
    return am_screen_text(pty);
}

char *bridge_feed_delta(const char *old_text, const char *new_text) {
    return am_feed_delta(old_text, new_text);
}

int bridge_roster_matches(const char *title, const char *project,
                           const char *id, const char *query) {
    return am_roster_matches(title, project, id, query);
}

char *bridge_relative_age(long long now_secs, long long then_secs) {
    return am_relative_age((int64_t)now_secs, (int64_t)then_secs);
}

int bridge_link_count(const AmCore *core, size_t row) {
    return am_link_count(core, row);
}

long long bridge_last_active(const AmCore *core, size_t row) {
    return (long long)am_last_active(core, row);
}

char *bridge_last_error(void) {
    return copy_last_error();
}

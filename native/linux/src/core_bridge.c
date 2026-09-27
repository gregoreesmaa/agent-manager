/* Core C ABI bridge implementation for the Linux GTK4 shell (issue #63).
 *
 * Thin, dependency-free wrappers over `include/agent_manager.h` at the
 * repo root. Mirrors swift/CoreBridge.swift: same owned-handle discipline
 * (core/PTY freed exactly once, strings freed with am_screen_text_free),
 * same status-code contract (Attention=0, Idle=1, Working=2; negatives
 * are null handle / out of bounds).
 */

#include "core_bridge.h"

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

char *bridge_last_error(void) {
    return copy_last_error();
}

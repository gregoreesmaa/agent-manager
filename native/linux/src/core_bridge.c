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

void bridge_pty_free(AmPty *pty) {
    am_pty_free(pty);
}

int bridge_pump(AmPty *pty) {
    return am_pump(pty) ? 1 : 0;
}

int bridge_pump_all(AmCore *core) {
    return am_pump_all(core) ? 1 : 0;
}

size_t bridge_live_count(const AmCore *core) {
    return am_live_count(core);
}

size_t bridge_max_runs(void) {
    return am_max_runs();
}

int bridge_is_live(const AmCore *core, const char *id) {
    return am_is_live(core, id) ? 1 : 0;
}

int bridge_run_spawn(AmCore *core, const char *cli, const char *cwd,
                     int yolo, unsigned cols, unsigned rows, char *id_out,
                     size_t id_cap, char **msg_out) {
    int rc = am_run_spawn(core, cli, cwd, (int32_t)yolo, (uint16_t)cols,
                          (uint16_t)rows, id_out, id_cap);
    if (rc != 0 && msg_out) {
        *msg_out = copy_last_error();
    }
    return rc;
}

int bridge_run_restart(AmCore *core, const char *id, unsigned cols,
                       unsigned rows, char **msg_out) {
    int rc = am_run_restart(core, id, (uint16_t)cols, (uint16_t)rows);
    if (rc != 0 && msg_out) {
        *msg_out = copy_last_error();
    }
    return rc;
}

int bridge_run_close(AmCore *core, const char *id) {
    return am_run_close(core, id);
}

int bridge_needs_quit_confirm(const AmCore *core) {
    return am_needs_quit_confirm(core) ? 1 : 0;
}

int bridge_run_pump(AmCore *core, const char *id) {
    return am_run_pump(core, id) ? 1 : 0;
}

int bridge_run_write(AmCore *core, const char *id,
                     const unsigned char *data, size_t len, char **msg_out) {
    int rc = am_run_write(core, id, data, len);
    if (rc != 0 && msg_out) {
        *msg_out = copy_last_error();
    }
    return rc;
}

void bridge_run_resize(AmCore *core, const char *id, unsigned cols,
                       unsigned rows) {
    am_run_resize(core, id, (uint16_t)cols, (uint16_t)rows);
}

char *bridge_run_screen_text(const AmCore *core, const char *id) {
    return am_run_screen_text(core, id);
}

char *bridge_run_spans_json(const AmCore *core, const char *id) {
    return am_run_spans_json(core, id);
}

int bridge_run_exited(const AmCore *core, const char *id) {
    return am_run_exited(core, id) ? 1 : 0;
}

int bridge_key_encode(const char *key, const char *key_char, int ctrl,
                      int alt, unsigned char *bytes_out, size_t cap) {
    return am_key_encode(key, key_char, ctrl, alt, bytes_out, cap);
}

char *bridge_feed_delta(const char *old_text, const char *new_text) {
    /* am_feed_delta returns a core-owned string (freed with
     * am_screen_text_free); copy it into a malloc'd buffer so the
     * caller frees uniformly with free(). NULL stays NULL. */
    char *core = am_feed_delta(old_text, new_text);
    if (!core) {
        return NULL;
    }
    size_t n = strlen(core) + 1;
    char *out = malloc(n);
    if (out) {
        memcpy(out, core, n);
    }
    am_screen_text_free(core);
    return out;
}

char *bridge_spawn_preview(const char *cli, const char *folder, int yolo) {
    char *core = am_spawn_preview(cli, folder, (int32_t)yolo);
    if (!core) {
        return NULL;
    }
    size_t n = strlen(core) + 1;
    char *out = malloc(n);
    if (out) {
        memcpy(out, core, n);
    }
    am_screen_text_free(core);
    return out;
}

int bridge_yolo_value(int selected) {
    return am_yolo_value((int32_t)selected);
}

char *bridge_age_string(long long now_unix, long long then_unix) {
    return am_age_string((int64_t)now_unix, (int64_t)then_unix);
}

char *bridge_status_glyph(int code) {
    return am_status_glyph((int32_t)code);
}

char *bridge_section_title(int code) {
    return am_section_title((int32_t)code);
}

double bridge_clamp_sidebar(double px) {
    return am_clamp_sidebar(px);
}

int bridge_row_matches(const AmCore *core, size_t row, const char *query) {
    return am_row_matches(core, row, query) ? 1 : 0;
}

void bridge_set_filter(AmCore *core, const char *query) {
    am_set_filter(core, query);
}

size_t bridge_selected(const AmCore *core) {
    return am_selected(core);
}

void bridge_select(AmCore *core, size_t row) {
    am_select(core, row);
}

void bridge_select_step(AmCore *core, int forward) {
    am_select_step(core, forward);
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

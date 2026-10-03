/* Headless runtime check of the linked core staticlib (issue #64).
 * Exercises the C ABI surface end to end through the bridge and prints
 * one line for harnesses to grep:
 *
 *     SMOKE-OK sessions=<n>
 *
 * Exit status is 0 on success, 1 on the first failure. No WinUI is
 * initialized here, so this runs under plain CI with no session.
 *
 * Usage:
 *   staap-win-smoke            # read-only surface: roster, status, OOB
 *   staap-win-smoke --smoke-live  # plus spawn/pump/write/resize against the
 *                              # real `muse` command (or a fake one on PATH;
 *                              # see tests/smoke_live.ps1, which builds a
 *                              # fake muse.exe from tests/fake_muse.c)
 */

#define _POSIX_C_SOURCE 200809L /* nanosleep under strict C11 */

#include "core_bridge.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#ifdef _WIN32
#include <windows.h>
static void sleep_ms(long ms) {
    Sleep((DWORD)ms);
}
#else
#include <unistd.h>
static void sleep_ms(long ms) {
    struct timespec ts;
    ts.tv_sec = ms / 1000;
    ts.tv_nsec = (ms % 1000) * 1000000L;
    nanosleep(&ts, NULL);
}
#endif

static int fail(const char *what) {
    fprintf(stderr, "SMOKE-FAIL %s\n", what);
    return 1;
}

/* Minimal JSON sanity check for a roster row: the core hands out a
 * serialized ChatSession object; the shell only needs it to be an object
 * carrying the fields it renders. */
static int row_json_sane(const char *json) {
    return json && json[0] == '{' && strstr(json, "\"id\"") &&
           strstr(json, "\"title\"");
}

static double now_seconds(void) {
    return (double)clock() / (double)CLOCKS_PER_SEC;
}

/* Live converse through the run registry: spawn attaches a real roster
 * row, converse/resize/close go by row id. */
static int live_converse(StaapCore *core) {
    char id[256] = { 0 };
    char *spawn_err = NULL;
    if (bridge_run_spawn(core, NULL, NULL, 0, 80, 24, id, sizeof id,
                         &spawn_err) != 0) {
        fprintf(stderr, "SMOKE-FAIL live spawn: %s\n",
                spawn_err ? spawn_err : "?");
        free(spawn_err);
        return 1;
    }

    /* Pump until the child produces visible output. */
    size_t first_bytes = 0;
    double start = now_seconds();
    while (now_seconds() - start < 15.0) {
        if (bridge_run_pump(core, id)) {
            char *text = bridge_run_screen_text(core, id);
            if (text) {
                int visible = 0;
                for (const char *p = text; *p; p++) {
                    if (*p != ' ' && *p != '\t' && *p != '\n' && *p != '\r') {
                        visible = 1;
                        break;
                    }
                }
                if (visible) {
                    first_bytes = strlen(text);
                }
                bridge_string_free(text);
                if (visible) {
                    break;
                }
            }
        }
        sleep_ms(50);
    }
    if (first_bytes == 0) {
        bridge_run_close(core, id);
        return fail("live: no output within 15s");
    }

    /* Input reaches the PTY: the write must succeed at the fd level. */
    static const unsigned char probe[] = "smoke-probe-64";
    char *write_err = NULL;
    if (bridge_run_write(core, id, probe, sizeof probe - 1, &write_err) !=
        0) {
        fprintf(stderr, "SMOKE-FAIL live write: %s\n",
                write_err ? write_err : "?");
        free(write_err);
        bridge_run_close(core, id);
        return 1;
    }

    /* Resize keeps the seam alive; the screen stays readable. */
    bridge_run_resize(core, id, 100, 30);
    sleep_ms(300);
    (void)bridge_run_pump(core, id);
    char *after = bridge_run_screen_text(core, id);
    int readable = after && after[0] != '\0';
    bridge_string_free(after);
    bridge_run_close(core, id);
    if (!readable) {
        return fail("live: unreadable screen after resize");
    }

    printf("SMOKE-LIVE-OK firstBytes=%zu\n", first_bytes);
    return 0;
}

int main(int argc, char **argv) {
    int live = argc > 1 && strcmp(argv[1], "--smoke-live") == 0;

    StaapCore *core = bridge_core_new();
    if (!core) {
        return fail("bridge_core_new NULL");
    }
    size_t n = bridge_session_count(core);

    /* Every row the count reports must decode with a valid status. */
    for (size_t i = 0; i < n; i++) {
        char *json = bridge_session_json(core, i);
        if (!row_json_sane(json)) {
            bridge_string_free(json);
            bridge_core_free(core);
            return fail("row JSON missing id/title");
        }
        bridge_string_free(json);
        int st = bridge_status(core, i);
        if (st < 0 || st > 2) {
            bridge_core_free(core);
            return fail("row status out of range");
        }
    }

    /* Out-of-bounds row: -2 status, NULL JSON, message recorded. */
    size_t oob = n + 1000000;
    if (bridge_status(core, oob) != -2) {
        bridge_core_free(core);
        return fail("bridge_status OOB != -2");
    }
    char *oob_json = bridge_session_json(core, oob);
    if (oob_json != NULL) {
        bridge_string_free(oob_json);
        bridge_core_free(core);
        return fail("bridge_session_json OOB non-null");
    }
    char *err = bridge_last_error();
    int err_empty = !err || err[0] == '\0';
    free(err);
    if (err_empty) {
        bridge_core_free(core);
        return fail("bridge_last_error empty after failure");
    }

    /* 2D-launch surface: the catalog lists every supported CLI in core
     * order (missing ones stay listed), recents decode as a JSON array,
     * and the effective CLI resolves non-empty on a null hint. */
    char *clis = bridge_clis_json();
    if (!clis || clis[0] != '[' || !strstr(clis, "\"muse\"") ||
        !strstr(clis, "\"claude\"") || !strstr(clis, "\"opencode\"") ||
        !strstr(clis, "\"codex\"")) {
        bridge_string_free(clis);
        bridge_core_free(core);
        return fail("catalog missing supported CLIs");
    }
    bridge_string_free(clis);
    char *recents = bridge_recent_json(core);
    if (!recents || recents[0] != '[') {
        bridge_string_free(recents);
        bridge_core_free(core);
        return fail("recents not a JSON array");
    }
    bridge_string_free(recents);
    char *eff = bridge_effective_cli(core, NULL);
    if (!eff || !eff[0]) {
        bridge_string_free(eff);
        bridge_core_free(core);
        return fail("effective CLI empty");
    }
    bridge_string_free(eff);

    /* Shared-helper surface: one copy in the core, same on every shell. */
    {
        unsigned char kbuf[16];
        if (bridge_key_encode("enter", NULL, 0, 0, kbuf, sizeof kbuf) != 1 ||
            kbuf[0] != '\r') {
            bridge_core_free(core);
            return fail("key encode enter != CR");
        }
        char *prev = bridge_spawn_preview("muse", "C:\\api", 1);
        int prev_ok =
            prev && strcmp(prev, "runs: muse in C:\\api + yolo") == 0;
        free(prev);
        if (!prev_ok) {
            bridge_core_free(core);
            return fail("spawn preview");
        }
        if (bridge_max_runs() != 10 || bridge_live_count(core) != 0) {
            bridge_core_free(core);
            return fail("registry cap/count");
        }
    }

    printf("SMOKE-OK sessions=%zu\n", n);

    int rc = 0;
    if (live) {
        rc = live_converse(core);
    }
    bridge_core_free(core);
    return rc;
}

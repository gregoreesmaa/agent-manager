/* agent-manager native core C ABI (epic #60, hardened #61).
 *
 * Checked-in snapshot of the `extern "C"` contract in `src/ffi.rs`.
 * Regenerate with cbindgen (see `cbindgen.toml` at the repo root):
 *
 *     cargo install cbindgen
 *     cbindgen --config cbindgen.toml --crate agent-manager --output include/agent_manager.h
 *
 * After regenerating, verify the export set still matches the staticlib:
 *
 *     cargo build
 *     nm -gU target/debug/libagent_manager.a | grep -c 'am_spawn\|am_pump\|am_write'
 *
 * Ownership rules (binding contract):
 * - `am_core_new` / `am_spawn` hand out owned handles; free exactly once
 *   with `am_core_free` / `am_pty_free`. All free functions accept NULL.
 * - `am_screen_text` / `am_spans_json` return freshly allocated strings
 *   freed with `am_screen_text_free`. NULL means null handle (or an
 *   unreachable NUL-byte failure; details in `am_last_error`).
 * - `am_last_error` is never NULL and needs no free; valid until the next
 *   failing `am_*` call on the same thread. Do not hold it across calls.
 */

#ifndef AGENT_MANAGER_H
#define AGENT_MANAGER_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Integer error codes returned by every fallible `am_*` function. */
enum AmError {
    AM_OK = 0,    /* Success. */
    AM_SPAWN = 1, /* Child spawn failed (bad program, PTY unavailable). */
    AM_IO = 2,    /* PTY input write failed. */
    AM_UTF8 = 3,  /* A `*const char` argument was not valid UTF-8. */
    AM_NULL = 4,  /* A required pointer argument was null. */
    AM_CONFIG = 5 /* Config persistence failed. */
};

/* 8-bit RGB triple (plain value). */
typedef struct AmRgb {
    uint8_t r;
    uint8_t g;
    uint8_t b;
} AmRgb;

/* Cell style as plain values (`has_fg`/`has_bg` = terminal default when
 * false; the `Option` lives on the Rust side only). */
typedef struct AmStyle {
    bool has_fg;
    AmRgb fg;
    bool has_bg;
    AmRgb bg;
    bool bold;
    bool italic;
    bool underline;
} AmStyle;

/* Opaque core handle: owns the roster `App` (which owns its `Config`). */
typedef struct AmCore AmCore;

/* Opaque PTY handle: owns one `EmbeddedPty`. */
typedef struct AmPty AmPty;

/* Build a core the way the app starts (discovery + persistence merge;
 * never fails except on allocation). Free with `am_core_free`. */
AmCore *am_core_new(void);

/* Free a core from `am_core_new`. NULL is a no-op. */
void am_core_free(AmCore *core);

/* Persist the core config. Returns an `AmError` code. */
int32_t am_core_save(const AmCore *core);

/* Spawn a fresh `muse` session PTY of `cols` x `rows`. `cwd` is NULL
 * (inherit) or a NUL-terminated UTF-8 path; the handle lands in `*out`.
 * Returns an `AmError` code. Free the handle with `am_pty_free`. */
int32_t am_spawn(const AmCore *core, AmPty **out, const char *cwd,
                 uint16_t cols, uint16_t rows);

/* Spawn a 2D-launch session PTY (folder x CLI + yolo) of `cols` x `rows`.
 * `cli` names the harness id (`muse`, `claude`, ...; NULL/empty repeats
 * the last-used/default resolution), `cwd` is NULL (inherit) or a path,
 * `yolo` nonzero forces the canonical yolo flag for this spawn only.
 * Returns an `AmError` code; the handle lands in `*out`. Free the handle
 * with `am_pty_free`. */
int32_t am_spawn_launch(const AmCore *core, AmPty **out, const char *cli,
                        const char *cwd, int32_t yolo,
                        uint16_t cols, uint16_t rows);

/* Feed queued output into the emulator. True when new output arrived or
 * the child newly exited (the repaint gate). NULL is false, never UB. */
bool am_pump(AmPty *pty);

/* Forward raw bytes (already key-encoded by the shell) to the child.
 * NULL `data` with `len == 0` is a no-op success. Returns `AmError`. */
int32_t am_write(AmPty *pty, const uint8_t *data, size_t len);

/* Resize the PTY and the emulator grid. NULL is a no-op. */
void am_resize(AmPty *pty, uint16_t cols, uint16_t rows);

/* Owned UTF-8 snapshot of the emulated screen (NULL on null handle);
 * free with `am_screen_text_free`. */
char *am_screen_text(const AmPty *pty);

/* Free a string from `am_screen_text` / `am_spans_json`. NULL no-op. */
void am_screen_text_free(char *s);

/* Owned styled spans as JSON (one array per grid row, each span
 * `{text,fg,bg,bold,italic,underline}`; fg/bg are `[r,g,b]` or null).
 * Free with `am_screen_text_free`. */
char *am_spans_json(const AmPty *pty);

/* Roster status of row `row`: 0 = Attention, 1 = Idle, 2 = Working.
 * Returns -1 on null handle, -2 when `row` is out of bounds. */
int32_t am_status(const AmCore *core, size_t row);

/* Number of roster rows in the core. Null core yields 0, never UB. */
size_t am_session_count(const AmCore *core);

/* Owned JSON of roster row `row` (a serialized `ChatSession`). NULL on
 * null handle, out-of-bounds row, or JSON failure; free with
 * `am_screen_text_free`. */
char *am_session_json(const AmCore *core, size_t row);

/* Owned JSON of the autodetected CLI catalog (2D launch):
 * `[{"id","program","path"|null,"available"}]` in `SUPPORTED_CLIS`
 * order. Never NULL on allocation success; free with
 * `am_screen_text_free`. */
char *am_clis_json(void);

/* Owned JSON of the persisted folder recents (2D launch): a string array,
 * MRU-first. NULL core yields an empty list, never UB; free with
 * `am_screen_text_free`. */
char *am_recent_json(const AmCore *core);

/* Record a confirmed launch (2D launch): refreshes last-used CLI + folder
 * MRU in the core config. NULL core is a null error; NULL/empty `cli`
 * keeps the previous CLI; NULL `cwd` records no folder. */
int32_t am_note_launch(AmCore *core, const char *cli, const char *cwd);

/* Last error message for this thread (UTF-8, NUL-terminated). Never
 * NULL; valid until the next failing `am_*` call on this thread. */
const char *am_last_error(void);

/* Free a PTY from `am_spawn` (reaps the child). NULL is a no-op. */
void am_pty_free(AmPty *pty);

#ifdef __cplusplus
}
#endif

#endif /* AGENT_MANAGER_H */

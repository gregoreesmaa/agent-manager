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

/* Opaque core handle: owns the run registry (roster `App` + live PTYs).
 * Native shells drive the roster, spawn, pump, and persistence through
 * this handle, so every shell shares one policy instead of reimplementing
 * it per OS. */
typedef struct AmCore AmCore;

/* Opaque PTY handle: owns one `EmbeddedPty`. */
typedef struct AmPty AmPty;

/* Build a core the way the app starts (discovery + persistence merge;
 * never fails except on allocation). Free with `am_core_free`. */
AmCore *am_core_new(void);

/* Free a core from `am_core_new`. NULL is a no-op. */
void am_core_free(AmCore *core);

/* Persist core state (user config + run list). Shells call this on a
 * timer tick while dirty, on close, and after closing a run — never via
 * a manual Save button (removed from every shell: persistence is
 * automatic, like the gpui shell's throttled pump persist). Returns an
 * `AmError` code. */
int32_t am_core_save(const AmCore *core);

/* Spawn a fresh `muse` session PTY of `cols` x `rows`. `cwd` is NULL
 * (inherit) or a NUL-terminated UTF-8 path; the handle lands in `*out`.
 * Returns an `AmError` code. Free the handle with `am_pty_free`. */
int32_t am_spawn(const AmCore *core, AmPty **out, const char *cwd,
                 uint16_t cols, uint16_t rows);

/* Spawn a 2D-launch session PTY (folder x CLI + yolo) of `cols` x `rows`.
 * `cli` names the harness id (`muse`, `claude`, ...; NULL/empty repeats
 * the last-used/default resolution), `cwd` is NULL (inherit) or a path,
 * `yolo` is tri-state: >0 forces the canonical yolo flag on for this
 * spawn only, <0 forces it off, 0 follows the per-agent config default.
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

/* Owned harness id of the effective CLI for `cli` (2D launch): the
 * explicit id when non-empty, else the core's last-used / configured /
 * autodetected resolution (same rule as `am_spawn_launch`). Free with
 * `am_screen_text_free`. */
char *am_effective_cli(const AmCore *core, const char *cli);

/* Owned JSON of the persisted folder recents (2D launch): a string array,
 * MRU-first. NULL core yields an empty list, never UB; free with
 * `am_screen_text_free`. */
char *am_recent_json(const AmCore *core);

/* Record a confirmed launch (2D launch): refreshes last-used CLI + folder
 * MRU in the core config. NULL core is a null error; NULL/empty `cli`
 * keeps the previous CLI; NULL `cwd` records no folder. */
int32_t am_note_launch(AmCore *core, const char *cli, const char *cwd);

/* Snapshot-to-stream feed reconciler for C shells (shared
 * `shell_shared::feed_delta`): feed text that advances a view showing
 * `old_text` to also show `new_text`, or NULL when the view is already
 * current. Either argument may be NULL (treated as ""). The result is
 * freshly allocated; free it with `am_screen_text_free`. Newlines are
 * normalized to CRLF; a redraw/reflow replays after an ESC[2J ESC[H
 * clear prefix. */
char *am_feed_delta(const char *old_text, const char *new_text);

/* True (1) when a roster row with this title/project/id passes the
 * sidebar `query` (case-insensitive substring; blank query passes
 * everything), else false (0). NULL means empty; invalid UTF-8 reports
 * false and records a message. */
int32_t am_roster_matches(const char *title, const char *project,
                          const char *id, const char *query);

/* Owned glanceable age label for `then_secs` (unix seconds) relative to
 * `now_secs` (`just now` / `Nm ago` / `Nh ago` / `Nd ago`); free with
 * `am_screen_text_free`. */
char *am_relative_age(int64_t now_secs, int64_t then_secs);

/* Total link count (PR + related) of roster row `row`, or -1 on null
 * handle / out-of-bounds row. */
int32_t am_link_count(const AmCore *core, size_t row);

/* Unix seconds of `last_active` for roster row `row`, or -1 on null
 * handle / out-of-bounds row (feed to `am_relative_age` with the
 * shell's own clock). */
int64_t am_last_active(const AmCore *core, size_t row);

/* Last error message for this thread (UTF-8, NUL-terminated). Never
 * NULL; valid until the next failing `am_*` call on this thread. */
const char *am_last_error(void);

/* Free a PTY from `am_spawn` (reaps the child). NULL is a no-op. */
void am_pty_free(AmPty *pty);

/* Live-run ceiling shared by every shell (was `gui::runs::MAX_LIVE_RUNS`,
 * hard-coded `10` in two native shells, unbounded on macOS). */
size_t am_max_runs(void);

/* Number of live (attached) PTYs in the registry. Null core yields 0. */
size_t am_live_count(const AmCore *core);

/* True when the row id owns a live PTY in the registry. Null-safe:
 * null core/id yields false. */
bool am_is_live(const AmCore *core, const char *id);

/* Pump every live run: feed output, rescan attention + links for changed
 * runs, reclassify statuses, re-sort pinned to selection. Returns true
 * when anything visible changed — the shell's only repaint gate (and its
 * roster/status refresh gate: statuses now actually move, unlike the
 * launch-snapshot rows the old shells polled). Null is false, never UB. */
bool am_pump_all(AmCore *core);

/* Spawn a 2D-launch session and attach it to a new roster row, under the
 * shared live-run cap. On success the row id is written to `id_out`
 * (up to `id_cap` bytes incl. NUL; truncated otherwise) and `AM_OK`
 * returns. At the cap the oldest-exited run is reaped first; when every
 * live run is still running this refuses with `AM_SPAWN` and a message
 * naming per-run close. `cli`/`cwd` follow the `am_spawn_launch`
 * convention; `yolo` is tri-state. */
int32_t am_run_spawn(AmCore *core, const char *cli, const char *cwd,
                     int32_t yolo, uint16_t cols, uint16_t rows,
                     char *id_out, size_t id_cap);

/* Spawn a PTY and attach it to an existing roster row (restart a dead
 * run, resume a historic entry): drops the dead PTY if any, spawns the
 * row's spawn kind on the same id, attaches on success. Unknown or live
 * rows report `AM_SPAWN`; the row keeps its title and links. */
int32_t am_run_restart(AmCore *core, const char *id, uint16_t cols,
                       uint16_t rows);

/* Close (kill) a run: drop its live PTY and remove its entry. Unknown
 * ids are a no-op success. */
int32_t am_run_close(AmCore *core, const char *id);

/* True when quitting deserves a confirmation step: any Working/Attention
 * row or any live (non-exited) PTY. Null core yields false. */
bool am_needs_quit_confirm(const AmCore *core);

/* Pump one attached run by id (feed output into its emulator). Returns
 * true when the screen may have changed. Unknown ids and nulls yield
 * false, never UB. */
bool am_run_pump(AmCore *core, const char *id);

/* Forward raw bytes (already key-encoded by the shell) to an attached
 * run's child. Returns an `AmError` code. */
int32_t am_run_write(AmCore *core, const char *id, const uint8_t *data,
                     size_t len);

/* Resize an attached run's PTY and emulator grid. Unknown ids and nulls
 * are no-ops. */
void am_run_resize(AmCore *core, const char *id, uint16_t cols,
                   uint16_t rows);

/* Owned UTF-8 snapshot of an attached run's emulated screen. Null on
 * null handle or unknown id; free with `am_screen_text_free`. */
char *am_run_screen_text(const AmCore *core, const char *id);

/* Owned styled spans of an attached run's screen as JSON (same shape as
 * `am_spans_json`). Null on null handle or unknown id; free with
 * `am_screen_text_free`. */
char *am_run_spans_json(const AmCore *core, const char *id);

/* True once an attached run's child has exited. Unknown ids and nulls
 * yield false. */
bool am_run_exited(const AmCore *core, const char *id);

/* Encode one logical keypress into child bytes — the single key table
 * every shell shares (ports of per-shell tables are deleted).
 * `key`/`key_char` are NUL-terminated UTF-8 (`key_char` may be null);
 * `ctrl`/`alt` are 0/1. On Forward the bytes land in `bytes_out` (up to
 * `cap` bytes) and the count returns; on Keep 0 returns (leave to the
 * native control). Null `key` returns -1. */
int32_t am_key_encode(const char *key, const char *key_char, int ctrl,
                      int alt, uint8_t *bytes_out, size_t cap);

/* Render an `am_spans_json` document to an SGR stream (styled-span
 * renderer), or null when it does not decode (the pump then falls back
 * to the plain-text snapshot). Free with `am_screen_text_free`. */
char *am_ansi_render(const char *json);

/* One-line spawn preview (`runs: muse in ~/api + yolo`). `cli`/`folder`
 * may be null (= default/inherit); `yolo` is the tri-state int. Free with
 * `am_screen_text_free`. */
char *am_spawn_preview(const char *cli, const char *folder, int32_t yolo);

/* Tri-state yolo int from a segmented-control index (1 = force on,
 * 2 = force off, else config default). */
int32_t am_yolo_value(int selected);

/* Human age for `last_active` (`just now`, `5m ago`, ...). Free with
 * `am_screen_text_free`. */
char *am_age_string(int64_t now_unix, int64_t then_unix);

/* Non-color status marker for a status code (`●`/`◐`/`○`). Free with
 * `am_screen_text_free`. */
char *am_status_glyph(int32_t code);

/* Section header for a status code (`Needs input`/`Idle`/`Working`).
 * Free with `am_screen_text_free`. */
char *am_section_title(int32_t code);

/* Clamp a sidebar width into the shared 220..480px range. */
double am_clamp_sidebar(double px);

/* True when roster row `row` passes the sidebar filter `query`
 * (case-insensitive substring over title/project/id). Null-safe: null
 * core/query yields false; out-of-bounds yields false. */
bool am_row_matches(const AmCore *core, size_t row, const char *query);

/* Replace the title filter, snapping the selection into the matches.
 * Null query clears. */
void am_set_filter(AmCore *core, const char *query);

/* Selected roster row index (the shell highlights this row). */
size_t am_selected(const AmCore *core);

/* Move selection to row `row` (clamped into range). */
void am_select(AmCore *core, size_t row);

/* Step selection next/prev (`forward` nonzero = next), wrapping within
 * the current filter matches — the same rule as the gpui list keys. */
void am_select_step(AmCore *core, int forward);

#ifdef __cplusplus
}
#endif

#endif /* AGENT_MANAGER_H */

/* Two-dimensional new-session picker for the Linux GTK4 shell.
 *
 * Pure-logic halves live in picker.c (GTK-free, meson-tested); the
 * dialog below renders them natively. Declared here so main.c stays
 * focused on the roster/terminal shell.
 */

#ifndef AM_PICKER_H
#define AM_PICKER_H

#include <stddef.h>

/* One CLI catalog row (parsed from `bridge_clis_json`). `path` is NULL
 * when the CLI is missing (render disabled with an install hint). */
typedef struct {
    char *id;
    char *program;
    char *path; /* NULL when unavailable */
    int available;
} PickerCli;

void picker_clis_free(PickerCli *clis, size_t n);
void picker_recents_free(char **recents, size_t n);

/* Parse helpers (pure, tested without a display). Never NULL on
 * allocation success; malformed input yields zero rows. */
PickerCli *picker_parse_clis(const char *json, size_t *n_out);
char **picker_parse_recents(const char *json, size_t *n_out);

/* Blank/whitespace folder means inherit (NULL). Returns a malloc'd
 * trimmed copy, or NULL for inherit. */
char *picker_effective_folder(const char *folder);

/* Tri-state yolo int for `bridge_spawn_launch` from the segmented
 * control index (1 = force on, 2 = force off, else config default). */
int picker_yolo_value(int selected);

/* One-line spawn preview (`runs: muse in ~/api + yolo`). Malloc'd. */
char *picker_preview(const char *cli, const char *folder, int yolo);

#endif /* AM_PICKER_H */

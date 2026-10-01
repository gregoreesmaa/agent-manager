/* Two-dimensional new-session picker for the Linux GTK4 shell.
 *
 * Folder × CLI + one-shot yolo over the core 2D-launch C ABI:
 * `bridge_clis_json` (autodetected catalog), `bridge_recent_json`
 * (folder recents), `bridge_spawn_launch` (explicit spawn),
 * `bridge_note_launch` (repeat-last memory). Split-button contract
 * (see docs/new-session-picker.md): the header "New run" button keeps
 * repeating the last launch instantly; the picker's dialog confirms an
 * explicit combination.
 *
 * Pure-logic halves (`picker_parse_clis`, `picker_parse_recents`,
 * `picker_preview`, `picker_effective_folder`, `picker_yolo_value`)
 * are GTK-free so the meson `picker` test pins them without a display.
 */

#include "picker.h"

#include <ctype.h>
#include <stdlib.h>
#include <string.h>

/* ------------------------------------------------------------------ */
/* Pure logic (unit-tested, no GTK).                                   */
/* ------------------------------------------------------------------ */

void picker_clis_free(PickerCli *clis, size_t n) {
    if (!clis) {
        return;
    }
    for (size_t i = 0; i < n; i++) {
        free(clis[i].id);
        free(clis[i].program);
        free(clis[i].path);
    }
    free(clis);
}

void picker_recents_free(char **recents, size_t n) {
    if (!recents) {
        return;
    }
    for (size_t i = 0; i < n; i++) {
        free(recents[i]);
    }
    free(recents);
}

/* Length of the next JSON string value (unescaped), or 0 when `p` does
 * not start a string. Used to pre-size decoded copies. */
static size_t json_string_len(const char *p, int *ok) {
    size_t len = 0;
    *ok = 1;
    if (*p != '"') {
        *ok = 0;
        return 0;
    }
    p++;
    for (; *p && *p != '"'; p++) {
        if (*p == '\\') {
            p++;
            if (*p == 'u') {
                p += 4;
            }
            if (!*p) {
                *ok = 0;
                return 0;
            }
        }
        len++;
    }
    if (*p != '"') {
        *ok = 0;
    }
    return len;
}

/* Decode one JSON string value into a malloc'd NUL-terminated copy.
 * Handles \" \\ \/ \b \f \n \r \t and \uXXXX (BMP -> UTF-8). */
static char *json_decode_string(const char *p) {
    int ok = 0;
    size_t len = json_string_len(p, &ok);
    if (!ok) {
        return NULL;
    }
    char *out = malloc(len + 1);
    if (!out) {
        return NULL;
    }
    size_t n = 0;
    p++; /* opening quote */
    for (; *p && *p != '"'; p++) {
        char c = *p;
        if (c == '\\') {
            p++;
            switch (*p) {
            case '"': c = '"'; break;
            case '\\': c = '\\'; break;
            case '/': c = '/'; break;
            case 'b': c = '\b'; break;
            case 'f': c = '\f'; break;
            case 'n': c = '\n'; break;
            case 'r': c = '\r'; break;
            case 't': c = '\t'; break;
            case 'u': {
                unsigned cp = 0;
                for (int i = 1; i <= 4; i++) {
                    char h = p[i];
                    cp <<= 4;
                    if (h >= '0' && h <= '9') {
                        cp |= (unsigned)(h - '0');
                    } else if (h >= 'a' && h <= 'f') {
                        cp |= (unsigned)(h - 'a' + 10);
                    } else if (h >= 'A' && h <= 'F') {
                        cp |= (unsigned)(h - 'A' + 10);
                    } else {
                        free(out);
                        return NULL;
                    }
                }
                p += 4;
                if (cp < 0x80) {
                    out[n++] = (char)cp;
                } else if (cp < 0x800) {
                    out[n++] = (char)(0xC0 | (cp >> 6));
                    out[n++] = (char)(0x80 | (cp & 0x3F));
                } else {
                    out[n++] = (char)(0xE0 | (cp >> 12));
                    out[n++] = (char)(0x80 | ((cp >> 6) & 0x3F));
                    out[n++] = (char)(0x80 | (cp & 0x3F));
                }
                continue;
            }
            default: c = *p; break;
            }
            if (!*p) {
                free(out);
                return NULL;
            }
        }
        out[n++] = c;
    }
    out[n] = '\0';
    return out;
}

/* Find the raw value span of top-level key `key` inside flat object
 * `obj` (NUL-terminated `{...}` span). Returns a pointer to the first
 * non-blank value byte, or NULL when absent. */
static const char *obj_value(const char *obj, const char *key) {
    char pat[128];
    snprintf(pat, sizeof pat, "\"%s\"", key);
    const char *p = strstr(obj, pat);
    if (!p) {
        return NULL;
    }
    p = strchr(p + strlen(pat), ':');
    if (!p) {
        return NULL;
    }
    p++;
    while (*p == ' ' || *p == '\t' || *p == '\n' || *p == '\r') {
        p++;
    }
    return p;
}

/* Split a top-level JSON array of flat objects into NUL-terminated
 * `{...}` spans (*n_out count). Stops at the first malformed span. */
static char **split_objects(const char *json, size_t *n_out) {
    *n_out = 0;
    const char *p = strchr(json, '[');
    if (!p) {
        return NULL;
    }
    p++;
    size_t cap = 4, n = 0;
    char **out = malloc(cap * sizeof *out);
    if (!out) {
        return NULL;
    }
    while (*p && *p != ']') {
        while (*p == ' ' || *p == '\t' || *p == '\n' || *p == '\r' ||
               *p == ',') {
            p++;
        }
        if (*p != '{') {
            break;
        }
        const char *start = p;
        int depth = 0, in_str = 0, esc = 0;
        for (; *p; p++) {
            char c = *p;
            if (in_str) {
                if (esc) {
                    esc = 0;
                } else if (c == '\\') {
                    esc = 1;
                } else if (c == '"') {
                    in_str = 0;
                }
            } else if (c == '"') {
                in_str = 1;
            } else if (c == '{') {
                depth++;
            } else if (c == '}') {
                if (--depth == 0) {
                    p++;
                    break;
                }
            }
        }
        if (depth != 0) {
            break;
        }
        size_t len = (size_t)(p - start);
        char *span = malloc(len + 1);
        if (!span) {
            break;
        }
        memcpy(span, start, len);
        span[len] = '\0';
        if (n == cap) {
            cap *= 2;
            char **nb = realloc(out, cap * sizeof *out);
            if (!nb) {
                free(span);
                break;
            }
            out = nb;
        }
        out[n++] = span;
    }
    *n_out = n;
    return out;
}

/* Parse the `am_clis_json` catalog into PickerCli rows (core order).
 * Missing `available` reads false; missing strings read "". Never
 * NULL on allocation success (empty catalog yields n=0 with a
 * non-NULL array). */
PickerCli *picker_parse_clis(const char *json, size_t *n_out) {
    *n_out = 0;
    if (!json) {
        return calloc(1, sizeof(PickerCli));
    }
    size_t nspans = 0;
    char **spans = split_objects(json, &nspans);
    PickerCli *out = calloc(nspans ? nspans : 1, sizeof *out);
    if (!out) {
        if (spans) {
            for (size_t i = 0; i < nspans; i++) {
                free(spans[i]);
            }
            free(spans);
        }
        return NULL;
    }
    size_t n = 0;
    for (size_t i = 0; i < nspans; i++) {
        const char *id = obj_value(spans[i], "id");
        const char *program = obj_value(spans[i], "program");
        const char *path = obj_value(spans[i], "path");
        const char *avail = obj_value(spans[i], "available");
        out[n].id = (id && *id == '"') ? json_decode_string(id) : strdup("");
        out[n].program =
            (program && *program == '"') ? json_decode_string(program) : strdup("");
        out[n].path =
            (path && *path == '"') ? json_decode_string(path) : NULL;
        out[n].available = (avail && strncmp(avail, "true", 4) == 0) ? 1 : 0;
        if (!out[n].id || !out[n].program) {
            free(out[n].id);
            free(out[n].program);
            free(out[n].path);
            continue;
        }
        n++;
    }
    if (spans) {
        for (size_t i = 0; i < nspans; i++) {
            free(spans[i]);
        }
        free(spans);
    }
    *n_out = n;
    return out;
}

/* Parse the `am_recent_json` string array into malloc'd items. */
char **picker_parse_recents(const char *json, size_t *n_out) {
    *n_out = 0;
    if (!json) {
        return calloc(1, sizeof(char *));
    }
    const char *p = strchr(json, '[');
    if (!p) {
        return calloc(1, sizeof(char *));
    }
    p++;
    size_t cap = 4, n = 0;
    char **out = malloc(cap * sizeof *out);
    if (!out) {
        return NULL;
    }
    while (*p && *p != ']') {
        while (*p == ' ' || *p == '\t' || *p == '\n' || *p == '\r' ||
               *p == ',') {
            p++;
        }
        if (*p != '"') {
            break;
        }
        char *val = json_decode_string(p);
        if (!val) {
            break;
        }
        /* Advance past the decoded span. */
        int esc = 0;
        p++; /* opening quote */
        for (; *p; p++) {
            if (esc) {
                esc = 0;
            } else if (*p == '\\') {
                esc = 1;
            } else if (*p == '"') {
                p++;
                break;
            }
        }
        if (n == cap) {
            cap *= 2;
            char **nb = realloc(out, cap * sizeof *out);
            if (!nb) {
                free(val);
                break;
            }
            out = nb;
        }
        out[n++] = val;
    }
    *n_out = n;
    return out;
}

/* Blank/whitespace folder means inherit (NULL). Returns a malloc'd
 * trimmed copy, or NULL for inherit. */
char *picker_effective_folder(const char *folder) {
    if (!folder) {
        return NULL;
    }
    while (*folder && isspace((unsigned char)*folder)) {
        folder++;
    }
    size_t len = strlen(folder);
    while (len > 0 && isspace((unsigned char)folder[len - 1])) {
        len--;
    }
    if (len == 0) {
        return NULL;
    }
    char *out = malloc(len + 1);
    if (!out) {
        return NULL;
    }
    memcpy(out, folder, len);
    out[len] = '\0';
    return out;
}

/* Tri-state yolo int for `am_spawn_launch`: segmented index 1 = force
 * on (+1), 2 = force off (-1), else the config default (0). */
int picker_yolo_value(int selected) {
    if (selected == 1) {
        return 1;
    }
    if (selected == 2) {
        return -1;
    }
    return 0;
}

/* One-line spawn preview (`runs: muse in ~/api + yolo`). `folder` is
 * the raw entry text (blank = inherit). `yolo` is the tri-state int.
 * Returns a malloc'd string. */
char *picker_preview(const char *cli, const char *folder, int yolo) {
    const char *c = (cli && *cli) ? cli : "muse";
    char *eff = picker_effective_folder(folder);
    const char *yolo_tag = "";
    if (yolo > 0) {
        yolo_tag = " + yolo";
    } else if (yolo < 0) {
        yolo_tag = " (yolo off)";
    }
    char *out;
    if (eff) {
        size_t n = strlen("runs: ") + strlen(c) + strlen(" in ") +
                   strlen(eff) + strlen(yolo_tag) + 1;
        out = malloc(n);
        if (out) {
            snprintf(out, n, "runs: %s in %s%s", c, eff, yolo_tag);
        }
        free(eff);
    } else {
        size_t n = strlen("runs: ") + strlen(c) + strlen(yolo_tag) + 1;
        out = malloc(n);
        if (out) {
            snprintf(out, n, "runs: %s%s", c, yolo_tag);
        }
    }
    return out ? out : strdup("runs: muse");
}

#ifdef AM_PICKER_TEST
#include <stdio.h>

static int failures = 0;
#define CHECK(cond, what)                                  \
    do {                                                   \
        if (!(cond)) {                                     \
            fprintf(stderr, "PICKER-FAIL %s\n", what);     \
            failures++;                                    \
        }                                                  \
    } while (0)

int main(void) {
    /* Catalog parse: core order, availability, paths, missing rows. */
    const char *catalog =
        "[{\"id\":\"muse\",\"program\":\"muse\",\"path\":\"/bin/muse\","
        "\"available\":true},"
        "{\"id\":\"claude\",\"program\":\"claude\",\"path\":null,"
        "\"available\":false}]";
    size_t n = 0;
    PickerCli *clis = picker_parse_clis(catalog, &n);
    CHECK(clis && n == 2, "catalog count");
    CHECK(strcmp(clis[0].id, "muse") == 0, "catalog order");
    CHECK(clis[0].available == 1, "muse available");
    CHECK(clis[0].path && strcmp(clis[0].path, "/bin/muse") == 0,
          "muse path");
    CHECK(strcmp(clis[1].id, "claude") == 0, "claude second");
    CHECK(clis[1].available == 0, "claude missing");
    CHECK(clis[1].path == NULL, "missing path null");
    picker_clis_free(clis, n);
    /* Empty catalog yields zero rows, never NULL. */
    size_t n0 = 99;
    PickerCli *none = picker_parse_clis("[]", &n0);
    CHECK(none && n0 == 0, "empty catalog");
    picker_clis_free(none, n0);
    /* Recents parse. */
    size_t nr = 0;
    char **recents = picker_parse_recents("[\"/tmp/api\",\"/tmp/b\"]", &nr);
    CHECK(recents && nr == 2, "recents count");
    CHECK(strcmp(recents[0], "/tmp/api") == 0, "recents order");
    picker_recents_free(recents, nr);
    /* Folder: blank/whitespace inherits, else trimmed. */
    CHECK(picker_effective_folder("") == NULL, "blank inherits");
    CHECK(picker_effective_folder("   ") == NULL, "spaces inherit");
    char *f = picker_effective_folder("  /tmp/api  ");
    CHECK(f && strcmp(f, "/tmp/api") == 0, "folder trimmed");
    free(f);
    /* Yolo tri-state mapping. */
    CHECK(picker_yolo_value(0) == 0, "yolo default");
    CHECK(picker_yolo_value(1) == 1, "yolo on");
    CHECK(picker_yolo_value(2) == -1, "yolo off");
    /* Preview names the combination. */
    char *p = picker_preview("muse", "/tmp/api", 1);
    CHECK(p && strcmp(p, "runs: muse in /tmp/api + yolo") == 0,
          "preview full");
    free(p);
    char *p2 = picker_preview("claude", "", 0);
    CHECK(p2 && strcmp(p2, "runs: claude") == 0, "preview bare");
    free(p2);
    char *p3 = picker_preview("muse", NULL, -1);
    CHECK(p3 && strcmp(p3, "runs: muse (yolo off)") == 0, "preview off");
    free(p3);
    if (failures == 0) {
        printf("PICKER-OK\n");
    }
    return failures ? 1 : 0;
}
#endif /* AM_PICKER_TEST */

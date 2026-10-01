/* Agent Manager — Linux GTK4/libadwaita shell over the core C ABI.
 *
 * Issue #63, second native shell after swift/ (#62). Every run feature goes
 * through `core_bridge.h` (which wraps `include/agent_manager.h`): the core
 * owns the roster, the PTYs, and the emulator; this shell owns widgets.
 *
 * Epic DoD wiring (mirrors swift/README.md's table):
 *   roster      am_session_count + am_session_json at launch, am_status ticks
 *   spawn       toolbar New-run button -> bridge_spawn (current VTE grid)
 *   converse    key controller -> bridge_write; pump -> am_feed_delta (core) -> VTE
 *   copy/paste  native VTE selection + Ctrl+Shift+C/V + right-click menu
 *   scroll      VTE scrollback (capped) in a GtkScrolledWindow
 *   search      sidebar filter entry; terminal find bar (Ctrl+F, VTE search)
 *   history     rows show project/harness/age, restored every launch;
 *               per-run scrollback retained in the VTE
 *   theme       System/Dark/Light (libadwaita + VTE palette), plain-file pref
 *   persist     Save button + close hook -> bridge_core_save
 *
 * The core owns its emulator; the VTE widget owns a second one fed with
 * snapshot deltas (the core reconciler in `src/shell_shared.rs`, bound as
 * `am_feed_delta`). No PTY is ever
 * spawned inside VTE: typed keys are encoded by the shell and forwarded
 * with bridge_write, and echoed output arrives via the pump. This keeps
 * exactly one line discipline (the core's) so nothing double-echoes.
 */

#define _POSIX_C_SOURCE 200809L /* strdup under strict C11 */

#include <ctype.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#include <adwaita.h>
#include <gtk/gtk.h>
#include <vte/vte.h>

#include "core_bridge.h"
<<<<<<< HEAD
#include "feed.h"
#include "picker.h"
=======
/* am_feed_delta / am_roster_matches / am_relative_age come from the core
 * itself now (`include/agent_manager.h`, via core_bridge.h): the vendored
 * feed.c port is gone, so every C shell reconciles through
 * `src/shell_shared.rs`. */
>>>>>>> 204d41b (Windows/macOS UX parity over shared core helpers)

/* Bounded per-session VTE scrollback (local-only trust + bounded growth:
 * an accumulate-forever buffer would leak memory over long agent runs). */
#define SCROLLBACK_LINES 10000
/* Roster/status refresh rides on the same 50ms pump tick as the Swift shell. */
#define PUMP_MS 50
#define DEFAULT_COLS 80
#define DEFAULT_ROWS 24

/* ------------------------------------------------------------------ */
/* Minimal JSON field extraction (roster rows are ChatSession objects). */
/* ------------------------------------------------------------------ */

/* Find the value of top-level string key `key` in a flat JSON object.
 * Returns a malloc'd unescaped string, or NULL when absent/non-string. */
static char *json_string(const char *json, const char *key) {
    char pat[128];
    snprintf(pat, sizeof pat, "\"%s\"", key);
    const char *p = json;
    while ((p = strstr(p, pat)) != NULL) {
        p += strlen(pat);
        while (*p == ' ' || *p == '\t' || *p == '\n' || *p == '\r') {
            p++;
        }
        if (*p != ':') {
            continue;
        }
        p++;
        while (*p == ' ' || *p == '\t' || *p == '\n' || *p == '\r') {
            p++;
        }
        if (*p != '"') {
            return NULL; /* present but not a string (e.g. null cwd) */
        }
        p++;
        /* First pass: decoded length. */
        size_t cap = 64, len = 0;
        char *out = malloc(cap);
        if (!out) {
            return NULL;
        }
        int ok = 1;
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
                    /* Basic-plane \uXXXX -> UTF-8 (enough for titles). */
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
                            ok = 0;
                        }
                    }
                    if (!ok) {
                        break;
                    }
                    p += 4;
                    char enc[3];
                    int n = 0;
                    if (cp < 0x80) {
                        enc[n++] = (char)cp;
                    } else if (cp < 0x800) {
                        enc[n++] = (char)(0xC0 | (cp >> 6));
                        enc[n++] = (char)(0x80 | (cp & 0x3F));
                    } else {
                        enc[n++] = (char)(0xE0 | (cp >> 12));
                        enc[n++] = (char)(0x80 | ((cp >> 6) & 0x3F));
                        enc[n++] = (char)(0x80 | (cp & 0x3F));
                    }
                    for (int i = 0; i < n; i++) {
                        if (len + 1 >= cap) {
                            cap *= 2;
                            char *nb = realloc(out, cap);
                            if (!nb) {
                                ok = 0;
                                break;
                            }
                            out = nb;
                        }
                        out[len++] = enc[i];
                    }
                    continue;
                }
                default:
                    c = *p; /* lenient: keep unknown escapes literally */
                    break;
                }
                if (*p == '\0' || !ok) {
                    break;
                }
            }
            if (!ok) {
                break;
            }
            if (len + 1 >= cap) {
                cap *= 2;
                char *nb = realloc(out, cap);
                if (!nb) {
                    ok = 0;
                    break;
                }
                out = nb;
            }
            out[len++] = c;
        }
        if (!ok || *p != '"') {
            free(out);
            return NULL;
        }
        out[len] = '\0';
        return out;
    }
    return NULL;
}

static long long json_int(const char *json, const char *key, long long dflt) {
    char pat[128];
    snprintf(pat, sizeof pat, "\"%s\"", key);
    const char *p = strstr(json, pat);
    if (!p) {
        return dflt;
    }
    p = strchr(p + strlen(pat), ':');
    if (!p) {
        return dflt;
    }
    return atoll(p + 1);
}

/* ------------------------------------------------------------------ */
/* Model. */
/* ------------------------------------------------------------------ */

typedef struct {
    char *id;
    char *title;
    char *project;
    char *harness;
    int status; /* bridge_status code: 0 attention, 1 idle, 2 working */
    long long last_active;
} SessionRow;

static void row_free(SessionRow *r) {
    if (!r) {
        return;
    }
    free(r->id);
    free(r->title);
    free(r->project);
    free(r->harness);
    free(r);
}

typedef struct {
    AmPty *pty;
    char *fed; /* snapshot already shown in the VTE ("" = nothing yet) */
} LivePty;

static void live_free(LivePty *l) {
    if (!l) {
        return;
    }
    bridge_pty_free(l->pty);
    free(l->fed);
    free(l);
}

typedef struct {
    AdwApplication *app;
    AmCore *core;
    GPtrArray *rows;           /* SessionRow*, launch snapshot order */
    GHashTable *live;          /* id -> LivePty* */
    GHashTable *row_widgets;   /* id -> GtkWidget* (badge label) */
    char *selected;            /* selected row id */
    GtkListBox *list;
    GtkSearchEntry *filter;
    VteTerminal *term;
    AdwWindowTitle *header_title;
    GtkButton *spawn_btn;
    GtkButton *save_btn;
    GtkDropDown *theme_drop;
    GtkRevealer *find_revealer;
    GtkSearchEntry *find_entry;
    GtkScrolledWindow *term_scroll;
    int cols, rows_grid;
    char *theme_path; /* plain-file theme pref (local-only, no gsettings) */
} Shell;

static const char *status_label(int st) {
    switch (st) {
    case 0: return "Needs input";
    case 2: return "Active";
    default: return "Idle";
    }
}

/* Relative-age label for a roster row: the shared core label
 * (`am_relative_age` in `src/shell_shared.rs`), so every shell renders
 * the same glanceable text. Owned; free with free(). */
static char *age_string(long long epoch) {
    return am_relative_age((int64_t)time(NULL), (int64_t)epoch);
}

/* ------------------------------------------------------------------ */
/* Theme (libadwaita chrome + VTE palette), plain-file pref. */
/* ------------------------------------------------------------------ */

static void apply_theme(Shell *sh, int idx) {
    AdwStyleManager *sm = adw_style_manager_get_default();
    AdwColorScheme scheme = ADW_COLOR_SCHEME_DEFAULT;
    if (idx == 1) {
        scheme = ADW_COLOR_SCHEME_FORCE_DARK;
    } else if (idx == 2) {
        scheme = ADW_COLOR_SCHEME_FORCE_LIGHT;
    }
    adw_style_manager_set_color_scheme(sm, scheme);

    /* VTE palette follows the same choice so the terminal never ends up
     * dark-on-dark or light-on-light against the window chrome. */
    GdkRGBA fg, bg;
    if (scheme == ADW_COLOR_SCHEME_FORCE_LIGHT ||
        (scheme == ADW_COLOR_SCHEME_DEFAULT &&
         !adw_style_manager_get_dark(sm))) {
        gdk_rgba_parse(&fg, "#1e1e1e");
        gdk_rgba_parse(&bg, "#ffffff");
    } else {
        gdk_rgba_parse(&fg, "#e6e6e6");
        gdk_rgba_parse(&bg, "#1e1e2e");
    }
    vte_terminal_set_colors(sh->term, &fg, &bg, NULL, 0);

    if (sh->theme_path) {
        char v[2] = { (char)('0' + idx), '\0' };
        g_file_set_contents(sh->theme_path, v, -1, NULL);
    }
}

static int load_theme_pref(Shell *sh) {
    char *contents = NULL;
    int idx = 0;
    if (g_file_get_contents(sh->theme_path, &contents, NULL, NULL) &&
        contents[0] >= '0' && contents[0] <= '2') {
        idx = contents[0] - '0';
    }
    g_free(contents);
    return idx;
}

/* ------------------------------------------------------------------ */
/* Roster. */
/* ------------------------------------------------------------------ */

static void reload_statuses(Shell *sh) {
    for (guint i = 0; i < sh->rows->len; i++) {
        SessionRow *r = sh->rows->pdata[i];
        int st = bridge_status(sh->core, i);
        if (st < 0) {
            continue;
        }
        r->status = st;
        GtkWidget *badge = g_hash_table_lookup(sh->row_widgets, r->id);
        if (badge) {
            gtk_label_set_text(GTK_LABEL(badge), status_label(st));
        }
    }
    int attention = 0;
    for (guint i = 0; i < sh->rows->len; i++) {
        if (((SessionRow *)sh->rows->pdata[i])->status == 0) {
            attention++;
        }
    }
    SessionRow *sel = NULL;
    for (guint i = 0; i < sh->rows->len; i++) {
        SessionRow *r = sh->rows->pdata[i];
        if (sh->selected && strcmp(r->id, sh->selected) == 0) {
            sel = r;
            break;
        }
    }
    char *sub;
    if (sel) {
        char *age = age_string(sel->last_active);
        LivePty *lp = g_hash_table_lookup(sh->live, sel->id);
        sub = g_strdup_printf("%s · %s · %s%s%s%s", sel->project,
                              sel->harness, age ? age : "",
                              lp ? " · live" : "",
                              attention ? " · needs input: " : "",
                              attention ? "" : "");
        free(age);
        if (attention) {
            char *with_n = g_strdup_printf("%s%d", sub, attention);
            g_free(sub);
            sub = with_n;
        }
    } else {
        sub = g_strdup_printf("%u runs%s", sh->rows->len,
                              attention ? "" : "");
        if (attention) {
            char *with_n = g_strdup_printf("%s · %d need input", sub, attention);
            g_free(sub);
            sub = with_n;
        }
    }
    adw_window_title_set_subtitle(sh->header_title, sub);
    g_free(sub);
}

/* Sidebar filter: the shared core match (`am_roster_matches` in
 * `src/shell_shared.rs`) — case-insensitive title/project/id, blank
 * passes — so every shell filters identically. */
static gboolean row_matches(GtkListBoxRow *row, gpointer data) {
    Shell *sh = data;
    const char *q = gtk_editable_get_text(GTK_EDITABLE(sh->filter));
    SessionRow *r = g_object_get_data(G_OBJECT(row), "am-row");
    if (!r) {
        return TRUE;
    }
    return am_roster_matches(r->title, r->project, r->id, q ? q : "");
}

/* Order: needs-input first, then active, then idle; recent first within. */
static int row_order(GtkListBoxRow *a, GtkListBoxRow *b, gpointer data) {
    (void)data;
    SessionRow *ra = g_object_get_data(G_OBJECT(a), "am-row");
    SessionRow *rb = g_object_get_data(G_OBJECT(b), "am-row");
    if (!ra || !rb) {
        return 0;
    }
    int pa = ra->status == 0 ? 0 : ra->status == 2 ? 1 : 2;
    int pb = rb->status == 0 ? 0 : rb->status == 2 ? 1 : 2;
    if (pa != pb) {
        return pa - pb;
    }
    if (ra->last_active != rb->last_active) {
        return ra->last_active < rb->last_active ? 1 : -1;
    }
    return strcmp(ra->title, rb->title);
}

/* ------------------------------------------------------------------ */
/* Forward declarations for callbacks. */
/* ------------------------------------------------------------------ */

static void show_selected_in_terminal(Shell *sh);
static gboolean pump_tick(gpointer data);

/* ------------------------------------------------------------------ */
/* Spawn / converse. */
/* ------------------------------------------------------------------ */

static void toast(Shell *sh, const char *msg) {
    GtkWindow *win =
        gtk_application_get_active_window(GTK_APPLICATION(sh->app));
    GtkWidget *overlay = g_object_get_data(G_OBJECT(win), "am-toast-overlay");
    if (overlay) {
        adw_toast_overlay_add_toast(ADW_TOAST_OVERLAY(overlay),
                                    adw_toast_new(msg));
    }
}

static void update_spawn_state(Shell *sh) {
    /* Split-button repeat-last mints a local id, so both header buttons
     * stay sensitive with or without a selection (the empty roster is a
     * starting point, not a dead end). */
    (void)sh;
}

static void on_spawn(GtkButton *btn, gpointer data) {
    (void)btn;
    Shell *sh = data;
    /* Split-button main: repeat the last launch instantly (the
     * null-CLI/null-folder/zero-yolo form resolves the core's effective
     * default, so a picker-confirmed claude/yolo combo repeats here).
     * The fresh PTY mints a local id like the picker path: the core
     * roster snapshot is launch-time, so there is no row to attach to.
     * Local PTYs are capped (same 10-run ceiling as the macOS shell) so
     * one-click spawning cannot grow the live set without bound. */
    if (g_hash_table_size(sh->live) >= 10) {
        toast(sh, "At 10 live sessions — close one first.");
        return;
    }
    char *err = NULL;
    AmPty *pty = bridge_spawn_launch(sh->core, NULL, NULL, 0,
                                     (unsigned)sh->cols,
                                     (unsigned)sh->rows_grid, &err);
    if (!pty) {
        char *msg = g_strdup_printf("Could not start session: %s",
                                    err ? err : "unknown error");
        toast(sh, msg);
        g_free(msg);
        free(err);
        return;
    }
    char *id = g_strdup_printf("local-%u", g_random_int());
    LivePty *lp = calloc(1, sizeof *lp);
    lp->pty = pty;
    lp->fed = strdup("");
    g_hash_table_insert(sh->live, id, lp);
    free(sh->selected);
    sh->selected = strdup(id);
    bridge_note_launch(sh->core, NULL, NULL, NULL);
    /* Name the effective CLI so the repeat is verifiable (the null
     * form resolves last-used / configured / autodetected). */
    char *eff = bridge_effective_cli(sh->core, NULL);
    char *preview = picker_preview(eff ? eff : NULL, NULL, 0);
    bridge_string_free(eff);
    toast(sh, preview ? preview : "Session started.");
    free(preview);
    show_selected_in_terminal(sh);
    update_spawn_state(sh);
    reload_statuses(sh);
}

/* ------------------------------------------------------------------ */
/* 2D new-session picker (folder x CLI + yolo).                        */
/* ------------------------------------------------------------------ */

typedef struct {
    Shell *sh;
    PickerCli *clis;
    size_t n_clis;
    GtkCheckButton **cli_btns;
    GtkEntry *folder;
    GtkDropDown *recent;
    char **recents;
    size_t n_recents;
    GtkDropDown *yolo;
    GtkLabel *preview;
    GtkLabel *error;
    AdwDialog *dialog;
} PickerUi;

/* Refresh the `runs: <cli> in <folder> + yolo` preview line from the
 * current widget state. */
static void picker_refresh_preview(PickerUi *pu) {
    const char *cli = "muse";
    for (size_t i = 0; i < pu->n_clis; i++) {
        if (gtk_check_button_get_active(pu->cli_btns[i])) {
            cli = pu->clis[i].id;
            break;
        }
    }
    const char *folder = gtk_editable_get_text(GTK_EDITABLE(pu->folder));
    int yolo =
        picker_yolo_value((int)gtk_drop_down_get_selected(pu->yolo));
    char *text = picker_preview(cli, folder, yolo);
    gtk_label_set_text(pu->preview, text ? text : "runs: muse");
    free(text);
}

static void picker_on_changed(PickerUi *pu) {
    gtk_label_set_text(pu->error, "");
    picker_refresh_preview(pu);
}

static void picker_cli_toggled(GtkCheckButton *btn, gpointer data) {
    if (!gtk_check_button_get_active(btn)) {
        return; /* the newly activated sibling refreshes */
    }
    picker_on_changed(data);
}

static void picker_folder_changed(GtkEditable *entry, gpointer data) {
    (void)entry;
    picker_on_changed(data);
}

static void picker_yolo_changed(GObject *obj, GParamSpec *pspec,
                                gpointer data) {
    (void)obj;
    (void)pspec;
    picker_on_changed(data);
}

/* A recent-folder pick refills the entry (which stays editable). */
static void picker_recent_changed(GObject *obj, GParamSpec *pspec,
                                  gpointer data) {
    (void)pspec;
    PickerUi *pu = data;
    guint idx = gtk_drop_down_get_selected(GTK_DROP_DOWN(obj));
    /* Index 0 is the "type a folder" placeholder; the rest are recents. */
    if (idx > 0 && idx - 1 < pu->n_recents) {
        gtk_editable_set_text(GTK_EDITABLE(pu->folder),
                              pu->recents[idx - 1]);
    }
    picker_on_changed(pu);
}

static void picker_free(PickerUi *pu) {
    if (!pu) {
        return;
    }
    picker_clis_free(pu->clis, pu->n_clis);
    picker_recents_free(pu->recents, pu->n_recents);
    free(pu->cli_btns);
    free(pu);
}

/* Spawn the confirmed combination: folder x CLI + one-shot yolo. A
 * missing folder refuses inline (the dialog stays open for a fix);
 * a spawn failure toasts and closes (the core error names the fix). */
static void picker_confirm(PickerUi *pu) {
    Shell *sh = pu->sh;
    char *folder_raw =
        g_strdup(gtk_editable_get_text(GTK_EDITABLE(pu->folder)));
    char *folder = picker_effective_folder(folder_raw);
    g_free(folder_raw);
    if (folder && !g_file_test(folder, G_FILE_TEST_IS_DIR)) {
        char *msg = g_strdup_printf("No such folder: %s", folder);
        gtk_label_set_text(pu->error, msg);
        g_free(msg);
        free(folder);
        return;
    }
    if (g_hash_table_size(sh->live) >= 10) {
        gtk_label_set_text(pu->error,
                           "At 10 live sessions — close one first.");
        free(folder);
        return;
    }
    const char *cli = "muse";
    for (size_t i = 0; i < pu->n_clis; i++) {
        if (gtk_check_button_get_active(pu->cli_btns[i])) {
            cli = pu->clis[i].id;
            break;
        }
    }
    int yolo =
        picker_yolo_value((int)gtk_drop_down_get_selected(pu->yolo));
    char *err = NULL;
    AmPty *pty = bridge_spawn_launch(sh->core, cli, folder, yolo,
                                     (unsigned)sh->cols,
                                     (unsigned)sh->rows_grid, &err);
    if (!pty) {
        char *msg = g_strdup_printf("Could not start session: %s",
                                    err ? err : "unknown error");
        toast(sh, msg);
        g_free(msg);
        free(err);
        free(folder);
        adw_dialog_close(pu->dialog);
        return;
    }
    /* Native shells mint their own live ids (the roster snapshot is
     * launch-time): track the PTY under a local id and select it. */
    char *id = g_strdup_printf("local-%u", g_random_int());
    LivePty *lp = calloc(1, sizeof *lp);
    lp->pty = pty;
    lp->fed = strdup("");
    g_hash_table_insert(sh->live, id, lp);
    free(sh->selected);
    sh->selected = strdup(id);
    bridge_note_launch(sh->core, cli, folder, NULL);
    char *preview = picker_preview(cli, folder, yolo);
    toast(sh, preview ? preview : "Session started.");
    free(preview);
    free(folder);
    adw_dialog_close(pu->dialog);
    show_selected_in_terminal(sh);
    update_spawn_state(sh);
    reload_statuses(sh);
}

static void picker_spawn_clicked(GtkButton *btn, gpointer data) {
    (void)btn;
    picker_confirm(data);
}

static void picker_closed(AdwDialog *dialog, gpointer data) {
    (void)dialog;
    picker_free(data);
}

/* Open the 2D picker dialog: folder entry + recents, CLI radio rows
 * (missing CLIs disabled with an install hint), yolo tri-state, and
 * the live `runs: ...` preview. Catalog + recents re-read on every
 * open so the list is never stale. */
static void on_pick_session(GtkButton *btn, gpointer data) {
    (void)btn;
    Shell *sh = data;
    GtkWindow *win =
        gtk_application_get_active_window(GTK_APPLICATION(sh->app));

    PickerUi *pu = calloc(1, sizeof *pu);
    if (!pu) {
        return;
    }
    pu->sh = sh;
    char *clis_json = bridge_clis_json();
    pu->clis = picker_parse_clis(clis_json ? clis_json : "[]", &pu->n_clis);
    bridge_string_free(clis_json);
    char *recents_json = bridge_recent_json(sh->core);
    pu->recents =
        picker_parse_recents(recents_json ? recents_json : "[]",
                             &pu->n_recents);
    bridge_string_free(recents_json);
    if (!pu->clis) {
        picker_free(pu);
        toast(sh, "Could not load the agent catalog.");
        return;
    }
    pu->cli_btns = calloc(pu->n_clis ? pu->n_clis : 1, sizeof *pu->cli_btns);
    if (!pu->cli_btns) {
        picker_free(pu);
        return;
    }

    AdwDialog *dialog = ADW_DIALOG(adw_dialog_new());
    pu->dialog = dialog;
    adw_dialog_set_title(dialog, "Start a new run");
    adw_dialog_set_content_width(dialog, 420);
    g_signal_connect(dialog, "closed", G_CALLBACK(picker_closed), pu);

    GtkWidget *box =
        gtk_box_new(GTK_ORIENTATION_VERTICAL, 12);
    gtk_widget_set_margin_start(box, 20);
    gtk_widget_set_margin_end(box, 20);
    gtk_widget_set_margin_top(box, 20);
    gtk_widget_set_margin_bottom(box, 20);

    GtkWidget *folder_label = gtk_label_new("Where should it work?");
    gtk_label_set_xalign(GTK_LABEL(folder_label), 0);
    gtk_widget_add_css_class(folder_label, "heading");
    gtk_box_append(GTK_BOX(box), folder_label);
    pu->folder = GTK_ENTRY(gtk_entry_new());
    gtk_entry_set_placeholder_text(pu->folder, "Blank = current folder");
    if (pu->n_recents > 0) {
        gtk_editable_set_text(GTK_EDITABLE(pu->folder), pu->recents[0]);
    }
    g_signal_connect(pu->folder, "changed",
                     G_CALLBACK(picker_folder_changed), pu);
    gtk_box_append(GTK_BOX(box), GTK_WIDGET(pu->folder));
    if (pu->n_recents > 0) {
        GtkStringList *recent_list =
            GTK_STRING_LIST(gtk_string_list_new(NULL));
        gtk_string_list_append(recent_list, "Type a folder…");
        for (size_t i = 0; i < pu->n_recents; i++) {
            gtk_string_list_append(recent_list, pu->recents[i]);
        }
        pu->recent = GTK_DROP_DOWN(gtk_drop_down_new(
            G_LIST_MODEL(recent_list), NULL));
        gtk_drop_down_set_selected(pu->recent, 0);
        g_signal_connect(pu->recent, "notify::selected",
                         G_CALLBACK(picker_recent_changed), pu);
        gtk_box_append(GTK_BOX(box), GTK_WIDGET(pu->recent));
    }
    pu->error = GTK_LABEL(gtk_label_new(""));
    gtk_label_set_xalign(pu->error, 0);
    gtk_widget_add_css_class(GTK_WIDGET(pu->error), "error");
    gtk_box_append(GTK_BOX(box), GTK_WIDGET(pu->error));

    GtkWidget *cli_label = gtk_label_new("Who should do it?");
    gtk_label_set_xalign(GTK_LABEL(cli_label), 0);
    gtk_widget_add_css_class(cli_label, "heading");
    gtk_box_append(GTK_BOX(box), cli_label);
    GtkCheckButton *group = NULL;
    for (size_t i = 0; i < pu->n_clis; i++) {
        char *label;
        if (pu->clis[i].available) {
            label = g_strdup_printf(
                "%s — ready%s%s", pu->clis[i].id,
                pu->clis[i].path ? " (" : "",
                pu->clis[i].path ? pu->clis[i].path : "");
            if (pu->clis[i].path) {
                char *tmp = g_strdup_printf("%s)", label);
                g_free(label);
                label = tmp;
            }
        } else {
            label = g_strdup_printf("%s — not installed", pu->clis[i].id);
        }
        GtkWidget *row = gtk_check_button_new_with_label(label);
        g_free(label);
        gtk_check_button_set_group(GTK_CHECK_BUTTON(row), group);
        if (!group) {
            group = GTK_CHECK_BUTTON(row);
        }
        gtk_widget_set_sensitive(row, pu->clis[i].available ? TRUE : FALSE);
        gtk_widget_set_tooltip_text(
            row, pu->clis[i].available
                     ? (pu->clis[i].path ? pu->clis[i].path : pu->clis[i].id)
                     : "Install this CLI and ensure it is on PATH.");
        g_signal_connect(row, "toggled", G_CALLBACK(picker_cli_toggled),
                         pu);
        gtk_box_append(GTK_BOX(box), row);
        pu->cli_btns[i] = GTK_CHECK_BUTTON(row);
    }
    /* Preselect the first available CLI (catalog order: muse first). */
    for (size_t i = 0; i < pu->n_clis; i++) {
        if (pu->clis[i].available) {
            gtk_check_button_set_active(pu->cli_btns[i], TRUE);
            break;
        }
    }

    GtkWidget *yolo_label = gtk_label_new("Permission mode");
    gtk_label_set_xalign(GTK_LABEL(yolo_label), 0);
    gtk_widget_add_css_class(yolo_label, "heading");
    gtk_box_append(GTK_BOX(box), yolo_label);
    const char *yolo_opts[] = { "Default", "On (once)", "Off (once)",
                                NULL };
    pu->yolo = GTK_DROP_DOWN(gtk_drop_down_new_from_strings(yolo_opts));
    gtk_widget_set_tooltip_text(
        GTK_WIDGET(pu->yolo),
        "Yolo lets the agent run commands without asking. "
        "Default follows the per-agent config; once-choices apply to "
        "this run only and are never saved.");
    g_signal_connect(pu->yolo, "notify::selected",
                     G_CALLBACK(picker_yolo_changed), pu);
    gtk_box_append(GTK_BOX(box), GTK_WIDGET(pu->yolo));

    pu->preview = GTK_LABEL(gtk_label_new("runs: muse"));
    gtk_label_set_xalign(pu->preview, 0);
    gtk_widget_add_css_class(GTK_WIDGET(pu->preview), "dim-label");
    gtk_box_append(GTK_BOX(box), GTK_WIDGET(pu->preview));

    GtkWidget *actions = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 8);
    gtk_widget_set_halign(actions, GTK_ALIGN_END);
    GtkWidget *cancel = gtk_button_new_with_label("Cancel");
    g_signal_connect_swapped(cancel, "clicked",
                             G_CALLBACK(adw_dialog_close), dialog);
    gtk_box_append(GTK_BOX(actions), cancel);
    GtkWidget *spawn = gtk_button_new_with_label("Spawn");
    gtk_widget_add_css_class(spawn, "suggested-action");
    g_signal_connect(spawn, "clicked", G_CALLBACK(picker_spawn_clicked),
                     pu);
    gtk_box_append(GTK_BOX(actions), spawn);
    gtk_box_append(GTK_BOX(box), actions);

    picker_refresh_preview(pu);
    adw_dialog_set_child(dialog, box);
    adw_dialog_present(dialog, GTK_WIDGET(win));
}

/* Encode one key press into the raw bytes the child expects and forward
 * them with bridge_write. Returns TRUE when handled (stop VTE from also
 * processing the key: with no VTE-side PTY there is exactly one line
 * discipline — the core's — so nothing double-echoes). */
static gboolean on_key_pressed(GtkEventControllerKey *ctl, guint keyval,
                               guint keycode, GdkModifierType state,
                               gpointer data) {
    (void)ctl;
    (void)keycode;
    Shell *sh = data;
    LivePty *lp =
        sh->selected ? g_hash_table_lookup(sh->live, sh->selected) : NULL;
    if (!lp) {
        return FALSE;
    }
    gboolean ctrl = (state & GDK_CONTROL_MASK) != 0;
    gboolean shift = (state & GDK_SHIFT_MASK) != 0;
    gboolean alt = (state & GDK_ALT_MASK) != 0;

    /* Let VTE keep its own copy/paste shortcuts. */
    if (ctrl && shift && (keyval == GDK_KEY_C || keyval == GDK_KEY_V)) {
        return FALSE;
    }

    char buf[16];
    size_t n = 0;
    switch (keyval) {
    case GDK_KEY_Return:
    case GDK_KEY_KP_Enter:
    case GDK_KEY_ISO_Enter:
        buf[0] = '\r';
        n = 1;
        break;
    case GDK_KEY_BackSpace: buf[0] = 0x7f; n = 1; break;
    case GDK_KEY_Tab: buf[0] = '\t'; n = 1; break;
    case GDK_KEY_Escape: buf[0] = 0x1b; n = 1; break;
    case GDK_KEY_Up: memcpy(buf, "\x1b[A", 3); n = 3; break;
    case GDK_KEY_Down: memcpy(buf, "\x1b[B", 3); n = 3; break;
    case GDK_KEY_Right: memcpy(buf, "\x1b[C", 3); n = 3; break;
    case GDK_KEY_Left: memcpy(buf, "\x1b[D", 3); n = 3; break;
    case GDK_KEY_Home: memcpy(buf, "\x1b[H", 3); n = 3; break;
    case GDK_KEY_End: memcpy(buf, "\x1b[F", 3); n = 3; break;
    case GDK_KEY_Insert: memcpy(buf, "\x1b[2~", 4); n = 4; break;
    case GDK_KEY_Delete: memcpy(buf, "\x1b[3~", 4); n = 4; break;
    case GDK_KEY_Page_Up: memcpy(buf, "\x1b[5~", 4); n = 4; break;
    case GDK_KEY_Page_Down: memcpy(buf, "\x1b[6~", 4); n = 4; break;
    case GDK_KEY_F1: memcpy(buf, "\x1bOP", 3); n = 3; break;
    case GDK_KEY_F2: memcpy(buf, "\x1bOQ", 3); n = 3; break;
    case GDK_KEY_F3: memcpy(buf, "\x1bOR", 3); n = 3; break;
    case GDK_KEY_F4: memcpy(buf, "\x1bOS", 3); n = 3; break;
    case GDK_KEY_F5: memcpy(buf, "\x1b[15~", 5); n = 5; break;
    case GDK_KEY_F6: memcpy(buf, "\x1b[17~", 5); n = 5; break;
    case GDK_KEY_F7: memcpy(buf, "\x1b[18~", 5); n = 5; break;
    case GDK_KEY_F8: memcpy(buf, "\x1b[19~", 5); n = 5; break;
    case GDK_KEY_F9: memcpy(buf, "\x1b[20~", 5); n = 5; break;
    case GDK_KEY_F10: memcpy(buf, "\x1b[21~", 5); n = 5; break;
    case GDK_KEY_F11: memcpy(buf, "\x1b[23~", 5); n = 5; break;
    case GDK_KEY_F12: memcpy(buf, "\x1b[24~", 5); n = 5; break;
    default: break;
    }
    if (n == 0) {
        guint32 uc = gdk_keyval_to_unicode(keyval);
        if (uc == 0) {
            return FALSE; /* Super, modifiers, media keys: leave to GTK. */
        }
        if (ctrl && !alt && uc < 128) {
            /* Ctrl+letter -> control code (Ctrl+C interrupts the child;
             * copy stays on Ctrl+Shift+C, handled above). */
            char lower = (char)tolower((int)uc);
            if (lower >= 'a' && lower <= 'z') {
                buf[0] = (char)(lower - 'a' + 1);
                n = 1;
            } else {
                return FALSE;
            }
        } else {
            char tmp[8];
            int m = g_unichar_to_utf8((gunichar)uc, tmp);
            if (m <= 0) {
                return FALSE;
            }
            if (alt) {
                buf[0] = 0x1b;
                memcpy(buf + 1, tmp, (size_t)m);
                n = (size_t)m + 1;
            } else {
                memcpy(buf, tmp, (size_t)m);
                n = (size_t)m;
            }
        }
    }

    char *err = NULL;
    if (bridge_write(lp->pty, (unsigned char *)buf, n, &err) != 0) {
        char *msg = g_strdup_printf("Could not send input: %s",
                                    err ? err : "unknown error");
        toast(sh, msg);
        g_free(msg);
        free(err);
    }
    return TRUE;
}

/* ------------------------------------------------------------------ */
/* Pump: the shell's only repaint gate (mirrors AppState.pump). */
/* ------------------------------------------------------------------ */

static void feed_to_vte(Shell *sh, const char *feed) {
    vte_terminal_feed(sh->term, feed, (gssize)strlen(feed));
}

/* Show the selected run's current snapshot in the VTE from scratch
 * (selection change or fresh spawn): reset, then feed the full screen. */
static void show_selected_in_terminal(Shell *sh) {
    vte_terminal_reset(sh->term, TRUE, TRUE);
    LivePty *lp =
        sh->selected ? g_hash_table_lookup(sh->live, sh->selected) : NULL;
    if (!lp) {
        vte_terminal_feed(sh->term, "(no live session — press New run)\r\n",
                          -1);
        return;
    }
    char *snap = bridge_screen_text(lp->pty);
    const char *text = snap ? snap : "";
    char *feed = am_feed_delta("", text);
    if (feed) {
        feed_to_vte(sh, feed);
        free(feed);
    }
    free(lp->fed);
    lp->fed = strdup(text);
    bridge_string_free(snap);
}

static gboolean pump_tick(gpointer data) {
    Shell *sh = data;
    GHashTableIter it;
    gpointer k, v;
    g_hash_table_iter_init(&it, sh->live);
    while (g_hash_table_iter_next(&it, &k, &v)) {
        const char *id = k;
        LivePty *lp = v;
        if (!bridge_pump(lp->pty)) {
            continue;
        }
        char *snap = bridge_screen_text(lp->pty);
        const char *text = snap ? snap : "";
        char *feed = am_feed_delta(lp->fed ? lp->fed : "", text);
        if (feed && feed[0] && sh->selected && strcmp(id, sh->selected) == 0) {
            feed_to_vte(sh, feed);
        }
        free(feed);
        free(lp->fed);
        lp->fed = strdup(text);
        bridge_string_free(snap);
    }
    reload_statuses(sh);
    return G_SOURCE_CONTINUE;
}

/* Track the VTE grid so spawns and resizes use the real size. */
static void on_term_resize(VteTerminal *term, guint w, guint h,
                           gpointer data) {
    (void)w;
    (void)h;
    Shell *sh = data;
    glong cols = vte_terminal_get_column_count(term);
    glong rows = vte_terminal_get_row_count(term);
    if (cols < 1 || rows < 1) {
        return;
    }
    if ((int)cols != sh->cols || (int)rows != sh->rows_grid) {
        sh->cols = (int)cols;
        sh->rows_grid = (int)rows;
        GHashTableIter it;
        gpointer k, v;
        g_hash_table_iter_init(&it, sh->live);
        while (g_hash_table_iter_next(&it, &k, &v)) {
            (void)k;
            bridge_resize(((LivePty *)v)->pty, (unsigned)cols, (unsigned)rows);
        }
        /* Reflow replays the selected screen from scratch. */
        show_selected_in_terminal(sh);
    }
}

/* ------------------------------------------------------------------ */
/* ------------------------------------------------------------------ */
/* UI construction. */
/* ------------------------------------------------------------------ */

static void on_row_selected(GtkListBox *box, GtkListBoxRow *row,
                            gpointer data) {
    (void)box;
    Shell *sh = data;
    SessionRow *r = row ? g_object_get_data(G_OBJECT(row), "am-row") : NULL;
    free(sh->selected);
    sh->selected = r ? strdup(r->id) : NULL;
    show_selected_in_terminal(sh);
    update_spawn_state(sh);
    reload_statuses(sh);
}

static void on_filter_changed(GtkSearchEntry *entry, gpointer data) {
    (void)entry;
    Shell *sh = data;
    gtk_list_box_invalidate_filter(sh->list);
}

static void on_save(GtkButton *btn, gpointer data) {
    (void)btn;
    Shell *sh = data;
    char *err = NULL;
    if (bridge_core_save(sh->core, &err) != 0) {
        char *msg =
            g_strdup_printf("Could not save: %s", err ? err : "unknown error");
        toast(sh, msg);
        g_free(msg);
        free(err);
        return;
    }
    toast(sh, "Saved");
}

static void on_theme_changed(GObject *obj, GParamSpec *ps, gpointer data) {
    (void)ps;
    Shell *sh = data;
    apply_theme(sh, (int)gtk_drop_down_get_selected(GTK_DROP_DOWN(obj)));
}

static void on_find_changed(GtkSearchEntry *entry, gpointer data) {
    Shell *sh = data;
    const char *q = gtk_editable_get_text(GTK_EDITABLE(entry));
    if (!q || !*q) {
        vte_terminal_search_set_regex(sh->term, NULL, 0);
        return;
    }
    /* Literal search: escape regex metacharacters, match case-insensitively
     * (PCRE2 CASELESS is part of VTE_REGEX_FLAGS_DEFAULT). */
    char *escaped = g_regex_escape_string(q, -1);
    GError *error = NULL;
    VteRegex *re = vte_regex_new_for_search(escaped, -1,
                                            VTE_REGEX_FLAGS_DEFAULT, &error);
    g_free(escaped);
    if (error) {
        g_error_free(error);
    }
    vte_terminal_search_set_regex(sh->term, re, 0);
    if (re) {
        vte_terminal_search_set_wrap_around(sh->term, TRUE);
        vte_terminal_search_find_next(sh->term);
        vte_regex_unref(re);
    }
}

static gboolean on_find_key(GtkEventControllerKey *ctl, guint keyval,
                            guint keycode, GdkModifierType state,
                            gpointer data) {
    (void)ctl;
    (void)keycode;
    Shell *sh = data;
    if (keyval == GDK_KEY_Escape) {
        gtk_revealer_set_reveal_child(sh->find_revealer, FALSE);
        gtk_widget_grab_focus(GTK_WIDGET(sh->term));
        return TRUE;
    }
    if (keyval == GDK_KEY_Return) {
        if (state & GDK_SHIFT_MASK) {
            vte_terminal_search_find_previous(sh->term);
        } else {
            vte_terminal_search_find_next(sh->term);
        }
        return TRUE;
    }
    return FALSE;
}

static void toggle_find(Shell *sh) {
    gboolean open = gtk_revealer_get_reveal_child(sh->find_revealer);
    gtk_revealer_set_reveal_child(sh->find_revealer, !open);
    if (!open) {
        gtk_widget_grab_focus(GTK_WIDGET(sh->find_entry));
    } else {
        gtk_widget_grab_focus(GTK_WIDGET(sh->term));
    }
}

static void on_copy(GSimpleAction *action, GVariant *param, gpointer data) {
    (void)action;
    (void)param;
    vte_terminal_copy_clipboard_format(data, VTE_FORMAT_TEXT);
}

static void on_paste(GSimpleAction *action, GVariant *param, gpointer data) {
    (void)action;
    (void)param;
    vte_terminal_paste_clipboard(data);
}

static void on_term_menu_closed(GtkPopover *pop, gpointer data) {
    (void)data;
    gtk_widget_unparent(GTK_WIDGET(pop));
}

static void on_term_menu(GtkGestureClick *gest, int n_press, double x,
                         double y, gpointer data) {
    (void)gest;
    (void)n_press;
    Shell *sh = data;
    GMenu *m = g_menu_new();
    if (vte_terminal_get_has_selection(sh->term)) {
        g_menu_append(m, "Copy", "term.copy");
    }
    g_menu_append(m, "Paste", "term.paste");
    GSimpleActionGroup *ag = g_simple_action_group_new();
    GSimpleAction *copy = g_simple_action_new("copy", NULL);
    g_signal_connect(copy, "activate", G_CALLBACK(on_copy), sh->term);
    GSimpleAction *paste = g_simple_action_new("paste", NULL);
    g_signal_connect(paste, "activate", G_CALLBACK(on_paste), sh->term);
    g_action_map_add_action(G_ACTION_MAP(ag), G_ACTION(copy));
    g_action_map_add_action(G_ACTION_MAP(ag), G_ACTION(paste));
    GtkWidget *pop = gtk_popover_menu_new_from_model(G_MENU_MODEL(m));
    gtk_widget_insert_action_group(pop, "term", G_ACTION_GROUP(ag));
    gtk_widget_set_parent(pop, GTK_WIDGET(sh->term));
    g_signal_connect(pop, "closed", G_CALLBACK(on_term_menu_closed), NULL);
    GdkRectangle pt = { (int)x, (int)y, 1, 1 };
    gtk_popover_set_pointing_to(GTK_POPOVER(pop), &pt);
    gtk_popover_popup(GTK_POPOVER(pop));
    g_object_unref(m);
    g_object_unref(ag);
}

/* App-level shortcuts: Ctrl+F find, Ctrl+S save, Ctrl+N spawn,
 * Ctrl+Shift+N picker. */
static gboolean on_window_key(GtkEventControllerKey *ctl, guint keyval,
                              guint keycode, GdkModifierType state,
                              gpointer data) {
    (void)ctl;
    (void)keycode;
    Shell *sh = data;
    if ((state & GDK_CONTROL_MASK) && !(state & GDK_ALT_MASK)) {
        if (keyval == GDK_KEY_f || keyval == GDK_KEY_F) {
            toggle_find(sh);
            return TRUE;
        }
        if (keyval == GDK_KEY_s || keyval == GDK_KEY_S) {
            on_save(NULL, sh);
            return TRUE;
        }
        if ((keyval == GDK_KEY_n || keyval == GDK_KEY_N) &&
            !(state & GDK_SHIFT_MASK)) {
            on_spawn(NULL, sh);
            return TRUE;
        }
        if ((keyval == GDK_KEY_n || keyval == GDK_KEY_N) &&
            (state & GDK_SHIFT_MASK)) {
            on_pick_session(NULL, sh);
            return TRUE;
        }
    }
    return FALSE;
}

static gboolean on_close(GtkWindow *win, gpointer data) {
    (void)win;
    Shell *sh = data;
    /* Best-effort persist on quit; a failure still closes (the error is
     * visible next launch via the core's degrade-to-empty policy). */
    char *err = NULL;
    if (bridge_core_save(sh->core, &err) != 0) {
        free(err);
    }
    return FALSE; /* let the window close */
}

static void build_ui(Shell *sh) {
    AdwApplicationWindow *win = ADW_APPLICATION_WINDOW(
        adw_application_window_new(GTK_APPLICATION(sh->app)));
    gtk_window_set_title(GTK_WINDOW(win), "Agent Manager");
    gtk_window_set_default_size(GTK_WINDOW(win), 1100, 700);
    g_signal_connect(win, "close-request", G_CALLBACK(on_close), sh);

    AdwToastOverlay *toasts = ADW_TOAST_OVERLAY(adw_toast_overlay_new());
    g_object_set_data(G_OBJECT(win), "am-toast-overlay", toasts);
    adw_application_window_set_content(win, GTK_WIDGET(toasts));

    /* Header: subtitle shows selection context + attention count. */
    AdwHeaderBar *bar = ADW_HEADER_BAR(adw_header_bar_new());
    GtkWidget *title = adw_window_title_new("Agent Manager", NULL);
    sh->header_title = ADW_WINDOW_TITLE(title);
    adw_header_bar_set_title_widget(bar, title);

    /* Header: split-button 2D launch — "New run" repeats the last
     * launch instantly, the caret opens the folder x CLI picker. */
    GtkWidget *new_box = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 0);
    gtk_widget_add_css_class(new_box, "linked");
    sh->spawn_btn = GTK_BUTTON(gtk_button_new_with_label("New run"));
    gtk_widget_add_css_class(GTK_WIDGET(sh->spawn_btn), "suggested-action");
    gtk_widget_set_tooltip_text(GTK_WIDGET(sh->spawn_btn),
                                "Repeat the last session (Ctrl+N)");
    g_signal_connect(sh->spawn_btn, "clicked", G_CALLBACK(on_spawn), sh);
    gtk_box_append(GTK_BOX(new_box), GTK_WIDGET(sh->spawn_btn));
    GtkWidget *pick_btn = gtk_button_new_with_label("▾");
    gtk_widget_set_tooltip_text(pick_btn,
                                "Choose folder, CLI, options… (Ctrl+Shift+N)");
    g_signal_connect(pick_btn, "clicked", G_CALLBACK(on_pick_session), sh);
    gtk_box_append(GTK_BOX(new_box), pick_btn);
    gtk_widget_set_sensitive(pick_btn, TRUE);
    adw_header_bar_pack_start(ADW_HEADER_BAR(bar), new_box);

    sh->save_btn = GTK_BUTTON(gtk_button_new_with_label("Save"));
    g_signal_connect(sh->save_btn, "clicked", G_CALLBACK(on_save), sh);
    adw_header_bar_pack_end(ADW_HEADER_BAR(bar), GTK_WIDGET(sh->save_btn));

    /* Theme picker: System / Dark / Light. */
    const char *themes[] = { "System", "Dark", "Light", NULL };
    sh->theme_drop = GTK_DROP_DOWN(
        gtk_drop_down_new_from_strings(themes));
    gtk_drop_down_set_selected(sh->theme_drop,
                               (guint)load_theme_pref(sh));
    gtk_widget_set_tooltip_text(GTK_WIDGET(sh->theme_drop), "Theme");
    g_signal_connect(sh->theme_drop, "notify::selected",
                     G_CALLBACK(on_theme_changed), sh);
    adw_header_bar_pack_end(ADW_HEADER_BAR(bar),
                            GTK_WIDGET(sh->theme_drop));

    /* Split view: roster sidebar + terminal. */
    AdwOverlaySplitView *split =
        ADW_OVERLAY_SPLIT_VIEW(adw_overlay_split_view_new());
    adw_overlay_split_view_set_max_sidebar_width(split, 340);
    adw_overlay_split_view_set_min_sidebar_width(split, 220);

    GtkWidget *side_box = gtk_box_new(GTK_ORIENTATION_VERTICAL, 0);
    sh->filter = GTK_SEARCH_ENTRY(gtk_search_entry_new());
    gtk_search_entry_set_placeholder_text(sh->filter, "Filter runs");
    gtk_search_entry_set_search_delay(sh->filter, 150);
    g_signal_connect(sh->filter, "search-changed",
                     G_CALLBACK(on_filter_changed), sh);
    gtk_box_append(GTK_BOX(side_box), GTK_WIDGET(sh->filter));

    GtkWidget *scroll = gtk_scrolled_window_new();
    gtk_widget_set_vexpand(scroll, TRUE);
    sh->list = GTK_LIST_BOX(gtk_list_box_new());
    gtk_list_box_set_selection_mode(sh->list, GTK_SELECTION_SINGLE);
    gtk_list_box_set_filter_func(sh->list, row_matches, sh, NULL);
    gtk_list_box_set_sort_func(sh->list, row_order, sh, NULL);
    g_signal_connect(sh->list, "row-selected", G_CALLBACK(on_row_selected),
                     sh);
    gtk_scrolled_window_set_child(GTK_SCROLLED_WINDOW(scroll),
                                  GTK_WIDGET(sh->list));
    gtk_box_append(GTK_BOX(side_box), scroll);
    adw_overlay_split_view_set_sidebar(split, side_box);

    /* Terminal side: find revealer on top, VTE in a scrolled window. */
    GtkWidget *term_box = gtk_box_new(GTK_ORIENTATION_VERTICAL, 0);
    sh->find_revealer = GTK_REVEALER(gtk_revealer_new());
    sh->find_entry = GTK_SEARCH_ENTRY(gtk_search_entry_new());
    gtk_search_entry_set_placeholder_text(sh->find_entry,
                                          "Find in terminal (Enter next, "
                                          "Shift+Enter previous, Esc close)");
    g_signal_connect(sh->find_entry, "search-changed",
                     G_CALLBACK(on_find_changed), sh);
    GtkEventController *find_keys = gtk_event_controller_key_new();
    g_signal_connect(find_keys, "key-pressed", G_CALLBACK(on_find_key), sh);
    gtk_widget_add_controller(GTK_WIDGET(sh->find_entry), find_keys);
    gtk_revealer_set_child(sh->find_revealer, GTK_WIDGET(sh->find_entry));
    gtk_box_append(GTK_BOX(term_box), GTK_WIDGET(sh->find_revealer));

    sh->term_scroll = GTK_SCROLLED_WINDOW(gtk_scrolled_window_new());
    gtk_widget_set_vexpand(GTK_WIDGET(sh->term_scroll), TRUE);
    gtk_widget_set_hexpand(GTK_WIDGET(sh->term_scroll), TRUE);
    sh->term = VTE_TERMINAL(vte_terminal_new());
    vte_terminal_set_scrollback_lines(sh->term, SCROLLBACK_LINES);
    vte_terminal_set_scroll_on_output(sh->term, TRUE);
    vte_terminal_set_scroll_on_keystroke(sh->term, TRUE);
    vte_terminal_set_cursor_blink_mode(sh->term, VTE_CURSOR_BLINK_ON);
    vte_terminal_set_allow_hyperlink(sh->term, TRUE);
    PangoFontDescription *font =
        pango_font_description_from_string("Monospace 11");
    vte_terminal_set_font(sh->term, font);
    pango_font_description_free(font);
    g_signal_connect(sh->term, "resize-window", G_CALLBACK(on_term_resize),
                     sh);
    GtkEventController *keys = gtk_event_controller_key_new();
    g_signal_connect(keys, "key-pressed", G_CALLBACK(on_key_pressed), sh);
    gtk_widget_add_controller(GTK_WIDGET(sh->term), keys);
    GtkGesture *right = gtk_gesture_click_new();
    gtk_gesture_single_set_button(GTK_GESTURE_SINGLE(right), 3);
    g_signal_connect(right, "pressed", G_CALLBACK(on_term_menu), sh);
    gtk_widget_add_controller(GTK_WIDGET(sh->term),
                              GTK_EVENT_CONTROLLER(right));
    gtk_scrolled_window_set_child(sh->term_scroll, GTK_WIDGET(sh->term));
    gtk_box_append(GTK_BOX(term_box), GTK_WIDGET(sh->term_scroll));
    adw_overlay_split_view_set_content(split, term_box);

    GtkWidget *toolbar = gtk_box_new(GTK_ORIENTATION_VERTICAL, 0);
    gtk_box_append(GTK_BOX(toolbar), GTK_WIDGET(bar));
    gtk_box_append(GTK_BOX(toolbar), GTK_WIDGET(split));
    gtk_widget_set_vexpand(GTK_WIDGET(split), TRUE);
    adw_toast_overlay_set_child(toasts, toolbar);

    GtkEventController *win_keys = gtk_event_controller_key_new();
    g_signal_connect(win_keys, "key-pressed", G_CALLBACK(on_window_key), sh);
    gtk_widget_add_controller(GTK_WIDGET(win), win_keys);

    /* Roster rows (launch snapshot; statuses refresh on the pump tick). */
    size_t n = bridge_session_count(sh->core);
    for (size_t i = 0; i < n; i++) {
        char *json = bridge_session_json(sh->core, i);
        if (!json) {
            continue;
        }
        SessionRow *r = calloc(1, sizeof *r);
        r->id = json_string(json, "id");
        r->title = json_string(json, "title");
        r->project = json_string(json, "project");
        r->harness = json_string(json, "harness");
        r->last_active = json_int(json, "last_active", 0);
        r->status = bridge_status(sh->core, i);
        bridge_string_free(json);
        if (!r->id) {
            row_free(r);
            continue;
        }
        if (!r->title) {
            r->title = strdup("(untitled)");
        }
        if (!r->project) {
            r->project = strdup("");
        }
        if (!r->harness) {
            r->harness = strdup("");
        }
        g_ptr_array_add(sh->rows, r);

        GtkWidget *row = gtk_list_box_row_new();
        g_object_set_data_full(G_OBJECT(row), "am-row-id", strdup(r->id),
                               free);
        g_object_set_data(G_OBJECT(row), "am-row", r);
        GtkWidget *hbox = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 8);
        gtk_widget_set_margin_start(hbox, 8);
        gtk_widget_set_margin_end(hbox, 8);
        gtk_widget_set_margin_top(hbox, 6);
        gtk_widget_set_margin_bottom(hbox, 6);
        GtkWidget *vbox = gtk_box_new(GTK_ORIENTATION_VERTICAL, 2);
        gtk_widget_set_hexpand(vbox, TRUE);
        GtkWidget *t = gtk_label_new(r->title);
        gtk_label_set_xalign(GTK_LABEL(t), 0);
        gtk_label_set_ellipsize(GTK_LABEL(t), PANGO_ELLIPSIZE_END);
        char *detail = g_strdup_printf("%s · %s", r->project, r->harness);
        GtkWidget *d = gtk_label_new(detail);
        g_free(detail);
        gtk_label_set_xalign(GTK_LABEL(d), 0);
        gtk_widget_add_css_class(d, "dim-label");
        gtk_box_append(GTK_BOX(vbox), t);
        gtk_box_append(GTK_BOX(vbox), d);
        GtkWidget *badge = gtk_label_new(status_label(r->status));
        gtk_label_set_xalign(GTK_LABEL(badge), 1);
        gtk_widget_add_css_class(badge, "caption");
        gtk_box_append(GTK_BOX(hbox), vbox);
        gtk_box_append(GTK_BOX(hbox), badge);
        gtk_list_box_row_set_child(GTK_LIST_BOX_ROW(row), hbox);
        gtk_list_box_append(sh->list, row);
        g_hash_table_insert(sh->row_widgets, strdup(r->id), badge);
    }

    apply_theme(sh, (int)gtk_drop_down_get_selected(sh->theme_drop));

    if (sh->rows->len > 0) {
        GtkListBoxRow *first = gtk_list_box_get_row_at_index(sh->list, 0);
        if (first) {
            gtk_list_box_select_row(sh->list, first);
        }
    } else {
        adw_window_title_set_subtitle(sh->header_title,
                                      "No runs yet — New run spawns one");
        vte_terminal_feed(sh->term, "No runs yet — press New run.\r\n", -1);
    }
    update_spawn_state(sh);
    reload_statuses(sh);

    gtk_window_present(GTK_WINDOW(win));
    g_timeout_add(PUMP_MS, pump_tick, sh);
}

static void on_activate(GtkApplication *app, gpointer data) {
    (void)app;
    build_ui(data);
}

/* ------------------------------------------------------------------ */
/* Entry point. */
/* ------------------------------------------------------------------ */

int main(int argc, char **argv) {
    Shell *sh = calloc(1, sizeof *sh);
    sh->cols = DEFAULT_COLS;
    sh->rows_grid = DEFAULT_ROWS;
    sh->rows = g_ptr_array_new_with_free_func((GDestroyNotify)row_free);
    sh->live = g_hash_table_new_full(g_str_hash, g_str_equal, free,
                                     (GDestroyNotify)live_free);
    sh->row_widgets =
        g_hash_table_new_full(g_str_hash, g_str_equal, free, NULL);

    sh->core = bridge_core_new();
    if (!sh->core) {
        g_printerr("agent-manager-gtk: core init failed: %s\n",
                   bridge_last_error());
        return 1;
    }

    char *cfg = g_build_filename(g_get_user_config_dir(), "agent-manager",
                                 "gtk-theme", NULL);
    char *dir = g_path_get_dirname(cfg);
    g_mkdir_with_parents(dir, 0700);
    g_free(dir);
    sh->theme_path = cfg;

    sh->app = ADW_APPLICATION(
        adw_application_new("com.example.agent-manager", G_APPLICATION_DEFAULT_FLAGS));
    g_signal_connect(sh->app, "activate", G_CALLBACK(on_activate), sh);
    int rc = g_application_run(G_APPLICATION(sh->app), argc, argv);

    g_hash_table_destroy(sh->live);
    g_hash_table_destroy(sh->row_widgets);
    g_ptr_array_free(sh->rows, TRUE);
    free(sh->selected);
    free(sh->theme_path);
    bridge_core_free(sh->core);
    g_object_unref(sh->app);
    free(sh);
    return rc;
}

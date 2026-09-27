/* Snapshot-to-stream reconciler (issue #63). See feed.h for the contract. */

#include "feed.h"

#include <stdlib.h>
#include <string.h>

/* Split `s` on '\n' keeping empty segments (mirrors Swift's
 * `omittingEmptySubsequences: false`). Returns a malloc'd array of
 * malloc'd lines; `*out_n` holds the count. */
static char **split_lines(const char *s, int *out_n) {
    int n = 1;
    for (const char *p = s; *p; p++) {
        if (*p == '\n') {
            n++;
        }
    }
    char **lines = malloc((size_t)n * sizeof *lines);
    if (!lines) {
        *out_n = 0;
        return NULL;
    }
    int i = 0;
    const char *start = s;
    for (;;) {
        const char *nl = strchr(start, '\n');
        size_t len = nl ? (size_t)(nl - start) : strlen(start);
        char *line = malloc(len + 1);
        if (!line) {
            for (int j = 0; j < i; j++) {
                free(lines[j]);
            }
            free(lines);
            *out_n = 0;
            return NULL;
        }
        memcpy(line, start, len);
        line[len] = '\0';
        lines[i++] = line;
        if (!nl) {
            break;
        }
        start = nl + 1;
    }
    *out_n = i;
    return lines;
}

static void free_lines(char **lines, int n) {
    if (!lines) {
        return;
    }
    for (int i = 0; i < n; i++) {
        free(lines[i]);
    }
    free(lines);
}

int am_feed_overlap(char **old_lines, int old_n, char **new_lines, int new_n) {
    int max_k = old_n < new_n ? old_n : new_n;
    for (int k = max_k; k > 0; k--) {
        int same = 1;
        for (int i = 0; i < k; i++) {
            if (strcmp(old_lines[old_n - k + i], new_lines[i]) != 0) {
                same = 0;
                break;
            }
        }
        if (same) {
            return k;
        }
    }
    return 0;
}

/* Append `src[0..len)` to `*buf`, growing it. Newlines go out as CRLF;
 * a CRLF pair in the source counts as one newline (mirrors Swift's
 * normalizeNewlines: collapse "\r\n" -> "\n" first, then expand). */
static int append_normalized(char **buf, size_t *len, size_t *cap,
                             const char *src, size_t src_len) {
    for (size_t i = 0; i < src_len; i++) {
        const char *chunk = "\r\n";
        size_t chunk_len = 2;
        if (src[i] == '\r' && i + 1 < src_len && src[i + 1] == '\n') {
            i++; /* CRLF pair: emit one CRLF below. */
        } else if (src[i] == '\n') {
            /* Bare LF: emit one CRLF below. */
        } else if (src[i] == '\r') {
            chunk = "\r";
            chunk_len = 1;
        } else {
            chunk = src + i;
            chunk_len = 1;
        }
        if (*len + chunk_len + 1 > *cap) {
            size_t ncap = (*cap == 0 ? 64 : *cap * 2) + chunk_len;
            char *nbuf = realloc(*buf, ncap);
            if (!nbuf) {
                return -1;
            }
            *buf = nbuf;
            *cap = ncap;
        }
        memcpy(*buf + *len, chunk, chunk_len);
        *len += chunk_len;
    }
    if (*buf) {
        (*buf)[*len] = '\0';
    }
    return 0;
}

static char *normalize(const char *s, size_t n) {
    char *buf = NULL;
    size_t len = 0, cap = 0;
    if (append_normalized(&buf, &len, &cap, s, n) != 0) {
        free(buf);
        return NULL;
    }
    if (!buf) {
        buf = malloc(1);
        if (buf) {
            buf[0] = '\0';
        }
    }
    return buf;
}

char *am_feed_delta(const char *old_text, const char *new_text) {
    const char *old_s = old_text ? old_text : "";
    const char *new_s = new_text ? new_text : "";
    if (strcmp(old_s, new_s) == 0) {
        return NULL;
    }
    size_t old_len = strlen(old_s);
    size_t new_len = strlen(new_s);

    /* Hot path: append-only snapshot feeds just the suffix. */
    if (old_len > 0 && new_len >= old_len && memcmp(new_s, old_s, old_len) == 0) {
        size_t tail = new_len - old_len;
        if (tail == 0) {
            return NULL;
        }
        return normalize(new_s + old_len, tail);
    }
    /* First snapshot feeds the whole screen. */
    if (old_len == 0) {
        return normalize(new_s, new_len);
    }

    int old_n = 0, new_n = 0;
    char **olds = split_lines(old_s, &old_n);
    char **news = split_lines(new_s, &new_n);
    if (!olds || !news) {
        free_lines(olds, old_n);
        free_lines(news, new_n);
        return NULL;
    }
    int overlap = am_feed_overlap(olds, old_n, news, new_n);
    char *feed = NULL;
    if (overlap > 0) {
        /* The overlapped head is already on screen; the cursor sits at
         * the end of the previously fed text, so start a new line. */
        char *buf = NULL;
        size_t len = 0, cap = 0;
        if (append_normalized(&buf, &len, &cap, "\r\n", 2) == 0) {
            for (int i = overlap; i < new_n && buf; i++) {
                if (i > overlap &&
                    append_normalized(&buf, &len, &cap, "\r\n", 2) != 0) {
                    break;
                }
                if (append_normalized(&buf, &len, &cap, news[i],
                                      strlen(news[i])) != 0) {
                    break;
                }
            }
        }
        feed = buf;
    } else {
        /* Redraw / reflow: clear first, then replay the snapshot. */
        const char *clear = AM_FEED_CLEAR;
        size_t clear_len = strlen(clear);
        char *body = normalize(new_s, new_len);
        if (body) {
            feed = malloc(clear_len + strlen(body) + 1);
            if (feed) {
                memcpy(feed, clear, clear_len);
                strcpy(feed + clear_len, body);
            }
            free(body);
        }
    }
    free_lines(olds, old_n);
    free_lines(news, new_n);
    return feed;
}

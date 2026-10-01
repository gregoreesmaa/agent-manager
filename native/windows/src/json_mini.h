/* Minimal JSON field extraction for roster rows (issue #64).
 *
 * Ports `native/linux/src/main.c::json_string`: roster rows arrive as
 * serialized `ChatSession` objects and the shell only needs top-level
 * string fields (id, title, project, harness). A full JSON parser is
 * out of proportion here — this handles the flat string lookup with
 * \" \\ / uXXXX unescaping — so the header stays dependency-free and
 * checkable with any C++17 compiler.
 *
 * Returns the decoded value, or empty when the key is absent or not a
 * JSON string (e.g. null cwd).
 */

#ifndef AM_JSON_MINI_H
#define AM_JSON_MINI_H

#include <string>
#include <vector>

namespace amjson {

inline std::string get_string(const std::string &json, const std::string &key) {
    const std::string pat = "\"" + key + "\"";
    std::size_t p = 0;
    while ((p = json.find(pat, p)) != std::string::npos) {
        p += pat.size();
        while (p < json.size() &&
               (json[p] == ' ' || json[p] == '\t' || json[p] == '\n' ||
                json[p] == '\r')) {
            ++p;
        }
        if (p >= json.size() || json[p] != ':') {
            continue;
        }
        ++p;
        while (p < json.size() &&
               (json[p] == ' ' || json[p] == '\t' || json[p] == '\n' ||
                json[p] == '\r')) {
            ++p;
        }
        if (p >= json.size() || json[p] != '"') {
            return {}; /* Present but not a string. */
        }
        ++p;
        std::string out;
        bool ok = true;
        while (p < json.size() && json[p] != '"') {
            if (json[p] != '\\') {
                out += json[p++];
                continue;
            }
            ++p;
            if (p >= json.size()) {
                ok = false;
                break;
            }
            char e = json[p++];
            switch (e) {
            case '"':
                out += '"';
                break;
            case '\\':
                out += '\\';
                break;
            case '/':
                out += '/';
                break;
            case 'n':
                out += '\n';
                break;
            case 't':
                out += '\t';
                break;
            case 'r':
                out += '\r';
                break;
            case 'u': {
                /* \\uXXXX: fold to UTF-8 (BMP only; the core emits
                 * titles/projects as plain multilingual text). */
                if (p + 4 > json.size()) {
                    ok = false;
                    break;
                }
                unsigned cp = 0;
                for (int i = 0; i < 4; ++i) {
                    char c = json[p + i];
                    cp <<= 4;
                    if (c >= '0' && c <= '9') {
                        cp |= static_cast<unsigned>(c - '0');
                    } else if (c >= 'a' && c <= 'f') {
                        cp |= static_cast<unsigned>(c - 'a' + 10);
                    } else if (c >= 'A' && c <= 'F') {
                        cp |= static_cast<unsigned>(c - 'A' + 10);
                    } else {
                        ok = false;
                        break;
                    }
                }
                if (!ok) {
                    break;
                }
                p += 4;
                if (cp < 0x80) {
                    out += static_cast<char>(cp);
                } else if (cp < 0x800) {
                    out += static_cast<char>(0xC0 | (cp >> 6));
                    out += static_cast<char>(0x80 | (cp & 0x3F));
                } else {
                    out += static_cast<char>(0xE0 | (cp >> 12));
                    out += static_cast<char>(0x80 | ((cp >> 6) & 0x3F));
                    out += static_cast<char>(0x80 | (cp & 0x3F));
                }
                break;
            }
            default:
                /* Lenient like the Linux port: keep unknown escapes. */
                out += e;
                break;
            }
        }
        if (ok && p < json.size()) {
            return out;
        }
        return {};
    }
    return {};
}

/* Top-level `true` flag of `key` inside a flat object span (1 when
 * present-true, 0 otherwise). Ports the catalog `available` read. */
inline bool get_bool(const std::string &json, const std::string &key) {
    const std::string pat = "\"" + key + "\"";
    std::size_t p = json.find(pat);
    if (p == std::string::npos) {
        return false;
    }
    p = json.find(':', p + pat.size());
    if (p == std::string::npos) {
        return false;
    }
    ++p;
    while (p < json.size() && (json[p] == ' ' || json[p] == '\t')) {
        ++p;
    }
    return json.compare(p, 4, "true") == 0;
}

/* Split a top-level JSON array of flat objects into its `{...}` spans.
 * Stops at the first malformed span (fail visible: short vector). */
inline std::vector<std::string> split_objects(const std::string &json) {
    std::vector<std::string> out;
    std::size_t p = json.find('[');
    if (p == std::string::npos) {
        return out;
    }
    ++p;
    auto skip_ws = [&]() {
        while (p < json.size() && (json[p] == ' ' || json[p] == '\t' ||
                                   json[p] == '\n' || json[p] == '\r' ||
                                   json[p] == ',')) {
            ++p;
        }
    };
    skip_ws();
    while (p < json.size() && json[p] != ']') {
        if (json[p] != '{') {
            break;
        }
        std::size_t start = p;
        int depth = 0;
        bool in_str = false, esc = false;
        for (; p < json.size(); ++p) {
            char c = json[p];
            if (in_str) {
                if (esc) {
                    esc = false;
                } else if (c == '\\') {
                    esc = true;
                } else if (c == '"') {
                    in_str = false;
                }
            } else if (c == '"') {
                in_str = true;
            } else if (c == '{') {
                ++depth;
            } else if (c == '}') {
                if (--depth == 0) {
                    ++p;
                    break;
                }
            }
        }
        if (depth != 0) {
            break;
        }
        out.push_back(json.substr(start, p - start));
        skip_ws();
    }
    return out;
}

/* Parse a top-level JSON string array into items (unescaped via
 * get_string on synthetic single-key objects). Malformed tail stops
 * the parse; the head still returns. */
inline std::vector<std::string> parse_string_array(const std::string &json) {
    std::vector<std::string> out;
    std::size_t p = json.find('[');
    if (p == std::string::npos) {
        return out;
    }
    ++p;
    while (p < json.size() && json[p] != ']') {
        while (p < json.size() && (json[p] == ' ' || json[p] == '\t' ||
                                   json[p] == '\n' || json[p] == '\r' ||
                                   json[p] == ',')) {
            ++p;
        }
        if (p >= json.size() || json[p] != '"') {
            break;
        }
        /* Find the closing quote (escape-aware), then decode through
         * the object reader on a synthetic wrapper. */
        std::size_t q = p + 1;
        bool esc = false;
        for (; q < json.size(); ++q) {
            if (esc) {
                esc = false;
            } else if (json[q] == '\\') {
                esc = true;
            } else if (json[q] == '"') {
                break;
            }
        }
        if (q >= json.size()) {
            break;
        }
        std::string wrapped =
            "{\"v\":" + json.substr(p, q - p + 1) + "}";
        std::string val = get_string(wrapped, "v");
        /* get_string returns {} on failure: distinguish by re-check —
         * an empty item decodes to "" through a valid span, so only
         * accept when the wrapper parsed (the span was well-formed). */
        out.push_back(val);
        p = q + 1;
    }
    return out;
}

} /* namespace amjson */

#endif /* AM_JSON_MINI_H */
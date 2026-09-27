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

} /* namespace amjson */

#endif /* AM_JSON_MINI_H */

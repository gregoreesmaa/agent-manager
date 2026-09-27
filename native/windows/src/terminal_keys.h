/* Key encoding for the Windows WinUI shell (issue #64).
 *
 * Ports the Linux shell's key controller
 * (`native/linux/src/main.c::on_key_pressed`) to Windows virtual-key
 * codes: one key press becomes the raw bytes the child expects, which
 * the shell forwards with `bridge_write`. With no WinUI-side PTY there
 * is exactly one line discipline — the core's — so nothing
 * double-echoes.
 *
 * This header is deliberately free of WinUI/Windows headers: keys arrive
 * as a plain struct (`VirtualKey` values match the Win32 `VK_*` numbers,
 * modifiers as bools, text as a UTF-32 code point), so the table is
 * unit-checkable with any C++17 compiler and has no framework to mock.
 * The WinUI code-behind (`winui/MainWindow.xaml.cpp`) only translates
 * `KeyRoutedEventArgs` into `WinKey` and forwards the bytes.
 *
 * Reserve rule (matches Linux): Ctrl+Shift+C / Ctrl+Shift+V stay with
 * the native text control for copy/paste and must NOT reach this
 * encoder; plain Ctrl+C encodes ETX so it interrupts the child.
 */

#ifndef AM_TERMINAL_KEYS_H
#define AM_TERMINAL_KEYS_H

#include <cstddef>
#include <cstdint>
#include <cstring>

namespace amkeys {

/* Win32 VK_* numbers needed here (kept local so this header stays
 * includable without <winuser.h>). */
constexpr int kVkReturn = 0x0D;
constexpr int kVkTab = 0x09;
constexpr int kVkEscape = 0x1B;
constexpr int kVkPrior = 0x21; /* PageUp */
constexpr int kVkNext = 0x22;  /* PageDown */
constexpr int kVkEnd = 0x23;
constexpr int kVkHome = 0x24;
constexpr int kVkLeft = 0x25;
constexpr int kVkUp = 0x26;
constexpr int kVkRight = 0x27;
constexpr int kVkDown = 0x28;
constexpr int kVkInsert = 0x2D;
constexpr int kVkDelete = 0x2E;
constexpr int kVkBack = 0x08;
constexpr int kVkF1 = 0x70; /* F1..F12 are kVkF1+0..kVkF1+11. */

struct WinKey {
    int vk;             /* Virtual-key code (VK_* numbers above). */
    char32_t text;      /* UTF-32 code point for printable keys, else 0. */
    bool ctrl = false;  /* Control held. */
    bool shift = false; /* Shift held. */
    bool alt = false;   /* Alt held (AltGr on some layouts: ctrl+alt). */
};

/* Special keys the native text control keeps: the shell must not
 * forward them to the child. Returns true for Ctrl+Shift+C,
 * Ctrl+Shift+V, and bare modifier presses. */
inline bool keep_for_control(const WinKey &k) {
    if (k.ctrl && k.shift && (k.vk == 'C' || k.vk == 'V')) {
        return true;
    }
    if (k.text == 0) {
        switch (k.vk) {
        case 0x10: /* VK_SHIFT */
        case 0x11: /* VK_CONTROL */
        case 0x12: /* VK_MENU (Alt) */
        case 0x5B: /* VK_LWIN */
        case 0x5C: /* VK_RWIN */
            return true;
        default:
            break;
        }
    }
    return false;
}

/* Encode `k` into `out` (capacity must be >= 8). Returns the byte count,
 * or 0 when the key is not forwardable (leave it to the control).
 * Mirrors the Linux switch case for case, including the F-key
 * application-cursor (`\x1bO`) vs tilde (`\x1b[n~`) split. */
inline std::size_t encode_key(const WinKey &k, char *out) {
    if (keep_for_control(k)) {
        return 0;
    }
    auto put = [&](const char *s, std::size_t n) {
        std::memcpy(out, s, n);
        return n;
    };
    switch (k.vk) {
    case kVkReturn:
        /* Like Linux KP_Enter/ISO_Enter: Return always sends CR. */
        return put("\r", 1);
    case kVkBack:
        out[0] = 0x7f; /* DEL, like Linux BackSpace. */
        return 1;
    case kVkTab:
        out[0] = '\t';
        return 1;
    case kVkEscape:
        out[0] = 0x1b;
        return 1;
    case kVkUp:
        return put("\x1b[A", 3);
    case kVkDown:
        return put("\x1b[B", 3);
    case kVkRight:
        return put("\x1b[C", 3);
    case kVkLeft:
        return put("\x1b[D", 3);
    case kVkHome:
        return put("\x1b[H", 3);
    case kVkEnd:
        return put("\x1b[F", 3);
    case kVkInsert:
        return put("\x1b[2~", 4);
    case kVkDelete:
        return put("\x1b[3~", 4);
    case kVkPrior:
        return put("\x1b[5~", 4);
    case kVkNext:
        return put("\x1b[6~", 4);
    case kVkF1:
        return put("\x1bOP", 3);
    case kVkF1 + 1:
        return put("\x1bOQ", 3);
    case kVkF1 + 2:
        return put("\x1bOR", 3);
    case kVkF1 + 3:
        return put("\x1bOS", 3);
    case kVkF1 + 4:
        return put("\x1b[15~", 5);
    case kVkF1 + 5:
        return put("\x1b[17~", 5);
    case kVkF1 + 6:
        return put("\x1b[18~", 5);
    case kVkF1 + 7:
        return put("\x1b[19~", 5);
    case kVkF1 + 8:
        return put("\x1b[20~", 5);
    case kVkF1 + 9:
        return put("\x1b[21~", 5);
    case kVkF1 + 10:
        return put("\x1b[23~", 5);
    case kVkF1 + 11:
        return put("\x1b[24~", 5);
    default:
        break;
    }
    if (k.text == 0 || k.text > 0x10FFFF) {
        return 0; /* Media keys, Super, unknown: leave to the control. */
    }
    if (k.ctrl && !k.alt && k.text < 128) {
        /* Ctrl+letter -> control code (Ctrl+C interrupts the child;
         * copy stays on Ctrl+Shift+C, kept above). */
        char32_t lower = k.text;
        if (lower >= U'A' && lower <= U'Z') {
            lower = lower - U'A' + U'a';
        }
        if (lower >= U'a' && lower <= U'z') {
            out[0] = static_cast<char>(lower - U'a' + 1);
            return 1;
        }
        return 0;
    }
    /* UTF-32 -> UTF-8 (replaces g_unichar_to_utf8; the shell forwards
     * raw bytes, already key-encoded, via bridge_write).
     *
     * AltGr arrives as Ctrl+Alt: on Windows layouts it is a character
     * modifier (e.g. AltGr+2 for @), not Meta, so it passes through as
     * plain input. Bare Alt keeps the Linux ESC-prefix parity. */
    char32_t cp = k.text;
    bool meta = k.alt && !k.ctrl;
    if (cp < 0x80) {
        if (meta) {
            out[0] = 0x1b;
            out[1] = static_cast<char>(cp);
            return 2;
        }
        out[0] = static_cast<char>(cp);
        return 1;
    }
    char tmp[4];
    std::size_t m = 0;
    if (cp < 0x800) {
        tmp[m++] = static_cast<char>(0xC0 | (cp >> 6));
        tmp[m++] = static_cast<char>(0x80 | (cp & 0x3F));
    } else if (cp < 0x10000) {
        tmp[m++] = static_cast<char>(0xE0 | (cp >> 12));
        tmp[m++] = static_cast<char>(0x80 | ((cp >> 6) & 0x3F));
        tmp[m++] = static_cast<char>(0x80 | (cp & 0x3F));
    } else {
        tmp[m++] = static_cast<char>(0xF0 | (cp >> 18));
        tmp[m++] = static_cast<char>(0x80 | ((cp >> 12) & 0x3F));
        tmp[m++] = static_cast<char>(0x80 | ((cp >> 6) & 0x3F));
        tmp[m++] = static_cast<char>(0x80 | (cp & 0x3F));
    }
    if (meta) {
        out[0] = 0x1b;
        std::memcpy(out + 1, tmp, m);
        return m + 1;
    }
    std::memcpy(out, tmp, m);
    return m;
}

} /* namespace amkeys */

#endif /* AM_TERMINAL_KEYS_H */

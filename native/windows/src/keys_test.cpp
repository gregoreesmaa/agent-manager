// Unit tests for the converse-key contract the WinUI shell relies on
// (terminal_keys.h): Return always encodes CR (prompt submit) and plain
// Ctrl+C encodes ETX (child interrupt), while the Ctrl+Shift+C/V reserve
// encodes to nothing so copy/paste stay with the native control.
//
// These are the exact keys `TermBox_PreviewKeyDown`
// (winui/MainWindow.xaml.cpp) tunnels: the read-only output box swallows
// them before they can bubble to `RootGrid_KeyDown`, so the shell must
// forward them first — through this same table, no fork. Exit 0 on
// success, 1 on first failure. Compiles as C++17 with no Windows headers.

#include "terminal_keys.h"

#include <cstdio>

static int failures = 0;

#define CHECK_BYTES(key_expr, want, want_n)                                   \
    do {                                                                     \
        amkeys::WinKey k_ = (key_expr);                                      \
        char out_[8]{};                                                      \
        std::size_t n_ = amkeys::encode_key(k_, out_);                       \
        if (n_ != (std::size_t)(want_n) ||                                   \
            (want_n > 0 &&                                                   \
             __builtin_memcmp(out_, (want), (want_n)) != 0)) {                \
            printf("FAIL %s:%d: n=%zu want %d\n", __func__, __LINE__, n_,    \
                   (int)(want_n));                                           \
            failures++;                                                      \
            return;                                                          \
        }                                                                    \
    } while (0)

static amkeys::WinKey key(int vk, char32_t text, bool ctrl, bool shift,
                          bool alt) {
    amkeys::WinKey k;
    k.vk = vk;
    k.text = text;
    k.ctrl = ctrl;
    k.shift = shift;
    k.alt = alt;
    return k;
}

// The tunneled submit key: Return encodes CR whatever the layout
// resolution produced (printable, dead, or nothing).
static void test_return_encodes_cr(void) {
    CHECK_BYTES(key(amkeys::kVkReturn, 0, false, false, false), "\r", 1);
    CHECK_BYTES(key(amkeys::kVkReturn, U'\r', false, false, false), "\r", 1);
    CHECK_BYTES(key(amkeys::kVkReturn, 0, false, true, false), "\r", 1);
}

// The tunneled interrupt key: plain Ctrl+C encodes ETX for the child.
static void test_ctrl_c_encodes_etx(void) {
    const char etx = 0x03;
    CHECK_BYTES(key('C', U'C', true, false, false), &etx, 1);
    CHECK_BYTES(key('C', U'c', true, false, false), &etx, 1);
}

// The reserve rule: Ctrl+Shift+C/V stay with the native control for
// copy/paste and must NOT reach the encoder output.
static void test_reserve_stays_with_control(void) {
    CHECK_BYTES(key('C', U'C', true, true, false), "", 0);
    CHECK_BYTES(key('V', U'V', true, true, false), "", 0);
}

int main(void) {
    test_return_encodes_cr();
    test_ctrl_c_encodes_etx();
    test_reserve_stays_with_control();
    if (failures == 0) {
        printf("KEYS-OK\n");
        return 0;
    }
    printf("KEYS-FAIL failures=%d\n", failures);
    return 1;
}

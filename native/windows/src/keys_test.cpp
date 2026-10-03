// Bridge tests for the converse-key contract the WinUI shell relies on
// (the shared core key table behind `bridge_key_encode`): Return always
// encodes CR (prompt submit) and plain Ctrl+C encodes ETX (child
// interrupt), while the Ctrl+Shift+C/V reserve encodes to nothing so
// copy/paste stay with the native control.
//
// These are the exact keys `TermBox_PreviewKeyDown`
// (winui/MainWindow.xaml.cpp) tunnels: the read-only output box swallows
// them before they can bubble to `RootGrid_KeyDown`, so the shell must
// forward them first — through the shared table, no fork. Exit 0 on
// success, 1 on first failure. Links the real staticlib via
// core_bridge.c.

#include "core_bridge.h"

#include <cstdio>
#include <cstring>

static int failures = 0;

#define CHECK_BYTES(key_, char_, ctrl_, alt_, want, want_n)                  \
    do {                                                                     \
        unsigned char out_[16]{};                                            \
        int n_ = bridge_key_encode((key_), (char_), (ctrl_), (alt_), out_,   \
                                   sizeof out_);                             \
        if (n_ != (int)(want_n) ||                                           \
            (want_n > 0 && memcmp(out_, (want), (want_n)) != 0)) {            \
            printf("FAIL %s:%d: n=%d want %d\n", __func__, __LINE__, n_,     \
                   (int)(want_n));                                           \
            failures++;                                                      \
            return;                                                          \
        }                                                                    \
    } while (0)

// The tunneled submit key: Return encodes CR (typed char or nothing).
static void test_return_encodes_cr(void) {
    CHECK_BYTES("enter", nullptr, 0, 0, "\r", 1);
    CHECK_BYTES("enter", "\r", 0, 0, "\r", 1);
}

// The tunneled interrupt key: plain Ctrl+C encodes ETX for the child.
static void test_ctrl_c_encodes_etx(void) {
    const char etx = 0x03;
    CHECK_BYTES("c", "C", 1, 0, &etx, 1);
    CHECK_BYTES("c", "c", 1, 0, &etx, 1);
}

// The reserve rule: Ctrl+Shift+C/V stay with the native control for
// copy/paste — the shell gates them before encoding, and the table
// itself has no such binding (unknown combos encode to nothing).
static void test_reserve_stays_with_control(void) {
    CHECK_BYTES("shift", nullptr, 0, 0, "", 0);
    CHECK_BYTES("capslock", nullptr, 0, 0, "", 0);
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

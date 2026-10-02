/* Unit tests for the WinUI picker logic (native/windows/src/picker.h).
 * Mirrors the Linux `am-picker-test` cases so both shells pin the same
 * catalog/recents/folder/yolo/preview behavior. Any C++17 compiler
 * builds it; exit 0 prints PICKER-OK, 1 on first failure. */

#include "picker.h"

#include <cstdio>
#include <string>

static int failures = 0;

#define CHECK(cond, what)                              \
    do {                                               \
        if (!(cond)) {                                 \
            std::printf("PICKER-FAIL %s\n", what);     \
            ++failures;                                \
        }                                              \
    } while (0)

static void catalog_parses_in_order(void) {
    std::string catalog =
        "[{\"id\":\"muse\",\"program\":\"muse\",\"path\":\"C:\\\\muse.exe\","
        "\"available\":true},"
        "{\"id\":\"claude\",\"program\":\"claude\",\"path\":null,"
        "\"available\":false}]";
    auto clis = picker::parse_clis(catalog);
    CHECK(clis.size() == 2, "catalog count");
    CHECK(clis[0].id == "muse", "catalog order");
    CHECK(clis[0].available, "muse available");
    CHECK(clis[0].path == "C:\\muse.exe", "muse path");
    CHECK(clis[1].id == "claude", "claude second");
    CHECK(!clis[1].available, "claude missing");
    CHECK(clis[1].path.empty(), "missing path empty");
    CHECK(picker::parse_clis("[]").empty(), "empty catalog");
    CHECK(picker::parse_clis("nope").empty(), "malformed catalog");
}

static void recents_parse(void) {
    auto recents = picker::parse_recents("[\"C:\\\\a\",\"D:\\\\b\"]");
    CHECK(recents.size() == 2, "recents count");
    CHECK(recents[0] == "C:\\a", "recents order");
    CHECK(picker::parse_recents("[]").empty(), "empty recents");
}

static void folder_yolo_preview(void) {
    CHECK(picker::effective_folder("").empty(), "blank inherits");
    CHECK(picker::effective_folder("   ").empty(), "spaces inherit");
    CHECK(picker::effective_folder("  C:\\a  ") == "C:\\a",
          "folder trimmed");
    CHECK(picker::yolo_value(0) == 0, "yolo default");
    CHECK(picker::yolo_value(1) == 1, "yolo on");
    CHECK(picker::yolo_value(2) == -1, "yolo off");
    CHECK(picker::preview("muse", "C:\\a", 1) ==
              "runs: muse in C:\\a + yolo",
          "preview full");
    CHECK(picker::preview("claude", "", 0) == "runs: claude",
          "preview bare");
    CHECK(picker::preview("muse", "", -1) == "runs: muse (yolo off)",
          "preview off");
}

int main(void) {
    catalog_parses_in_order();
    recents_parse();
    folder_yolo_preview();
    if (failures == 0) {
        std::printf("PICKER-OK\n");
    }
    return failures ? 1 : 0;
}

# Windows shell C suite over the core C ABI (issue #64).
#
# Configure (from the repo root, after installing the MSVC environment,
# e.g. ilammy/msvc-dev-cmd on CI):
#   cargo build --lib                    # produces target/debug/agent_manager.lib
#   cmake -S native/windows -B native/windows/build "-Dcore_lib_dir=$PWD/target/debug"
#   cmake --build native/windows/build --config Release
#   ctest --test-dir native/windows/build -C Release --output-on-failure
#
# The WinUI app itself is MSVC+XAML (AgentManagerWinUI.vcxproj) and is
# built with msbuild after `nuget restore`; this CMake project covers
# the portable C core every shell shares (reconciler, bridge, smoke) so
# the logic stays green even where XAML cannot compile.
cmake_minimum_required(VERSION 3.21)
project(agent-manager-win C CXX)

enable_testing()

set(CMAKE_C_STANDARD 11)
set(CMAKE_C_STANDARD_REQUIRED ON)

set(CORE_LIB_DIR "" CACHE STRING
  "Directory holding agent_manager.lib (default: <repo>/target/debug). Build it first with: cargo build --lib")
if(CORE_LIB_DIR STREQUAL "")
  get_filename_component(REPO_ROOT "${CMAKE_CURRENT_SOURCE_DIR}/../.." ABSOLUTE)
  set(CORE_LIB_DIR "${REPO_ROOT}/target/debug")
endif()
find_library(CORE_LIB agent_manager PATHS "${CORE_LIB_DIR}" REQUIRED NO_DEFAULT_PATH)
message(STATUS "core staticlib: ${CORE_LIB}")

include_directories(
  "${CMAKE_CURRENT_SOURCE_DIR}/src"
  "${CMAKE_CURRENT_SOURCE_DIR}/../../include")

# The Rust staticlib's Windows link closure (sockets, env, crypto, ntdll).
set(SYS_LIBS ${CORE_LIB} ws2_32 userenv bcrypt ntdll)

# Shared-logic contract tests: the reconciler, key table, preview, and
# picker helpers now live in the core (`am_feed_delta`, `am_key_encode`,
# `am_spawn_preview`, ...), pinned by `cargo test --lib`. The per-shell C
# ports are deleted — shells call the core instead of reimplementing it.
# These targets pin the bridge's own thin wrappers (malloc/free
# discipline, NULL conventions) against the real staticlib; no WinUI
# needed, runs anywhere.
add_executable(am-win-feed-test src/feed_test.c src/core_bridge.c)
target_link_libraries(am-win-feed-test PRIVATE ${SYS_LIBS})
add_test(NAME feed COMMAND am-win-feed-test)

# 2D-launch picker logic tests (catalog/recents/folder/yolo/preview):
# header-only C++, no WinUI, runs anywhere.
add_executable(am-win-picker-test src/picker_test.cpp)
target_compile_features(am-win-picker-test PRIVATE cxx_std_17)
add_test(NAME picker COMMAND am-win-picker-test)
set_tests_properties(picker PROPERTIES PASS_REGULAR_EXPRESSION "PICKER-OK")

# Converse-key contract tests (bridge wrappers over the shared core key
# table, no WinUI): pins the Return / Ctrl+C encodings the terminal
# preview-tunnel relies on, plus the Ctrl+Shift+C/V reserve that stays
# with the native control.
add_executable(am-win-keys-test src/keys_test.cpp src/core_bridge.c)
target_link_libraries(am-win-keys-test PRIVATE ${SYS_LIBS})
set_target_properties(am-win-keys-test PROPERTIES CXX_STANDARD 17)
add_test(NAME keys COMMAND am-win-keys-test)
set_tests_properties(keys PROPERTIES PASS_REGULAR_EXPRESSION "KEYS-OK")

# Headless C ABI runtime check: no session needed, CI-greppable output.
add_executable(am-win-smoke src/smoke.c src/core_bridge.c)
target_link_libraries(am-win-smoke PRIVATE ${SYS_LIBS})
add_test(NAME smoke COMMAND am-win-smoke)
set_tests_properties(smoke PROPERTIES PASS_REGULAR_EXPRESSION "SMOKE-OK")

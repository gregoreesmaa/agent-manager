# Linux GTK4/libadwaita shell over the core C ABI (issue #63).
#
# Configure (from the repo root):
#   cargo build --lib                       # produces target/debug/libagent_manager.a
#   meson setup native/linux/build native/linux \
#     -Dcore_lib_dir=$PWD/target/debug
#   meson compile -C native/linux/build
#   meson test -C native/linux/build
project('agent-manager-gtk', 'c',
  version: '0.1.0',
  default_options: ['c_std=c11', 'warning_level=2', 'werror=true'])

cc = meson.get_compiler('c')
gtk4 = dependency('gtk4')
adw = dependency('libadwaita-1')
vte = dependency('vte-2.91-gtk4')

core_lib_dir = get_option('core_lib_dir')
if core_lib_dir == ''
  # Absolute host path is fine for find_library dirs (not an include dir);
  # meson.current_build_dir() never lives inside the source tree here.
  core_lib_dir = meson.current_source_dir() / '..' / '..' / 'target' / 'debug'
endif
# The core staticlib: plain archive, no pkg-config file. The Rust objects
# it references pull in only libdl/pthread/m (linked below).
core_lib = cc.find_library('agent_manager', dirs: core_lib_dir, required: true)
# Relative include dirs (meson forbids absolute source-tree include paths).
core_inc = include_directories('../../include')
src_inc = include_directories('src')

<<<<<<< HEAD
# Pure-logic reconciler unit tests: no GTK needed, runs anywhere.
feed_test_exe = executable('am-feed-test',
  'src/feed.c', 'src/feed_test.c',
  include_directories: src_inc,
  install: false)
test('feed', feed_test_exe)

# 2D-launch picker logic tests: catalog/recents parsing, folder/yolo
# mapping, preview copy. No GTK, no core link (pure C under
# AM_PICKER_TEST), so this runs on any runner.
picker_test_exe = executable('am-picker-test',
  'src/picker.c',
  c_args: '-DAM_PICKER_TEST',
  include_directories: src_inc,
  install: false)
test('picker', picker_test_exe)

=======
>>>>>>> 204d41b (Windows/macOS UX parity over shared core helpers)
# The Rust staticlib's own link closure (sockets, threads, math).
sys_libs = [core_lib,
            cc.find_library('dl', required: false),
            cc.find_library('pthread', required: false),
            cc.find_library('m', required: false)]

# Shared-presentation contract tests over the real staticlib: the feed
# reconciler, roster match, and age label live in the core now
# (`src/shell_shared.rs`), so this pins the C ABI edge instead of a
# vendored copy. No GTK needed, runs anywhere.
feed_test_exe = executable('am-feed-test',
  'src/feed_test.c',
  include_directories: [src_inc, core_inc],
  dependencies: sys_libs,
  install: false)
test('feed', feed_test_exe)

# Headless C ABI runtime check: no display needed, CI-greppable output.
smoke_exe = executable('am-gtk-smoke',
  'src/smoke.c', 'src/core_bridge.c',
  include_directories: [src_inc, core_inc],
  dependencies: sys_libs,
  install: false)
test('smoke', smoke_exe)
test('smoke-live', find_program('tests/smoke_live.sh'),
  args: [smoke_exe.full_path()],
  timeout: 120)

# The GTK shell itself.
executable('agent-manager-gtk',
<<<<<<< HEAD
  'src/main.c', 'src/core_bridge.c', 'src/feed.c', 'src/picker.c',
=======
  'src/main.c', 'src/core_bridge.c',
>>>>>>> 204d41b (Windows/macOS UX parity over shared core helpers)
  include_directories: [src_inc, core_inc],
  dependencies: [gtk4, adw, vte] + sys_libs,
  install: true)

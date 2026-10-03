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

# The Rust staticlib's own link closure (sockets, threads, math).
sys_libs = [core_lib,
            cc.find_library('dl', required: false),
            cc.find_library('pthread', required: false),
            cc.find_library('m', required: false)]

# Shared-logic contract tests: the reconciler, key table, preview, and
# picker helpers now live in the core (`am_feed_delta`, `am_key_encode`,
# `am_spawn_preview`, ...), pinned by `cargo test --lib`. The per-shell C
# ports are deleted — shells call the core instead of reimplementing it.
# These two targets pin the bridge's own thin wrappers (malloc/free
# discipline, NULL conventions) against the real staticlib; no GTK needed.
feed_test_exe = executable('am-feed-test',
  'src/feed_test.c', 'src/core_bridge.c',
  include_directories: [src_inc, core_inc],
  dependencies: sys_libs,
  install: false)
test('feed', feed_test_exe)

picker_test_exe = executable('am-picker-test',
  'src/picker_test.c', 'src/core_bridge.c',
  include_directories: [src_inc, core_inc],
  dependencies: sys_libs,
  install: false)
test('picker', picker_test_exe)

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

# The GTK shell itself (dumb renderer: widgets + events only; all
# behavior lives in the core behind core_bridge.c).
executable('agent-manager-gtk',
  'src/main.c', 'src/core_bridge.c',
  include_directories: [src_inc, core_inc],
  dependencies: [gtk4, adw, vte] + sys_libs,
  install: true)

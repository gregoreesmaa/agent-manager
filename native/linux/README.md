# AgentManagerGTK — native Linux shell (issue #63)

GTK4 + libadwaita + VTE front end over the core staticlib C ABI
(`include/agent_manager.h` at the repo root). Second native shell after
`swift/` (#62): it proves the C ABI is portable, not Swift-only. Core Rust
files are untouched by this shell — it links the framework-free core
library only (the macOS GUI-stack deps in `Cargo.toml` are
platform-scoped, so Linux never compiles that stack).

## Prerequisites (Ubuntu 24.04)

```sh
sudo apt install libgtk-4-dev libadwaita-1-dev libvte-2.91-gtk4-dev \
  meson ninja-build pkg-config gcc xvfb
```

Versions this shell is proved against (container `ubuntu:24.04`):
GTK 4.14.5, libadwaita 1.5.0, VTE 0.76.0 (`vte-2.91-gtk4`).

## Build

The core staticlib must exist first (meson searches `target/debug` by
default, or pass `-Dcore_lib_dir=`):

```sh
cargo build --lib            # produces target/debug/libagent_manager.a
meson setup native/linux/build native/linux
meson compile -C native/linux/build
./native/linux/build/agent-manager-gtk
```

## Tests (no display needed)

```sh
meson test -C native/linux/build
```

| Test | What it proves |
|---|---|
| `feed` | `am-feed-test`: the snapshot→stream reconciler, mirroring Swift's `TerminalFeedTests` 1:1 |
| `smoke` | `am-gtk-smoke`: roster count/JSON/status over the real staticlib, OOB contract (`SMOKE-OK sessions=<n>`) |
| `smoke-live` | `smoke_live.sh`: spawn/pump/write/resize against a fake `muse` on `PATH` in a scratch `HOME` (`SMOKE-LIVE-OK`), hermetic — no real agent, no live config |

Headless UI run (window opens, pump ticks, quits on timeout):

```sh
xvfb-run -a ./native/linux/build/agent-manager-gtk
```

## Wiring (all through the C ABI)

| Feature | Path |
|---|---|
| Roster | `am_session_count` + `am_session_json` at launch, `am_status` every 50 ms tick |
| Spawn | New-run button / Ctrl+N → `bridge_spawn` at the live VTE grid size; 2D-launch entry points (`am_spawn_launch` with folder × CLI + yolo, `am_clis_json` catalog, `am_recent_json` recents, `am_note_launch` memory) available for the picker follow-up |
| Converse | key controller encodes → `bridge_write`; pump → `am_feed_delta` → `vte_terminal_feed` |
| Select / copy / paste | native VTE selection + Ctrl+Shift+C/V + right-click menu |
| Scroll | VTE scrollback capped at 10 000 lines, in a `GtkScrolledWindow` |
| Search / filter | sidebar `GtkSearchEntry` filters rows; Ctrl+F find bar via `VteRegex` search |
| History | rows show project/harness/age, restored every launch; per-run VTE scrollback |
| Theme | System/Dark/Light (`AdwStyleManager` + VTE palette), plain-file pref, no GSettings schema |
| Persistence | Save button / Ctrl+S / close hook → `bridge_core_save` |

## Notes

- The core owns its emulator; the VTE widget owns a second one fed with
  snapshot deltas (`src/feed.c`, a C port of Swift's `TerminalFeed`).
  No PTY is ever spawned inside VTE: typed keys are shell-encoded and
  forwarded, echoed output arrives via the pump. Exactly one line
  discipline (the core's) exists, so nothing double-echoes.
- The shell's own key encoding sends Return as CR, BackSpace as DEL,
  arrows/Home/End/navigation as xterm sequences, Ctrl+letter as control
  codes (Ctrl+C interrupts the child); Ctrl+Shift+C/V stay with VTE for
  copy/paste. Window resizes report the grid back via `bridge_resize`.
- Spawning runs the configured `muse` command; without it on PATH the
  shell toasts the core's error message.
- This directory must stay free of the macOS GUI framework in code and
  prose alike (CI enforces it with a literal grep gate): the Linux shell
  binds the C ABI only.

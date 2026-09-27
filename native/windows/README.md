# AgentManagerWinUI — native Windows shell (issue #64)

WinUI 3 + ConPTY front end over the core staticlib C ABI
(`include/agent_manager.h` at the repo root). Third native shell after
`swift/` (#62) and `native/linux/` (#63): it completes the epic's
per-OS native promise. Core Rust files are untouched by this shell —
it links the framework-free core library only (the macOS GUI-stack
deps in `Cargo.toml` are platform-scoped, so Windows never compiles
that stack).

ConPTY note: no console is created on the WinUI side. The core's
`EmbeddedPty` on Windows is ConPTY-backed (`portable-pty` uses the
native Console Pseudo-terminal API), so `bridge_spawn` *is* the ConPTY
spawn; the WinUI surface renders core snapshots through feed deltas.
Exactly one line discipline (the core's) exists, so nothing
double-echoes — the same single-emulator rule as the Linux shell's
"no PTY inside VTE".

## Prerequisites (Windows 11)

- Visual Studio 2022 17.x with the **Desktop development with C++**
  workload, the **Windows 11 SDK**, and the **C++/WinRT** VSIX
  (all present on the `windows-latest` CI runner)
- Rust 1.90.0 (for the core staticlib)
- CMake 3.21+ (for the portable C suite; on CI it ships with the runner)

## Build

The core staticlib must exist first (CMake and msbuild search
`target/debug` by default, or pass `-Dcore_lib_dir=` /
`/p:CoreLibDir=`):

```powershell
cargo build --lib   # produces target/debug/agent_manager.lib
cmake -S native/windows -B native/windows/build
cmake --build native/windows/build --config Release
ctest --test-dir native/windows/build -C Release --output-on-failure
```

The WinUI app itself (unpackaged exe — no MSIX/installer, which is a
non-goal of #64 and lives with the CI/packaging issue):

```powershell
msbuild -t:restore native/windows/AgentManagerWinUI.vcxproj
msbuild native/windows/AgentManagerWinUI.vcxproj `
  /p:Configuration=Release /p:Platform=x64
```

`Microsoft.WindowsAppSDK` is pinned to the exact 1.6 build CI resolved
(`Version="1.6.250602001"` in the vcxproj) so restores never drift.

## Tests

```powershell
ctest --test-dir native/windows/build -C Release --output-on-failure
powershell -ExecutionPolicy Bypass `
  -File native/windows/tests/smoke_live.ps1 `
  -Smoke native/windows/build/Release/am-win-smoke.exe
```

| Test | What it proves |
|---|---|
| `feed` | `am-win-feed-test`: the snapshot→stream reconciler, mirroring Swift's `TerminalFeedTests` 1:1 |
| `smoke` | `am-win-smoke`: roster count/JSON/status over the real staticlib, OOB contract (`SMOKE-OK sessions=<n>`) |
| `smoke-live` | `smoke_live.ps1`: compiles `tests/fake_muse.c` to `muse.exe`, then spawn/pump/write/resize against it in a scratch profile (`SMOKE-LIVE-OK`), hermetic — no real agent, no live config |

## Wiring (all through the C ABI)

| Feature | Path |
|---|---|
| Roster | `am_session_count` + `am_session_json` at launch, `am_status` every 50 ms tick |
| Spawn | New-run button / Ctrl+N → `bridge_spawn` at 80x25 |
| Converse | key encoder (`src/terminal_keys.h`, layout-aware via ToUnicode) → `bridge_write`; pump → `am_feed_delta` → append to the output box |
| Select / copy / paste | native read-only TextBox selection + Ctrl+Shift+C; Ctrl+V pastes via Clipboard → `bridge_write`; Ctrl+C forwards ETX (interrupts the child) |
| Scroll | output TextBox in a `ScrollViewer`, auto-tails; per-run text retained (capped at 100 000 chars) |
| Search / filter | sidebar filter box trims the roster; find box + Ctrl+F selects the next case-insensitive terminal match |
| History | rows show status/title/project/harness, restored every launch; per-run output retained while the window lives |
| Theme | System/Dark/Light (`RequestedTheme`), kept in `LocalSettings` — local-only, no sync |
| Persistence | Save button / Ctrl+S / close hook → `bridge_core_save` |

## Notes

- The shell's own key encoding sends Return as CR, BackSpace as DEL,
  arrows/Home/End/navigation as xterm sequences, Ctrl+letter as control
  codes (Ctrl+C interrupts the child); Ctrl+Shift+C/V stay with the
  native control for copy. Printable keys resolve through the current
  thread layout (`ToUnicode`), so non-US layouts type correctly; AltGr
  (Ctrl+Alt) passes through as a character modifier while bare Alt keeps
  the Linux ESC-prefix parity.
- Spawning runs the configured `muse` command; without it on PATH the
  shell reports the core's error message in the status bar.
- This directory must stay free of the macOS GUI framework in code and
  prose alike (CI enforces it with a literal grep gate): the Windows
  shell binds the C ABI only.

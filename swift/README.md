# AgentManagerMac — native macOS shell (issue #62)

SwiftUI + SwiftTerm front end over the core staticlib C ABI
(`include/agent_manager.h` at the repo root). Core Rust files are
untouched by this shell except the FFI layer's additive roster getters
(`am_session_count`, `am_session_json`).

## Build

The core staticlib must exist first (the linker searches
`target/debug` and `target/release`):

```sh
cargo build --lib          # produces target/debug/libagent_manager.a
cd swift && swift build    # SPM-only build (no Xcode project needed)
swift test                 # pure-Swift unit tests (snapshot delta)
.build/debug/AgentManagerMac --smoke   # headless C ABI runtime check
```

Run the app with `swift run` or `.build/debug/AgentManagerMac`.

## Wiring (dumb renderer over the core run registry)

The core owns the roster, selection, filter, PTYs, statuses, links, key
table, feed reconciler, preview copy, and persistence; this shell owns
SwiftUI views, event wiring, and byte transport only. Same contract as
the Linux/Windows shells (native look, identical behavior).

| Feature | Path |
|---|---|
| Roster | core registry rows (`am_session_count` + `am_session_json` live, `am_pump_all` refreshes statuses/links), grouped Needs input → Working → Idle → History; single selection stored in the core (`am_selected`/`am_select`) |
| Spawn | split-button 2D launch: `Repeat last session` (Cmd-N) replays the last folder × CLI + yolo via `am_run_spawn` (null CLI/folder, attaches a real roster row under the shared cap); `Choose Folder, CLI, Options…` (Cmd-Shift-N) opens the picker sheet (folder field + recents from `am_recent_json`, CLI radio over `am_clis_json`, tri-state yolo, core preview `am_spawn_preview`) → `am_run_spawn` |
| Restart / close | footer buttons / Cmd-R / Cmd-W + row context menus → `am_run_restart` (same id, keeps title/links) / `am_run_close` + autosave; ended rows offer restart inline in the detail pane |
| Converse | keystrokes `send` -> `am_run_write`; output `am_run_pump` -> `am_run_screen_text`/`am_run_spans_json` -> `am_feed_delta`/`am_ansi_render` -> view feed |
| Select / copy / paste / scroll | native SwiftTerm view and scrollback |
| Search / filter | sidebar search field writes the core filter (`am_set_filter`), rows match via `am_row_matches`; terminal find via Cmd-F (SwiftTerm find bar) |
| History | roster rows carry project, harness, last-active age (core strings); restored every launch; history group holds rows with no live PTY |
| Theme | follows the system appearance (no manual override) + native terminal colors |
| Persistence | automatic: throttled pump autosave + quit/close hooks call `am_core_save` (no Save button) |

## Notes

- The core owns its emulator; the view owns a second one fed with
  snapshot deltas (`am_feed_delta` via `Core.feedDelta`, the shared
  reconciler — the pure-Swift `TerminalFeed` stays for unit tests only).
  Streaming output and typing converge line by line; full redraws
  (resize, cursor-addressed programs) clear and replay the snapshot.
  Styled spans render through the shared `am_ansi_render`.
- New windows resize the view; the view reports its grid back and the
  shell forwards it with `am_run_resize` (every attached run resizes on
  Linux; macOS/Windows resize the selected run's PTY).
- Spawning runs the effective CLI (last-used, configured default, or
  first autodetected, with the stored per-agent flags — the core loads
  the user config, so native spawns honor it); without any CLI on PATH
  the shell shows the core's error message.

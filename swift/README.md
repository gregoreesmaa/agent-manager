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

## Wiring (all through the C ABI)

| Feature | Path |
|---|---|
| Roster | `am_session_count` + `am_session_json` at launch |
| Spawn | split-button 2D launch: `Repeat last session` (Cmd-N) replays the last folder × CLI + yolo via `am_spawn_launch` (null CLI/folder); `Choose Folder, CLI, Options…` (Cmd-Shift-N) opens the picker sheet (folder field + recents from `am_recent_json`, CLI radio over `am_clis_json`, tri-state yolo, spawn preview) → `am_spawn_launch` + `am_note_launch` |
| Converse | keystrokes `send` -> `am_write`; output `am_pump` -> `am_screen_text` -> view feed |
| Select / copy / paste / scroll | native SwiftTerm view and scrollback |
| Search / filter | sidebar search field; terminal find via Cmd-F (SwiftTerm find bar) |
| History | roster rows carry project, harness, last-active age; restored every launch |
| Theme | follows the system appearance (no manual override) + native terminal colors |
| Persistence | Save button and quit hook call `am_core_save` |

## Notes

- The core owns its emulator; the view owns a second one fed with
  snapshot deltas (`ShellSupport.TerminalFeed`). Streaming output and
  typing converge line by line; full redraws (resize, cursor-addressed
  programs) clear and replay the snapshot as plain text.
- New windows resize the view; the view reports its grid back and the
  shell forwards it with `am_resize`.
- Spawning runs the effective CLI (last-used, configured default, or
  first autodetected); without any CLI on PATH the shell shows the
  core's error message.

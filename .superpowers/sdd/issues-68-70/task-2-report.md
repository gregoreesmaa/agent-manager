# Task 2 report — Issue #69: Terminal detail owns full height; header chrome moved into sidebar

## What implemented
`swift/Sources/AgentManagerMac/ContentView.swift` (only file changed):

- Removed the `.toolbar` block (Spawn / Save / Theme picker `ToolbarItem`s) from the
  `NavigationSplitView` sidebar content. The window toolbar is now empty, so macOS
  gives the full window height to the detail column and the terminal grid renders
  edge-to-edge vertically with zero chrome rows above it.
- Added a `sidebarFooter` (Divider + Spawn/Save buttons + System/Dark/Light segmented
  Theme picker) pinned to the bottom of the sidebar via a `VStack(spacing: 0)`.
- Preserved on the moved controls: `Cmd-N` spawn and `Cmd-S` save
  `.keyboardShortcut`s, `canSpawn` disabled state, and both `.help` strings.
  `Cmd-S` remains additionally wired app-wide in `AgentManagerMacApp.commands`
  (pre-existing, untouched).
- Untouched: `NavigationSplitView` structure (narrow/collapsed behavior per #32
  unchanged), `.navigationTitle` + `.searchable` on the sidebar, row labels
  (title / project / harness / age subtitle, status dot, "live" badge — non-color
  cues preserved), the live-session detail branch (still a bare `CoreTerminalView`,
  no title/chrome added), and all of Task 1's area (`AppState`, `CoreBridge`,
  `AnsiFeed` — not modified).

## Tests + results
- `swift build` (swift/ package, includes `AgentManagerMac` target with edited
  `ContentView.swift`): Build complete, no errors.
- `swift test`: all 15 `ShellSupportTests` pass (AnsiFeed + TerminalFeed), 0 failures —
  Task 1's feed behavior not regressed.
- Structural check: repo-wide search confirms zero `ToolbarItem`/`.toolbar` in
  `ContentView.swift`; the only `.navigationTitle` left is the sidebar's; the
  live-session detail branch renders `CoreTerminalView` directly.
- GUI not visually verified (headless session; no window run). Compile + structural
  checks are the verification basis.

## Files changed
- `swift/Sources/AgentManagerMac/ContentView.swift` (+44/−36): toolbar → sidebar footer.

## Self-review findings
- Completeness: every brief clause covered — zero chrome rows above terminal,
  Spawn/Save/Theme reachable in sidebar, Cmd-N/Cmd-S kept, narrow/collapsed
  unchanged, keyboard/non-color/local-only parity kept.
- Quality: single-file, minimal diff; doc comment updated (toolbar → sidebar footer).
- YAGNI: deliberately did NOT surface `savedFlash` in the footer (it is write-only
  in `AppState`, never reset — showing it would add sticky, unrequested behavior);
  did NOT touch `AgentManagerMacApp.swift` commands (Cmd-S app-wide wiring already
  exists; Cmd-N keeps the same view-level shortcut mechanism as the old toolbar
  button, so shadowing behavior is unchanged).
- Real-behavior verification: build + tests + structural grep, all observed this
  session. No new tests added — there is no UI-test harness in the Swift package
  (tests cover ShellSupport only) and the change is view layout, compile-verified.

## Concerns
- Minor/unverified: `.searchable` now sits on a `List` nested in a sidebar `VStack`
  rather than directly as the sidebar root. It propagates to the same navigation
  context, but this was not exercised at runtime — worth a glance on next app run
  (filter field still appears under the sidebar title).
- Minor/unverified: no runtime check that macOS fully collapses the empty window
  toolbar; expected from standard behavior, but confirm visually on next launch.

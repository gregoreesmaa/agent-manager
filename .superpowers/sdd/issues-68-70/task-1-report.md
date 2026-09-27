# Task 1 report — Issue #70: Terminal view is monochrome; restore CLI colors

## What implemented

The pump path was plain-text only (`AppState.pump()` → `Pty.screenText()` →
`TerminalFeed.delta` → `view.feed(text:)`), so SGR colors never reached
SwiftTerm. Chose the brief's second option — extend the snapshot with
ANSI/SGR attributes the Swift side re-emits — because the core half
already existed (`am_spans_json`, documented in `include/agent_manager.h`)
while no raw-byte FFI exists; adding one would have been a larger change
(ordering/replay semantics) for no extra fidelity.

- `swift/Sources/ShellSupport/AnsiFeed.swift` (new): decodes the
  `am_spans_json` rows (`{text,fg,bg,bold,italic,underline}`, fg/bg
  `[r,g,b]` or null) and re-emits each span with a complete SGR sequence
  (`ESC[0;1;3;4;38;2;r;g;b;48;2;r;g;bm`, default → `ESC[0m`). Every span
  is self-contained (reset-first), so any delta fragment (append suffix,
  scroll tail, clear+replay) leaves the view in the right style state and
  the existing `TerminalFeed.delta` works on rendered strings unchanged.
  Returns nil on bad JSON so the pump falls back to plain text.
- `swift/Sources/AgentManagerMac/CoreBridge.swift`: declared the existing
  `am_spans_json` ABI entry (signature matches `include/agent_manager.h`)
  and added `Pty.spansJson()`.
- `swift/Sources/AgentManagerMac/AppState.swift` (`pump`): feeds the
  SGR render when spans decode, else the plain snapshot as before;
  `fedText` stores whichever form was shown. Delta/reconciler,
  no-duplicate, and resize-safe behavior untouched (`TerminalFeed.swift`
  has zero changes). Status/marker UI untouched (non-color parity per #8).
- `swift/Sources/AgentManagerMac/CoreTerminalView.swift`: doc-comment
  update for the new flow. No behavior change.
- No new dependencies (Foundation only); no header/ABI change.

## Tests + results

- Swift `swift/Tests/ShellSupportTests/AnsiFeedTests.swift` (new, 7 tests):
  red-span SGR round-trip (`fg [205,0,0]` → `ESC[0;38;2;205;0;0mred`),
  default reset, bold+underline+bg combining, row LF joining, bad-JSON→nil
  fallback, styled append feeds suffix only, styled first snapshot feeds
  whole screen, color-only change forces clear+replay.
- Rust `src/embedded.rs::red_sgr_reaches_snapshot_spans`: `\x1b[31mred`
  through a real vt100 parser → span fg is `Some(SnapRgb(205,0,0))`.
- Rust `src/ffi.rs::spans_json_preserves_sgr_red` (unix-only): spawns a
  real `printf '\033[31mred\033[0m\n'` child; asserts plain snapshot
  contains `red` with no ESC (documents the old root cause) and spans
  JSON contains `red` + `[205,0,0]`.
- Results: `swift test --filter ShellSupportTests` 15/15 pass (8 existing
  `TerminalFeedTests` untouched and green); `swift build` (full app
  target incl. SwiftTerm link) clean; `cargo test` full suite 105 + 115
  pass, 0 fail; `cargo fmt --check` clean; `cargo clippy --all-targets
  -- -D warnings` clean; `grep paywall|billing|telemetry src/
  swift/Sources/` zero matches.

## Files changed

- `swift/Sources/ShellSupport/AnsiFeed.swift` (new)
- `swift/Tests/ShellSupportTests/AnsiFeedTests.swift` (new)
- `swift/Sources/AgentManagerMac/CoreBridge.swift` (spans ABI + accessor)
- `swift/Sources/AgentManagerMac/AppState.swift` (styled pump + fallback)
- `swift/Sources/AgentManagerMac/CoreTerminalView.swift` (comment only)
- `src/embedded.rs` (red-SGR spans unit test only)
- `src/ffi.rs` (printf end-to-end spans test only)

## Self-review findings

- Completeness: all brief clauses covered; `TerminalFeed` delta semantics
  preserved bit-for-bit for plain text; color-only screen changes now
  redraw instead of being silently dropped.
- Quality: per-span reset-first SGR keeps every delta fragment
  self-contained (verified by the append/tail/clear tests); spans-first
  ordering in `pump()` avoids a second snapshot walk per tick.
- YAGNI: no new FFI, no header change, no deps, no status-UI changes;
  malformed colors degrade to default rather than failing the frame.
- Real behavior: tests go through a real vt100 parser, a real child
  process, and hand-computed SGR expectations — not mirrors of the code.

## Concerns

- Live-app eyeball check not done: no running session was launched to
  visually confirm colored output in SwiftTerm (headless session); the
  three-level test chain (parser → FFI bytes → Swift render+delta) is the
  verification. Truecolor SGR (`38;2`) is assumed supported by the pinned
  SwiftTerm 1.20 — standard for that version, but not exercised against
  the real view here.
- `fedText` now stores SGR-decorated strings; a fallback transition
  (spans↔plain between ticks, only on core failure) costs one full
  redraw — accepted as unreachable-in-practice.

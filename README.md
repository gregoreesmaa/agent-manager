# agent-manager

Native GUI harness (gpui, no webview) for managing and chatting with live
`muse` agent CLI sessions. Each run is a real interactive `muse` process
behind an embedded PTY; the window shows the run list beside the live
terminal of the selected run.

## Prerequisites

- macOS with **full Xcode** installed (not just CommandLineTools), the
  accepted license (`sudo xcodebuild -license`), and the Metal toolchain
  component (`xcodebuild -downloadComponent MetalToolchain`). gpui compiles
  its Metal shaders at build time and the `metal` compiler ships only with
  Xcode.
- Rust toolchain (`cargo`).
- The `muse` CLI on `PATH`.

## Build & run

```sh
cargo build
./target/debug/agent-manager
```

`cargo test` runs the suite (65 tests); `cargo fmt --check` must stay clean.

## Using it

- The sessions list starts empty. Press `n` or the **+ New** button for a
  new `muse` run. Runs start with animal placeholder titles (`otter`,
  `fox`, …); your first submitted prompt renames the run so the list stays
  distinguishable.
- The sessions panel is library chrome (`gpui-component` sidebar: header,
  groups, menu rows), dark-themed to match the terminal. Click a run (or
  `j`/`k`) to switch; background runs keep streaming and never die on
  switch. Runs group by **Needs input**, **Idle**, **Active** with
  per-group counts. Every GitHub PR URL ever seen in a run appears under
  its title (accumulated first-seen order, so links that scrolled off stay
  visible); the panel scrolls, and clicking a link copies it.
- Click the terminal (or `Tab`/`i`) to type into `muse`. The terminal title
  is bright while it owns the keyboard; `muse` captures keys **only** in
  this focus. `Tab`/`Esc` returns to the list, `q` quits.
- Drag across the terminal to highlight text (copy-on-select). `y` or
  `Cmd+C` copies the selection (or the whole screen when nothing is
  selected); `p` or `Cmd`/`Ctrl+V` pastes the clipboard into `muse`.

## How it works

- `src/embedded.rs` — PTY spawn/pump/resize/reap via `portable-pty`, plus a
  vt100 emulator. It answers terminal cursor-position queries the way a
  real terminal would; without that, `muse` times out its startup
  handshake and exits.
- `src/gui/terminal.rs` — vt100 screen → styled text rows (framework-free).
- `src/gui/keys.rs` — keystroke → PTY bytes (framework-free).
- `src/gui/shell.rs` — thin gpui view: component-library sessions panel
  (`Sidebar`/`Button`, via `Root` + dark theme in `main.rs`), hand-rolled
  terminal pane, status bar, pump loop, clipboard.
- `src/app.rs` — run list state, titles, activity sort.
- `src/parsers/` — modular link parsers (GitHub PR URLs today).
- `src/providers/` — session-provider abstraction (parked for a future
  historic-attach flow; live runs are spawned in-app).

## Regression shield

gpui moves fast, so framework-volatility protection is a deliverable, not
an afterthought: `terminal.rs`/`keys.rs` and the shell's pump/refresh/key
logic are framework-free and unit-tested, including a test that executes
the exact `StyledText::with_runs` call that once crashed new sessions;
`gpui_version_is_pinned` fails loudly if `gpui` moves off 0.2.2 and
`chrome_stays_on_the_gpui_02_component_line` fails loudly if
`gpui-component` leaves 0.5.x (0.6+ needs the `gpui-pre` fork), so API
changes get reviewed consciously.

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

`cargo test --all-targets` runs the suite; `cargo fmt --check` and
`cargo clippy --all-targets -- -D warnings` must stay clean (all three
run in CI on every push and pull request).

## Free and open source

agent-manager is free software under the MIT license ([LICENSE-MIT](LICENSE-MIT),
also recorded as `license = "MIT"` in `Cargo.toml`). No account, no paywall,
no telemetry, no paid tiers: every run, parser, and provider ships free.
Local state (preferences, persisted runs) stays in plain JSON files on your
machine and never leaves it.

## Using it

### Keyboard

Sessions-first flow: a new user can spawn, switch, copy, and paste without
reading source. `Tab` toggles which pane owns the keyboard.

| Key | List focus | Terminal focus |
| --- | --- | --- |
| `n` | new `muse` run (takes the keyboard) | types into `muse` |
| `j`/`k`, `↓`/`↑` | move selection | types into `muse` |
| `PgDn`/`PgUp` | page the list | types into `muse` |
| `o` | cycle link focus across the selected run's links | types into `muse` |
| `Enter` | copy the focused link, or type into `muse` when none | newline to `muse` (first line titles the run) |
| `i` | type into `muse` | types into `muse` |
| `Tab` | type into `muse` | back to the list |
| `y`, `Cmd+C` | copy selection (or whole screen) | copy selection (or whole screen) |
| `p`, `Cmd`/`Ctrl+V` | paste clipboard into `muse` | paste clipboard into `muse` |
| `r` | restart ended run / retry failed spawn | same, on a dead pane |
| `x` | close (kill) the selected run | — (types `x`) |
| `d` | dismiss the sticky error | — (types `d`) |
| `?` (or `/`) | toggle the in-app help panel | types into `muse` |
| `q`, `Esc` | quit (confirms first with live runs) | back to the list |
| drag | highlight terminal text (copy-on-select) | highlight terminal text |

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
  visible); the panel scrolls, and clicking a link copies it. Keyboard:
  `PgUp`/`PgDn` page the list, `o` moves link focus across the selected
  run's links, and `Enter` copies the focused link (`Enter`/`i` with no
  link focused types into `muse`).
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

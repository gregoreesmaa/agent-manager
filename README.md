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
| `n` | new session (repeats last folder × CLI, takes the keyboard) | types into the agent |
| `N` | new-session picker (choose folder × CLI + yolo) | types into the agent |
| `j`/`k`, `↓`/`↑` | move selection | types into `muse` |
| `PgDn`/`PgUp` | page the list | types into `muse` |
| `Shift+PgDn`/`Shift+PgUp` | scroll the run's retained output (pager) | scroll the run's retained output (pager) |
| `o` | cycle link focus across the selected run's links | types into `muse` |
| `Enter` | copy the focused link, or type into `muse` when none | newline to `muse` (first line titles the run) |
| `i` | type into `muse` | types into `muse` |
| `Tab` | type into `muse` | back to the list |
| `Cmd+1` / `Cmd+2` | sessions list / type into `muse` | sessions list / type into `muse` |
| `y`, `Cmd+C` | copy selection (or whole screen) | copy selection (or whole screen) |
| `e` | export selected run to markdown (local file) | — (types `e`) |
| `p`, `Cmd`/`Ctrl+V` | paste clipboard into `muse` | paste clipboard into `muse` |
| `r` | restart ended run / retry failed spawn | same, on a dead pane |
| `x` | close (kill) the selected run | — (types `x`) |
| `d` | dismiss the sticky error | — (types `d`) |
| `/` | filter sessions by title substring (`Enter` keeps, `Esc` clears) | types into `muse` |
| `+`/`-`, `[`/`]` | terminal font size / panel width (saved to the config file) | types into `muse` |
| `?` | toggle the in-app help panel | types into `muse` |
| `t` | cycle theme (dark → light → system, saved) | — (types `t`) |
| `q`, `Esc` | quit (confirms first with live runs) | back to the list |
| drag | highlight terminal text (copy-on-select) | highlight terminal text |

- The sessions list starts empty. Press `n` or the **+ New** button for a
  new session — it repeats your last folder × CLI instantly (first-ever
  spawn is plain `muse`). Press `N` for the full picker: choose the
  working folder and the CLI (`muse`, `claude`, `opencode`, `codex` —
  autodetected from `PATH`, missing ones listed disabled with install
  guidance), optionally toggle yolo for this run only. Runs start with
  animal placeholder titles (`otter`,
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
- Click the terminal (or `Tab`/`i`/`Cmd+2`) to type into `muse`; the
  status line reads `▸ terminal · typing in muse …` while it owns the
  keyboard (`▸ sessions …` otherwise, plus a frame around the terminal
  pane — focus never depends on color alone). `muse` captures keys
  **only** in this focus, and `/` types there instead of opening the
  filter. `Tab`/`Esc` returns to the list, `Cmd+1`/`Cmd+2` jump
  directly to either pane (even from inside the filter), `q` quits.
  There is no title bar: the terminal owns the full height, and status
  hints live in the sessions-panel footer (a slim bar under the terminal
  on narrow windows).
- Drag across the terminal to highlight text (copy-on-select). `y` or
  `Cmd+C` copies the selection (or the whole screen when nothing is
  selected); `p` or `Cmd`/`Ctrl+V` pastes the clipboard into `muse`.

## Configuration

`~/.config/agent-manager/config.json` (JSON, all keys optional; a missing
or malformed file means defaults). The empty terminal pane always shows
the effective spawn command, so you can see your flags before launch.

### Per-agent startup flags

```json
{ "agents": { "muse": { "extra_args": ["--yolo"] } } }
```

`extra_args` append to every spawn of that agent binary — e.g. `muse`
launches as `muse --yolo`, `claude` could carry
`--dangerously-skip-permissions`. Keys are program names, so any
supported agent gets its own flags.

### New-session picker defaults (folder × CLI + yolo)

```json
{
  "agents": { "muse": { "yolo": true } },
  "default_cli": "muse",
  "default_cwd": "/Users/you/projects/agent-manager"
}
```

`agents.<cli>.yolo` opts a CLI into yolo by default (off unless set —
destructive flags are opt-in, never silent); the picker's yolo toggle
overrides it for one run only and never writes back. `default_cli` /
`default_cwd` preselect the picker axes; every confirmed spawn refreshes
`last_cli` + `recent_folders` (top 10, MRU-first) so `n` repeats your
last combination. The supported CLIs (`muse`, `claude`, `opencode`,
`codex`) are autodetected from `PATH` on every picker open — missing
ones stay listed (disabled) with install guidance instead of vanishing.

### Theme: dark / light / follow system

```json
{ "theme": "system" }
```

`"dark"` or `"light"` pins the component chrome; `"system"` (default)
follows the OS appearance at startup. Press `t` in the sessions list to
cycle dark → light → system — the choice applies immediately and is
saved back to the config file.

### Terminal font

```json
{ "terminal": { "font_family": "JetBrainsMono Nerd Font", "font_size": 13.0 } }
```

The terminal panes default to **JetBrainsMono Nerd Font** (OFL-licensed,
unambiguous glyphs, full box-drawing/block coverage, Nerd Font symbols
for agent status lines). Recommended install:

```sh
brew install --cask font-jetbrains-mono-nerd-font
```

(Linux: download the `JetBrainsMono` Nerd Font release zip from
ryanoasis/nerd-fonts and install the TTFs.) Behind the primary runs an
explicit fallback chain — system emoji, CJK monospace fallbacks, then
system monospace — so emoji/CJK render at correct double width instead
of tofu. Set `terminal.font_family` to any installed patched font to
override just the head of the chain; `terminal.font_size` and
`terminal.fallback_fonts` are overridable too.

### Comfort keys

In the sessions list, `+`/`-` resize the terminal font and `[`/`]`
resize the sessions panel — both write back to the config file
(`terminal.font_size`, top-level `sidebar_width`), so they survive
restarts. `/` filters the panel by title substring (`Enter` keeps the
filter, `Esc` clears it) without changing sort order.

## How it works

- `src/embedded.rs` — PTY spawn/pump/resize/reap via `portable-pty`, plus a
  vt100 emulator. It answers terminal cursor-position queries the way a
  real terminal would; without that, `muse` times out its startup
  handshake and exits.
- `src/gui/terminal.rs` — vt100 screen → styled text rows (framework-free).
- `src/gui/keys.rs` — keystroke → PTY bytes (framework-free).
- `src/gui/shell.rs` — thin gpui view: component-library sessions panel
  (`Sidebar`/`Button`, via `Root` + theme in `main.rs`), hand-rolled
  terminal pane, status line (panel footer; slim bar on narrow windows),
  pump loop, clipboard.
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

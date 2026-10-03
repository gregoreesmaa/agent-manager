# staap

Local-first mission control for agent runs. Each run is a real interactive
agent CLI session behind an embedded PTY; the window shows the run list
beside the live terminal of the selected run. One framework-free Rust core
(run registry, PTY pipeline, link parsers, providers, C ABI in
`include/staap.h`) with four thin native shells over it: the original macOS
gpui shell (`src/main.rs` + `src/gui/`), `StaapMac` (SwiftUI + SwiftTerm),
`staap-gtk` (GTK4/libadwaita + VTE), and `StaapWinUI` (WinUI 3 + ConPTY).

## Why staap?

`staap` is Estonian for (military) headquarters — the command post where
field units report in and the duty officer sees, at a glance, where
attention is needed. That is what this app is: your HQ for agent runs.

Each run is a field unit reporting back over a live channel. The session
behind the terminal is a real interactive PTY, never a simulation, so what
you see on the board is what is actually happening in the field. The roster
is the situation board: every unit pinned where you can scan it in seconds.
When a run needs input, it is the flag stuck in the map table — triage by
urgency, the duty officer's glance, surfacing in the run list, the status
bar, and the background window title within a minute.

The HQ works with the radios off: local-first and offline, state in plain
JSON files on your machine, zero accounts, zero telemetry, nothing ever
leaving the building. And it recovers in one click — no silent losses, no
link or transcript ever dropped, because a headquarters that loses reports
is no headquarters at all. Free and open source forever: an HQ anyone can
build.

## Prerequisites

- Rust toolchain (`cargo`) — every shell needs the core staticlib first
  (`cargo build --lib`).
- At least one agent CLI on `PATH` (`muse`, `claude`, `opencode`, `codex`
  — autodetected; `muse` stays first as the historic default).
- Per-shell toolchain (see the shell READMEs for detail):
  - gpui shell + `StaapMac`: macOS with **full Xcode** (the gpui shell
    compiles its Metal shaders at build time, which needs the Xcode Metal
    toolchain, not just CommandLineTools).
  - `staap-gtk`: Ubuntu 24.04 GTK4/libadwaita/VTE dev libs + meson
    (`native/linux/README.md`).
  - `StaapWinUI`: Visual Studio 2026 with the UWP build-tools workload
    (`native/windows/README.md`).

## Build & run

```sh
cargo build --lib   # core staticlib every shell links (target/debug/libstaap.a)
```

Then one shell:

| Shell | Build & run |
| --- | --- |
| gpui (macOS) | `cargo build` then `./target/debug/staap` |
| `StaapMac` | `(cd swift && swift build)` then `swift/.build/debug/StaapMac` (or `./build-and-run.sh`) |
| `staap-gtk` | `meson setup native/linux/build native/linux && meson compile -C native/linux/build`, then `./native/linux/build/staap-gtk` |
| `StaapWinUI` | `msbuild native/windows/StaapWinUI.vcxproj` (see `native/windows/README.md`; or `build-and-run.ps1`) |

`cargo test --all-targets` runs the suite; `cargo fmt --check` and
`cargo clippy --all-targets -- -D warnings` must stay clean (all three
run in CI on every push and pull request).

## Free and open source

staap is free software under the MIT license ([LICENSE-MIT](LICENSE-MIT),
also recorded as `license = "MIT"` in `Cargo.toml`). No account, no paywall,
no telemetry, no paid tiers: every run, parser, and provider ships free.
Local state (preferences, persisted runs) stays in plain JSON files on your
machine and never leaves it.

## Using it

Every shell shares one contract: sessions-first flow (spawn, switch, type,
copy, paste), runs grouped **Needs input → Working → Idle → History** with
per-group counts, background runs keep streaming and never die on switch,
and every link ever seen in a run stays attached under its title
(accumulated first-seen order, click/keyboard to copy). New sessions repeat
your last folder × CLI instantly, or open the picker (folder + autodetected
CLI + per-run yolo toggle). Shortcuts below are the gpui shell's; each
native shell's README documents its own keys for the same actions.

### Keyboard (gpui shell)

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
  switch. Runs group by **Needs input → Working → Idle**, then **History**
  (ended runs, restorable), with per-group counts. Every link ever seen in
  a run — GitHub PR/issue/commit URLs and `file:line` references —
  appears under its title (accumulated first-seen order, so links that
  scrolled off stay visible); the panel scrolls, and clicking a link
  copies it. Keyboard: `PgUp`/`PgDn` page the list, `o` moves link focus
  across the selected run's links, and `Enter` copies the focused link
  (`Enter`/`i` with no link focused types into `muse`).
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

`~/.config/staap/config.json` (JSON, all keys optional; a missing
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
  "default_cwd": "/Users/you/projects/staap"
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

Framework-free Rust core (no toolkit types; its headless tests are the
cross-platform contract), exposed to native shells through the C ABI in
`include/staap.h` (regenerate with cbindgen after any FFI change):

- `src/embedded.rs` — PTY spawn/pump/resize/reap via `portable-pty`, plus a
  vt100 emulator. It answers terminal cursor-position queries the way a
  real terminal would; without that, the agent CLI times out its startup
  handshake and exits.
- `src/app.rs` + `src/runs.rs` — run registry: titles, attention status,
  urgency sort, eviction.
- `src/keys.rs`, `src/shell_shared.rs` — shared key table and
  feed-reconciler every shell uses instead of reimplementing.
- `src/parsers/` — modular link parsers (GitHub PR/issue/commit URLs,
  `file:line` references).
- `src/providers/` + `src/launch.rs` — per-CLI providers and launch
  catalog (`muse`, `claude`, `opencode`, `codex`).
- `src/config.rs`, `src/persist.rs` — plain-file local state
  (`~/.config/staap/config.json`).
- `src/transcript.rs`, `src/scrollback.rs` — export and retained output.

Thin shells over the core (seam spec: `docs/native-core-seam.md`):

| Shell | Stack | Detail |
| --- | --- | --- |
| gpui (`src/main.rs` + `src/gui/`) | gpui, macOS-only | Original shell; component-library sessions panel, hand-rolled terminal pane |
| `swift/` | SwiftUI + SwiftTerm | `StaapMac` (`swift/README.md`) |
| `native/linux/` | GTK4/libadwaita + VTE | `staap-gtk` (`native/linux/README.md`) |
| `native/windows/` | WinUI 3 + ConPTY | `StaapWinUI` (`native/windows/README.md`) |

## Regression shield

gpui moves fast, so framework-volatility protection is a deliverable, not
an afterthought: `terminal.rs`/`keys.rs` and the shell's pump/refresh/key
logic are framework-free and unit-tested, including a test that executes
the exact `StyledText::with_runs` call that once crashed new sessions;
`gpui_version_is_pinned` fails loudly if `gpui` moves off 0.2.2 and
`chrome_stays_on_the_gpui_02_component_line` fails loudly if
`gpui-component` leaves 0.5.x (0.6+ needs the `gpui-pre` fork), so API
changes get reviewed consciously.

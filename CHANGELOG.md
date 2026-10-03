# Changelog

All notable changes, newest first. Issue numbers track `gh issue`.

## Unreleased (on main)

- BREAKING: project renamed `agent-manager` → `staap` (no shims). Binary,
  package, and lib are now `staap` (`staap.lib` / `libstaap.a`), the C
  header is `include/staap.h` (guard `STAAP_H`), all FFI entry points are
  `staap_*` with `Staap*` types, config moved to
  `~/.config/staap/config.json` (override via `STAAP_CONFIG`), the data
  dir moved to the `staap` subdir, shells are `StaapMac` / `staap-gtk` /
  `StaapWinUI`, CI artifacts are `StaapMac-macos` / `StaapWindows` /
  `staap-linux`, app id is `com.example.staap`. Old binary name, config
  path, and the old `am_*` ABI no longer exist — reinstall and re-point
  any scripts. Historic entries below keep the old name.
- UX: two-dimensional new session (folder × CLI + yolo) — `n` / `+ New`
  repeats the last launch instantly, `N` opens the full picker (folder
  axis + CLI axis + one-shot yolo tri-state, autodetected from `PATH`
  on every open, missing CLIs listed disabled with install guidance).
  Per-agent yolo defaults + `default_cli`/`default_cwd` persist in the
  local config; `last_cli` + folder MRU refresh on every spawn. Core
  seam (`src/launch.rs`, `SpawnKind::NewOn`, `App::start_launch`, FFI
  `am_spawn_launch`/`am_clis_json`/`am_recent_json`/`am_note_launch`;
  see `docs/new-session-picker.md` for the 10-persona synthesis).
  Native shells bind the same model: Swift picker sheet + repeat-last
  menu (`swift test` pins the pure-Swift picker model), GTK split-button
  + AdwDialog picker (`am-picker-test` + smoke catalog check), WinUI
  repeat + ContentDialog picker (`am-win-picker-test` + smoke catalog
  check).
- UX: restore window resizing — hidden titlebar is `Some`-transparent
  again (`None` dropped `NSResizableWindowMask` on macOS), edges/corners
  resize, minimum + narrow/wide breakpoints unchanged (#58).
- UX: sidebar native-rendering spike — keep `gpui-component` sidebar,
  native `NSOutlineView` interop rejected (see
  `docs/57-sidebar-native-eval.md`) (#57).

- UX: sticky errors for spawn + PTY write failures (#2) — transient info
  keeps the 3s TTL, errors stay until dismissed (`d`) or the next success.
- UX: ended-run Restart/Rerun affordance, header button + `r` (#3).
- UX: dirty-quit confirm + per-run close/kill (`x`) (#4).
- UX: parsed-link children capped at 20 rows + `N more` (#5).
- Tech: dirty-gated 20Hz pump repaint (#11) — tick reports dirt, resort
  gated, exit transitions count.
- Tech: static PR regex + allocation-free attention scan (#12).
- Tech: single `Run` struct replacing parallel HashMaps (#13).
- Tech: `gui/shell.rs` split into focused modules, none over ~400 lines (#14).
- Tech: one `app::classify` status owner for live + historic (#15).
- Tech: CI (`cargo test` + `fmt --check` + `clippy -D warnings`) (#17).
- Tech: registry links capped at 50 + truncation flag (#18).
- Tech: repro hygiene — toolchain pin, CWD-independent pin tests,
  `.DS_Store` ignored (#20).
- Product: MIT license + free-software statement (#21).
- Product: historic attach groundwork — parser garden, related links (#28).
- Product: keyboard-help onboarding table in README (#27).
- Product: free run lifecycle — close/kill + restart (#23).
- UX: full keyboard operability — link focus (`o`), Enter-to-copy,
  PgUp/PgDn paging (#9).
- UX: first-run orientation bundle — New CTA, rename flash,
  empty-vs-error icons (#10).
- UX: responsive narrow layout — sidebar collapse, window minimum,
  compact hints (#6).

## 0.1.0

- Initial native Rust CLI harness (Muse CLI provider, GitHub PR parser,
  ratatui UI) plus the gpui native-shell migration baseline.

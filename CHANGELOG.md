# Changelog

All notable changes, newest first. Issue numbers track `gh issue`.

## Unreleased (on main)

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

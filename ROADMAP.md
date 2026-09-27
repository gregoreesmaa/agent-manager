# Roadmap

Drawn from `VISION.md` / `VISION-UX.md` / `VISION-TECHNICAL.md` /
`VISION-PRODUCT.md` (aligned-vision backlog, approach B: vision-only).

## Now (open issues)

- Contributor + sustainability foundation (#30) — this file set.
- Local-only comfort UX: prefs + filter (#29); theme choice (#34) rides
  with it (persisted locally, no account).
- Parser garden follow-ups (#28 done: issues/commits/file refs).
- Persist + export run state, local-only (#26).
- Scrollback pager over the retained 2000-line buffer (#25).
- Background attention signal: badge + title count + bell (#24).
- Historic attach on startup (#22) + parked-modules fate (#16).
- Per-frame terminal cache; hoist mono_metrics (#19).
- Run eviction policy: max 10 PTYs, oldest-exited reaped first (#31).
- Non-color + contrast cues (#8); in-app help (#7); vertical space (#32);
  per-agent extra CLI flags (#33); CI green (#17).

## Next

- Parser/provider rotation staffing (see CONTRIBUTING.md).
- `sccache` / CI-cache guidance if Metal-shader builds stay heavy.
- Audible bell + high-contrast toggle follow-ups behind #24/#8.

## Later (3-year VISION.md)

- Native per-OS shells over the shared Rust core C ABI (#60): macOS
  SwiftUI → Linux GTK4/VTE → WinUI/ConPTY (core API: `docs/native-core-seam.md`).
- Brand-swappable backends (muse/claude/opencode/codex) behind the
  single `Provider` seam; per-CLI adapters stay additive.
- Historical note (superseded-by-#60): the framework-volatility shield as a pinned gpui 0.2.2 line with loud pin tests is superseded-by-#60;
  the framework-free core with headless coverage continues as the cross-platform contract for the native shells.

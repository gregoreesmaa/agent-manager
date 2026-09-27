<!-- Aligned Product vision from 5 product subagents (2026-09-26). Constraint: app remains free and open-source; no paywalls or paid tiers. Generic deltas applied (approach B: vision-only; per-CLI adapters deferred, brands additive). -->
# Product Vision

agent-manager is a **free and open-source native harness** (native per-OS shells over the shared Rust core, no webview; see #60) for managing and chatting with live `muse` agent CLI sessions. Each run is a real interactive `muse` process behind an embedded PTY; the window shows the run list beside the live terminal of the selected run. Background runs keep streaming and never die on switch.

The vision: the fastest, most trustworthy way to triage and work with many concurrent `muse` runs — every run visible, every approval prompt noticed within 60 seconds, every useful link kept — with **zero account, zero paywall, zero telemetry, forever**. Comfort features (fonts, filters, export), lifecycle features (close, restart, reattach), and extension points (parsers, providers) are all free, local-only capabilities, never monetized gates.

## Principles

1. **Free and open-source first.** An explicit OSI-approved license at the repo root, `license =` in Cargo.toml, and a README "Free and open-source" statement verifiable by `ls LICENSE*` and `grep -ri paywall|billing src/` returning empty. Any PR introducing a paywall, paid tier, entitlement check, or telemetry fails review by definition.
2. **Local-only, no account.** All state (runs, prefs, links, transcripts) lives on disk as plain files. No login, no cloud sync, no tracking — a background run's approval prompt is detected by a local 60s-activity heuristic, not a service.
3. **Live PTY fidelity.** The terminal renders full vt100 state (cursor, colors, alt-screen) exactly as `muse` draws it, polling at 20Hz so background runs keep streaming.
4. **Triage by urgency.** Needs input > Idle > Active ordering with per-group counts; attention must be visible from the status bar and window title when the window is in the background.
5. **Links never lost.** Every GitHub URL seen in a run accumulates first-seen-ordered under its title and survives relaunch; scrolled-off content stays reachable via scrollback.
6. **Native shells over a shared core (#60).** Terminal/keys/pump logic stays framework-free and unit-tested in the shared Rust core; shells go native per-OS: macOS SwiftUI → Linux GTK4/VTE → WinUI/ConPTY.

> Historical note (superseded-by-#60): this principle was previously worded as framework-volatility protection with pinned gpui 0.2.2 / gpui-component 0.5.x pins failing loudly on drift (superseded-by-#60).
7. **Maintainer-leverage extensibility.** Parser (`Parser` trait + `RegistryParser::with_strategies`) and Provider (`Provider` trait) seams stay documented extension points so contributors add parsers/providers without maintainer help.

## Top 10 Improvements (ranked)

1. **Explicit OSS license + free-software statement.** Add `LICENSE` (MIT or Apache-2.0, maintainer's pick), match it with `license =` in Cargo.toml, and add a README "Free and open-source" section (free, no account/paywall/telemetry). Falsifiable: `ls LICENSE*` succeeds; paywall/billing grep over `src/` is empty.
2. **Historic attach on startup.** Add `SpawnKind::Resume(id)` (`muse --resume`) and seed `App::new` with `MuseCliProvider::discover_sessions`. Falsifiable: opening the app with an existing session dir shows that session without starting a new run.
3. **Free run lifecycle: close/kill + restart.** Key `x` removes the `App` entry and drops its `EmbeddedPty` so `Drop` reaps the child; restart spawns a fresh `muse` for the same entry reusing its title base. Falsifiable: closing reduces session count by one with no zombie `muse`; restarting an exited run makes its PTY alive again.
4. **Background attention signal.** Needs-input count in the status bar and window title plus an audible/visual bell when a non-selected run flips to Attention. Falsifiable: a background run printing an approval prompt increments the badge while the selected run is unchanged, detectable within 60s.
5. **Scrollback pager over the retained buffer.** Pager (Shift+PgUp/PgDn or wheel) into the existing 2000-line vt100 buffer with copy support. Falsifiable: after 100 lines of output the user can scroll up and copy scrolled-off text.
6. **Persist + export run state, local-only.** Persist the accumulated per-run parsed-link list (PRs, issues, commits, file refs per the active registry, plus transcript) across restarts in a plain local file; add export of the selected run's visible text plus links to markdown. Falsifiable: links survive app relaunch; export writes a file containing screen text plus `pr_links`.
7. **Keyboard-help onboarding in README.** A keyboard-help table (`j`/`k`/`n`/`Tab`/`i`/`y`/`p`/`Esc`/`q`, plus `x`, `/` as they land) so a new user completes a first spawn-switch-copy-paste run without reading source.
8. **Parser garden: issues, commits, file refs.** Extend link parsers beyond full PR URLs to `issues/123`, commit/file references with the same click-to-copy row treatment. Falsifiable: an `issues/123` URL in output appears under the run title like PR links do.
9. **Local-only comfort UX: prefs + filter.** Font-size/panel-width keys (`+`/`-`/`[`/`]`) persisted in a simple local JSON prefs file, and title-substring filter (key `/`) that filters the sessions panel without changing sort. Falsifiable: a font change survives restart; typing `fox` shows only fox-titled runs. No account, no paywall.
10. **Contributor and sustainability foundation.** `CONTRIBUTING.md` (build/test/fmt commands, PR checklist, good-first-issues in `src/parsers/` + `src/providers/`), Contributor Covenant `CODE_OF_CONDUCT.md`, issue/PR templates, `ci.yml` (`cargo test`, `cargo fmt --check`), `CHANGELOG.md`/`ROADMAP.md`, and named volunteer rotations (triage/docs/parser-gardeners/release-shepherd) with `docs/` copy-paste minimal parser/provider examples. Falsifiable: each file exists; the example compiles; no pricing/paid-tier text anywhere.

## Non-goals (paid features rejected)

- No paywalls, paid tiers, per-seat pricing, subscriptions, or feature-gated limits (run counts, parsers, export, fonts, filters, and attach/resume stay free).
- No billing/entitlement/auth/telemetry code or dependencies in the tree.
- No account system, cloud sync, or hosted service as a precondition for any feature above — persistence and prefs are plain local files.
- No monetized marketplace for parsers/providers; extension points stay documented and free.
- No Linux port as a paid deliverable — OS support is a technical/docs decision (see Open questions), never a tier.
- No per-brand or per-parser paywalls: new agent adapters (claude, opencode, codex, …) and parser strategies ship as free capabilities, never tiers.

## Community / Sustainability (non-monetized)

- Single maintainer today (`gregoreesmaa`, repo `gregoreesmaa/agent-manager`, one local commit); grow via low-friction volunteering, not revenue.
- Ladder: good-first-issues in the parser garden and provider seam → triage/docs roles → release-shepherd rotation, documented in `docs/` and `ROADMAP.md`.
- Health artifacts only: `CHANGELOG.md` (dated entries), `ROADMAP.md` (this vision's Top 10), CI green checks on every PR, CoC with a listed contact.
- Shared-core contract stays a community commitment: framework-free tested core with headless coverage as the cross-platform contract for the native shells (#60), so dependency churn never silently breaks new sessions.

## Open Questions

1. **License choice:** MIT vs Apache-2.0 — maintainer's single pick, then close.
2. **OS support:** per-OS shell order is macOS SwiftUI → Linux GTK4/VTE → WinUI/ConPTY (#60). Needs a stated supported-OS list plus a build result on each shipped OS.
3. **Persistence format:** one JSON file per run vs a single sessions file — which shape, and does transcript persist fully or tail-only?
4. **Resume UX:** does a resumed historic session replay transcript into the PTY view or start live-only with links restored?
5. **Attention UX:** status-bar count plus bell, or badge-only? What silences the bell (selecting the run)?
6. **Filter semantics:** substring only, or fuzzy/regex; does the filter also match PR-link text?
7. **Scrollback bounds:** is the 2000-line buffer the right cap once a pager exposes it, or should it grow with a documented limit?
8. **Brand surfaces:** which UX surfaces show the active brand, and does a mid-run brand switch keep transcript + links? Swappable brands are additive — the README `muse`-on-PATH default and PR-URL rows stay the default-brand behavior.

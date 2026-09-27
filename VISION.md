<!-- High-level 3-year vision synthesized from 5 personas (OSS strategist, systems architect, dev-tools UX lead, CLI-ecosystem expert, maintainer/product steward), 2026-09-26. Detail lives in VISION-UX.md, VISION-TECHNICAL.md, VISION-PRODUCT.md. Near-term posture: approach B (vision-only; per-CLI adapters deferred until a second CLI is needed). -->
# Vision

## North star (3 years)

Agent Manager is the local-first mission control for agent runs: every run is a real interactive session, any run needing input surfaces in seconds, no link or transcript is ever lost, and everything works offline, free and open-source forever.

In three years, success looks like this: a new user goes from clean checkout to a live agent prompt in under a minute with no docs; a user with dozens of runs — live and historic, from `muse` and other CLIs — knows in under five seconds which run needs them; failures stay visible with one-click recovery; all state survives quit, resize, and relaunch as plain local files; contributors add link types and agent adapters without touching core; and the project is still MIT/Apache-2.0, no accounts, no paywalls, no telemetry.

## Durable principles

1. **Live fidelity over simulation.** Every run is a real interactive child process behind a stable PTY pipeline. Fidelity regressions fail CI. Mocks and screenshots never substitute for a live prompt.
2. **Triage by urgency, glanceable in seconds.** One classifier owns run urgency. Live and historic views never disagree. Needs-input surfaces in list, status bar, and background title within 60s.
3. **Local-only trust.** Plain-file state, zero accounts, zero telemetry, zero sync. Offline build and run is the acceptance test. Tracking, billing, and entitlement code fail review.
4. **Small core, bounded growth.** Framework-free terminal, key, and run logic with headless tests; UI toolkits are swappable chrome. Every accumulate-forever store has a documented cap or eviction rule. Idle costs near zero no matter how many runs exist.
5. **Open seams, agent-neutral.** Parser (link) and provider (session source) seams stay documented with compiling minimal examples. New link types and new CLIs land without touching core or shell. Missing capabilities degrade visibly, never silently.
6. **Fail visible, recover in one click.** No silent data loss, no mouse-only flows, no color-only or wide-only state. Errors persist until dismissed or superseded; keyboard parity and non-color parity hold.

## Promises

- **Triage promise:** spawn, switch, type, copy, paste, and recover stay keyboard-operable. Any background run needing input surfaces within 60s. Runs, links, and scrollback survive switch, resize, narrow widths, and relaunch.
- **Fidelity promise:** live PTY is the moat. Shells go native per-OS over the shared Rust core (#60): framework-free tested core, green CI per commit.
- **Local-only promise:** everything works with no account and no network. Session state is plain local files. `grep paywall|billing|telemetry src/` stays empty.
- **Free-OSS promise:** every capability is free forever under an OSI license. No tiers, paywalls, or entitlements. Health artifacts (CHANGELOG, ROADMAP, templates, CoC) and CI (test/fmt/clippy) stay green.

## Scope

**In scope:** interactive PTY hosting for multiple agent CLIs via adapters; historic session attach and resume; attention/urgency classification shared by live and historic runs; agent-agnostic link extraction (PR/issue/commit/file) that stays attached, searchable, and exportable per run; keyboard-first triage UX with bounded chrome; documented parser/provider extension points.

**Non-goals:** cloud hosting, sync, or accounts; hosted orchestration, scheduling, or multi-user collaboration; paywalled tiers or usage metering; Linux/remote as a paid tier; model hosting or agent logic itself — Agent Manager hosts and triages sessions, it does not replace the CLI.

## Sustainability

Stay small enough to maintain and open enough to extend: named maintainer plus light governance (CoC with contact, issue/PR templates, reports answered promptly); contributor ladder from good-first-issues to docs/triage to release-shepherd with 2+ onboarded yearly; parser/provider seams documented with examples that build clean each year; CHANGELOG + ROADMAP updated per release; parked abstractions resolve — every compiled-but-unused module ships into startup or is deleted within one release.

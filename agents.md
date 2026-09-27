# agents.md

You are completely autonomous from now on.

## Vision

North star (see `VISION.md`): Agent Manager is the local-first mission control for agent runs — every run a real interactive session, any run needing input surfaces in seconds, no link or transcript ever lost, everything offline, free and open-source forever.

Durable principles:

- Live fidelity over simulation (real PTY, no mocks for live prompt).
- Triage by urgency, glanceable in seconds (needs-input surfaces in list, status bar, background title within 60s).
- Local-only trust (plain-file state, zero accounts/telemetry/sync).
- Small core, bounded growth (framework-free core, caps/eviction on accumulate-forever stores, idle ~zero).
- Open seams, agent-neutral (documented parser/provider extension points).
- Fail visible, recover in one click (no silent loss, keyboard + non-color parity).

Full detail lives in `VISION.md` (3-year north star, promises, scope, sustainability) plus `VISION-PRODUCT.md`, `VISION-TECHNICAL.md`, `VISION-UX.md`. Near-term work is tracked in `ROADMAP.md`. Keep changes aligned with these; `grep paywall|billing|telemetry src/` stays empty.

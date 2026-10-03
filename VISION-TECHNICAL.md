<!-- Aligned Technical vision from 5 technical subagents (2026-09-26). Conflicts resolved by synthesis vote; see Conflicts resolved section. Generic deltas applied (approach B: vision-only; per-CLI adapters deferred). -->
# Technical Vision

staap is a **live-first native terminal harness**: one real interactive `muse` PTY per run, thin native per-OS shells over a framework-free shared core (#60), pinned by headless + byte-exact regression tests. The north star: **idle costs ~zero, live stays faithful, framework volatility can't leak into the core, and every parked abstraction either ships or dies.**

The exact core API a native shell binds against is documented in `docs/native-core-seam.md`.

## Principles

1. **Shared core, framework-free** — `app` / `gui/keys` / `gui/terminal` stay framework-free and unit-tested; `gui/shell.rs` is layout + events + pump only. No UI-framework types in the core.
2. **Live-first, headless-proven** — real PTYs in tests (`echo` fakes, no window needed); pump/spawn/refresh covered without a UI-framework harness.
3. **Dirty-gated work** — no per-frame, per-tick, or per-run work unless something is fresh. `fresh_any`/selection/focus changes gate `notify()` and recompute.
4. **Bounded by construction** — every accumulate-forever structure (PTY count, pr_links, channels, scrollback) has a cap or eviction rule.
5. **One classifier, one regex, one title chain** — status/PR/title heuristics live in exactly one module; duplicates are bugs.
6. **Parked ≠ parked forever** — `parsers`/`providers`/`transcript` are either wired into startup (historic attach) or deleted; `allow(dead_code)` at module scope is not a steady state.
7. **Falsifiable gates** — CI enforces `cargo test` + `fmt --check` + `clippy -D warnings`; pin tests fail loudly on upgrade; toolchain recorded.

## Top 10 improvements (ranked)

1. **Gate 20 Hz pump repaint on dirtiness** — `tick()`/`render()` ignore `refresh()->fresh_any` today and `cx.notify()` fires unconditionally, so idle burns full refresh + repaint at 20 Hz. Gate notify + resort on fresh/selection/focus change. *Why #1: biggest idle-CPU win, enables all other perf work. Risk: low — missed-dirty bugs show as stale UI; cover with headless dirty/clean test.*
2. **Kill per-tick full-screen allocs: static PR regex + case-insensitive attention scan** — `extract_pr_links` (today PR-URL-only; scope generalizes to the active parser registry: PRs, issues, commits, file refs, errors) compiles `Regex` per run per tick; `needs_attention` does `text.to_lowercase()` per run per tick over up to 400×200 grids. Hoist to `LazyLock<Regex>`, precompiled marker set / memmem scan, skip when screen hash unchanged. *Why #2: turns every idle tick from O(screen) allocs into ~zero. Risk: low — pin with existing parser/attention tests.*
3. **Replace ShellView's parallel HashMaps with a single `Run` struct** — `ptys` + `last_output` + `pending_inputs` keyed by run id can desync. One `Run { session, pty, last_output, pending_input }` map. *Why #3: removes a whole bug class before splitting the file. Risk: medium — touches spawn/pump/key paths; headless tests cover it.*
4. **Split 1199-line `gui/shell.rs` into focused modules** — runs-panel render, terminal-pane render, pump/spawn headless logic, nav-key handling; no file >~400 lines. *Why #4: unblocks all future shell work; do after #3 so the Run type is the shared vocabulary. Risk: medium — pure move, keep headless tests green.*
5. **Unify the three status heuristics into one classifier** — `app::needs_attention` marker list vs `muse_cli::classify` substring list vs `WORKING_WINDOW` recency in shell disagree by construction. Single `classify(screen, last_output, exited)` owned by `app`. *Why #5: correctness — runs can show different status live vs historic. Risk: low — reconcile marker lists once, test both paths.*
6. **Resolve the parked-modules fate: wire historic-attach or delete** — `parsers`/`providers`/`transcript` are compiled+tested but `allow(dead_code)`-parked and the live GUI never uses transcript caps. Either wire `MuseCliProvider` discovery into startup or remove the tree. *Why #6: ends the modular-future/YAGNI stalemate; subsumes both researchers' duplicate ask. Risk: high if wired (new UX + discovery scope) / low if deleted — needs product call, see Open Q1.*
7. **Add CI: `cargo test` + `fmt --check` + `clippy -D warnings`** — README gates exist but no `.github/workflows`, so they rot. *Why #7: cheapest regression insurance; makes #1–#6 checkable per commit. Risk: near-zero; flaky live-PTY timing is the only hazard (see #10).*
8. **Bound unbounded growth: cap parsed links (`pr_links`) + cap/evict live runs** — parsed links (`pr_links`, generalized per registry output, not PR-only) accumulate forever with `Vec::contains` linear scans (plus registry merge = O(n²)); `ptys` map grows without eviction. Cap links per registry output (e.g. 50, truncation flag; cap-not-drop semantics kept) and runs (e.g. max 10 PTYs, oldest-exited reaped first). Brand copy uses a single `{agent-brand}` source (default `muse`). *Why #8: the only memory-blowup path (10k-link screen). Risk: medium — eviction policy is a product decision; assert with headless cap tests.*
9. **Cache per-frame terminal work; hoist `mono_metrics`** — `screen_rows` + `layout_text` + `to_hsla` rebuild every frame; `fit_pty` hits the font system per frame. Key row/layout cache on screen version/cursor; recompute metrics only on font/window change. *Why #9: frame-cost win after tick-cost wins (#1–#2). Risk: low — stale-cache artifacts caught by render tests.*
10. **Repro hygiene: toolchain pin + CWD-independent pin tests + ignore `.DS_Store`** — no `rust-toolchain.toml`, pin tests read `Cargo.lock` via relative path (fail outside root), `.DS_Store` committed with `/target`-only gitignore. *Why #10: fresh-checkout builds and CI stability. Risk: near-zero.*

Explicitly deferred: generalizing single-variant `SpawnKind` (collapse to a plain function until a second variant exists — YAGNI wins); PTY chunk coalescing and `cpr_replies` alloc trims (micro-opts after #1/#2 prove need); converting sleep/poll test loops to deadline helpers except where flakiness actually bites (fold into CI work, mark slowest live-PTY tests `#[ignore]` only if red).

## Conflicts resolved

- **Parked modules (researcher-5 vs researcher-6):** identical ask, one item (#6). Winner: single explicit decision — wire or delete — with falsifiable end state (zero module-scope `allow(dead_code)`). Keeping them parked+tested indefinitely loses.
- **Status logic (3 copies):** winner is one `app`-owned classifier (#5); per-module marker tweaks lose.
- **`SpawnKind` generalize vs collapse:** collapse wins; a one-variant enum with a `command()` dispatch buys nothing until `Attach { session_id }` (or similar) ships with #6.
- **Perf ordering:** dirty-gating (#1) and alloc removal (#2) beat frame caching (#9) and drain coalescing (deferred); measure idle first.
- **Test sleeps vs `#[ignore]`:** deadline-bounded helpers first; `#[ignore]`/feature-gate only the slowest live-PTY cases if CI proves flaky — blanket-ignore loses coverage.
- **Scope of parsed links:** keep the tested contract (full-URL-only + first-seen order) as the default-registry behavior; new content types land as registry strategies with the same cap-not-drop semantics (#8) rather than changing match semantics. `Parser` trait + `RegistryParser` fan-out stay the seam — no new per-type pipelines.

## Open questions

1. **Historic attach: ship or cut?** Does v1 need provider-discovered past sessions alongside live runs, or live-runs-only with the parked tree deleted?
2. **Missing researchers:** input claimed 5 outputs but carried 3 (plus an empty researcher-8 ref). Any dropped claims (notably the `dedupe-fnv64` thread) need re-supply before final roadmap lock.
3. **Run eviction UX:** refuse the 11th run, reap oldest-exited, or user-driven close-run? Needs a product pick before #8.
4. **Shell direction, decided by #60 (superseded-by-#60):** the former gpui upgrade-path question — stay pinned on gpui 0.2.2 / gpui-component 0.5.1 vs schedule the 0.6+ fork migration — is closed (superseded-by-#60); shells go native per-OS over the shared core, so no gpui upgrade will happen (superseded-by-#60).
5. **Parsed-link scope:** full `…/pull/<n>` URLs only (current default registry) vs issues/commits/file refs — confirm per-type before touching the parser; brand adapters stay additive, and the generic `Provider` trait needs no new seam for brand swap.
6. **Build weight:** `target/` at ~9 GB with Metal-shader builds — worth `sccache`/CI-cache guidance in README?
7. **Brand plumbing:** single `{agent-brand}` source for title/header/empty/error copy (default `muse`); transcript + parsed-link retention across a mid-run brand switch is undecided — no storage change proposed.

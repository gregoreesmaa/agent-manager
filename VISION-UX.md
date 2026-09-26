<!-- Aligned UX vision from 5 UX designer subagents (2026-09-26). Conflicts resolved by synthesis vote; see Conflicts resolved section. Generic deltas applied (approach B: vision-only; per-CLI adapters deferred). -->
# UX Vision

Agent Manager is a native multi-run agent console (backend brand swappable: muse/claude/opencode/codex; default `muse`) where every run is a live PTY that never dies on switch. The vision: **first run succeeds in under a minute, every failure is visible until resolved with a one-click recovery, and list/terminal/status stay usable narrow, keyboard-only, and grayscale.**

## Principles

1. **Fail visibly, recover in one click.** Errors (missing agent binary, spawn failure, PTY write failure, exited child) stay on screen until dismissed or superseded, and each carries its recovery (Retry / Restart / Rerun). Error copy names the active brand via `{agent-brand}`, never a hardcoded CLI name. No silent clears, no dead-end states.
2. **Orient before you chrome.** Empty, first-run, rename, and ended states teach the next action (clickable CTA + key hint + confirmation), instead of assuming the user read the README.
3. **Keyboard and non-color parity.** Anything clickable is keyboard-reachable; anything color-coded (focus, selection, Attention/Idle/Working) has a shape/text/contrast redundant cue.
4. **Cap chrome, preserve data.** Sidebar, parsed-link rows, and status hints are bounded displays over unbounded data — collapse/truncate the view, never drop the underlying runs or links. This holds for PRs, issues, commits, file refs, and error markers alike.

## Top 10 improvements (ranked)

1. **Actionable first-run spawn failure + Retry** — Why #1: the happy path assumes the active agent CLI (`{agent-brand}`, default `muse`) on PATH (README prereq) but in-app failure is a passive `spawn failed: {brand}: {e}` line with no next step; new users stall at zero sessions. Winner over silent placeholder. Affected area: terminal empty pane + spawn path (`shell.rs:589-603`, `spawn_queued`).
2. **Sticky errors for spawn + PTY write failures** — Why #2: 3s `STATUS_TTL` (`app.rs:157-187`) plus `spawn_error` rendered only when no active view (`shell.rs:590-603`, set at `:227`/`:356`) means write failures with a live session and any error past a repaint vanish. Split policy: info stays transient, errors stay until dismissed/next success. Affected area: status bar + terminal pane.
3. **Explicit ended-run recovery (Restart/Rerun)** — Why #3: exited runs show only `{Brand} [ended] + [process exited]` (`shell.rs:686-706`, `embedded.rs:207-215`) with cursor hidden and no action — a dead end for the most common lifecycle event. Affected area: terminal header/pane.
4. **Safe lifecycle: dirty-quit confirm + per-run close/kill** — Why #4: `q`/`Esc` quits instantly with live PTYs (`nav_action` + `cx.quit()` at `shell.rs:464-467`) and no affordance removes a hung/exited run. Highest regret risk. Compromise: confirm only when runs are Working/Attention or PTYs live; instant-quit when empty/idle; per-run close asks once. Affected area: key dispatch + sidebar row actions.
5. **Cap/collapse parsed-link children per run** — Why #5: links accumulate unbounded first-seen order (`refresh()` merge) and every link is a live row (PRs, issues, commits, file refs, errors per the active parser registry); long histories push cost into scroll/perf. Winner: show first ~20 + `N more` disclosure, keep full list copyable/searchable. Affected area: sidebar (`shell.rs:546-562`).
6. **Responsive narrow layout** — Why #6: fixed 264px sidebar + 28px bar + untruncated hints break below ~700px while `fit_pty` floors at 200x120 (`shell.rs:478-496`). Layered fix: collapsible sidebar under ~700px, min window matching PTY floors (cols>=20/rows>=10), wrap/truncate status hints. Affected area: root layout + `window_options` (no min-size today).
7. **Discoverable in-app help (`?` / Help)** — Why #7: full keymap (n/j/k/Tab/i/y/p/q/Esc, drag/Cmd+C/paste) lives only in README/crate docs; empty-state hints cover 2 keys. Cheap unlock for all other features. Affected area: key dispatch + status bar/overlay.
8. **Non-color + contrast cues for selection, focus, state** — Why #8: typing focus is bright-vs-dim only (`:710`), selected row is `active` highlight only (`:565`), Idle row marker is a blank space (`:539-543`), secondary text is dim `0x888888` on dark. Fails grayscale/low-vision. Fix: bold/label prefix on selection, underline/border or `[typing]` prefix on title, non-blank idle marker, raise secondary contrast toward 4.5:1 or toggle. Affected area: sidebar + terminal header + status/empty styles.
9. **Full keyboard operability (parsed links + scroll keys)** — Why #9: parsed-link rows are click-only, `Focus::Nav` docs mention PgUp/PgDn with no handler (`nav_action` has none). Keyboard users lose link-copy and big-scroll. Fix docs-or-bind + Enter-to-copy on focused link. Affected area: input handling.
10. **First-run orientation bundle: clickable New CTA + rename confirm + empty-vs-error distinction** — Why #10: three small confusions compound on day one — placeholder has no button (`Press n` only), silent first-prompt rename (`note_submitted_prompt`) goes unnoticed, and `No session yet` vs `spawn failed` differ by text alone. One bundle: CTA button in empty pane, `renamed to '<title>'` flash, icon/color split for empty vs failed. Affected area: terminal placeholder + status flash. Deferred as polish below this line: italic rendering (dropped in `layout_text`) and `yanked N lines` → `copied N lines` wording.

## Conflicts resolved

- **Transient vs sticky status:** both researchers right in part. Resolution: keep 3s transient for info (`copied/pasted`), sticky-until-dismissed for errors. Reason: recency for confirmations, persistence for blockers.
- **Where spawn/write errors live:** placeholder-only vs status-bar-only. Resolution: mirror in both — pane line + sticky status. Reason: pane is seen when idle, bar is seen when a live session hides the pane.
- **Sidebar collapse vs min-window vs link cap:** treated as competing; resolution: complementary layered defense (collapse + min-size + cap). Reason: each guards a different overflow axis.
- **Instant-quit speed vs confirmation:** resolution: dirty-check — confirm iff Working/Attention runs or live PTYs exist. Reason: preserves fast quit for safe states.
- **Parsed-link accumulation vs cap:** resolution: cap display, retain data. Reason: README promise (links that scrolled off stay visible) survives without unbounded rows.
- **Key-hint vs button empty state:** resolution: both — hint text plus clickable `+ New` CTA. Reason: discoverability for mouse and keyboard together.
- **Color-only vs added cues:** resolution: keep color, add redundant shape/text. Reason: no regression for current users, unlock for grayscale.
- **Blank idle marker vs visible marker:** resolution: non-blank marker (e.g. `·`). Reason: blank is indistinguishable from padding.
- **`yanked` vs `copied`:** resolution: `copied N lines`. Reason: matches `copied selection/copied PR link` vocabulary.
- **Italic distinctness:** no conflict, deferred below Top 10 as low-frequency cosmetic.
- **researcher-3 (empty):** no vote; ties broken by inspected source, not headcount.

## Open questions

- Retry/Restart semantics: reuse the same `run-N` id and transcript, or spawn a fresh id? Does Restart clear `pr_links`?
- Sticky-error dismissal: explicit ✕, any successful spawn, or timeout ceiling (e.g. 60s)? Who owns dismissal per surface?
- Dirty-quit definition: Working/Attention only, or any live PTY including Idle? Should `q` in Terminal focus also confirm?
- Per-run close: kill PTY immediately, keep transcript read-only, or archive? Confirm per run or with don't-ask-again?
- Parsed-link cap N and disclosure shape: first 20 + count, paged, or per-group collapsible? Still measured <1s at 100 links?
- Brand-swap UX: which surfaces (title, header, empty pane, errors) show the active brand, and does a mid-run brand switch keep transcript + links?
- Responsive numbers: collapse threshold (700px?), min window (720x420?), status wrap vs two-line vs marquee?
- Help scope: modal cheatsheet vs `?` toggle vs status-bar expansion, and who documents future keys?
- Contrast target: 4.5:1 everywhere or scoped to status/empty/selection with a high-contrast toggle?
- Link keyboard model: focus moves into child rows, or a `o`-to-open/cycle-links binding?
- Offline/degraded surfacing: should unreachable session store (`discover_sessions → Ok(empty)`) surface like spawn failure?

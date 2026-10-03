# staap Rename Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rename the project from agent-manager to staap everywhere, with a README narrative, all gates green.

**Architecture:** Four independent workstreams (narrative, Rust core + C header, native shells + scripts + CI, docs sweep) executed as parallel subagents on one branch, integrated via grep gates plus the full test suites. Mechanical rename: `agent-manager`->`staap`, `agent_manager`->`staap`, `AgentManager`->`Staap`, `AGENT_MANAGER`->`STAAP`, `am_`->`staap_`, `Am(Error|Rgb|Style|Core|Pty)`->`Staap$1`.

**Tech Stack:** Rust (cargo, cbindgen), Swift/SwiftPM, C (meson, MSVC), GitHub Actions, PowerShell/Bash.

**Spec:** `docs/superpowers/specs/2026-10-03-staap-rename-design.md`

## Global Constraints

- Deep rename everywhere, no compat shims for old binary/config/header/ABI names.
- FFI prefix is `staap_*`, types `Staap*`, header `include/staap.h` with guard `STAAP_H`.
- Single atomic change across all shells; never fix one shell by editing another.
- Gates: `cargo test --all-targets`, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `swift test`, Linux meson suite.
- CHANGELOG historic entries keep the old name; everything else is renamed.
- `grep paywall|billing|telemetry src/` stays empty.

## Review Focus

- A stale `am_*` symbol reference in any shell that only fails at link time, not compile time — the plan pins a link-level check (Swift `--smoke`, meson build, header syntax check) per shell task.
- A missed `AGENT_MANAGER_CONFIG` env reference causing tests to read the wrong config — each task greps its scope for the old env name.
- A missed artifact/binary name in CI or packaging that publishes under the old name — shell + CI tasks list every renamed artifact explicitly.
- The old config/state path lingering in a doc example users copy — docs task greps for `~/.config/agent-manager` and `.local/share/agent-manager`.
- cbindgen drift: regenerated `include/staap.h` differing from `src/ffi.rs` — core task regenerates and diff-checks.

---

### Task 1: Narrative + README

**Files:**
- Modify: `README.md:1-35`

**Interfaces:**
- Consumes: none.
- Produces: `Why staap?` section copy + `staap` title consumed by Task 4 (docs must match README wording).

- [ ] **Step 1: Write the `Why staap?` narrative and retitle README to `staap`.**

`Why staap?` (~150-250 words): `staap` is Estonian for (military) headquarters. Frame the app as the HQ/command post: runs are field units reporting back, needs-input is the flag on the map table, the roster is the situation board, triage-by-urgency is the duty officer's glance, local-first/offline is the HQ that works with the radios off, free/open-source is the HQ anyone can build. Tie each durable trait (real PTY fidelity, glanceable urgency, plain-file state, one-click recovery) to one HQ image. Keep it factual: no invented etymology beyond headquarters, no new features.

- [ ] **Step 2: Read back the README heading + narrative and confirm title is `# staap`.**
- [ ] **Step 3: Commit.**

```bash
git add README.md
git commit -m "docs: staap name narrative"
```

### Task 2: Rust core + C header

**Files:**
- Modify: `Cargo.toml:1-14`, `Cargo.lock`, `cbindgen.toml`, `src/lib.rs`, `src/main.rs`, `src/ffi.rs` (all `am_*` fns -> `staap_*`, `AmError/AmRgb/AmStyle/AmCore/AmPty` -> `Staap*`), `src/config.rs` (config path + `AGENT_MANAGER_CONFIG` -> `STAAP_CONFIG`), `src/persist.rs` (data dir), `src/app.rs`, `src/embedded.rs`, `src/launch.rs`, `src/providers/*.rs` + all test temp-dirs `agent-manager-*` -> `staap-*`, `.gitignore`
- Create: `include/staap.h` (via cbindgen); Delete: `include/agent_manager.h`

**Interfaces:**
- Consumes: none.
- Produces: `staap_*` C ABI + `Staap*` types + `STAAP_CONFIG` env + `libstaap.a`/`staap.lib` consumed by Task 3 (shells link these exact names).

- [ ] **Step 1: Rename crate/binary/lib in `Cargo.toml` and update all Rust identifiers, paths, env vars, and test temp-dirs per the Interfaces.**

Exact renames: package `agent-manager`->`staap`, lib `agent_manager`->`staap`, bin `agent-manager`->`staap`; `AmError`->`StaapError`, `AmRgb`->`StaapRgb`, `AmStyle`->`StaapStyle`, `AmCore`->`StaapCore`, `AmPty`->`StaapPty`; every `pub unsafe extern "C" fn am_X` -> `staap_X`; `AGENT_MANAGER_CONFIG`->`STAAP_CONFIG`; `~/.config/agent-manager`->`~/.config/staap`; persist subdir `agent-manager`->`staap`; `agent_manager::{...}` use in `src/main.rs` -> `staap::{...}`.

- [ ] **Step 2: Regenerate the header and verify it matches the staticlib.**

Run: `cbindgen --config cbindgen.toml --crate staap --output include/staap.h`
Then: `cargo build --lib` and `cc -fsyntax-only -std=c99 -Wall include/staap.h`. Expected: both succeed; header contains `staap_core_new`, guard `STAAP_H`, no `am_` symbols.

- [ ] **Step 3: Run the Rust gates.**

Run: `cargo test --all-targets`
Expected: PASS.
Run: `cargo fmt --check`
Expected: clean.
Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 4: Grep the scope for leftovers.**

Run: content search for `agent-manager|agent_manager|AgentManager|AGENT_MANAGER|\bam_|AmError|AmRgb|AmStyle|AmCore|AmPty` in `src/`, `include/`, `Cargo.toml`, `Cargo.lock`, `cbindgen.toml`, `.gitignore`.
Expected: zero matches.

- [ ] **Step 5: Commit.**

```bash
git add Cargo.toml Cargo.lock cbindgen.toml src include .gitignore
git commit -m "refactor: rename core to staap"
```

### Task 3: Native shells + scripts + CI

**Files:**
- Modify: `swift/Package.swift`, `swift/Sources/**` (dir `AgentManagerMac`->`StaapMac`, `@_silgen_name("am_*")`->`staap_*`), `swift/README.md`, `swift/Tests/**`, `native/linux/meson.build`, `native/linux/meson_options.txt`, `native/linux/src/**` (`#include "agent_manager.h"`->`"staap.h"`, `am_*` calls->`staap_*`), `native/linux/tests/smoke_live.sh` (`AGENT_MANAGER_CONFIG`->`STAAP_CONFIG`), `native/linux/README.md`, `native/windows/AgentManagerWinUI.vcxproj` (+ rename project/target to `StaapWinUI`), `native/windows/CMakeLists.txt`, `native/windows/app.manifest`, `native/windows/src/**` (incl. `src/winui/*` namespace `AgentManagerWinUI`->`StaapWinUI`), `native/windows/tests/smoke_live.ps1`, `native/windows/README.md`, `build-and-run.sh`, `build-and-run.ps1`, `.github/workflows/ci.yml`, `.github/workflows/release.yml`, `packaging/README.md`, `docs/native-core-seam.md` C-shell sections, `.github/ISSUE_TEMPLATE/bug_report.md`

**Interfaces:**
- Consumes: `staap_*` ABI + `STAAP_CONFIG` + `libstaap.a`/`staap.lib` from Task 2.
- Produces: shells building against `include/staap.h`, consumed by the verification step.

- [ ] **Step 1: Rename all three shells, their bridges, scripts, CI jobs, and packaging artifacts.**

Exact renames: `AgentManagerMac`->`StaapMac`, `agent-manager-gtk`->`staap-gtk`, `AgentManagerWinUI`->`StaapWinUI`; `agent_manager` lib link->`staap`; `AgentManagerMac-macos`/`AgentManagerWindows`/`agent-manager-linux` artifacts->`StaapMac-macos`/`StaapWindows`/`staap-linux`; `com.example.agent-manager` app id->`com.example.staap`; `AGENT_MANAGER_CONFIG`->`STAAP_CONFIG` in CI + smoke scripts. Depends on Task 2's ABI names; coordinate on branch (pull before starting).

- [ ] **Step 2: Verify the header syntax check passes.**

Run: `cc -fsyntax-only -std=c99 -Wall include/staap.h`
Expected: clean.

- [ ] **Step 3: Verify the Swift shell links and its smoke test passes.**

Run: `cargo build --lib` then `swift build` (workdir `swift`), then `swift/.build/debug/StaapMac --smoke`.
Expected: build succeeds, smoke exits 0.

- [ ] **Step 4: Verify the Linux shell builds (meson suite owns further checks).**

Run: Linux meson build per `native/linux/README.md`.
Expected: `staap-gtk` binary builds.

- [ ] **Step 5: Grep the scope for leftovers.**

Run: content search for `agent-manager|agent_manager|AgentManager|AGENT_MANAGER|\bam_` in `swift/`, `native/`, `build-and-run.sh`, `build-and-run.ps1`, `.github/`, `packaging/`.
Expected: zero matches.

- [ ] **Step 6: Commit.**

```bash
git add swift native build-and-run.sh build-and-run.ps1 .github packaging
git commit -m "refactor: rename shells and CI to staap"
```

### Task 4: Docs sweep + release notes

**Files:**
- Modify: `VISION.md`, `VISION-PRODUCT.md`, `VISION-TECHNICAL.md`, `VISION-UX.md`, `ROADMAP.md`, `AGENTS.md`, `agents.md`, `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, `LICENSE-MIT` (copyright line), `docs/new-session-picker.md`, `docs/57-sidebar-native-eval.md`, `CHANGELOG.md` (new entry), per-shell READMEs missed by Task 3

**Interfaces:**
- Consumes: `Why staap?` copy from Task 1 (match its wording for the name explanation); final binary/lib/header names from Tasks 2-3.
- Produces: consistent docs; nothing downstream.

- [ ] **Step 1: Rename the project in all vision/product/docs files and add a CHANGELOG entry noting the breaking rename (binary, config path, no shims).**
- [ ] **Step 2: Grep the whole repo for leftovers.**

Run: content search for `agent-manager|agent_manager|AgentManager|AGENT_MANAGER|\bam_|AmError|AmRgb|AmStyle|AmCore|AmPty` across the repo.
Expected: zero matches outside `CHANGELOG.md` historic entries and `docs/superpowers/specs/2026-10-03-staap-rename-design.md`.

- [ ] **Step 3: Run the Rust gates as a final check.**

Run: `cargo test --all-targets`
Expected: PASS.
Run: `cargo fmt --check; cargo clippy --all-targets -- -D warnings`
Expected: both clean.

- [ ] **Step 4: Commit.**

```bash
git add VISION.md VISION-PRODUCT.md VISION-TECHNICAL.md VISION-UX.md ROADMAP.md AGENTS.md agents.md CONTRIBUTING.md CODE_OF_CONDUCT.md LICENSE-MIT docs CHANGELOG.md
git commit -m "docs: rename project to staap"
```

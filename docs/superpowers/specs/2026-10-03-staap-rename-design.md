# staap Rename Design

## Context

Rename the project from `agent-manager` to `staap` (Estonian for
military headquarters). Depth: deep everywhere, no compat shims.
FFI prefix: `staap_*` (option A). Atomic big-bang across all shells.

## Goals

- Zero `agent-manager|agent_manager|AgentManager|AGENT_MANAGER` strings
  left in code, headers, build files, scripts, CI, docs (historic
  CHANGELOG entries exempt).
- Zero `am_*` / `Am*` identifiers left in the C ABI and Rust core
  (replaced by `staap_*` / `Staap*`).
- README explains the name with a creative narrative (subagent-written).
- All gates green: `cargo test --all-targets`, `cargo fmt --check`,
  `cargo clippy --all-targets -- -D warnings`, `swift test`, Linux
  meson suite.

## Non-goals

- No migration shims for old config/state paths, old binary names, or
  the old `am_*` ABI.
- No behavior changes; rename only (plus the README narrative section).
- No phased rollout; single atomic change across all shells.

## Design

### 1. Rust core

- `Cargo.toml`: package `staap`, lib `staap`, bin `staap`.
- `src/lib.rs`, `src/main.rs`: crate docs + `pub use staap::{...}`.
- `src/ffi.rs`: `AmError/AmRgb/AmStyle/AmCore/AmPty` -> `Staap*`;
  every `am_*` fn -> `staap_*`; doc comments updated.
- Test temp-dirs `agent-manager-*` -> `staap-*`.
- `src/config.rs`: `~/.config/staap/config.json`.
- `src/persist.rs`: data dir `staap` subdir.
- `.gitignore`: `staap-state.json`, `staap-prefs.json`.
- `cbindgen.toml`: crate `staap`.

### 2. C seam

- `include/agent_manager.h` -> `include/staap.h` (delete old, add new).
- `cbindgen.toml`: `include_guard = "agent_manager"` -> `"staap"`
  (generates `#ifndef STAAP_H`).
- Regenerate with cbindgen; update the `nm` verification line to
  `libstaap.a | grep -c 'staap_spawn\|staap_pump\|staap_write'`.

### 3. Shells + scripts + CI

- Swift: `AgentManagerMac` -> `StaapMac` (`swift/Package.swift`,
  sources, `swift/README.md`, `build-and-run.sh` launch path).
- Linux: `agent-manager-gtk` -> `staap-gtk` (`native/linux/meson.build`,
  `meson_options.txt`, `native/linux/README.md`, CI screenshot/release
  jobs, packaging tarball names).
- Windows: `AgentManagerWinUI` -> `StaapWinUI`
  (`native/windows/*.vcxproj*`, `build-and-run.ps1`, `native/windows/README.md`).
- CI (`.github/workflows/ci.yml`, `release.yml`): binary/artifact names.
- `packaging/README.md`: tarball names.

### 4. State paths (breaking, no shim)

- Config: `~/.config/staap/config.json`.
- Data: platform data dir `staap` subdir.
- Note the break in CHANGELOG.md.

### 5. Docs + narrative

- README: title `staap` + new `Why staap?` section (creative subagent:
  staap = headquarters — mission control, triage by urgency, the
  command post where runs report and attention surfaces; local-first,
  offline, free).
- `VISION.md`, `VISION-PRODUCT.md`, `VISION-TECHNICAL.md`,
  `VISION-UX.md`, `ROADMAP.md`, `AGENTS.md`, `agents.md`,
  `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, `docs/native-core-seam.md`,
  per-shell READMEs: project name + binary/lib/header/FFI references.
- `LICENSE-MIT` copyright line: `staap contributors`.

### 6. Verification

- `cargo test --all-targets`, `cargo fmt --check`,
  `cargo clippy --all-targets -- -D warnings`.
- `swift test` (after `cargo build --lib`), Linux meson suite.
- Grep gates: no `agent-manager|agent_manager|AgentManager` and no
  `\bam_` / `Am(Error|Rgb|Style|Core|Pty)` outside CHANGELOG history.

## Execution

Four parallel workstreams (see implementation plan): narrative, Rust
core + C header, native shells + scripts + CI, docs sweep. Integrate
with the grep gates + full test suites before done.

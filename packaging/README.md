# Packaging + releases

Each per-OS CI job ships a runnable artifact; `v*` tags publish the
same per-shell build paths as signed-off GitHub release assets (see
`.github/workflows/release.yml`). The store-grade formats below are
explicitly stubbed, not started. Non-goals (unchanged):
no notarization/signing setup, no store submission, no shell features.

| OS | CI job | Ships now | Stubbed (tracking note = this file) |
|---|---|---|---|
| macOS | `macos` | `StaapMac-macos` artifact (`StaapMac-macos.zip`, ditto'd Swift binary) + `v*` release asset per arch | `.dmg`: no installer layout yet; the zip is the distribution format until a `dmg` step is proposed |
| Linux | `linux` | `staap-linux` artifact (`staap-linux.tar.gz` with `staap-gtk`) + `v*` release asset per arch | `.deb` / Flatpak: no manifest yet; the tarball is the distribution format until a maintainer proposes one |
| Windows | `windows` | `StaapWindows` artifact (`StaapWindows.zip`: portable C suite exes from `native/windows/build/Release/`) + `v*` release asset per arch (unpackaged WinUI app folder) | MSIX: the WinUI app ships unpackaged on purpose; no manifest/identity until a maintainer proposes one |

Proposing a stubbed format means adding its manifest + a CI step that
builds it on its own runner only (per-shell gating, issue #65), and
updating this table.

## Releases (`v*` tags -> GitHub release)

Pushing a `v*` tag runs `.github/workflows/release.yml`: the three
per-shell build paths CI proves (Swift macOS app, GTK tarball, WinUI
app folder) re-link against the `--release` core staticlib on both
CPU arches per OS and publish to the tag's release. Assets:

| OS | Asset (per `<tag>`, `<arch>`) | Runners |
|---|---|---|
| macOS | `StaapMac-<tag>-macos-<arch>.zip` (ditto'd Swift binary, `<arch>` = `arm64` / `x64`) | `macos-latest`, `macos-26-intel` |
| Linux | `staap-<tag>-linux-<arch>.tar.gz` (`staap-gtk`, `<arch>` = `x64` / `arm64`) | `ubuntu-24.04`, `ubuntu-24.04-arm` |
| Windows | `StaapWinUI-<tag>-windows-<arch>.zip` (unpackaged app folder, `<arch>` = `x64` / `arm64`) | `windows-latest`, `windows-11-vs2026-arm` (`-vs2026-` carries the VS 2026 v145 toolset the vcxproj tracks) |

All six ships ride the tag's release - a red arch fails the publish,
never a partial upload. `workflow_dispatch` dry-runs the matrix
without publishing (tag gate in the `publish` job), leaving the
archives as run artifacts. Release notes are generated from the tag
with an unsigned-build disclaimer prepended. Local-only trust holds:
no accounts/telemetry/sync in the pipeline, plain-file state only.

## WinUI MSBuild packaging (hosted runners + dev machine)

The WinUI 3 shell (`native/windows/StaapWinUI.vcxproj`) builds
on hosted runners - CI's `windows` job and the release workflow both
build the unpackaged exe there (restore-then-build msbuild with the
pinned `Microsoft.WindowsAppSDK` 2.5.1 + `Microsoft.Windows.CppWinRT`
3.0.260818.1). x64 builds on `windows-latest` (VS 2026, v145
toolset); arm64 builds on `windows-11-vs2026-arm` (the VS 2026 arm64
image).

Historical note: this section once claimed hosted runners could not
build the shell (a NuGet UAP-handshake failure with the same symptom
with and without the UWP workload, 13 attempts under #64). A later
run of the same restore-then-build shape went green on
`windows-latest`, so the claim was stale - kept here so nobody
re-litigates it from memory.

Prerequisites:

- Visual Studio 2022 17.x+ **with** the Universal Windows Platform
  build-tools workload (`Microsoft.VisualStudio.Workload.UniversalBuildTools`)
- WindowsAppSDK 2.5.1 (`Microsoft.WindowsAppSDK`, pinned in
  the vcxproj)
- CppWinRT 3.0.260818.1 (`Microsoft.Windows.CppWinRT`, pinned in the vcxproj)

Build (restore and build in a single evaluation, from the repo root):

```powershell
msbuild native/windows/StaapWinUI.vcxproj /restore `
  /p:Configuration=Release /p:Platform=x64 `
  "/p:CoreLibDir=$env:GITHUB_WORKSPACE/target/debug"
```

(This is the exact invocation the removed CI step used, copied from the
`ci.yml` history — on a dev machine, replace `$env:GITHUB_WORKSPACE`
with the repo root.)

To re-enable the CI step, restore the removed `WinUI build (unpackaged
exe, no MSIX)` step (plus its `install UWP build-tools workload`
prerequisite) in `.github/workflows/ci.yml`. (Resolved instead by the
later green restore-then-build runs; kept for archaeology.)

See #64 for the 13-attempt hosted-runner history.

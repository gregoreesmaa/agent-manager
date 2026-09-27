# Packaging stubs (issue #65)

Each per-OS CI job ships a runnable artifact; the store-grade formats
below are explicitly stubbed, not started. Non-goals (unchanged):
no notarization/signing setup, no store submission, no shell features.

| OS | CI job | Ships now | Stubbed (tracking note = this file) |
|---|---|---|---|
| macOS | `macos` | `AgentManagerMac-macos` artifact (`AgentManagerMac-macos.zip`, ditto'd Swift binary) | `.dmg`: no installer layout yet; the zip is the distribution format until a `dmg` step is proposed |
| Linux | `linux` | `agent-manager-linux` artifact (`agent-manager-linux.tar.gz` with `agent-manager-gtk`) | `.deb` / Flatpak: no manifest yet; the tarball is the distribution format until a maintainer proposes one |
| Windows | `windows` | `AgentManagerWindows` artifact (`AgentManagerWindows.zip`: C suite + unpackaged WinUI exe) | MSIX: the WinUI app builds unpackaged on purpose; no manifest/identity until a maintainer proposes one |

Proposing a stubbed format means adding its manifest + a CI step that
builds it on its own runner only (per-shell gating, issue #65), and
updating this table.

## WinUI MSBuild packaging (dev machine only)

The WinUI 3 shell (`native/windows/AgentManagerWinUI.vcxproj`) does not
build on hosted `windows-latest` runners: the NuGet UAP handshake
(`ResolveNuGetPackageAssets` / `does not reference "UAP,Version=v10.0"`)
fails identically with and without the UWP workload installed, so the
CI `windows` job no longer attempts it. Build it on a dev machine instead.

Prerequisites:

- Visual Studio 2022 17.x+ **with** the Universal Windows Platform
  build-tools workload (`Microsoft.VisualStudio.Workload.UniversalBuildTools`)
- WindowsAppSDK 1.6 (`Microsoft.WindowsAppSDK 1.6.250602001`, pinned in
  the vcxproj)
- CppWinRT 2.0.250303.1 (`Microsoft.Windows.CppWinRT`, pinned in the vcxproj)

Build (restore and build in a single evaluation, from the repo root):

```powershell
msbuild native/windows/AgentManagerWinUI.vcxproj /restore `
  /p:Configuration=Release /p:Platform=x64 `
  "/p:CoreLibDir=$env:GITHUB_WORKSPACE/target/debug"
```

(This is the exact invocation the removed CI step used, copied from the
`ci.yml` history — on a dev machine, replace `$env:GITHUB_WORKSPACE`
with the repo root.)

To re-enable the CI step, restore the removed `WinUI build (unpackaged
exe, no MSIX)` step (plus its `install UWP build-tools workload`
prerequisite) in `.github/workflows/ci.yml`.

See #64 for the 13-attempt hosted-runner history.

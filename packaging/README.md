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

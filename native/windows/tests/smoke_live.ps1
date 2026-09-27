#Requires -Version 5.1
<# Live converse proof for the Windows shell (issue #64): spawn, pump for
output, write, resize against a fake `muse.exe` on PATH — the same
hermetic trick as the core's `public_spawn_success_path` test and the
Linux `smoke_live.sh`, but with a real PE: CreateProcess cannot execute
the shell-script fake the unix harnesses use, so this script compiles
tests/fake_muse.c with the runner's MSVC first (msvc-dev-cmd on CI).

Usage (from the repo root, inside an MSVC environment):
  powershell -ExecutionPolicy Bypass -File native/windows/tests/smoke_live.ps1 `
    -Smoke ./native/windows/build/Release/am-win-smoke.exe
#>
param(
  [Parameter(Mandatory = $true)][string]$Smoke
)

$ErrorActionPreference = 'Stop'

$fakeBin = Join-Path ([IO.Path]::GetTempPath()) (
  'fake-muse-64-' + [IO.Path]::GetRandomFileName())
$scratch = Join-Path ([IO.Path]::GetTempPath()) (
  'am-win-64-' + [IO.Path]::GetRandomFileName())
New-Item -ItemType Directory -Path $fakeBin | Out-Null
New-Item -ItemType Directory -Path $scratch | Out-Null
try {
  $here = Split-Path -Parent $MyInvocation.MyCommand.Path
  $repoRoot = Resolve-Path (Join-Path $here '../../..')
  cl /nologo /O2 /Fe:"$fakeBin\muse.exe" (
    Join-Path $repoRoot 'native/windows/tests/fake_muse.c') | Out-Null
  if ($LASTEXITCODE -ne 0) { throw "cl failed to build fake muse.exe" }

  # Hermetic config + profile: discovery/persistence must degrade to
  # empty and never touch the developer's live files.
  $saved = @{}
  foreach ($name in @('PATH', 'AGENT_MANAGER_CONFIG',
      'USERPROFILE', 'APPDATA', 'LOCALAPPDATA')) {
    $saved[$name] = [Environment]::GetEnvironmentVariable($name)
  }
  try {
    [Environment]::SetEnvironmentVariable(
      'PATH', "$fakeBin;$($saved['PATH'])")
    [Environment]::SetEnvironmentVariable(
      'AGENT_MANAGER_CONFIG', (Join-Path $scratch 'config.json'))
    [Environment]::SetEnvironmentVariable('USERPROFILE', $scratch)
    [Environment]::SetEnvironmentVariable(
      'APPDATA', (Join-Path $scratch 'AppData/Roaming'))
    [Environment]::SetEnvironmentVariable(
      'LOCALAPPDATA', (Join-Path $scratch 'AppData/Local'))
    New-Item -ItemType Directory -Force `
      (Join-Path $scratch 'AppData/Roaming') | Out-Null
    New-Item -ItemType Directory -Force `
      (Join-Path $scratch 'AppData/Local') | Out-Null
    & $Smoke --smoke-live
    if ($LASTEXITCODE -ne 0) { throw "am-win-smoke --smoke-live failed" }
  } finally {
    foreach ($name in $saved.Keys) {
      [Environment]::SetEnvironmentVariable($name, $saved[$name])
    }
  }
} finally {
  Remove-Item -Recurse -Force $fakeBin, $scratch -ErrorAction SilentlyContinue
}

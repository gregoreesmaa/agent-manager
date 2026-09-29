#Requires -Version 5.1
<#
.SYNOPSIS
  Build the Rust core staticlib + AgentManagerWinUI shell, then launch the app.

.DESCRIPTION
  Windows counterpart of build-and-run.sh (see native/windows/README.md):
    cargo build --lib [--release]          # target/{debug,release}/agent_manager.lib
    msbuild -t:restore AgentManagerWinUI.vcxproj
    msbuild agentManagerWinUI.vcxproj /p:Configuration=Release /p:Platform=x64

  The WinUI app needs the VS2026 (v145) toolchain; the script locates it via
  vswhere and imports its environment, so it works from a plain prompt.
  One build pass is enough, including on a clean checkout: the page .g.hpp
  sources compile in the stock post-Pass2 batch (CompilerIteration=
  XamlGenerated in the vcxproj). A failed build is retried once in case
  codegen outputs landed late.

.PARAMETER Release
  Release core staticlib instead of debug (the app itself always builds
  Release|x64).

.PARAMETER BuildOnly
  Build without launching; prints the exe path.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File build-and-run.ps1
#>
[CmdletBinding()]
param(
    [switch]$Release,
    [switch]$BuildOnly,
    [switch]$Help
)

$ErrorActionPreference = 'Stop'

if ($Help) {
    Get-Help $PSCommandPath -Detailed
    exit 0
}

$root = Split-Path -Parent $PSCommandPath

function Find-CommandOrThrow($name, $hint, $extraDir) {
    $cmd = Get-Command $name -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    if ($extraDir) {
        $candidate = Join-Path $extraDir "$name.exe"
        if (Test-Path $candidate) { return $candidate }
    }
    throw "$name not found on PATH. $hint"
}

# cargo (rustup shims live outside PATH in some shells)
$cargoBin = Join-Path $HOME '.cargo\bin'
$cargo = Find-CommandOrThrow 'cargo' 'Install Rust 1.90.0 via rustup.' $cargoBin
if ($env:PATH -notlike "*$([IO.Path]::GetDirectoryName($cargo))*") {
    $env:PATH = "$([IO.Path]::GetDirectoryName($cargo));$env:PATH"
}

# VS2026 via vswhere (v145 toolset is required; VS2022 cannot build this)
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
if (-not (Test-Path $vswhere)) {
    throw "vswhere not found. Install Visual Studio 2026 18.x with the Desktop development with C++ workload."
}
$vsInstall = & $vswhere -products * -version '[18,19)' -property installationPath |
    Select-Object -First 1
if (-not $vsInstall) {
    throw "No Visual Studio 2026 instance found. Install VS2026 18.x (v145 toolset) + UWP C++ build tools + Windows 11 SDK."
}
$vcvars = Join-Path $vsInstall 'VC\Auxiliary\Build\vcvars64.bat'
$msbuild = Join-Path $vsInstall 'MSBuild\Current\Bin\msbuild.exe'
foreach ($p in @($vcvars, $msbuild)) {
    if (-not (Test-Path $p)) { throw "Missing expected VS2026 file: $p" }
}

# Import the vcvars64 environment (INCLUDE/LIB/PATH) into this session
Write-Host "==> importing VS2026 x64 environment"
cmd /c "`"$vcvars`" >NUL && set" | ForEach-Object {
    $i = $_.IndexOf('=')
    if ($i -gt 0) {
        [Environment]::SetEnvironmentVariable($_.Substring(0, $i), $_.Substring($i + 1))
    }
}

$vcxproj = Join-Path $root 'native\windows\AgentManagerWinUI.vcxproj'

Write-Host "==> cargo build $(if ($Release) { '--release ' })--lib"
$cargoArgs = @('build')
if ($Release) { $cargoArgs += '--release' }
$cargoArgs += '--lib'
& $cargo $cargoArgs
if ($LASTEXITCODE -ne 0) { throw "cargo build failed with exit $LASTEXITCODE" }

Write-Host '==> msbuild -t:restore'
& $msbuild -t:restore $vcxproj
if ($LASTEXITCODE -ne 0) { throw "msbuild restore failed with exit $LASTEXITCODE" }

$buildArgs = @($vcxproj, '/p:Configuration=Release', '/p:Platform=x64', '/nologo', '/v:m')
if ($Release) {
    $buildArgs += "/p:CoreLibDir=$root\target\release"
}
Write-Host '==> msbuild Release|x64'
& $msbuild @buildArgs
if ($LASTEXITCODE -ne 0) {
    # Retry once in case XAML codegen outputs landed after the compile.
    Write-Host '==> retrying once for Pass2 codegen outputs'
    & $msbuild @buildArgs
    if ($LASTEXITCODE -ne 0) { throw "msbuild failed with exit $LASTEXITCODE" }
}

$exe = Get-ChildItem (Join-Path $root 'native\windows') -Recurse -Filter 'AgentManagerWinUI.exe' |
    Where-Object { $_.FullName -match 'Release' } |
    Select-Object -First 1 -ExpandProperty FullName
if (-not $exe) { throw 'Build reported success but AgentManagerWinUI.exe was not found.' }

if ($BuildOnly) {
    Write-Host "built: $exe"
    exit 0
}

Write-Host "==> launching $exe"
Start-Process $exe

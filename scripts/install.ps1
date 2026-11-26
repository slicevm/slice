# Slice (Nexus-JS) Windows Installer Script
# Builds or copies the release binary into ~/.slice/bin and adds it to user PATH.

param(
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
$RootDir = Split-Path -Parent $PSScriptRoot
Set-Location $RootDir

$InstallDir = Join-Path $HOME ".slice\bin"
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null

$ReleaseExe = Join-Path $RootDir "target\release\slice.exe"
if (-not $SkipBuild) {
    Write-Host "Compiling Slice v0.14.5 release binary..."
    cargo build --release --locked
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
} elseif (-not (Test-Path $ReleaseExe)) {
    throw "-SkipBuild specified, but $ReleaseExe does not exist"
}

$DestExe = Join-Path $InstallDir "slice.exe"
Copy-Item -Force $ReleaseExe $DestExe
Write-Host "Installed Slice v0.14.5 to $DestExe"
& $DestExe info


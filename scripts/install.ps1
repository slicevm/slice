# Slice (Nexus-JS) Windows Installer Script
# Builds or copies the release binary into ~/.slice/bin and adds it to user PATH.

$ErrorActionPreference = "Stop"
$RootDir = Split-Path -Parent $PSScriptRoot
Set-Location $RootDir

$InstallDir = Join-Path $HOME ".slice\bin"
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null

$ReleaseExe = Join-Path $RootDir "target\release\slice.exe"
if (-not (Test-Path $ReleaseExe)) {
    Write-Host "Compiling Slice release binary..."
    cargo build --release
}

$DestExe = Join-Path $InstallDir "slice.exe"
Copy-Item -Force $ReleaseExe $DestExe
Write-Host "Installed Slice v0.10.7 to $DestExe"
& $DestExe info


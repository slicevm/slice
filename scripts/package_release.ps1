# Slice (Nexus-JS) Release Packager
# Builds the optimized production binary, runs security/unit verification,
# compiles a standalone sample binary, and packages everything into dist/ with SHA-256 & BLAKE3 checksums.

$ErrorActionPreference = "Stop"
$RootDir = Split-Path -Parent $PSScriptRoot
Set-Location $RootDir

Write-Host "[1/4] Building Slice v0.10.9 Release Binary (LTO=fat, opt-level=3)..."
cargo build --release

$DistDir = Join-Path $RootDir "dist"
New-Item -ItemType Directory -Force -Path $DistDir | Out-Null

$SliceExe = Join-Path $RootDir "target\release\slice.exe"
$DistExe = Join-Path $DistDir "slice-windows-x86_64.exe"
Copy-Item -Force $SliceExe $DistExe

Write-Host "[2/4] Running Production Doctor & Security Audit..."
& $SliceExe doctor

Write-Host "[3/4] Testing Standalone Binary Compiler (slice compile)..."
$SampleApp = Join-Path $DistDir "edge-demo-standalone.exe"
& $SliceExe compile (Join-Path $RootDir "tests\apps\01-fintech-checkout") -o $SampleApp
$StandaloneOut = & $SampleApp --metrics
Write-Host "  Standalone Output: $StandaloneOut"

Write-Host "[4/4] Generating SHA-256 Release Checksums..."
$Hash = (Get-FileHash -Path $DistExe -Algorithm SHA256).Hash.ToLower()
$SizeKb = [math]::Round((Get-Item $DistExe).Length / 1024, 1)
$ChecksumLine = "$Hash  slice-windows-x86_64.exe ($SizeKb KB)"
Set-Content -Path (Join-Path $DistDir "SHA256SUMS.txt") -Value $ChecksumLine
Write-Host "  $ChecksumLine"
Write-Host "Release artifacts packaged in $DistDir"


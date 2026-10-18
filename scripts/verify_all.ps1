# Master Verification Script for Slice (Nexus-JS)
# 1. Runs all Rust unit tests across the workspace
# 2. Builds optimized release binary
# 3. Verifies security & OCap containment
# 4. Runs Windows local benchmarks against Node.js, Deno, and Bun
# 5. Builds and runs Linux Docker benchmarks against Node.js, Deno, and Bun

$ErrorActionPreference = "Stop"
$RootDir = Split-Path -Parent $PSScriptRoot
Set-Location $RootDir

Write-Host "[1/4] Running Rust workspace unit tests..."
cargo test --workspace

Write-Host "[2/4] Building optimized release binary..."
cargo build --release

Write-Host "[3/4] Running Windows Local Benchmark Suite..."
powershell -ExecutionPolicy Bypass -File .\scripts\benchmark_windows.ps1

Write-Host "[4/4] Building and Running Linux Docker Benchmark Suite..."
docker build -t slice-linux-bench .
docker run --rm --privileged slice-linux-bench


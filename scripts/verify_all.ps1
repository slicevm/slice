# Master Verification Script for Slice (Nexus-JS)
# 1. Runs all Rust unit tests across the workspace
# 2. Builds optimized release binary
# 3. Verifies security & OCap containment
# 4. Runs Windows local benchmarks against Node.js, Deno, and Bun
# 5. Builds and runs Linux Docker benchmarks against Node.js, Deno, and Bun

$ErrorActionPreference = "Stop"
$RootDir = Split-Path -Parent $PSScriptRoot
Set-Location $RootDir

Write-Host "[1/5] Running Rust workspace clippy & unit tests..."
cargo clippy --all-targets -- -D warnings
cargo test --workspace

Write-Host "[2/5] Building optimized release binary..."
cargo build --release

Write-Host "[3/5] Running Windows Local Benchmark Suite..."
powershell -ExecutionPolicy Bypass -File .\scripts\benchmark_windows.ps1

Write-Host "[4/5] Building and Running Linux Docker Synthetic & MicroVM Benchmark Suite..."
docker build -t slice-linux-bench .
docker run --rm --privileged slice-linux-bench

Write-Host "[5/5] Building and Running Linux Docker Folder-Apps, Frameworks, NPM & Disk I/O Suite..."
docker build -f tests/Dockerfile -t slice-real-stack-bench .
docker run --rm slice-real-stack-bench



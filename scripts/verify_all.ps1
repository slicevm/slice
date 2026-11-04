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

Write-Host "[3/4] Running Local MicroVM Swarm & Security Containment Verification..."
& (Join-Path $RootDir "target\release\slice.exe") microvm-bench --tenants=1000
& (Join-Path $RootDir "target\release\slice.exe") verify-security

Write-Host "[4/4] Building and Running Linux Docker Applications & Frameworks Suite..."
docker build -f tests/Dockerfile -t slice-real-stack-bench .
docker run --rm slice-real-stack-bench



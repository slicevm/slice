param(
    [Parameter(Mandatory = $false)]
    [string]$Tags = "all",
    [Parameter(Mandatory = $false)]
    [string]$OnlyApps = "",
    [Parameter(Mandatory = $false)]
    [switch]$SkipDockerBuild
)

$ErrorActionPreference = "Stop"
$RootDir = Split-Path -Parent $PSScriptRoot
Set-Location $RootDir

$CargoToml = Get-Content (Join-Path $RootDir "Cargo.toml") -Raw
if ($CargoToml -match '(?m)^\[workspace\.package\]\s*\r?\nversion\s*=\s*"([^"]+)"') {
    $Version = $Matches[1]
} else {
    $Version = "0.10.5"
}

try {
    $Commit = (git rev-parse --short HEAD 2>$null).Trim()
} catch {
    $Commit = "unknown"
}

$TimestampUtc = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
$ReportsDir = Join-Path $RootDir "tests\reports"
if (-not (Test-Path $ReportsDir)) {
    New-Item -ItemType Directory -Path $ReportsDir | Out-Null
}

Write-Host "========================================================================="
Write-Host "  SLICE AUTOMATED DOCKER BENCHMARK & REPORT RUNNER"
Write-Host "  Version : v$Version | Commit : $Commit | Tags : $Tags"
Write-Host "========================================================================="

if (-not $SkipDockerBuild) {
    Write-Host "[1/3] Building Linux x86_64 Docker benchmark image..."
    docker build -f tests/Dockerfile -t slice-real-stack-bench .
    if ($LASTEXITCODE -ne 0) {
        throw "Docker build failed with exit code $LASTEXITCODE"
    }
}

Write-Host "[2/3] Running benchmarks and generating timestamped report in tests/reports/..."
docker run --rm `
    -e "SLICE_BENCH_TAGS=$Tags" `
    -e "SLICE_ONLY_APPS=$OnlyApps" `
    -e "SLICE_VERSION=$Version" `
    -e "SLICE_GIT_COMMIT=$Commit" `
    -e "SLICE_BENCH_TS_UTC=$TimestampUtc" `
    -e "SLICE_SYNC_DOCS=0" `
    -v "${ReportsDir}:/app/tests/reports" `
    slice-real-stack-bench

if ($LASTEXITCODE -ne 0) {
    throw "Docker benchmark run failed with exit code $LASTEXITCODE"
}

Write-Host "[3/3] Synchronizing summarized benchmark tables across README.md and docs/..."
node tests/scripts/sync_doc_benchmarks.mjs
if ($LASTEXITCODE -ne 0) {
    throw "Doc benchmark sync failed with exit code $LASTEXITCODE"
}

Write-Host "Done! Latest report: tests/reports/latest.md"

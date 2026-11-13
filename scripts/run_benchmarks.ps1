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
    $Version = "0.11.0"
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
    Write-Host "[1/4] Building Windows release binary + Linux x86_64 Docker benchmark image..."
    cargo build --release --bin slice
    if ($LASTEXITCODE -ne 0) {
        throw "Windows cargo build --release failed with exit code $LASTEXITCODE"
    }
    docker build -f tests/Dockerfile -t slice-real-stack-bench .
    if ($LASTEXITCODE -ne 0) {
        throw "Docker build failed with exit code $LASTEXITCODE"
    }
} elseif (-not (Test-Path (Join-Path $RootDir "target\release\slice.exe"))) {
    Write-Host "[1/4] Building Windows release binary..."
    cargo build --release --bin slice
    if ($LASTEXITCODE -ne 0) {
        throw "Windows cargo build --release failed with exit code $LASTEXITCODE"
    }
}

Write-Host "[2/4] Running Linux x86_64 Docker benchmarks and generating timestamped report..."
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

if ($Tags -eq "all" -or $Tags -match "(^|,)swarm(,|$)") {
    Write-Host "[3/4] Measuring live Windows host MicroVM swarm + IOCP telemetry..."
    $WinSliceExe = Join-Path $RootDir "target\release\slice.exe"
    $WinMicrovmJsonPath = Join-Path $ReportsDir "_tmp_windows_microvm.json"
    $WinOut = & $WinSliceExe microvm-bench --tenants=1000 --json
    if ($LASTEXITCODE -ne 0) {
        throw "Windows microvm-bench failed with exit code $LASTEXITCODE"
    }
    [System.IO.File]::WriteAllText($WinMicrovmJsonPath, ($WinOut -join "`n"), [System.Text.UTF8Encoding]::new($false))
    node tests/scripts/generate_bench_report.mjs --from-json tests/reports/latest.json --windows-microvm $WinMicrovmJsonPath
    Remove-Item -Force $WinMicrovmJsonPath -ErrorAction SilentlyContinue
    if ($LASTEXITCODE -ne 0) {
        throw "Merging Windows microvm telemetry failed with exit code $LASTEXITCODE"
    }
}

Write-Host "[4/4] Synchronizing summarized benchmark tables across README.md and docs/..."
node tests/scripts/sync_doc_benchmarks.mjs
if ($LASTEXITCODE -ne 0) {
    throw "Doc benchmark sync failed with exit code $LASTEXITCODE"
}

Write-Host "Done! Latest report: tests/reports/latest.md"

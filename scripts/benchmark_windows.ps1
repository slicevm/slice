# PowerShell Benchmark Suite: Slice (Nexus-JS) vs Node.js, Deno, and Bun on Windows
$ErrorActionPreference = "Stop"

$RootDir = Split-Path -Parent $PSScriptRoot
Set-Location $RootDir

$SliceBin = Join-Path $RootDir "target\release\slice.exe"
if (-not (Test-Path $SliceBin)) {
    Write-Host "Building slice release binary..."
    cargo build --release
}

$ColdStartTs = Join-Path $RootDir "benchmarks\workloads\cold_start.ts"
$ColdStartCas = Join-Path $RootDir "benchmarks\workloads\cold_start.cas"
$ComputeJs = Join-Path $RootDir "benchmarks\workloads\compute_heavy.js"
$ComputeCas = Join-Path $RootDir "benchmarks\workloads\compute_heavy.cas"
$ModGraphDir = Join-Path $RootDir "benchmarks\workloads\module_graph\generated"
$ModGraphCas = Join-Path $RootDir "benchmarks\workloads\module_graph\bundle500.cas"

# 1. Generate 500-module graph for Node/Deno/Bun vs Slice Zero-Stat CAS
New-Item -ItemType Directory -Force -Path $ModGraphDir | Out-Null
$IndexContent = ""
for ($i = 0; $i -lt 500; $i++) {
    $ModFile = Join-Path $ModGraphDir "mod_$i.mjs"
    Set-Content -Path $ModFile -Value "export const VAL_$i = $($i + 1);" -NoNewline
    $IndexContent += "import { VAL_$i } from './mod_$i.mjs';`n"
}
$IndexContent += "console.log(VAL_499);`n"
$IndexFile = Join-Path $ModGraphDir "index.mjs"
Set-Content -Path $IndexFile -Value $IndexContent -NoNewline

# Pre-pack CAS archives for Slice
& $SliceBin pack $ColdStartTs $ColdStartCas | Out-Null
& $SliceBin pack $ComputeJs $ComputeCas | Out-Null
& $SliceBin pack $ModGraphDir $ModGraphCas --entry=./index.mjs | Out-Null

function Measure-CommandAvgMs {
    param(
        [string]$Name,
        [scriptblock]$Block,
        [int]$Iterations = 15
    )
    # Warmup
    & $Block | Out-Null
    $Times = @()
    for ($i = 0; $i -lt $Iterations; $i++) {
        $sw = [System.Diagnostics.Stopwatch]::StartNew()
        & $Block | Out-Null
        $sw.Stop()
        $Times += $sw.Elapsed.TotalMilliseconds
    }
    $Sorted = $Times | Sort-Object
    $P50 = $Sorted[[int]($Iterations / 2)]
    $Min = $Sorted[0]
    return [PSCustomObject]@{
        Runtime = $Name
        MinMs   = [math]::Round($Min, 3)
        P50Ms   = [math]::Round($P50, 3)
    }
}

Write-Host "========================================================================="
Write-Host "  1. CLI PROCESS COLD-START BENCHMARK (TypeScript Execution, 15 Runs)"
Write-Host "========================================================================="
$ColdResults = @()
$ColdResults += Measure-CommandAvgMs "Slice (CAS Snapshot)" { & $SliceBin run $ColdStartCas --quiet }
$ColdResults += Measure-CommandAvgMs "Slice (Raw .ts)"      { & $SliceBin run $ColdStartTs --quiet }
$ColdResults += Measure-CommandAvgMs "Bun v1.3.14"          { bun run $ColdStartTs }
$ColdResults += Measure-CommandAvgMs "Deno v2.8.3"          { deno run $ColdStartTs }
$ColdResults += Measure-CommandAvgMs "Node.js v22.22.3"     { node --experimental-strip-types $ColdStartTs 2>$null }
$ColdResults | Format-Table -AutoSize

Write-Host "========================================================================="
Write-Host "  2. 500-MODULE GRAPH RESOLUTION BENCHMARK (Zero-Stat CAS vs FS Stat)"
Write-Host "========================================================================="
$ModResults = @()
$ModResults += Measure-CommandAvgMs "Slice (Zero-Stat CAS)" { & $SliceBin run $ModGraphCas --quiet } 10
$ModResults += Measure-CommandAvgMs "Bun v1.3.14"           { bun run $IndexFile } 10
$ModResults += Measure-CommandAvgMs "Deno v2.8.3"           { deno run $IndexFile } 10
$ModResults += Measure-CommandAvgMs "Node.js v22.22.3"      { node $IndexFile } 10
$ModResults | Format-Table -AutoSize

Write-Host "========================================================================="
Write-Host "  3. COMPUTE KERNEL BENCHMARK (Fib(40) + 10,000,000 Loop)"
Write-Host "========================================================================="
$CompResults = @()
$CompResults += Measure-CommandAvgMs "Slice (Tier-2 Native)" { & $SliceBin run $ComputeCas --quiet } 10
$CompResults += Measure-CommandAvgMs "Bun v1.3.14"           { bun run $ComputeJs } 10
$CompResults += Measure-CommandAvgMs "Deno v2.8.3"           { deno run $ComputeJs } 10
$CompResults += Measure-CommandAvgMs "Node.js v22.22.3"      { node $ComputeJs } 10
$CompResults | Format-Table -AutoSize

Write-Host "========================================================================="
Write-Host "  4. SLICE IN-PROCESS MICROVM SWARM (1,000 Isolated Tenants)"
Write-Host "========================================================================="
& $SliceBin microvm-bench --tenants=1000
& $SliceBin verify-security

$SummaryJson = @{
    cold_start = $ColdResults
    module_500 = $ModResults
    compute    = $CompResults
    microvm    = (& $SliceBin microvm-bench --tenants=1000 --json | ConvertFrom-Json)
} | ConvertTo-Json -Depth 6

Set-Content -Path (Join-Path $RootDir "docs\raw_results_windows.json") -Value $SummaryJson
Write-Host "Saved Windows benchmark telemetry to docs/raw_results_windows.json"


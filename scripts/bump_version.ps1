# Slice (Nexus-JS) Single-Source Version Bump Script (PowerShell)
# Usage:
#   .\scripts\bump_version.ps1 0.9.2     # Set explicit version
#   .\scripts\bump_version.ps1 patch     # Increment patch (default if omitted)
#   .\scripts\bump_version.ps1 minor     # Increment minor (resets patch to 0)
#   .\scripts\bump_version.ps1 major     # Increment major (resets minor & patch to 0)

param(
    [Parameter(Position = 0)]
    [string]$Target = "patch"
)

$ErrorActionPreference = "Stop"
$RootDir = Split-Path -Parent $PSScriptRoot
Set-Location $RootDir

$CargoTomlPath = Join-Path $RootDir "Cargo.toml"
$CargoToml = [System.IO.File]::ReadAllText($CargoTomlPath)

if ($CargoToml -notmatch '(?m)^\[workspace\.package\]\s*\r?\nversion\s*=\s*"(\d+)\.(\d+)\.(\d+)"') {
    throw "Could not locate [workspace.package] version in Cargo.toml"
}

$CurMajor = [int]$Matches[1]
$CurMinor = [int]$Matches[2]
$CurPatch = [int]$Matches[3]
$OldVersion = "$CurMajor.$CurMinor.$CurPatch"

switch ($Target.ToLower()) {
    "patch" {
        $NewVersion = "$CurMajor.$CurMinor.$($CurPatch + 1)"
    }
    "minor" {
        $NewVersion = "$CurMajor.$($CurMinor + 1).0"
    }
    "major" {
        $NewVersion = "$($CurMajor + 1).0.0"
    }
    default {
        if ($Target -notmatch '^\d+\.\d+\.\d+$') {
            throw "Invalid version target '$Target'. Expected 'patch', 'minor', 'major', or 'X.Y.Z'."
        }
        $NewVersion = $Target
    }
}

if ($OldVersion -eq $NewVersion) {
    Write-Host "Version is already $NewVersion (nothing to change)."
    exit 0
}

Write-Host "Bumping Slice codebase version: v$OldVersion -> v$NewVersion"

# 1. Update Cargo.toml [workspace.package] version
$UpdatedCargoToml = [regex]::Replace(
    $CargoToml,
    '(?m)(^\[workspace\.package\]\s*\r?\nversion\s*=\s*)"\d+\.\d+\.\d+"',
    "`${1}`"$NewVersion`""
)
[System.IO.File]::WriteAllText($CargoTomlPath, $UpdatedCargoToml)
Write-Host "  [updated] Cargo.toml"

# 2. Update scripts and benchmark runner
$TargetFiles = @(
    "scripts/install.ps1",
    "scripts/install.sh",
    "scripts/package_release.ps1",
    "tests/run_real_stack_benchmarks.sh",
    "README.md",
    "bench_latest.md",
    "tests/README.md",
    "tests/frameworks/README.md"
)

Get-ChildItem -Path (Join-Path $RootDir "docs") -Filter "*.md" -Recurse | ForEach-Object {
    $Rel = $_.FullName.Substring($RootDir.Length + 1).Replace('\', '/')
    $TargetFiles += $Rel
}

$EscOld = [regex]::Escape($OldVersion)

foreach ($RelPath in $TargetFiles) {
    $FullPath = Join-Path $RootDir $RelPath
    if (-not (Test-Path $FullPath)) {
        continue
    }
    $Content = [System.IO.File]::ReadAllText($FullPath)
    $Updated = $Content `
        -replace "v$EscOld\b", "v$NewVersion" `
        -replace "`"version`":\s*`"$EscOld`"", "`"version`": `"$NewVersion`"" `
        -replace "`"version`":`"$EscOld`"", "`"version`":`"$NewVersion`"" `
        -replace "Slice-NexusJS/$EscOld\b", "Slice-NexusJS/$NewVersion"

    if ($Updated -ne $Content) {
        [System.IO.File]::WriteAllText($FullPath, $Updated)
        Write-Host "  [updated] $RelPath"
    }
}

# 3. Synchronize Cargo.lock workspace package entries
Write-Host "  [syncing] Cargo.lock via cargo update --workspace..."
cargo update --workspace --quiet
Write-Host "Successfully bumped Slice to v$NewVersion"


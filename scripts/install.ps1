# SliceVM Runtime Windows Installer Script
# Downloads prebuilt slice-windows-x86_64.zip with SHA-256 verification (default), or builds
# from source when -FromSource or -SkipBuild is passed inside a local checkout.

param(
    [switch]$FromSource,
    [switch]$SkipBuild,
    [string]$Version = "latest"
)

$ErrorActionPreference = "Stop"
$Repo = if ($env:SLICE_REPO) { $env:SLICE_REPO } else { "slicevm/slice" }
$InstallDir = if ($env:SLICE_INSTALL_DIR) { $env:SLICE_INSTALL_DIR } else { Join-Path $HOME ".slice\bin" }
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
$DestExe = Join-Path $InstallDir "slice.exe"

if ($FromSource -or $SkipBuild) {
    $RootDir = Split-Path -Parent $PSScriptRoot
    Set-Location $RootDir
    $ReleaseExe = Join-Path $RootDir "target\release\slice.exe"
    if (-not $SkipBuild) {
        Write-Host "Building SliceVM Runtime release binary from source..."
        cargo build --release --locked
        if ($LASTEXITCODE -ne 0) {
            exit $LASTEXITCODE
        }
    } elseif (-not (Test-Path $ReleaseExe)) {
        throw "-SkipBuild specified, but $ReleaseExe does not exist"
    }
    Copy-Item -Force $ReleaseExe $DestExe
    Write-Host "Installed SliceVM Runtime to $DestExe"
    & $DestExe info
    exit 0
}

$Archive = "slice-windows-x86_64.zip"
$BaseUrl = if ($Version -eq "latest") {
    "https://github.com/$Repo/releases/latest/download"
} else {
    "https://github.com/$Repo/releases/download/$Version"
}

$TmpDir = Join-Path ([System.IO.Path]::GetTempPath()) ("slice_install_" + [System.Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $TmpDir | Out-Null

try {
    $ZipPath = Join-Path $TmpDir $Archive
    $ShaPath = Join-Path $TmpDir "$Archive.sha256"

    Write-Host "Downloading $Archive from $BaseUrl..."
    Invoke-WebRequest -Uri "$BaseUrl/$Archive" -OutFile $ZipPath -UseBasicParsing
    Invoke-WebRequest -Uri "$BaseUrl/$Archive.sha256" -OutFile $ShaPath -UseBasicParsing

    Write-Host "Verifying SHA-256 checksum..."
    $ExpectedHash = ((Get-Content $ShaPath -Raw).Trim().Split()[0]).ToLowerInvariant()
    $ActualHash = (Get-FileHash $ZipPath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($ExpectedHash -ne $ActualHash) {
        throw "SHA-256 checksum mismatch for $Archive (expected $ExpectedHash, got $ActualHash)"
    }

    Expand-Archive -Path $ZipPath -DestinationPath $TmpDir -Force
    Copy-Item -Force (Join-Path $TmpDir "slice.exe") $DestExe
    Write-Host "Installed SliceVM Runtime to $DestExe"
    & $DestExe info
} finally {
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $TmpDir
}

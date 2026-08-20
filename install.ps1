#!/usr/bin/env pwsh
# install.ps1 — Install HeliosLite (formerly Forgecode) on Windows / PowerShell
#
# Usage:
#   iwr -useb https://helioslite.phenotype.space/install.ps1 | iex
#
#   # Pin a specific version:
#   iwr -useb https://helioslite.phenotype.space/install.ps1 | iex - -Version 1.2.3
#
#   # Local install (no download): run from repo root
#   pwsh ./install.ps1 -Local
#
# Installs the HeliosLite CLI as a single-binary `helioslite` on PATH.
# On Windows we download the matching raw `forge-*.exe` release binary from
# GitHub Releases and install it as `helioslite.exe`.

[CmdletBinding()]
param(
    [string]$Version,
    [switch]$Local,
    [switch]$SkipForgeAlias,
    [switch]$SkipUpdateCheck
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

function Write-Step($msg) { Write-Host "  → $msg" -ForegroundColor Cyan }
function Write-OK($msg)   { Write-Host "  ✓ $msg" -ForegroundColor Green }
function Write-Warn($msg) { Write-Host "  ⚠ $msg" -ForegroundColor Yellow }
function Write-Err($msg)  { Write-Host "  ✖ $msg" -ForegroundColor Red }
function Test-SemVer([string]$Value) {
    return $Value -match '^[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?$'
}
function Assert-SemVer([string]$Value, [string]$Context) {
    if (-not (Test-SemVer $Value)) {
        Write-Err "$Context is not a valid semantic version: '$Value'"
        exit 1
    }
}

# 1) Resolve target version
$ReleaseRepo = if ($env:HELIOSLITE_RELEASE_REPO) { $env:HELIOSLITE_RELEASE_REPO } else { "KooshaPari/forgecode" }
if ($ReleaseRepo -notmatch '^[A-Za-z0-9._-]+/[A-Za-z0-9._-]+$') {
    Write-Err "Invalid release repository: '$ReleaseRepo'"
    exit 1
}
$ReleasesApi = "https://api.github.com/repos/$ReleaseRepo/releases"
if (-not $Version -and -not $Local) {
    try {
        $relJson = Invoke-RestMethod -Uri "$ReleasesApi/latest" -Headers @{ "User-Agent" = "helioslite-install" }
        if (-not $relJson.tag_name) {
            throw "GitHub latest release response did not contain tag_name"
        }
        $Version = $relJson.tag_name.ToString().TrimStart("v")
    } catch {
        Write-Err "Could not determine latest version from GitHub; refusing an unpinned install: $_"
        exit 1
    }
}
if (-not $Local) {
    $Version = $Version.TrimStart("v")
    Assert-SemVer $Version "Target version"
    Write-Step "Target version: $Version"
} else {
    Write-Step "Target version: local build"
}

# 2) Pick install location
$InstallDir = if ($env:HELIOSLITE_INSTALL_DIR) { $env:HELIOSLITE_INSTALL_DIR } else { "$env:LOCALAPPDATA\helioslite\bin" }
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null

if ($Local) {
    Write-Step "Local install — building from source via cargo..."
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        Write-Err "cargo not on PATH — install rustup: https://rustup.rs/"
        exit 1
    }
    Push-Location (Resolve-Path "$PSScriptRoot")
    try {
        cargo build --release --bin helioslite
        Copy-Item -Force "target\release\helioslite.exe" "$InstallDir\helioslite.exe"
    } finally {
        Pop-Location
    }
} else {
    $Asset = "forge-x86_64-pc-windows-msvc.exe"
    $Url   = "https://github.com/$ReleaseRepo/releases/download/v$Version/$Asset"
    $Tmp   = Join-Path $env:TEMP "helioslite-install-$Version-$([Guid]::NewGuid().ToString('N'))"
    New-Item -ItemType Directory -Force -Path $Tmp | Out-Null

    Write-Step "Downloading $Url"
    $BinaryPath = Join-Path $Tmp "helioslite.exe"
    $ChecksumPath = Join-Path $Tmp "helioslite.exe.sha256"
    try {
        Invoke-WebRequest -Uri $Url -OutFile $BinaryPath -UseBasicParsing
        Invoke-WebRequest -Uri "$Url.sha256" -OutFile $ChecksumPath -UseBasicParsing
    } catch {
        Write-Err "Download or checksum retrieval failed; refusing an unverified binary: $_"
        Remove-Item -Recurse -Force $Tmp -ErrorAction SilentlyContinue
        exit 1
    }

    $ExpectedSha = ((Get-Content -Raw -Path $ChecksumPath).Trim() -split '\s+')[0]
    if ($ExpectedSha -notmatch '^[0-9a-fA-F]{64}$') {
        Write-Err "Invalid SHA-256 checksum format; refusing the binary"
        Remove-Item -Recurse -Force $Tmp -ErrorAction SilentlyContinue
        exit 1
    }
    $ActualSha = (Get-FileHash -Algorithm SHA256 -Path $BinaryPath).Hash
    if (-not [String]::Equals($ExpectedSha, $ActualSha, [StringComparison]::OrdinalIgnoreCase)) {
        Write-Err "SHA-256 verification failed; refusing the binary"
        Remove-Item -Recurse -Force $Tmp -ErrorAction SilentlyContinue
        exit 1
    }
    Write-OK "SHA-256 verified"
    Copy-Item -Force $BinaryPath "$InstallDir\helioslite.exe"
    Remove-Item -Recurse -Force $Tmp
}

# 3) PATH
$UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($UserPath -notlike "*$InstallDir*") {
    Write-Step "Adding $InstallDir to user PATH"
    [Environment]::SetEnvironmentVariable("Path", "$UserPath;$InstallDir", "User")
    $env:Path = "$env:Path;$InstallDir"
}

# 4) Optional: legacy `forge`/`forge-dev` alias
if (-not $SkipForgeAlias) {
    foreach ($old in @("forge", "forge-dev")) {
        $oldPath = Join-Path $InstallDir "$old.exe"
        $newPath = Join-Path $InstallDir "helioslite.exe"
        if (-not (Test-Path $oldPath)) {
            Copy-Item -Force $newPath $oldPath
            Write-OK "Created legacy alias $oldPath"
        }
    }
}

# 5) Verify
$Ver = & "$InstallDir\helioslite.exe" --version 2>&1 | Select-Object -First 1
if ($LASTEXITCODE -ne 0) {
    Write-Err "helioslite --version failed."
    exit 1
}
if (-not $Ver -or $Ver -notmatch '(^|\s)v?[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?(\s|$)') {
    Write-Err "helioslite --version did not report a semantic version; refusing the install."
    exit 1
}
if (-not $Local) {
    $ExpectedVersionPattern = [regex]::Escape($Version)
    if ($Ver -notmatch "(^|\s)v?$ExpectedVersionPattern(\s|$)") {
        Write-Err "Installed binary version does not match requested version $Version."
        exit 1
    }
}
Write-OK "helioslite reports: $Ver"

Write-Host ""
Write-Host "  🎉 HeliosLite installed." -ForegroundColor Green
Write-Host "     Try:  helioslite --help" -ForegroundColor Green
Write-Host "     Docs: https://helioslite.phenotype.space" -ForegroundColor Green
Write-Host "     Old commands still work: forge / forge-dev (deprecated)" -ForegroundColor DarkGray

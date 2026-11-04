# Builds a portable Windows release: dist\TinyPlayer-<version>-windows-x64.zip
# Usage (from anywhere):  powershell -ExecutionPolicy Bypass -File scripts\package-windows.ps1
param([string]$Out = "dist")

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
$name = "TinyPlayer-$version-windows-x64"
$dll = "third_party\mpv\windows\libmpv-2.dll"
if (-not (Test-Path $dll)) {
    throw "Missing $dll - download mpv-dev-x86_64 from https://github.com/shinchiro/mpv-winbuild-cmake/releases"
}

Write-Host "Building release..."
cargo build --release
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

$dir = Join-Path $Out $name
if (Test-Path $dir) { Remove-Item -Recurse -Force $dir }
New-Item -ItemType Directory -Force $dir | Out-Null

Copy-Item "target\release\tinyplayer.exe" (Join-Path $dir "TinyPlayer.exe")
Copy-Item $dll $dir
Copy-Item README.md $dir
Copy-Item THIRD_PARTY.md $dir

$zip = Join-Path $Out "$name.zip"
if (Test-Path $zip) { Remove-Item -Force $zip }
Compress-Archive -Path (Join-Path $dir "*") -DestinationPath $zip

$exeMB = [math]::Round((Get-Item (Join-Path $dir "TinyPlayer.exe")).Length / 1MB, 1)
$dllMB = [math]::Round((Get-Item (Join-Path $dir "libmpv-2.dll")).Length / 1MB, 1)
$zipMB = [math]::Round((Get-Item $zip).Length / 1MB, 1)
Write-Host ""
Write-Host "Packaged: $zip"
Write-Host "  TinyPlayer.exe  $exeMB MB"
Write-Host "  libmpv-2.dll    $dllMB MB"
Write-Host "  zip             $zipMB MB"

# Packs the source of the libmpv build bundled on Windows into
# dist\third-party-sources-<version>-windows-x64.tar, for the GPL.
# Usage: powershell -ExecutionPolicy Bypass -File scripts\collect-sources-windows.ps1 -Tag <release> -Ffmpeg <ffmpeg.exe>
# Tag is the shinchiro/mpv-winbuild-cmake release the DLL came from, and
# ffmpeg.exe comes from that same release.
param(
    [Parameter(Mandatory)][string]$Tag,
    [Parameter(Mandatory)][string]$Ffmpeg,
    [string]$Out = "dist"
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$repo = "shinchiro/mpv-winbuild-cmake"
$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
$assets = (gh release view $Tag --repo $repo --json assets --jq ".assets[].name") -join "`n"
if ($LASTEXITCODE -ne 0) { throw "can't read release $Tag" }
if ($assets -notmatch 'mpv-dev-x86_64-\d+-git-([0-9a-f]+)\.7z') { throw "no mpv build in $Tag" }
$mpv = $Matches[1]
if ($assets -notmatch 'ffmpeg-x86_64-git-([0-9a-f]+)\.7z') { throw "no FFmpeg build in $Tag" }
$ffmpegCommit = $Matches[1]

$stage = Join-Path ([IO.Path]::GetTempPath()) "third-party-sources"
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory -Force $stage | Out-Null

$sources = @{
    "mpv-$mpv.tar.gz" = "https://github.com/mpv-player/mpv/archive/$mpv.tar.gz"
    "ffmpeg-$ffmpegCommit.tar.gz" = "https://github.com/FFmpeg/FFmpeg/archive/$ffmpegCommit.tar.gz"
    "mpv-winbuild-cmake-$Tag.tar.gz" = "https://github.com/$repo/archive/refs/tags/$Tag.tar.gz"
}
foreach ($name in $sources.Keys) {
    Write-Host "Downloading $name"
    curl.exe -fsSL --retry 5 --retry-all-errors -o (Join-Path $stage $name) $sources[$name]
    if ($LASTEXITCODE -ne 0) { throw "can't download $name" }
}

$config = & $Ffmpeg -hide_banner -version
@"
Source of the libmpv build bundled with TinyPlayer $version for Windows.

Build: https://github.com/$repo/releases/tag/$Tag
mpv commit: $mpv
FFmpeg commit: $ffmpegCommit

mpv-winbuild-cmake-$Tag.tar.gz holds the build recipes, which list every other
library linked into libmpv-2.dll and where it comes from.

FFmpeg's version and configuration:

$($config -join "`n")
"@ | Set-Content -Encoding utf8 (Join-Path $stage "SOURCES.txt")

New-Item -ItemType Directory -Force $Out | Out-Null
$tar = Join-Path (Resolve-Path $Out).Path "third-party-sources-$version-windows-x64.tar"
tar.exe -cf $tar -C (Split-Path -Parent $stage) (Split-Path -Leaf $stage)
if ($LASTEXITCODE -ne 0) { throw "tar failed" }
Write-Host "Packaged: $tar ($([math]::Round((Get-Item $tar).Length / 1MB, 1)) MB)"

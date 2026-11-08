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

# GitHub's archive downloads (codeload) sometimes answer 504 for long
# stretches, so the source comes through git itself: one commit, no history.
function Save-Source([string]$From, [string]$Ref, [string]$Name) {
    $out = Join-Path $stage "$Name.tar.gz"
    $sha = gh api "repos/$From/commits/$Ref" --jq .sha
    if ($LASTEXITCODE -ne 0 -or -not $sha) { throw "can't find $Ref in $From" }
    for ($attempt = 1; ; $attempt++) {
        Write-Host "Fetching $Name ($sha)"
        $clone = Join-Path ([IO.Path]::GetTempPath()) "src-$Name"
        if (Test-Path $clone) { Remove-Item -Recurse -Force $clone }
        git init -q $clone
        git -C $clone fetch -q --depth 1 "https://github.com/$From.git" $sha
        if ($LASTEXITCODE -eq 0) {
            git -C $clone archive --format=tar.gz --prefix="$Name/" -o $out FETCH_HEAD
        }
        $ok = $LASTEXITCODE -eq 0
        Remove-Item -Recurse -Force $clone
        if ($ok) { return }
        if ($attempt -ge 4) { throw "can't fetch $Name" }
        Write-Host "Fetching $Name failed, retrying in $(15 * $attempt)s"
        Start-Sleep -Seconds (15 * $attempt)
    }
}

Save-Source "mpv-player/mpv" $mpv "mpv-$mpv"
Save-Source "FFmpeg/FFmpeg" $ffmpegCommit "ffmpeg-$ffmpegCommit"
Save-Source $repo "refs/tags/$Tag" "mpv-winbuild-cmake-$Tag"

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

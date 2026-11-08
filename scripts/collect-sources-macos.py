#!/usr/bin/env python3
"""Packs the source of every Homebrew library in dist/TinyPlayer.app into
dist/third-party-sources-<version>-macos-<arch>.tar, for the GPL.

Run it after scripts/package-macos.sh, on the machine that built the package.
`brew fetch --build-from-source` can't be used: it fails on some of these
formulas, so this reads the same formulas and downloads what they build from.
Each library gets its source, the patches and resources it builds with, and
the formula, which says how it's built.
"""

import glob
import hashlib
import json
import os
import platform
import re
import subprocess
import tarfile
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FRAMEWORKS = ROOT / "dist/TinyPlayer.app/Contents/Frameworks"
CORE = "https://raw.githubusercontent.com/Homebrew/homebrew-core"


def run(*args, cwd=None):
    return subprocess.run(args, cwd=cwd, check=True, capture_output=True, text=True).stdout


RELEASE_ASSET = re.compile(r"https://github\.com/([^/]+/[^/]+)/releases/download/([^/]+)/([^/?]+)$")
GITHUB_ARCHIVE = re.compile(r"https://github\.com/([^/]+/[^/]+)/archive/(?:refs/tags/)?(.+?)\.(?:tar\.gz|tgz)$")
# Sources taken from git because GitHub's archive download failed.
FROM_GIT = []


def download(url, path, tries=4):
    # GitHub's releases/download links sometimes fail for long stretches
    # (504, connection resets) while the API serves the same file, so
    # release assets go through gh, which uses the API.
    asset = RELEASE_ASSET.match(url)
    for attempt in range(tries):
        try:
            if asset:
                repo, tag, name = asset.groups()
                run("gh", "release", "download", tag, "--repo", repo, "--pattern", name, "--output", str(path), "--clobber")
            else:
                run("curl", "-fsSL", "--retry", "5", "--retry-all-errors", "--retry-delay", "3", "-o", str(path), url)
            return
        except subprocess.CalledProcessError as e:
            path.unlink(missing_ok=True)
            if attempt == tries - 1:
                raise
            print(f"download failed ({e.stderr.strip()[:200]}), retrying in {15 * (attempt + 1)}s: {url}", flush=True)
            time.sleep(15 * (attempt + 1))


def fetch(url, sha256, folder):
    path = folder / (url.split("?")[0].rstrip("/").rsplit("/", 1)[-1] or "download")
    archive = GITHUB_ARCHIVE.match(url)
    try:
        download(url, path, tries=1 if archive else 4)
    except subprocess.CalledProcessError:
        if not archive:
            raise
        # GitHub's archive downloads (codeload) answer 504 for minutes at a
        # time; git serves the same tree. Its archive isn't byte for byte the
        # one the formula's checksum is for, so that check is skipped.
        repo, ref = archive.groups()
        print(f"archive download failed, taking {ref} from git: {url}", flush=True)
        archive_ref(f"https://github.com/{repo}.git", ref, path, f"{repo.split('/')[1]}-{ref}")
        FROM_GIT.append(url)
        return
    if sha256 and hashlib.sha256(path.read_bytes()).hexdigest() != sha256:
        raise SystemExit(f"checksum mismatch: {url}")


def archive_ref(url, ref, path, prefix):
    """One commit or tag of a repository, as a .tar.gz, without its history."""
    for attempt in range(4):
        try:
            with tempfile.TemporaryDirectory() as clone:
                run("git", "init", "-q", clone)
                run("git", "fetch", "-q", "--depth", "1", url, ref, cwd=clone)
                run("git", "archive", "--format=tar.gz", f"--prefix={prefix}/", "-o", str(path), "FETCH_HEAD", cwd=clone)
            return
        except subprocess.CalledProcessError as e:
            path.unlink(missing_ok=True)
            if attempt == 3:
                raise
            print(f"git fetch failed ({e.stderr.strip()[:200]}), retrying in {15 * (attempt + 1)}s: {url} {ref}", flush=True)
            time.sleep(15 * (attempt + 1))


def fetch_git(url, revision, folder):
    name = url.rstrip("/").rsplit("/", 1)[-1].removesuffix(".git")
    with tempfile.TemporaryDirectory() as clone:
        run("git", "clone", "-q", "--filter=blob:none", url, clone)
        run("git", "checkout", "-q", revision, cwd=clone)
        run("git", "archive", "--format=tar.gz", "-o", str(folder / f"{name}-{revision}.tar.gz"), "HEAD", cwd=clone)


def owners():
    """Which formula each library file belongs to."""
    prefix = run("brew", "--prefix").strip()
    found = {}
    for formula in run("brew", "list", "--formula").split():
        for path in glob.glob(f"{prefix}/opt/{formula}/lib/**/*.dylib", recursive=True):
            found.setdefault(os.path.basename(path), formula)
    return found


def extras(ruby):
    """(url, sha256, revision, file) of the patches and resources in a formula."""
    found, block = [], None
    for line in ruby.splitlines():
        if block is None:
            m = re.match(r'(\s*)(?:patch do|patch :\w+ do|resource "([^"]*)"(, :test)? do)\s*$', line)
            if m and not m[3] and not (m[2] or "").startswith("homebrew-"):
                block = {"indent": m[1]}
        elif line.rstrip() == block["indent"] + "end":
            found.append(block)
            block = None
        else:
            m = re.match(r'\s*(url|sha256|revision:?|file)\s+"([^"]*)"', line)
            if m:
                # The first of each: a nested livecheck block has its own url.
                block.setdefault(m[1].rstrip(":"), m[2])
            m = re.search(r'revision:\s+"([^"]*)"', line)
            if m:
                block.setdefault("revision", m[1])
    return found


def collect(info, folder):
    name = info["name"]
    stable = info["versions"]["stable"]
    installed = [v["version"].rsplit("_", 1)[0] for v in info["installed"]]
    if stable not in installed:
        raise SystemExit(f"{name}: formula is {stable} but {installed} is installed")

    folder.mkdir(parents=True)
    head = info["tap_git_head"]
    ruby_path = folder / f"{name}.rb"
    download(f"{CORE}/{head}/{info['ruby_source_path']}", ruby_path)

    source = info["urls"]["stable"]
    if source.get("revision"):
        fetch_git(source["url"], source["revision"], folder)
    else:
        fetch(source["url"], source.get("checksum"), folder)

    seen = set()
    for extra in extras(ruby_path.read_text()):
        key = extra.get("url") or extra.get("file")
        if key is None or key in seen:
            continue
        seen.add(key)
        if "file" in extra:
            download(f"{CORE}/{head}/{extra['file']}", folder / os.path.basename(extra["file"]))
        elif "revision" in extra:
            fetch_git(extra["url"], extra["revision"], folder)
        else:
            fetch(extra["url"], extra.get("sha256"), folder)
    print(f"{name} {stable}: {len(list(folder.iterdir()))} files", flush=True)
    return f"{name} {stable}"


def main():
    version = re.search(r'^version = "(.*)"', (ROOT / "Cargo.toml").read_text(), re.M)[1]
    libraries = sorted(FRAMEWORKS.glob("*.dylib"))
    if not libraries:
        raise SystemExit(f"no libraries in {FRAMEWORKS}")
    known = owners()
    missing = [p.name for p in libraries if p.name not in known]
    if missing:
        raise SystemExit(f"no Homebrew formula has {missing}")
    formulas = sorted({known[p.name] for p in libraries})
    details = json.loads(run("brew", "info", "--json=v2", *formulas))["formulae"]

    out = ROOT / f"dist/third-party-sources-{version}-macos-{platform.machine()}.tar"
    with tempfile.TemporaryDirectory() as work:
        stage = Path(work) / "third-party-sources"
        lines = [collect(info, stage / info["name"]) for info in details]
        (stage / "SOURCES.txt").write_text(
            f"Source of the Homebrew libraries bundled with TinyPlayer {version} for macOS.\n"
            "Each folder holds a library's source, the patches and resources it builds\n"
            "with, and its Homebrew formula, which says how it's built.\n\n"
            + "\n".join(lines)
            + "\n"
            + "".join(
                f"\nGitHub's download failed, so this came from git at the same tag or commit:\n{url}\n" for url in FROM_GIT
            )
        )
        with tarfile.open(out, "w") as tar:
            tar.add(stage, arcname=stage.name)
    print(f"Packaged: {out} ({out.stat().st_size / 1e6:.0f} MB)")


if __name__ == "__main__":
    main()

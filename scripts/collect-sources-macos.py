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


def download(url, path):
    # GitHub release downloads sometimes reset mid-transfer (curl exit 56),
    # and curl's own quick retries can all land in the same bad patch.
    for attempt in range(4):
        try:
            run("curl", "-fsSL", "--retry", "5", "--retry-all-errors", "--retry-delay", "3", "-o", str(path), url)
            return
        except subprocess.CalledProcessError:
            path.unlink(missing_ok=True)
            if attempt == 3:
                raise
            print(f"download failed, retrying in {15 * (attempt + 1)}s: {url}", flush=True)
            time.sleep(15 * (attempt + 1))


def fetch(url, sha256, folder):
    path = folder / (url.split("?")[0].rstrip("/").rsplit("/", 1)[-1] or "download")
    download(url, path)
    if sha256 and hashlib.sha256(path.read_bytes()).hexdigest() != sha256:
        raise SystemExit(f"checksum mismatch: {url}")


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
        )
        with tarfile.open(out, "w") as tar:
            tar.add(stage, arcname=stage.name)
    print(f"Packaged: {out} ({out.stat().st_size / 1e6:.0f} MB)")


if __name__ == "__main__":
    main()

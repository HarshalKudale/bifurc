#!/usr/bin/env python3
"""Edits the update manifest: the file a running Bifurc reads to learn that a newer version exists.

It is published as updates/v1/stable.json in the public releases repository and served at
https://bifurc.app/updates/v1/stable.json. What each field means to an install is in
crates/engine/src/updates/manifest.rs.

  update_manifest.py release --manifest stable.json --version 0.5.0 --tag v0.5.0 \\
      --repo HarshalKudale/bifurc-releases --assets <folder of the release's files> \\
      [--summary "…"] [--min-direct-from 0.4.0] [--critical] [--minimum-supported 0.4.0] [--date 2026-10-20]

      Adds the release (or replaces the entry of the same version), with the address, SHA-256 and size of
      each file an install is replaced by.

  update_manifest.py channel --manifest stable.json --channel store --version 0.5.0

      Records the version a store or a package manager now has. An install from it is offered nothing newer,
      so this is run when that store has accepted the release, not when the release is made.
"""

import argparse
import datetime
import hashlib
import json
import os
import re
import sys

SCHEMA = 1
CHANGELOG_URL = "https://bifurc.app/changelog"
CHANNELS = ["store", "winget", "scoop", "aur"]
# The key of a file in a release's `packages`, by the name `cargo xtask package` gives it. An install looks its
# own file up by this key (`Channel::package_key`).
PACKAGES = [
    (re.compile(r"^Bifurc-.+-setup\.exe$"), "nsis-x64"),
    (re.compile(r"^Bifurc-.+-win-(x64|arm64)\.zip$"), "zip-{0}"),
    (re.compile(r"^Bifurc-.+-(x86_64|aarch64)\.AppImage$"), "appimage-{0}"),
    (re.compile(r"^bifurc_.+_(amd64|arm64)\.deb$"), "deb-{0}"),
    (re.compile(r"^Bifurc-.+-linux-(x86_64|aarch64)\.tar\.gz$"), "tarball-{0}"),
    (re.compile(r"^Bifurc-.+-mac-(x64|arm64)\.dmg$"), "dmg-{0}"),
]
VERSION = re.compile(r"^\d+\.\d+\.\d+$")
# The exit code of `release` for a folder with none of those files: a release from before them, which the
# manifest has no use for.
NO_PACKAGES = 3


def version_key(text):
    return tuple(int(part) for part in text.split("."))


def a_version(text):
    text = text.strip().removeprefix("v")
    if not VERSION.match(text):
        raise argparse.ArgumentTypeError(f"{text!r} is not a version (major.minor.patch)")
    return text


def load(path):
    if not os.path.exists(path) or os.path.getsize(path) == 0:
        return {"schema": SCHEMA, "channels": {}, "releases": []}
    with open(path, encoding="utf-8") as file:
        manifest = json.load(file)
    manifest.setdefault("schema", SCHEMA)
    manifest.setdefault("channels", {})
    manifest.setdefault("releases", [])
    return manifest


def save(path, manifest):
    with open(path, "w", encoding="utf-8", newline="\n") as file:
        json.dump(manifest, file, indent=2, ensure_ascii=False)
        file.write("\n")


def package_key(name):
    for pattern, key in PACKAGES:
        match = pattern.match(name)
        if match:
            return key.format(*match.groups())
    return None


def sha256_of(path):
    digest = hashlib.sha256()
    with open(path, "rb") as file:
        for block in iter(lambda: file.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def packages_in(folder, repo, tag):
    packages = {}
    for name in sorted(os.listdir(folder)):
        key = package_key(name)
        if key is None:
            continue
        path = os.path.join(folder, name)
        packages[key] = {
            "url": f"https://github.com/{repo}/releases/download/{tag}/{name}",
            "sha256": sha256_of(path),
            "size": os.path.getsize(path),
        }
    return packages


def add_release(args):
    manifest = load(args.manifest)
    packages = packages_in(args.assets, args.repo, args.tag)
    if not packages:
        print(f"{args.assets} holds no file an install is replaced by", file=sys.stderr)
        sys.exit(NO_PACKAGES)
    earlier = next((r for r in manifest["releases"] if r.get("version") == args.version), {})
    release = {
        "version": args.version,
        "date": args.date or earlier.get("date") or datetime.datetime.now(datetime.timezone.utc).date().isoformat(),
        "critical": bool(args.critical or earlier.get("critical", False)),
        "summary": args.summary or earlier.get("summary") or f"Bifurc {args.version}",
        "notesUrl": CHANGELOG_URL,
        "packages": packages,
    }
    # Left out, a release may replace any version in one step.
    min_direct_from = args.min_direct_from or earlier.get("minDirectFrom")
    if min_direct_from:
        release["minDirectFrom"] = min_direct_from
    others = [r for r in manifest["releases"] if r.get("version") != args.version]
    manifest["releases"] = sorted(others + [release], key=lambda r: version_key(r["version"]), reverse=True)
    if args.minimum_supported:
        manifest["minimumSupported"] = args.minimum_supported
    save(args.manifest, manifest)
    print(f"{args.version}: {', '.join(packages)}")


def set_channel(args):
    manifest = load(args.manifest)
    if not any(r.get("version") == args.version for r in manifest["releases"]):
        sys.exit(f"the manifest has no release {args.version}: publish it first")
    manifest["channels"][args.channel] = args.version
    save(args.manifest, manifest)
    print(f"{args.channel}: {args.version}")


def main():
    parser = argparse.ArgumentParser(description="Edit Bifurc's update manifest.")
    commands = parser.add_subparsers(dest="command", required=True)

    release = commands.add_parser("release", help="add or replace a release")
    release.add_argument("--manifest", required=True)
    release.add_argument("--version", required=True, type=a_version)
    release.add_argument("--tag", required=True)
    release.add_argument("--repo", required=True)
    release.add_argument("--assets", required=True)
    release.add_argument("--summary", default="")
    release.add_argument("--min-direct-from", type=a_version)
    release.add_argument("--minimum-supported", type=a_version)
    release.add_argument("--critical", action="store_true")
    release.add_argument("--date", default="")
    release.set_defaults(run=add_release)

    channel = commands.add_parser("channel", help="record the version a store or package manager has")
    channel.add_argument("--manifest", required=True)
    channel.add_argument("--channel", required=True, choices=CHANNELS)
    channel.add_argument("--version", required=True, type=a_version)
    channel.set_defaults(run=set_channel)

    args = parser.parse_args()
    args.run(args)


if __name__ == "__main__":
    main()

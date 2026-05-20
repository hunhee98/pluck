#!/usr/bin/env python3
"""Validate release version metadata.

The release workflow calls this with --tag. Local dry runs can use
--allow-unreleased while a minor train is still accumulating changes.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
WORKSPACE_PACKAGES = ("pluck-core", "pluck-mcp", "pluck-cli", "pluck-bench")
PUBLISHABLE_INTERNAL_DEPS = (
    (Path("crates/pluck-cli/Cargo.toml"), "pluck-core"),
    (Path("crates/pluck-mcp/Cargo.toml"), "pluck-core"),
)
SEMVER_RE = re.compile(r"^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$")


def fail(message: str) -> None:
    print(f"version-check: {message}", file=sys.stderr)
    raise SystemExit(1)


def read_text(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except FileNotFoundError:
        fail(f"missing {path.relative_to(ROOT)}")


def workspace_version() -> str:
    cargo = read_text(ROOT / "Cargo.toml")
    match = re.search(
        r"(?ms)^\[workspace\.package\].*?^version\s*=\s*\"([^\"]+)\"",
        cargo,
    )
    if not match:
        fail("Cargo.toml is missing [workspace.package] version")
    version = match.group(1)
    if not SEMVER_RE.match(version):
        fail(f"workspace version is not SemVer: {version}")
    return version


def check_internal_deps(version: str) -> None:
    for manifest, dep in PUBLISHABLE_INTERNAL_DEPS:
        text = read_text(ROOT / manifest)
        match = re.search(
            rf"{re.escape(dep)}\s*=\s*\{{[^}}]*version\s*=\s*\"([^\"]+)\"",
            text,
        )
        if not match:
            fail(f"{manifest} is missing a versioned {dep} path dependency")
        if match.group(1) != version:
            fail(
                f"{manifest} depends on {dep} {match.group(1)}, "
                f"but workspace version is {version}"
            )


def cargo_lock_versions() -> dict[str, str]:
    lock = read_text(ROOT / "Cargo.lock")
    versions: dict[str, str] = {}
    for block in lock.split("[[package]]"):
        name = re.search(r'^name\s*=\s*"([^"]+)"', block, re.MULTILINE)
        version = re.search(r'^version\s*=\s*"([^"]+)"', block, re.MULTILINE)
        if name and version and name.group(1) in WORKSPACE_PACKAGES:
            versions[name.group(1)] = version.group(1)
    return versions


def check_cargo_lock(version: str) -> None:
    versions = cargo_lock_versions()
    missing = [name for name in WORKSPACE_PACKAGES if name not in versions]
    if missing:
        fail(f"Cargo.lock is missing workspace packages: {', '.join(missing)}")
    mismatched = {
        name: found for name, found in versions.items() if found != version
    }
    if mismatched:
        details = ", ".join(
            f"{name}={found}" for name, found in sorted(mismatched.items())
        )
        fail(f"Cargo.lock does not match workspace version {version}: {details}")


def check_changelog_release(version: str) -> None:
    changelog = read_text(ROOT / "CHANGELOG.md")
    # The closing `]` is followed by whitespace or end-of-line in the
    # Keep-a-Changelog format ("## [0.5.0] — 2026-05-19"). The previous
    # `\b` anchor never matched because `\b` needs a word/non-word
    # transition and both `]` and ` ` are non-word — leaving the check
    # silently broken for every release after the script landed.
    if not re.search(
        rf"^## \[{re.escape(version)}\](?:\s|$)", changelog, re.MULTILINE
    ):
        fail(f"CHANGELOG.md is missing a ## [{version}] release section")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--tag", help="release tag, for example v0.4.0")
    parser.add_argument("--print-version", action="store_true")
    parser.add_argument(
        "--allow-unreleased",
        action="store_true",
        help="skip the changelog release-section requirement",
    )
    args = parser.parse_args()

    version = workspace_version()
    if args.print_version:
        print(version)
        return

    check_internal_deps(version)
    check_cargo_lock(version)

    if args.tag:
        expected = f"v{version}"
        if args.tag != expected:
            fail(f"tag {args.tag} does not match workspace version {version}")
        check_changelog_release(version)
    elif not args.allow_unreleased:
        check_changelog_release(version)

    print(f"version-check: OK ({version})")


if __name__ == "__main__":
    main()

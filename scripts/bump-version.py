#!/usr/bin/env python3
"""Bump the workspace version and internal crate dependency pins.

Examples:
    scripts/bump-version.py patch
    scripts/bump-version.py minor
    scripts/bump-version.py 0.3.1
    scripts/bump-version.py patch --dry-run
"""

from __future__ import annotations

import argparse
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
WORKSPACE_PACKAGES = {"pluck-core", "pluck-mcp", "pluck-cli", "pluck-bench"}
INTERNAL_DEP_MANIFESTS = (
    ROOT / "crates/pluck-cli/Cargo.toml",
    ROOT / "crates/pluck-mcp/Cargo.toml",
)
SEMVER_RE = re.compile(r"^(\d+)\.(\d+)\.(\d+)(?:[-+][0-9A-Za-z.-]+)?$")


def read(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def write(path: Path, text: str, dry_run: bool) -> None:
    if dry_run:
        return
    path.write_text(text, encoding="utf-8")


def current_version() -> str:
    cargo = read(ROOT / "Cargo.toml")
    match = re.search(
        r"(?ms)^\[workspace\.package\].*?^version\s*=\s*\"([^\"]+)\"",
        cargo,
    )
    if not match:
        raise SystemExit("workspace version not found in Cargo.toml")
    return match.group(1)


def next_version(current: str, bump: str) -> str:
    match = SEMVER_RE.match(current)
    if not match:
        raise SystemExit(f"current version is not SemVer: {current}")
    major, minor, patch = map(int, match.groups())

    if bump == "patch":
        return f"{major}.{minor}.{patch + 1}"
    if bump == "minor":
        return f"{major}.{minor + 1}.0"
    if bump == "major":
        return f"{major + 1}.0.0"
    if SEMVER_RE.match(bump):
        return bump
    raise SystemExit(
        "bump must be 'patch', 'minor', 'major', or an exact version like 0.4.1"
    )


def replace_once(text: str, pattern: str, repl: str, path: Path) -> str:
    updated, count = re.subn(pattern, repl, text, count=1, flags=re.MULTILINE | re.DOTALL)
    if count != 1:
        raise SystemExit(f"expected exactly one version replacement in {path}")
    return updated


def update_workspace_manifest(old: str, new: str, dry_run: bool) -> None:
    path = ROOT / "Cargo.toml"
    text = read(path)
    updated = replace_once(
        text,
        rf"(^\[workspace\.package\].*?^version\s*=\s*\"){re.escape(old)}(\")",
        rf"\g<1>{new}\2",
        path,
    )
    write(path, updated, dry_run)


def update_internal_deps(old: str, new: str, dry_run: bool) -> None:
    for path in INTERNAL_DEP_MANIFESTS:
        text = read(path)
        updated = replace_once(
            text,
            rf"(pluck-core\s*=\s*\{{[^}}]*version\s*=\s*\"){re.escape(old)}(\")",
            rf"\g<1>{new}\2",
            path,
        )
        write(path, updated, dry_run)


def update_lock(new: str, dry_run: bool) -> None:
    path = ROOT / "Cargo.lock"
    text = read(path)
    parts = text.split("[[package]]")
    updated_parts = [parts[0]]
    changed = 0
    for block in parts[1:]:
        name = re.search(r'^name\s*=\s*"([^"]+)"', block, flags=re.MULTILINE)
        if name and name.group(1) in WORKSPACE_PACKAGES:
            block, count = re.subn(
                r'(^version\s*=\s*")[^"]+(")',
                rf"\g<1>{new}\2",
                block,
                count=1,
                flags=re.MULTILINE,
            )
            changed += count
        updated_parts.append("[[package]]" + block)
    if changed != len(WORKSPACE_PACKAGES):
        raise SystemExit(
            f"expected {len(WORKSPACE_PACKAGES)} Cargo.lock updates, got {changed}"
        )
    write(path, "".join(updated_parts), dry_run)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("bump", help="'patch', 'minor', 'major', or exact SemVer")
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()

    old = current_version()
    new = next_version(old, args.bump)
    if old == new:
        raise SystemExit(f"version is already {new}")

    update_workspace_manifest(old, new, args.dry_run)
    update_internal_deps(old, new, args.dry_run)
    update_lock(new, args.dry_run)

    prefix = "would bump" if args.dry_run else "bumped"
    print(f"{prefix}: {old} -> {new}")
    if not args.dry_run:
        print("next: python3 scripts/version-check.py --allow-unreleased")


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Print local maintainer state and the next release decisions.

This script is intentionally advisory: it exits zero when it can inspect the
repo, and prints the decisions a maintainer or coding agent must not skip.
"""

from __future__ import annotations

import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SEMVER_RE = re.compile(r"^v?(\d+)\.(\d+)\.(\d+)$")


def run_git(*args: str) -> str:
    proc = subprocess.run(
        ["git", *args],
        cwd=ROOT,
        text=True,
        capture_output=True,
        check=False,
    )
    if proc.returncode != 0:
        return ""
    return proc.stdout.strip()


def semver_key(value: str) -> tuple[int, int, int]:
    match = SEMVER_RE.match(value.strip())
    if not match:
        return (-1, -1, -1)
    return tuple(int(part) for part in match.groups())


def workspace_version() -> str:
    cargo = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    match = re.search(
        r"(?ms)^\[workspace\.package\].*?^version\s*=\s*\"([^\"]+)\"",
        cargo,
    )
    if not match:
        raise SystemExit("workspace version not found in Cargo.toml")
    return match.group(1)


def release_tags() -> list[str]:
    tags = run_git("tag", "--list", "v[0-9]*.[0-9]*.[0-9]*").splitlines()
    valid = (tag.strip() for tag in tags if semver_key(tag) != (-1, -1, -1))
    return sorted(valid, key=semver_key)


def changelog() -> str:
    return (ROOT / "CHANGELOG.md").read_text(encoding="utf-8")


def unreleased_body(text: str) -> str:
    match = re.search(r"(?ms)^## \[Unreleased\]\s*(.*?)(?=^## \[)", text)
    return match.group(1).strip() if match else ""


def has_notes(body: str) -> bool:
    return bool(re.search(r"(?m)^\s*-\s+\S", body))


def has_release_section(text: str, version: str) -> bool:
    return bool(re.search(rf"(?m)^## \[{re.escape(version)}\]\b", text))


def print_section(title: str, rows: list[str]) -> None:
    print(title)
    for row in rows:
        print(f"- {row}")


def main() -> None:
    version = workspace_version()
    tags = release_tags()
    latest_tag = tags[-1] if tags else "(none)"
    expected_tag = f"v{version}"
    current_branch = run_git("branch", "--show-current") or "(detached)"
    status = run_git("status", "--short")
    dirty = bool(status.strip())
    changelog_text = changelog()
    unreleased = unreleased_body(changelog_text)
    notes_pending = has_notes(unreleased)
    current_section = has_release_section(changelog_text, version)
    tag_exists = expected_tag in tags

    print_section(
        "Maintainer Status",
        [
            f"branch: {current_branch}",
            f"working tree: {'dirty' if dirty else 'clean'}",
            f"workspace version: {version}",
            f"latest release tag: {latest_tag}",
            f"matching tag for workspace version: {'yes' if tag_exists else 'no'}",
            f"CHANGELOG release section for {version}: {'yes' if current_section else 'no'}",
            f"CHANGELOG [Unreleased] has notes: {'yes' if notes_pending else 'no'}",
        ],
    )

    decisions: list[str] = []
    if dirty:
        decisions.append("Commit/stash local edits before release or branch surgery.")
    if latest_tag != "(none)" and semver_key(version) > semver_key(latest_tag):
        decisions.append(
            f"{version} is ahead of {latest_tag}; either keep accumulating the active train or cut {expected_tag}."
        )
    if not tag_exists:
        decisions.append(
            f"No {expected_tag} tag exists. A release cut must create it after CHANGELOG is finalized."
        )
    if not current_section:
        decisions.append(
            f"Tagging {expected_tag} will fail release metadata checks until CHANGELOG has ## [{version}]."
        )
    if notes_pending:
        decisions.append(
            "Unreleased notes exist; after the current merge, decide explicitly: defer, patch backport, or release now."
        )
    decisions.append(
        "For shipped bugs, decide whether to backport to the latest release branch before treating main as enough."
    )
    decisions.append(
        "For roadmap work, confirm the ROADMAP.md milestone mapping before bumping versions."
    )

    print()
    print_section("Required Decisions", decisions)


if __name__ == "__main__":
    main()

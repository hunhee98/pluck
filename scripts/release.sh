#!/usr/bin/env bash
# Release pluck to crates.io.
#
# Usage:
#   scripts/release.sh             # dry-run (verifies, doesn't publish)
#   scripts/release.sh --publish   # actually publish
#
# What it does:
#   1. Fails fast if the working tree is dirty.
#   2. Runs the full workspace test suite.
#   3. cargo publish (--dry-run by default) for each publishable crate, in
#      dependency order: pluck-core → pluck-mcp → pluck-cli.
#   4. pluck-bench is `publish = false` and stays local.
#
# crates.io requires path deps to also carry a `version =` entry, which
# the crate manifests already declare. If you bump the workspace
# version, every internal dep entry must be bumped in lock-step.

set -euo pipefail

DRY_RUN=true
if [ "${1:-}" = "--publish" ]; then
  DRY_RUN=false
fi

cd "$(git rev-parse --show-toplevel)"

echo "==> sanity: working tree clean?"
if ! git diff --quiet || ! git diff --cached --quiet; then
  echo "    working tree is dirty — commit or stash first." >&2
  exit 1
fi

echo "==> running tests"
cargo test --workspace --quiet

# Publish order matters: dependents go last, since crates.io needs the
# dependency to already be indexed when the dependent is published.
ORDER=(pluck-core pluck-mcp pluck-cli)

for crate in "${ORDER[@]}"; do
  echo "==> publish: $crate ($([ "$DRY_RUN" = true ] && echo DRY-RUN || echo LIVE))"
  if [ "$DRY_RUN" = true ]; then
    ( cd "crates/$crate" && cargo publish --dry-run )
  else
    ( cd "crates/$crate" && cargo publish )
    # crates.io indexing can lag a few seconds between dependents.
    if [ "$crate" != "${ORDER[$((${#ORDER[@]} - 1))]}" ]; then
      echo "    sleeping 20s so the next crate sees this one indexed..."
      sleep 20
    fi
  fi
done

echo
if [ "$DRY_RUN" = true ]; then
  echo "Dry run OK. Re-run with --publish to push to crates.io."
else
  echo "Published. Next: tag the commit, push tags, update the brew tap."
  echo "  git tag v\$(cargo metadata --no-deps --format-version 1 \
    | jq -r '.packages[] | select(.name == \"pluck-core\") | .version')"
  echo "  git push --tags"
fi

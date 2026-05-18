#!/usr/bin/env bash
# Release pluck to crates.io.
#
# Usage:
#   scripts/release.sh             # dry-run (verifies, doesn't publish)
#   scripts/release.sh --publish   # actually publish
#
# What it does:
#   1. Fails fast if the working tree is dirty.
#   2. Checks version metadata.
#   3. Runs the full workspace test suite.
#   4. Dry-run mode verifies pluck-core with `cargo publish --dry-run`
#      and package-lists dependent crates. Live mode publishes each
#      publishable crate in dependency order:
#      pluck-core → pluck-mcp → pluck-cli.
#   5. pluck-bench is `publish = false` and stays local.
#   6. The tag-push workflow builds release tarballs and creates the
#      GitHub Release. Always watch that workflow and verify assets before
#      calling the release complete.
#
# crates.io requires path deps to also carry a `version =` entry, which
# the crate manifests already declare. If you bump the workspace
# version, use `scripts/bump-version.py patch`, `minor`, or an exact
# version so every internal dep entry and Cargo.lock move in lock-step.

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

VERSION="$(python3 scripts/version-check.py --print-version)"
echo "==> sanity: version metadata"
if [ "$DRY_RUN" = true ]; then
  python3 scripts/version-check.py --allow-unreleased
else
  python3 scripts/version-check.py --tag "v${VERSION}"
fi

echo "==> running tests"
cargo test --workspace --quiet

# Publish order matters: dependents go last, since crates.io needs the
# dependency to already be indexed when the dependent is published.
ORDER=(pluck-core pluck-mcp pluck-cli)

for crate in "${ORDER[@]}"; do
  echo "==> publish: $crate ($([ "$DRY_RUN" = true ] && echo DRY-RUN || echo LIVE))"
  if [ "$DRY_RUN" = true ]; then
    if [ "$crate" = "pluck-core" ]; then
      ( cd "crates/$crate" && cargo publish --dry-run )
    else
      # `cargo publish --dry-run` checks registry resolution. Before the
      # parent version is actually published, dependent crates such as
      # pluck-mcp cannot resolve `pluck-core = <new version>` from crates.io.
      # Workspace tests above cover the local path dependency; this package
      # listing verifies the publish file set without pretending the parent
      # is already indexed.
      ( cd "crates/$crate" && cargo package --list >/dev/null )
      echo "    package file list OK; registry verification runs during --publish after parent indexing."
    fi
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
  echo "Dependent publish verification is intentionally deferred until the parent crate version exists on crates.io."
else
  version=$(cargo metadata --no-deps --format-version 1 \
    | jq -r '.packages[] | select(.name == "pluck-core") | .version')
  tag="v${version}"
  echo "Published. Next: tag the commit, push it, and verify the GitHub Release."
  echo "  git tag -a ${tag} -m \"${tag}\""
  echo "  git push origin main"
  echo "  git push origin ${tag}"
  echo "  gh run watch \$(gh run list --workflow Release --limit 1 --json databaseId --jq '.[0].databaseId') --exit-status"
  echo "  gh release view ${tag} --json url,assets,isDraft,isPrerelease"
  echo
  echo "Only call the release complete after the GitHub Release exists and all expected tarballs are attached."
fi

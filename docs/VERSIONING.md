# Versioning

pluck uses the roadmap as the release train. The rule is simple:
every PR must say what version lane it belongs to before it merges.

This exists because agent-facing retrieval tools accumulate behavior fast:
new formats, new MCP surfaces, ranking changes, and benchmark claims all
change what users think they installed. Version decisions cannot live only
in maintainer memory or an agent's chat context.

## Release lanes

| Lane | Use for | Cargo version bump |
|---|---|---|
| `patch` | Bug fixes, security updates, CI/release repairs, and documentation that clarifies shipped behavior. | In the patch release PR or maintenance branch. |
| `minor-train` | New languages, new formats, new MCP tools, ranking behavior, storage layers, benchmark surfaces, or install/adoption behavior. | In the release train once `main` moves to the next minor. |
| `release-now` | The PR that cuts a GitHub/crates.io release. | Required. |
| `no-release` | Internal refactors with no user-visible behavior, generated maintenance, or test-only changes. | None. |

New language or file-format support is never a patch release. It rides the
next minor train even if the implementation is small.

## Current train

- Latest shipped release: `v0.3.0`.
- `main` is now the `v0.4.0` train.
- Patch releases are first-class. Bug fixes that must ship without the
  v0.4.0 feature set should be cherry-picked to a `release/v0.3.x` branch
  and released as `v0.3.y`.

That distinction matters. A TSX parser bug fix on `main` belongs to
`v0.4.0` after Java/HTML support has landed there. The same fix can be a
`v0.3.1` patch only if it is cherry-picked onto a patch-only maintenance
branch.

## Patch releases

Patch releases are for already-shipped behavior. They should not wait for
the next minor train when users on the published version need the fix.

Use a patch release when all of these are true:

- The change fixes a bug, security issue, CI/release breakage, or shipped-doc
  mistake.
- The fix is safe to ship without new minor-train features.
- Users of the latest published release would benefit immediately.

Cut patches from the released line, not from a `main` branch that already
contains new minor features:

```bash
git fetch --tags origin
git checkout -b release/v0.3.x v0.3.0
git cherry-pick <fix-commit>
python3 scripts/bump-version.py patch
```

Then move the patch notes into `## [0.3.1] - YYYY-MM-DD`, verify, tag, and
push:

```bash
python3 scripts/version-check.py --tag v0.3.1
scripts/release.sh
git tag v0.3.1
git push origin release/v0.3.x v0.3.1
```

After `v0.4.0` ships, the same pattern becomes `release/v0.4.x` and
`v0.4.1`, `v0.4.2`, and so on.

## PR loop

Every PR should carry this release decision explicitly:

1. Pick a release lane before coding: `patch`, `minor-train`,
   `release-now`, or `no-release`.
2. Add a CHANGELOG entry under `[Unreleased]` unless the PR is truly
   `no-release`.
3. Update `ROADMAP.md` when the PR changes a user-visible capability,
   language/format coverage, benchmark surface, or release target.
4. If the PR bumps Cargo versions, bump the workspace version and every
   internal crate dependency in lock-step.
5. Before merge, confirm whether the change ships in the current minor
   train or needs a maintenance-branch patch release.

## Release PR checklist

Release PRs do the mechanical version work in one place:

- Move relevant `[Unreleased]` notes into `## [x.y.z] - YYYY-MM-DD`.
- Keep a fresh empty `[Unreleased]` section.
- Update the `[Unreleased]` compare link to start from the new tag.
- Verify `Cargo.toml`, internal dependency versions, and `Cargo.lock` all
  agree.
- Use `python3 scripts/bump-version.py patch`, `minor`, or an exact version
  instead of editing version strings by hand.
- Run `python3 scripts/version-check.py --tag vx.y.z`.
- Run `scripts/release.sh` for a dry run before pushing the tag.

Tag pushes trigger `.github/workflows/release.yml`, which reruns
`scripts/version-check.py` before tests, artifacts, GitHub Release creation,
and crates.io publishing.

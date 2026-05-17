# Versioning

pluck uses SemVer plus a roadmap milestone map. The rule is simple:
every PR must say what kind of version movement it implies before it merges.

This exists because agent-facing retrieval tools accumulate behavior fast:
new formats, new MCP surfaces, ranking changes, and benchmark claims all
change what users think they installed. Version decisions cannot live only
in maintainer memory or an agent's chat context.

## Roadmap map vs release decision

`ROADMAP.md` assigns planned work to milestone versions such as `v0.4.0`,
`v0.5.0`, and `v0.6.0`. Those labels are product planning buckets, not a
reason to mechanically bump the minor number for every PR.

Before changing versions, decide which case applies:

| Case | Version decision |
|---|---|
| Planned roadmap capability | Target the roadmap milestone that owns it. Do not bump Cargo for every PR; bump when the train starts or the release is cut. |
| Roadmap milestone now feels too broad or too narrow | Update `ROADMAP.md` first, then target the corrected milestone. |
| Bug or issue against already shipped behavior | Patch release candidate: `v0.x.y`. Backport to the shipped line if users need it before the next minor. |
| Bug in unreleased train-only behavior | Keep it in the active minor train. No patch release. |
| CI/release/security fix affecting published releases | Patch release candidate. |
| Internal refactor, test-only change, or unreleased-doc cleanup | `no-release` unless it changes the user-facing contract. |

So yes: roadmap work often moves through `0.4`, `0.5`, `0.6` because the
roadmap already named those minor milestones. Separately filed bugs, support
issues, dependency/security fixes, or corrections to shipped behavior should
use patch numbers when patch is the right SemVer shape.

## Release lanes

| Lane | Use for | Cargo version bump |
|---|---|---|
| `patch` | Bug fixes, security updates, CI/release repairs, and documentation that clarifies shipped behavior. | In the patch release PR or maintenance branch. |
| `minor-train` | New languages, new formats, new MCP tools, ranking behavior, storage layers, benchmark surfaces, or install/adoption behavior. | In the release train once `main` moves to the next minor. |
| `release-now` | The PR that cuts a GitHub/crates.io release. | Required. |
| `no-release` | Internal refactors with no user-visible behavior, generated maintenance, or test-only changes. | None. |

New language or file-format support is normally a minor milestone because it is
new user-visible capability. If the roadmap mapping is stale, change the
roadmap mapping rather than pretending the capability is a patch.

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

1. Check whether the work is already mapped in `ROADMAP.md`.
2. If it is mapped, confirm the mapped milestone still makes sense.
3. If it is not mapped, classify it on its own merits: shipped bug,
   security/dependency/CI fix, new capability, internal-only, or release cut.
4. Pick a release lane before coding: `patch`, `minor-train`,
   `release-now`, or `no-release`.
5. Add a CHANGELOG entry under `[Unreleased]` unless the PR is truly
   `no-release`.
6. Update `ROADMAP.md` when the PR changes a user-visible capability,
   language/format coverage, benchmark surface, or release target.
7. If the PR bumps Cargo versions, bump the workspace version and every
   internal crate dependency in lock-step.
8. Before merge, confirm whether the change ships in the current minor
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

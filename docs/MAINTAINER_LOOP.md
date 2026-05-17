# Maintainer Loop

This is the operating loop for human maintainers and coding agents working on
pluck. It exists so a change does not stop at "merged" when it still needs a
version decision, release cut, crates.io publish, issue close-out, or roadmap
follow-up.

## Start-of-turn inventory

Run this before choosing the next action:

```bash
git status --short --branch
gh issue list --state open --limit 20
gh pr list --state open --limit 20
python3 scripts/maintainer-status.py
```

If `gh` is unavailable, continue with the local checks and say that remote
issue/PR state was not inspected.

## Classification

Every change gets two labels before implementation:

1. **Roadmap mapping**
   - Existing roadmap item.
   - Outside roadmap, classified directly by SemVer impact.
   - Roadmap mapping is wrong and must be updated first.
2. **Release lane**
   - `patch`: shipped bug, security/dependency fix, CI/release repair, or
     shipped-doc correction.
   - `minor-train`: planned user-visible capability or new behavior.
   - `release-now`: this PR cuts a GitHub/crates.io release.
   - `no-release`: internal-only/test-only maintenance.

If the change fixes behavior users can hit on the latest published tag, decide
whether it needs a maintenance-branch patch release before merging it only into
`main`.

## Implementation loop

1. Make the smallest coherent change.
2. Update `CHANGELOG.md` unless the change is truly `no-release`.
3. Update `ROADMAP.md` when the change affects a roadmap capability, release
   target, or public claim.
4. Run the local checks appropriate to the touched files.
5. Push a PR with the PR template filled out, including roadmap mapping, patch
   backport decision, and post-merge release action.
6. Watch CI to completion. Inspect logs on failure before guessing.

## Post-merge loop

After every merge to `main`, run:

```bash
python3 scripts/maintainer-status.py
```

Then make one explicit decision:

| Status | Required action |
|---|---|
| Patch needed for the latest published version | Create or update `release/v0.x.y`, cherry-pick the fix, run `scripts/bump-version.py patch`, update changelog, tag, and publish. |
| Active minor train should keep accumulating work | Say which roadmap item is next and why no release is cut yet. |
| Active minor train should ship now | Move `[Unreleased]` notes into `## [x.y.z] - YYYY-MM-DD`, run release checks, tag, and monitor release/crates.io. |
| No user-visible or release impact | Say no release action is needed and move to the next queued issue/roadmap item. |

Do not end a maintainer turn with release state implicit. The final response
must state the release lane and next release action, even when the answer is
"defer release".

## Release cut loop

For a patch or minor release:

```bash
python3 scripts/maintainer-status.py
python3 scripts/bump-version.py patch   # or minor / exact version
python3 scripts/version-check.py --allow-unreleased
```

Then update `CHANGELOG.md`:

- keep a fresh empty `[Unreleased]` section;
- move the released notes into `## [x.y.z] - YYYY-MM-DD`;
- update compare links at the bottom.

Verify and publish:

```bash
python3 scripts/version-check.py --tag vx.y.z
scripts/release.sh
git tag vx.y.z
git push origin <branch> vx.y.z
```

Watch `.github/workflows/release.yml` until it finishes. Confirm:

- GitHub Release exists and has binaries;
- crates.io has `pluck-core`, `pluck-mcp`, and `pluck-cli` at the tag version;
- README badges / install docs do not contradict the published version;
- `python3 scripts/maintainer-status.py` reflects the new latest tag after
  fetching tags.

## Stop conditions

Do not call the work done while any of these are unknown:

- CI status.
- Version lane.
- Roadmap mapping.
- Patch backport decision.
- Whether CHANGELOG/ROADMAP need updates.
- Whether a release/tag/crates.io publish is now due.
- Whether linked issues or PR comments need follow-up.

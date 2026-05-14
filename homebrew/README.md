# homebrew/

`pluck.rb` is the Homebrew formula for pluck. Two delivery modes:

## A. Personal tap (current — fastest path to ship)

Maintainers fork or maintain a tap repo named `homebrew-pluck`:

```
homebrew-pluck/
└── Formula/
    └── pluck.rb        # copy of this file
```

Users install with:

```bash
brew tap hunhee98/pluck
brew install pluck
```

The release flow (per version):

1. Tag the release in this repo (`git tag v0.2.0 && git push --tags`).
2. Download the source tarball and compute its sha256:
   ```bash
   curl -L -o /tmp/v0.2.0.tar.gz https://github.com/hunhee98/pluck/archive/refs/tags/v0.2.0.tar.gz
   shasum -a 256 /tmp/v0.2.0.tar.gz
   ```
3. In the tap repo, bump `url` and `sha256` in `Formula/pluck.rb` to the
   new version + digest. Commit. `brew install --HEAD` picks it up; tagged
   users get it on next `brew update`.

## B. homebrew-core submission (later, after stabilization)

Once we have a public adoption story and at least one tagged release that's
been out for a few weeks, we can submit `pluck.rb` to homebrew-core for
`brew install pluck` without a tap. Requirements:

- Stable, versioned URL
- Tests that pass without network access (the formula's `test do` block here
  already does this)
- No vendored binaries (we build from source via cargo, which is fine)
- License header and audit pass (`brew audit --new pluck`)

## What this formula does at install time

1. `cargo install` from `crates/pluck-cli` → `bin/pluck`
2. `cargo install` from `crates/pluck-mcp` → `bin/pluckd`
3. Confirms both binaries print their `--version`
4. End-to-end smoke test — indexes a one-file tempdir, searches for a
   symbol, confirms the hit comes back

Build deps: `rust` (build-time only), `ripgrep` (runtime — `pluck.grep` is
a passthrough to `rg`).

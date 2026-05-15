<!-- See CONTRIBUTING.md for the full checklist. This template only
asks for the parts that affect review speed. -->

## Summary

<!-- 1-3 sentences. What changes, and why this PR rather than a
different shape. -->

## Test plan

<!-- Bulleted list of what you ran. Be specific.

- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace -- -D warnings`
- [ ] Manual smoke: <command + expected vs actual>
-->

## Engine-core changes?

Did this PR touch anything under
`crates/pluck-core/src/{index,store,chunker,watcher,tokenizer,ranking}.rs`
or the MCP session-dedup map?

- [ ] No — skip the next block.
- [ ] Yes — pasted `python3 scripts/regression-gate.py` output below:

```
(paste gate output here, including the metric table)
```

## Perf claims (only if you added user-facing numbers)

Where does each new number cite back to? `baseline.json` row, file
under `benchmarks/results/`, or a fresh bench you're committing in
this PR?

- (cite each new claim)

## Breaking changes

- [ ] None.
- [ ] Yes — listed below with the migration path:

<!-- If anything in this list is unclear, link the related issue or
the README version-arc bullet so a reviewer can place this change
in context. -->

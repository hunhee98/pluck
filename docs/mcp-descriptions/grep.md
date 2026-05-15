Keyword / regex search across repo files. Wraps ripgrep flags.

## WHEN

Use `pluck.grep` whenever you would run `rg` or `grep -rn` inside the
repo: exact strings, regexes, TODOs, error messages, or all call-site
mentions of a known symbol.

## WHY

It preserves ripgrep behavior while keeping retrieval inside the pluck
daemon. The agent gets fast literal search without leaving the indexed
workflow or re-opening files unnecessarily.

## FALLBACK

Use bash when the target is outside the indexed repo, ripgrep is not
installed, you need shell pipes/redirection inline, or the daemon is
unreachable.

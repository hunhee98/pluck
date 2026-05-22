Read a repo file with a smart outline by default, or exact bytes with
`raw: true` / `lines: "A-B"`. Call from the main conversation;
sub-agent delegation skips this tool and falls back to `cat` / `Read`,
losing the outline-mode savings.

## WHEN

Use `pluck.read` whenever you would use `cat` or an agent Read tool on
a file inside the indexed repo. Use outline mode for large code files,
`raw: true` for byte-exact output, and `lines` for a focused range.

## WHY

Outline mode gives the agent the file's symbols, line ranges, and tiny
helper bodies without paying for every body. The original bytes stay
reachable, so defaulting to pluck costs fewer tokens without losing
capability.

## FALLBACK

Use bash only when the file is binary, larger than the tool size cap,
outside the repo, or the daemon is unreachable.

Read exactly one named function, method, class, struct, enum, impl, or
trait body.

## WHEN

Use `pluck.symbol` when you know the symbol name and need its body:
after `search`/`grep`, before editing a function, or when a bare name
is enough. Use `path/name` to disambiguate collisions.

## WHY

The response is the AST chunk the indexer already extracted, so the
agent avoids reading an entire file just to scroll to one definition.
Session dedup collapses repeated chunks to placeholders.

## FALLBACK

Use `pluck.search` or `pluck.grep` for inline callbacks or non-symbol
matches. Use bash only for files outside the repo or when the daemon is
unreachable.

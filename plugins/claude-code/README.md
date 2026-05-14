# pluck — Claude Code plugin

One-line install for Claude Code:

```bash
claude plugin add pluck
```

The plugin bundles:

1. **MCP server registration** — `pluckd` starts on demand over stdio.
2. **`CLAUDE.md` snippet injection** — teaches the agent when to prefer
   `pluck.*` over Bash `cat`/`grep`.
3. **Pre-allowed permissions** — `mcp__pluck__*` runs without per-call
   prompts.
4. **Background daemon control** — `pluckd` keeps the index warm; a file
   watcher reindexes on save.

## Manual install

If you prefer to wire it up yourself:

```bash
# 1. Install the binary
brew install pluck      # or: cargo install pluck

# 2. Register the MCP server
claude mcp add pluck pluckd -- --stdio

# 3. Add the snippet from CLAUDE.md.tmpl to your project's CLAUDE.md
```

## Uninstall

```bash
claude plugin remove pluck
```

Removes the MCP entry and the injected snippet. Leaves the binary in place
(remove via your package manager if you want it gone).

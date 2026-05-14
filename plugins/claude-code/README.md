# pluck — Claude Code plugin

Fast and token-friendly code reading for AI coding agents. Six MCP
tools that replace `cat` / `grep` with symbol-aware retrieval,
sub-millisecond warm search, and a `raw` fallback that preserves
cat/grep parity byte-for-byte.

## Install

Install the binary first (the plugin manifest references `pluckd`
on PATH):

```bash
cargo install --git https://github.com/hunhee98/pluck pluck-mcp
# binary lands at ~/.cargo/bin/pluckd
```

Then enable the plugin via Claude Code's plugin marketplace flow:

```text
/plugin marketplace add hunhee98/pluck
/plugin install pluck@hunhee98-pluck
```

Or, during local development, point Claude Code at this directory:

```bash
claude --plugin-dir /path/to/pluck/plugins/claude-code
```

## What the plugin contributes

| File | What it does |
|------|--------------|
| `.claude-plugin/plugin.json` | Plugin identity (name, version, homepage) |
| `.mcp.json` | Registers `pluckd` as an MCP server, scoped to `${REPO_ROOT}` |
| `settings.json` | Pre-allows `mcp__pluck__*` so the agent never gets prompted per tool call |
| `CLAUDE.md.tmpl` | Suggested CLAUDE.md snippet — add it to your project root or `~/.claude/CLAUDE.md` manually (auto-injection is not yet a Claude Code feature) |

## Verify

After install, run any prompt that would normally cat or grep:

```text
Find authentication-related code in this project.
```

Claude Code should call `mcp__pluck__search` (or `read` / `peek`) and
return ranked chunks. If it still shells out to `cat` / `rg`, check:

1. `pluckd --version` runs from your shell.
2. `claude mcp list` shows `pluck` as connected.
3. Your project's CLAUDE.md (or `~/.claude/CLAUDE.md`) includes the
   contents of `CLAUDE.md.tmpl` — without that snippet the agent has
   no policy hint to prefer pluck over Bash.

## Uninstall

```text
/plugin uninstall pluck
```

Removes the MCP entry and the permission allowlist. Leaves
`~/.cargo/bin/pluckd` in place; remove with `cargo uninstall pluck-mcp`
if you want it gone.

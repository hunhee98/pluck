# Agent Install Prompt

Use this when the user has an AI coding agent that pluck does not know by name
yet, or when the agent has its own MCP / tool-permission format. The prompt
asks the agent to install pluck, register the MCP server, and then use the
strongest available pluck-first enforcement layer.

## Recommended All-in-One Prompt

Copy the whole block. It installs/registers pluck and, in the same prompt,
asks the agent to apply the strongest pluck-first retrieval layer it officially
supports.

```text
Install and configure pluck for this repo.

Install / register:
- Find the repository root.
- Check whether `pluck`, `pluckd`, `cargo`, and `brew` are available.
- If pluck is missing and Rust/Cargo is available, install:
  `cargo install pluck-mcp pluck-cli`
- If Homebrew is the standard package manager here, prefer:
  `brew tap hunhee98/pluck && brew install pluck`
- Register pluckd as an MCP server named `pluck` for this repo.
- Prefer project-scoped configuration when the agent supports it.
- Verify with `pluck --version`, `pluckd --version`, and the agent's MCP
  server/tool listing command if one exists.

To make pluck the first retrieval choice:
- Use the strongest official mechanism this agent supports.
- Claude Code:
  - Use project MCP config or:
    `claude mcp add --transport stdio --scope project pluck -- <absolute-path-to-pluckd> --repo <repo-root>`
  - Approve the `pluck` MCP server through settings/permissions if required.
    In Claude Code permissions, approve the MCP server name (`mcp__pluck`);
    do not rely on wildcard MCP permission patterns.
  - Add durable project instructions in `CLAUDE.md`.
  - Use PreToolUse hooks or permission deny rules only when they can safely
    redirect repo-local cat/grep/rg/read retrieval to mcp__pluck__* tools
    without blocking legitimate fallbacks.
- Codex:
  - Use:
    `codex mcp add pluck -- <absolute-path-to-pluckd> --repo <repo-root>`
    or add `[mcp_servers.pluck]` to `~/.codex/config.toml` / trusted project
    `.codex/config.toml`.
  - Add the pluck-first retrieval policy to `AGENTS.md`, because Codex uses
    AGENTS.md for persistent project instructions.
- Cursor:
  - Create project `.cursor/mcp.json` for the `pluck` stdio server, using
    `${workspaceFolder}` when helpful.
  - Add an always-applied Project Rule in `.cursor/rules/` or use `AGENTS.md`
    as the workspace instruction layer.
  - If using `cursor-agent`, verify with `cursor-agent mcp list` and
    `cursor-agent mcp list-tools pluck`.
- Other MCP-capable agents:
  - Inspect the agent's official MCP config, permission, hook, and project-rule
    mechanisms.
  - Apply the strongest available combination: MCP registration first, then
    permissions/allowlists/hooks when safe, then durable project instructions.

Pluck-first retrieval policy:
- Use `mcp__pluck__read` before `cat`, `head`, `tail`, `sed -n`, or built-in
  file reads for files inside the indexed repo. Outline mode is the default;
  use `raw: true` only when byte-exact output is required.
- Use `mcp__pluck__search` for conceptual lookup when you do not know the
  exact identifier or path.
- Use `mcp__pluck__grep` before `grep` / `rg` for exact strings, regexes,
  TODOs, or all textual matches inside the repo.
- Use `mcp__pluck__peek` when you need a symbol's interface and direct callees
  without paying for the body.
- Use `mcp__pluck__symbol` when you know the symbol and need the body.
- Use `mcp__pluck__expand` for local call chains, `mcp__pluck__impact` before
  refactors, and `mcp__pluck__deps` for import relationships.
- Use `mcp__pluck__digest` for long cargo, npm, pytest, or GitHub Actions logs
  before pasting raw output into context.
- Fall back to Bash or built-in reads only for binary files, paths outside the
  repo, byte-exact shell pipelines, unsupported formats, or when pluck is
  unavailable.

Safety:
- Preserve existing config entries and comments where the config format allows.
- Do not delete unrelated MCP servers, rules, hooks, or user instructions.
- Do not print secrets from config files.
- Ask before destructive changes.
- Show the files changed and the exact verification result.

Verify:
- Restart or reload the agent if required by its MCP implementation.
- Confirm the MCP server named `pluck` is connected.
- Run a repository retrieval check that should use pluck first, for example:
  "Find the auth token validation flow in this repo."
- Confirm the agent calls `mcp__pluck__search`, `mcp__pluck__read`,
  `mcp__pluck__peek`, or another `mcp__pluck__*` tool before falling back to
  Bash or built-in file reads.
```

Official mechanisms checked:
- Claude Code MCP, settings, permissions, and hooks:
  <https://code.claude.com/docs/en/mcp>,
  <https://code.claude.com/docs/en/settings>,
  <https://code.claude.com/docs/en/hooks>
- Codex MCP configuration and AGENTS.md behavior:
  <https://platform.openai.com/docs/docs-mcp>,
  <https://github.com/openai/codex/blob/main/docs/config.md>,
  <https://github.com/openai/codex/blob/main/docs/agents_md.md>
- Cursor MCP configuration, Project Rules, and cursor-agent MCP checks:
  <https://docs.cursor.com/advanced/model-context-protocol>,
  <https://docs.cursor.com/en/context>,
  <https://docs.cursor.com/cli/mcp>

## Enforcement Ladder

Pluck should be installed with the strongest layer the agent environment can
actually enforce:

1. **Hard gate**: tool permissions, MCP allowlists, pre-tool hooks, command
   blockers, or deny rules that prevent repo-local `cat` / `grep` / `rg`
   retrieval and route the agent to `mcp__pluck__*`.
2. **MCP registration**: a `pluck` MCP server launched as
   `pluckd --repo <repo-root>`.
3. **Project policy**: an always-on project rule such as `AGENTS.md`,
   `CLAUDE.md`, `.cursor/rules/*.mdc`, or the agent's equivalent.
4. **Manual fallback**: if the agent cannot safely edit its own config, it
   should print the exact MCP block and policy text for the user to paste.

Known behavior today:

| Agent | Strongest pluck layer |
|-------|-----------------------|
| Claude Code | `pluck init --target claude --mode aggressive` registers MCP, approves the `pluck` MCP server, installs project rules, and adds a PreToolUse hook that denies Bash retrieval commands. |
| Codex | `pluck init --target codex --mode strong` registers MCP in the Codex config and writes `AGENTS.md` pluck-first policy. |
| Cursor | `pluck init --target cursor --mode strong` registers MCP and writes an always-apply Cursor rule plus `AGENTS.md`. |
| Other MCP agents | Use the copy-paste prompt above so the agent discovers its own config format and applies the strongest supported MCP / permission / rule layer. |

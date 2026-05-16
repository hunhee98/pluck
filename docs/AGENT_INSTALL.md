# Agent Install Prompt

Use this when the user has an AI coding agent that pluck does not know by name
yet, or when the agent has its own MCP / tool-permission format. The prompt
asks the agent to install pluck, register the MCP server, and use the strongest
available pluck-first enforcement layer.

## Recommended Short Prompt

Copy this first. It is intentionally short so users can paste it into any
coding agent without thinking about that agent's config format.

```text
Install and configure pluck for this repo. Make pluck the default code
retrieval layer: prefer mcp__pluck__read, mcp__pluck__search, mcp__pluck__grep,
and the other mcp__pluck__* tools before cat, grep, rg, or built-in file reads.
If pluck is missing, install pluck-mcp and pluck-cli. Register pluckd as an MCP
server for this repo. Use the strongest setup your environment supports: MCP
allowlist, tool permissions, hooks/command blockers, or project rules. Verify
that a repo code search uses mcp__pluck__* before finishing.
```

## Full Prompt

Use this stricter version when the agent needs more explicit safety checks,
fallback instructions, or enforcement details.

```text
You are configuring this coding environment to use pluck as the default
repository code retrieval layer.

Goal:
- For every code read/search inside this repository where pluck can help, use
  pluck before built-in file reads, terminal cat/head/tail/sed -n, grep, or rg.
- Keep Bash / built-in reads only as fallback for binary files, paths outside
  the repository, byte-exact shell pipelines, unsupported formats, or when the
  pluck daemon is unavailable.

Install:
1. Check whether `pluck`, `pluckd`, and `cargo` are already available.
2. If pluck is missing and Rust/Cargo is available, install:
   `cargo install pluck-mcp pluck-cli`
3. If Homebrew is the standard package manager in this environment, prefer:
   `brew tap hunhee98/pluck && brew install pluck`
4. Verify with `pluck --version` and `pluckd --version`.

Register MCP:
1. Find the repository root.
2. If this is Claude Code, run:
   `pluck init --target claude --mode aggressive --repo <repo-root>`
3. If this is Codex, run:
   `pluck init --target codex --mode strong --repo <repo-root>`
4. If this is Cursor, run:
   `pluck init --target cursor --mode strong --repo <repo-root>`
5. If this is another MCP-capable agent, inspect its local MCP configuration
   format and register a server named `pluck` with:
   command: `<absolute-path-to-pluckd>`
   args: `["--repo", "<absolute-repo-root>"]`

Enforce pluck-first behavior:
1. Use the strongest enforcement mechanism this agent supports.
2. If the agent supports tool permissions or allowlists, allow all
   `mcp__pluck__*` tools without repeated prompts.
3. If the agent supports hard hooks, tool-deny rules, or command blockers,
   block terminal code-retrieval commands (`cat`, `head`, `tail`, `sed -n`,
   `grep`, `rg`) inside the indexed repo and redirect to pluck tools.
4. If hard enforcement is not available, add or update the strongest project
   instruction file this agent obeys (`AGENTS.md`, `CLAUDE.md`, Cursor rules,
   or the environment's equivalent) with this policy:

   Use pluck before Bash for repository code retrieval.
   - Use `mcp__pluck__read` instead of `cat`, built-in Read, `head`, `tail`,
     or `sed -n` for files inside the indexed repo. Outline mode is the
     default; use `raw: true` only when byte-exact output is required.
   - Use `mcp__pluck__search` for conceptual lookup when you do not know the
     exact identifier or path.
   - Use `mcp__pluck__grep` instead of `grep` / `rg` for exact strings,
     regexes, TODOs, or all textual matches inside the repo.
   - Use `mcp__pluck__peek` when you need a symbol's interface and direct
     callees without paying for the body.
   - Use `mcp__pluck__symbol` when you know the symbol and need the body.
   - Use `mcp__pluck__expand` for local call chains,
     `mcp__pluck__impact` before refactors, and `mcp__pluck__deps` for import
     relationships.
   - Use `mcp__pluck__digest` for long cargo, npm, pytest, or GitHub Actions
     logs before pasting raw output into context.

Safety:
- Preserve existing config entries and comments where the config format allows.
- Do not delete unrelated MCP servers, rules, hooks, or user instructions.
- Do not print secrets from config files.
- Ask before destructive changes.
- Show the files changed and the exact verification result.

Verify:
1. Restart or reload the agent if required by its MCP implementation.
2. Confirm the MCP server named `pluck` is connected.
3. Run a repository retrieval check that should use pluck first, for example:
   "Find the auth token validation flow in this repo."
4. Confirm the agent calls `mcp__pluck__search`, `mcp__pluck__read`,
   `mcp__pluck__peek`, or another `mcp__pluck__*` tool before falling back to
   Bash or built-in file reads.
```

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
| Claude Code | `pluck init --target claude --mode aggressive` registers MCP, allows `mcp__pluck__*`, installs project rules, and adds a PreToolUse hook that denies Bash retrieval commands. |
| Codex | `pluck init --target codex --mode strong` registers MCP in the Codex config and writes `AGENTS.md` pluck-first policy. |
| Cursor | `pluck init --target cursor --mode strong` registers MCP and writes an always-apply Cursor rule plus `AGENTS.md`. |
| Other MCP agents | Use the copy-paste prompt above so the agent discovers its own config format and applies the strongest supported MCP / permission / rule layer. |

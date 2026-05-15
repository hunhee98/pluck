Exploration recommender: "given this task, what should I retrieve next?"

## WHEN to call

Call `pluck.plan` at the **start of an unfamiliar task** — a new repo,
a vague ticket, a bug report that names a feature but not a file. It
probes the index with the task description and returns 3-5 concrete
next-call recommendations (which tool, on what target, why) plus a
confidence indicator.

- New repo, no mental model yet → `plan "explain how authentication
  works"` returns the entry points to read first.
- JIRA-style task → `plan "fix the bug where session tokens expire
  too early"` surfaces the function to inspect and adds an `impact`
  step so the agent sees callers before changing the contract.
- After a string of dead-end greps, the confidence indicator tells
  the agent to stop trying to guess identifiers and broaden the
  search.

## WHY it saves tokens

Without `plan`, the agent runs an exploratory loop of `search` →
`read` → `search` again, often re-reading the same chunks before it
finds the lead. `plan` collapses that loop to one call: top probe
hits, the right tool for each, and an honest confidence signal. The
agent skips the speculative reads.

## Parameters

- `task` — free-form English description of what you're trying to
  do. The more concrete (file names, error messages, identifiers),
  the higher the confidence.
- `top_k` — maximum number of next-call recommendations. Clamped to
  [1, 5]. Default 4.

## Output shape

```
=== plan: "fix the bug where auth tokens expire too early" — confidence: high ===

Top probe results:
  0.84  src/auth/session.ts:L20-50  validateSession (Function)
  0.71  src/auth/token.ts:L8-30     refreshToken (Function)
  0.58  src/auth/expiry.ts:L1-15    EXPIRY_SECONDS (Module)

Recommended next calls:
  1. pluck.symbol validateSession
     → small function — full body fits within a tight token budget
  2. pluck.impact validateSession
     → top probe hit — see who depends on this before changing the contract
  3. pluck.peek refreshToken
     → large function — peek returns signature + direct callees without paying for the body
  4. pluck.read src/auth/expiry.ts
     → module-level chunk — outline the file to see every symbol
```

When the score distribution is flat (no clear lead), `plan` returns
`confidence: low` and a broaden hint:

```
=== plan: "improve performance" — confidence: low ===

Top probe results:
  0.12  src/foo.rs:L1-100  foo
  ...

Score distribution is flat — none of these is a clear lead. Broaden
with `pluck.grep` on a concrete identifier or rephrase the task with
more specific terms.
```

## How recommendations are chosen

- **`read`** when 2+ chunks from the same file rank in the top results
  (outline once, share the context).
- **`peek`** for functions / methods longer than 40 lines (signature +
  callees without paying for the body).
- **`symbol`** for everything smaller (small function, struct, enum,
  class, impl, trait — read in one shot).
- **`impact`** is added as a follow-up step for the top hit when it's
  a function or method — caller blast-radius before any contract
  change.

## FALLBACK to bash when

- The task is purely textual (a literal error message, a string
  constant) — `pluck.grep` finds it directly without the planner.
- The target is outside the indexed repo (cross-repo work).
- The daemon is unreachable — fall back to `rg -l <keyword>` for the
  smallest version of what plan would do.

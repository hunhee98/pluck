Compress verbose build, test, install, and CI logs.

## WHEN

Use `pluck.digest` on raw output from `cargo`, npm/pnpm/yarn/bun,
pytest, or GitHub Actions logs. Pass `format` only when auto-detection
is wrong.

## WHY

It collapses progress noise such as hundreds of compile or install
lines while preserving errors, failures, panics, tracebacks,
file:line:col positions, and failed-step bodies. The agent sees signal
instead of log bulk.

## FALLBACK

Do not call it for already short output, unsupported formats, or when
byte-exact log output is required. Use bash/plain output when the daemon
is unreachable.

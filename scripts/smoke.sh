#!/usr/bin/env bash
#
# scripts/smoke.sh — install verification.
#
# Runs end-to-end against the locally installed binaries and proves
# that:
#   1. `pluck` and `pluckd` are on PATH and respond to --version
#   2. `pluck index` writes an index for a tiny fixture repo
#   3. `pluck search` returns a non-empty result for a query that
#      definitely matches the fixture
#   4. `pluck read` returns the outline for a fixture file
#   5. `pluck grep` returns the line the pattern definitely matches
#
# Exit codes:
#   0  every check passed
#   1  one or more checks failed
#   2  fatal setup error (no temp dir, no binaries to test, etc.)
#
# Usage:
#   scripts/smoke.sh                 # binaries on $PATH (post `cargo install`)
#   scripts/smoke.sh --release       # use ./target/release/{pluck,pluckd}
#   scripts/smoke.sh --bin-dir DIR   # use DIR/{pluck,pluckd}

set -uo pipefail

BIN_DIR=""
USE_RELEASE=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --release)
      USE_RELEASE=1
      shift
      ;;
    --bin-dir)
      BIN_DIR="$2"
      shift 2
      ;;
    -h|--help)
      sed -n '/^# scripts/,/^$/p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "unknown flag: $1" >&2
      exit 2
      ;;
  esac
done

if [[ "$USE_RELEASE" -eq 1 ]]; then
  REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
  BIN_DIR="${REPO_ROOT}/target/release"
fi

if [[ -n "$BIN_DIR" ]]; then
  PLUCK="${BIN_DIR}/pluck"
  PLUCKD="${BIN_DIR}/pluckd"
else
  PLUCK="$(command -v pluck || true)"
  PLUCKD="$(command -v pluckd || true)"
fi

if [[ -z "$PLUCK" || ! -x "$PLUCK" ]]; then
  echo "fatal: pluck binary not found. Try: cargo install pluck-cli" >&2
  exit 2
fi
if [[ -z "$PLUCKD" || ! -x "$PLUCKD" ]]; then
  echo "fatal: pluckd binary not found. Try: cargo install pluck-mcp" >&2
  exit 2
fi

# ── set up fixture repo ────────────────────────────────────────────────────

TMPDIR_BASE="${TMPDIR:-/tmp}"
WORK="$(mktemp -d "${TMPDIR_BASE%/}/pluck-smoke.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

mkdir -p "$WORK/src"
cat > "$WORK/src/auth.rs" <<'EOF'
/// Validate the bearer token attached to an incoming request.
pub fn validate_bearer(token: &str) -> bool {
    !token.is_empty() && token.starts_with("Bearer ")
}

/// Refresh an access token using a stored refresh token.
pub fn refresh_access_token(refresh: &str) -> Result<String, &'static str> {
    if refresh.is_empty() {
        return Err("empty refresh token");
    }
    Ok(format!("new-access-from-{refresh}"))
}
EOF
cat > "$WORK/src/lib.rs" <<'EOF'
pub mod auth;
EOF

# ── run checks ─────────────────────────────────────────────────────────────

PASS=0
FAIL=0
NOTES=()

check() {
  local label="$1"; shift
  if "$@" >/dev/null 2>&1; then
    PASS=$((PASS + 1))
    echo "  PASS  $label"
  else
    FAIL=$((FAIL + 1))
    echo "  FAIL  $label"
    NOTES+=("$label — re-run: $*")
  fi
}

check_contains() {
  local label="$1"; local needle="$2"; shift 2
  local out
  if ! out="$("$@" 2>&1)"; then
    FAIL=$((FAIL + 1))
    echo "  FAIL  $label (command failed: $*)"
    NOTES+=("$label — command failed")
    return
  fi
  if [[ "$out" == *"$needle"* ]]; then
    PASS=$((PASS + 1))
    echo "  PASS  $label"
  else
    FAIL=$((FAIL + 1))
    echo "  FAIL  $label (output missing \"$needle\")"
    NOTES+=("$label — expected \"$needle\" in output, got:" "$out")
  fi
}

echo "smoke: using"
echo "  pluck:  $PLUCK"
echo "  pluckd: $PLUCKD"
echo "  fixture: $WORK"
echo

# --version output starts with the package name; clap uses CARGO_PKG_NAME,
# which is `pluck-cli` / `pluck-mcp` rather than the binary name. Match on
# the project prefix so the check tolerates either.
check_contains "pluck --version responds"  "pluck" "$PLUCK"  --version
check_contains "pluckd --version responds" "pluck" "$PLUCKD" --version

check "pluck index <fixture> succeeds" "$PLUCK" index "$WORK"
check_contains "pluck search 'validate bearer token' finds auth.rs" \
  "auth.rs" "$PLUCK" search --repo "$WORK" "validate bearer token"
check_contains "pluck read shows validate_bearer outline" \
  "validate_bearer" "$PLUCK" read "$WORK/src/auth.rs"
check_contains "pluck grep finds refresh_access_token" \
  "refresh_access_token" "$PLUCK" grep "refresh_access_token" "$WORK/src/auth.rs"

# ── report ─────────────────────────────────────────────────────────────────

echo
echo "smoke: $PASS passed, $FAIL failed"

if [[ "$FAIL" -gt 0 ]]; then
  echo
  echo "details:"
  for n in "${NOTES[@]}"; do
    echo "  $n"
  done
  exit 1
fi
exit 0

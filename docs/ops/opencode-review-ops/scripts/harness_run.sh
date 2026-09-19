#!/bin/sh
# harness_run.sh - one script serving BOTH node ops of harness_runner:
#   dag.ops harness_run_quick -> command ".../harness_run.sh quick"
#   dag.ops harness_run_full  -> command ".../harness_run.sh full"
# The registered command string is whitespace-split into argv (no shell),
# so the mode arrives as $1 exactly as written in the config.
#
# Contract (see ../README.md):
#   - streams NDJSON stage lines to stdout, one JSON object per line:
#       {"stage": "build", "status": "ok"}
#       {"stage": "unit", "status": "fail", "detail": "..."}
#     Unparsable lines around them are ignored, so short human progress
#     lines are fine. A missing/unknown "status" counts as "fail".
#   - the verdict is computed by the MODULE, not the script, but it must
#     agree: exit 0 only when every stage ran "ok", 1 otherwise.
#   - stdout+stderr are captured together, capped at 256 KiB total:
#     stream stage lines and one-line summaries, never compiler logs.
#
# The checks below are PLACEHOLDERS (example-shaped, always-ok stubs).
# Swap each TODO(real check) for the real gatekeeping command.

set -eu

# env_clear() drops PATH; scripts must self-locate their tools.
PATH="/usr/local/cargo/bin:/usr/local/bin:/usr/bin:/bin"
export PATH

MODE="${1:-full}"
WORKSPACE="${REVIEW_WORKSPACE:-/data00/github/opencoder}"

# Self-test hook: demonstrate the FAILING stage shape for docs/debugging.
# Run by hand: ./harness_run.sh selftest-fail   (prints one fail line, exit 1)
if [ "$MODE" = "selftest-fail" ]; then
  printf '%s\n' '{"stage": "unit", "status": "fail", "detail": "selftest: forced failure (cargo test exited 1: tests/session.rs:88)"}'
  exit 1
fi

case "$MODE" in
  quick|full) ;;
  *)
    echo "harness_run.sh: unknown mode '$MODE' (expected quick|full)" >&2
    exit 2
    ;;
esac

echo "[harness_run] mode=$MODE workspace=$WORKSPACE token=${REVIEW_GITHUB_TOKEN:+set}"

FAILED=0

emit() {
  # emit <name> <ok|fail> [detail]; detail must avoid " and \ (no JSON
  # escaping in this example).
  if [ "$#" -ge 3 ]; then
    printf '{"stage": "%s", "status": "%s", "detail": "%s"}\n' "$1" "$2" "$3"
  else
    printf '{"stage": "%s", "status": "%s"}\n' "$1" "$2"
  fi
}

run_stage() {
  # run_stage <name> <check-command...>: runs the check quietly, emits
  # the stage line, and bumps FAILED on a non-zero exit.
  name="$1"; shift
  if "$@" >/dev/null 2>&1; then
    emit "$name" ok
  else
    code=$?
    emit "$name" fail "$* exited $code (see CI log for details)"
    FAILED=$((FAILED + 1))
  fi
}

# ---- placeholder checks: TODO(real check) swap-in points -----------------

stub_true() {
  # TODO(real check): replace with the real gate, e.g. for fmt:
  #   cargo fmt --check
  return 0
}

# Stages for both modes. TODO(real check) commands (examples):
#   build:  cargo build --locked
#   fmt:    cargo fmt --check
#   unit:   cargo test --lib --tests
#   clippy: cargo clippy --all-targets -- -D warnings
run_stage build  stub_true
run_stage fmt    stub_true
run_stage unit   stub_true
run_stage clippy stub_true

# Full mode adds the slow end-to-end suite; quick mode skips it.
if [ "$MODE" = "full" ]; then
  # TODO(real check): cargo test --test operator_e2e --test dag_e2e ...
  run_stage e2e stub_true
fi

echo "[harness_run] mode=$MODE failed_stages=$FAILED"
[ "$FAILED" -eq 0 ]

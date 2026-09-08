#!/usr/bin/env bash
# Poll every GitHub Actions workflow in the current repository and fail when a
# run changes to a non-success conclusion.
#
# The first poll establishes a baseline. Existing failures are not reported;
# only failures observed after the watcher starts fail the process.
#
# Environment:
#   GH_CI_INTERVAL       Poll interval in seconds (default: 60)
#   GH_CI_LIMIT          Number of recent runs to inspect (default: 100)
#   GH_CI_STATE_FILE     Snapshot path (default: .tmp/gh/ci-runs.json)
#   GH_BIN               gh executable (default: gh)
#
# Pass --once to perform one poll. This is useful for deterministic checks.
set -euo pipefail

INTERVAL="${GH_CI_INTERVAL:-60}"
LIMIT="${GH_CI_LIMIT:-100}"
STATE_FILE="${GH_CI_STATE_FILE:-.tmp/gh/ci-runs.json}"
GH_BIN="${GH_BIN:-gh}"
ONCE=false

if [[ "${1:-}" == "--once" ]]; then
  ONCE=true
elif [[ $# -ne 0 ]]; then
  printf 'usage: %s [--once]\n' "$0" >&2
  exit 2
fi

STATE_DIR="$(dirname "$STATE_FILE")"
mkdir -p "$STATE_DIR"

snapshot() {
  "$GH_BIN" run list \
    --limit "$LIMIT" \
    --json databaseId,status,conclusion,headBranch,workflowName,displayTitle,url \
    | jq -S 'sort_by(.databaseId)'
}

poll() {
  local next failures
  next="$(mktemp "${STATE_FILE}.XXXXXX")"
  if ! snapshot > "$next"; then
    rm -f "$next"
    return 1
  fi

  if [[ ! -f "$STATE_FILE" ]]; then
    mv "$next" "$STATE_FILE"
    printf 'GitHub CI baseline established (%s runs)\n' "$(jq 'length' "$STATE_FILE")"
    return 0
  fi

  failures="$(jq -r --slurpfile previous "$STATE_FILE" '
    ($previous[0] // []) as $old
    | [
        .[] as $run
        | select($run.status == "completed")
        | ([$old[] | select(.databaseId == $run.databaseId)] | first) as $before
        | select($before == null
            or $before.status != "completed"
            or $before.conclusion != $run.conclusion)
        | select($run.conclusion != "success")
        | $run
      ]
    | .[]
    | "\(.workflowName) | \(.conclusion) | \(.headBranch) | \(.displayTitle) | \(.url // "")"
  ' "$next")"

  mv "$next" "$STATE_FILE"

  if [[ -n "$failures" ]]; then
    printf 'GitHub CI failed:\n%s\n' "$failures" >&2
    return 1
  fi

  return 0
}

while true; do
  poll
  "$ONCE" && exit 0
  sleep "$INTERVAL"
done

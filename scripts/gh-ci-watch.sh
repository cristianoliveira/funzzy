#!/usr/bin/env bash
# Poll GitHub Actions for main and every open pull request in the current
# repository. Fail when the latest run for a workflow and branch has a
# non-success conclusion.
#
# Environment:
#   GH_CI_INTERVAL       Poll interval in seconds (default: 60)
#   GH_CI_LIMIT          Number of runs per branch to inspect (default: 100)
#   GH_CI_PR_LIMIT       Number of open pull requests to inspect (default: 100)
#   GH_CI_STATE_FILE     Snapshot path (default: .tmp/gh/open-prs-main-runs.json)
#   GH_BIN               gh executable (default: gh)
#
# Pass --once to perform one poll. This is useful for deterministic checks.
set -euo pipefail

INTERVAL="${GH_CI_INTERVAL:-60}"
LIMIT="${GH_CI_LIMIT:-100}"
PR_LIMIT="${GH_CI_PR_LIMIT:-100}"
STATE_FILE="${GH_CI_STATE_FILE:-.tmp/gh/open-prs-main-runs.json}"
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

branches() {
  {
    printf '%s\n' main
    "$GH_BIN" pr list \
      --state open \
      --limit "$PR_LIMIT" \
      --json headRefName \
      --jq '.[].headRefName'
  } | LC_ALL=C sort -u
}

snapshot() {
  while IFS= read -r branch; do
    "$GH_BIN" run list \
      --branch "$branch" \
      --limit "$LIMIT" \
      --json databaseId,status,conclusion,headBranch,workflowName,displayTitle,url
  done < <(branches) \
    | jq -s -S 'add
      | sort_by(.databaseId)
      | group_by([.workflowName, .headBranch])
      | map(last)'
}

poll() {
  local next failures
  next="$(mktemp "${STATE_FILE}.XXXXXX")"
  if ! snapshot > "$next"; then
    rm -f "$next"
    return 1
  fi

  reported="$(jq -c '(.reported // [])' "$STATE_FILE" 2>/dev/null || printf '[]')"
  failures_json="$(jq -c --argjson reported "$reported" '
    [
      .[]
      | select(.status == "completed" and .conclusion != "success")
      | select((.databaseId | tostring) as $id | ($reported | index($id)) == null)
    ]
  ' "$next")"
  failures="$(jq -r '.[] | "\(.workflowName) | \(.conclusion) | \(.headBranch) | \(.displayTitle) | \(.url // "")"' <<< "$failures_json")"
  reported="$(jq -c --argjson reported "$reported" --argjson failures "$failures_json" '
    ($reported + ($failures | map(.databaseId | tostring))) | unique
  ' <<< '{}')"

  jq -n --slurpfile runs "$next" --argjson reported "$reported" \
    '{runs: $runs[0], reported: $reported}' > "$next.state"
  mv "$next.state" "$STATE_FILE"
  rm -f "$next"

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

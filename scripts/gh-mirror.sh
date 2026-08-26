#!/bin/sh
# GitHub Actions bridge for fzz: polls `gh run list` and mirrors the
# state into a local file that fzz jobs can watch.
#
# Contract (proven 26-08-26, .tmp/reports/26-08-26/gh-actions-watch-design-a.md):
# - state file is rewritten ONLY on change (cmp guard): unchanged polls
#   never fire watchers
# - publish is an atomic mv: watchers never read a partial file
# - the service is stateless: restart-by-reinclusion (TASK-0133 model)
#   loses nothing
#
# Env:
#   OUT        mirror path (default .tmp/gh/runs.json)
#   INTERVAL   poll seconds (default 60)
#   GH_BRANCH  fixed branch filter; unset = current branch, none = all
set -u
OUT="${OUT:-.tmp/gh/runs.json}"
INTERVAL="${INTERVAL:-60}"
LOG="$(dirname "$OUT")/mirror.log"

mkdir -p "$(dirname "$OUT")"
log() { echo "$(date +%H:%M:%S) $1" >> "$LOG"; }
log "mirror started (out=$OUT interval=${INTERVAL}s branch=${GH_BRANCH:-auto})"

poll() {
  BRANCH="${GH_BRANCH:-$(git branch --show-current 2>/dev/null || true)}"
  if [ -n "$BRANCH" ]; then
    gh run list --branch "$BRANCH" --limit 20 \
      --json databaseId,status,conclusion,headBranch,workflowName,displayTitle
  else
    gh run list --limit 20 \
      --json databaseId,status,conclusion,headBranch,workflowName,displayTitle
  fi | jq -S 'sort_by(.databaseId)'
}

while true; do
  if poll > "$OUT.new" 2>>"$LOG"; then
    if cmp -s "$OUT.new" "$OUT"; then
      rm -f "$OUT.new"
    else
      mv "$OUT.new" "$OUT"
      log "state changed, published"
    fi
  else
    log "gh failed, keeping previous state"
  fi
  sleep "$INTERVAL"
done

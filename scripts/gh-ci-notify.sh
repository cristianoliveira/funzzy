#!/bin/sh
# Reactor for the GitHub Actions bridge: fires on mirror state change.
# Diffs against last.json to find newly completed (or re-concluded)
# runs, sends one desktop notification per change, then updates last.
#
# Env:
#   NOTIFY_CMD  notification command; default autodetects osascript
set -u
NEW="${1:?missing changed file path}"
STATE_DIR="$(dirname "$NEW")"
LOG="$STATE_DIR/events.log"
mkdir -p "$STATE_DIR"

NOTIFY_CMD="${NOTIFY_CMD:-}"
notify() { # $1 title, $2 body
  if [ -z "$NOTIFY_CMD" ]; then
    if command -v osascript >/dev/null 2>&1; then
      TITLE=$(printf '%s' "$1" | tr -d '"')
      BODY=$(printf '%s' "$2" | tr -d '"')
      osascript -e "display notification \"$BODY\" with title \"$TITLE\"" >/dev/null 2>&1 || true
    fi
  else
    $NOTIFY_CMD "$1" "$2" || true
  fi
}

LAST="$STATE_DIR/last.json"
touch "$LOG"

# First ever fire: seed baseline without notifying (initial history is
# not news).
if [ ! -f "$LAST" ]; then
  cp "$NEW" "$LAST.new" && mv "$LAST.new" "$LAST"
  echo "$(date +%H:%M:%S) seeded baseline" >> "$LOG"
  exit 0
fi

CHANGED=$(jq -cr --slurpfile old "$LAST" '
  ($old[0] // []) as $o
  | [ .[] as $n
      | select($n.status == "completed")
      | ([$o[] | select(.databaseId == $n.databaseId)] | first) as $p
      | select($p == null or ($p.conclusion // "?") != ($n.conclusion // "?"))
      | $n | {workflowName, conclusion, headBranch, displayTitle}
  ]' "$NEW")

echo "$(date +%H:%M:%S) $CHANGED" >> "$LOG"

echo "$CHANGED" | jq -cr '.[] | "\(.workflowName)|\(.conclusion)|\(.headBranch)|\(.displayTitle)"' \
| while IFS='|' read -r WF CONCLUSION BRANCH TITLE; do
  ICON_OK="✅"
  ICON_FAIL="❌"
  case "$CONCLUSION" in
    success) ICON="$ICON_OK" ;;
    *) ICON="$ICON_FAIL" ;;
  esac
  notify "fzz gh $ICON" "$CONCLUSION · $BRANCH · $TITLE"
done

# Publish new baseline atomically after processing.
cp "$NEW" "$LAST.new" && mv "$LAST.new" "$LAST"

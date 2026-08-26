#!/bin/sh
# Reactor for the GitHub Actions bridge: fires on mirror state change.
# Diffs against last.json to find newly completed (or re-concluded)
# runs, sends one desktop notification per change, alerts the crew via
# pi-bebop durable intake, then updates last.
#
# Env:
#   NOTIFY_CMD    desktop notification command; default autodetects osascript
#   CREW_NOTIFY   failures|all|off (default failures)
#   CREW_MANIFEST crew manifest path (default .pi/bebop/crew.json)
#   PI_BEBOP_BIN  pi-bebop CLI (default pi-bebop on PATH)
set -u
NEW="${1:?missing changed file path}"
STATE_DIR="$(dirname "$NEW")"
LOG="$STATE_DIR/events.log"
mkdir -p "$STATE_DIR"

NOTIFY_CMD="${NOTIFY_CMD:-}"
notify() { # $1 title, $2 body
  if [ -z "$NOTIFY_CMD" ]; then
    if command -v osascript >/dev/null 2>&1; then
      N_TITLE=$(printf '%s' "$1" | tr -d '"')
      N_BODY=$(printf '%s' "$2" | tr -d '"')
      osascript -e "display notification \"$N_BODY\" with title \"$N_TITLE\"" >/dev/null 2>&1 || true
    fi
  else
    $NOTIFY_CMD "$1" "$2" || true
  fi
}

crew_alert() { # $1 workflow, $2 conclusion, $3 branch, $4 title, $5 url
  MODE="${CREW_NOTIFY:-failures}"
  [ "$MODE" = "off" ] && return 0
  if [ "$MODE" = "failures" ] && [ "$2" = "success" ]; then
    return 0
  fi
  PI_BEBOP_BIN="${PI_BEBOP_BIN:-pi-bebop}"
  CREW_MANIFEST="${CREW_MANIFEST:-.pi/bebop/crew.json}"
  if ! command -v "$PI_BEBOP_BIN" >/dev/null 2>&1; then
    echo "$(date +%H:%M:%S) crew alert skipped: $PI_BEBOP_BIN not found" >> "$LOG"
    return 0
  fi
  ICON="❌"
  [ "$2" = "success" ] && ICON="✅"
  printf 'CI %s %s - %s - %s - %s\n%s\n' "$ICON" "$2" "$1" "$3" "$4" "$5" \
  | "$PI_BEBOP_BIN" send --crew "$CREW_MANIFEST" --from "gh-actions" --stdin \
    >> "$LOG" 2>&1 || true
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
      | $n | {workflowName, conclusion, headBranch, displayTitle, url}
  ]' "$NEW")

echo "$(date +%H:%M:%S) $CHANGED" >> "$LOG"

echo "$CHANGED" | jq -cr '.[] | "\(.workflowName)|\(.conclusion)|\(.headBranch)|\(.displayTitle)|\(.url // "")"' \
| while IFS='|' read -r WF CONCLUSION BRANCH TITLE URL; do
  ICON="❌"
  [ "$CONCLUSION" = "success" ] && ICON="✅"
  notify "fzz gh $ICON" "$CONCLUSION · $BRANCH · $TITLE"
  crew_alert "$WF" "$CONCLUSION" "$BRANCH" "$TITLE" "$URL"
done

# Publish new baseline atomically after processing.
cp "$NEW" "$LAST.new" && mv "$LAST.new" "$LAST"

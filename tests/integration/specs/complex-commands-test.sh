#!/usr/bin/env bash
source "$HELPERS"

test "it accepts complex commands with piping"

echo "
- name: run complex command
  run: \"echo 'runnning command' | sed  s/command/foobar/g | sed  s/runnning/blabla/g\"
  change: \"$WORKDIR/**\"
  run_on_init: true
" > $WORKDIR/.oninit.yaml

"$TEST_DIR"/funzzy --config "$WORKDIR"/.oninit.yaml > "$WORKDIR"/output.log &
FUNZZY_PID=$!

wait_for_file "$WORKDIR/output.log"
assert_file_contains "$WORKDIR/output.log" "blabla foobar"

cleanup

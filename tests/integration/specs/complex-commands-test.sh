#!/usr/bin/env bash
source "$HELPERS"

test "it accepts complex commands with piping"

echo "
- name: run complex command
  run: \"echo 'runnning command' | sed  s/command/foobar/g | sed  s/runnning/blabla/g\"
  change: \"$WORKDIR/**\"
  run_on_init: true
" > $WORKDIR/.oninit.yaml

funzzy --config $WORKDIR/.oninit.yaml > $WORKDIR/output.txt &
FUNZZY_PID=$!

wait_for_file "$WORKDIR/output.txt"
assert_file_contains "$WORKDIR/output.txt" "blabla foobar"

cleanup

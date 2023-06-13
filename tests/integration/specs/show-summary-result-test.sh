#!/usr/bin/env bash
source "$HELPERS"

test "it allows a list of commands for the same task (on init)"

echo "
- name: run complex command
  run: ['echo first', 'exit 1', 'cat unknow', 'echo complex | sed s/complex/third/g']
  change: \"$WORKDIR/**\"
  run_on_init: true
" > $WORKDIR/.oninit.yaml

$TEST_DIR/funzzy --config $WORKDIR/.oninit.yaml > $WORKDIR/output.log &
FUNZZY_PID=$!

assert_file_contains "$WORKDIR/output.log" "Watching..."
assert_file_contains "$WORKDIR/output.log" "Funzzy result"
assert_file_contains "$WORKDIR/output.log" "Failed tasks: 2"
assert_file_contains "$WORKDIR/output.log" "Command exit 1 has failed with exit status: 1"
assert_file_contains "$WORKDIR/output.log" "Command cat unknow has failed with exit status: 1"

cleanup

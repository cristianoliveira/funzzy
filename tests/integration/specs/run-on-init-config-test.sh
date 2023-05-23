#!/usr/bin/env bash
source "$HELPERS"
test "it allows configuring commands to run on init"

echo "
- name: run complex command
  run: \"echo 'runnnig command on init'\"
  change: \"$WORKDIR/**\"
  run_on_init: true
" > $WORKDIR/.oninit.yaml

$TEST_DIR/funzzy --config $WORKDIR/.oninit.yaml > $WORKDIR/output.txt &
FUNZZY_PID=$!

wait_for_file "$WORKDIR/output.txt"
assert_file_contains "$WORKDIR/output.txt" "runnnig command on init"

cleanup

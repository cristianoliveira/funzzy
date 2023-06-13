#!/usr/bin/env bash
source "$HELPERS"
test "it allows configuring commands to run on init"

echo "
- name: run complex command
  run: \"echo 'running command on init'\"
  change: \"$WORKDIR/**\"
  run_on_init: true
" > $WORKDIR/.oninit.yaml

$TEST_DIR/funzzy --config $WORKDIR/.oninit.yaml > $WORKDIR/output.log &
FUNZZY_PID=$!

wait_for_file "$WORKDIR/output.log"
assert_file_contains "$WORKDIR/output.log" "running command on init"

cleanup

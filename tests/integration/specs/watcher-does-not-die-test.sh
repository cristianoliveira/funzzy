#!/usr/bin/env bash
source "$HELPERS"
test "process does not die when a one or more commands fail (list)"

echo "
- name: run the list and do not die
  run: [
    'echo before',
    'exit 1',
    'cat foo/bar/baz',
    'exit 125',
    'echo after'
  ]
  change: \"$WORKDIR/**\"
  run_on_init: true
" > $WORKDIR/.dontdie.yaml

$TEST_DIR/funzzy --config $WORKDIR/.dontdie.yaml > $WORKDIR/output.txt &
FUNZZY_PID=$!

wait_for_file "$WORKDIR/output.txt"
assert_file_contains "$WORKDIR/output.txt" "running: echo before"
assert_file_contains "$WORKDIR/output.txt" "running: exit 1"
assert_file_contains "$WORKDIR/output.txt" "running: cat foo/bar/baz"
assert_file_contains "$WORKDIR/output.txt" "running: exit 125"
assert_file_contains "$WORKDIR/output.txt" "running: echo after"

assert_file_contains "$WORKDIR"/output.txt "Failed tasks: 3"
assert_file_contains "$WORKDIR"/output.txt "Command exit 1 has failed with exit status: 1"
assert_file_contains "$WORKDIR"/output.txt "Command cat foo/bar/baz has failed with exit status: 1"
assert_file_contains "$WORKDIR"/output.txt "Command exit 125 has failed with exit status: 125"


cleanup

test "process does not die when a task fail (multiple tasks)"

echo "
- name: run the first task
  run: 'echo before'
  change: \"$WORKDIR/**\"
  run_on_init: true

- name: run the first task
  run: 'cat baz/bar/foo'
  change: \"$WORKDIR/**\"
  run_on_init: true

- name: run finally
  run: 'echo finally'
  change: \"$WORKDIR/**\"
  run_on_init: true
" > $WORKDIR/.dontdie.yaml

$TEST_DIR/funzzy --config $WORKDIR/.dontdie.yaml > $WORKDIR/output.txt &
FUNZZY_PID=$!

wait_for_file "$WORKDIR/output.txt"
assert_file_contains "$WORKDIR/output.txt" "running: echo before"
assert_file_contains "$WORKDIR/output.txt" "running: cat baz/bar/foo"
assert_file_contains "$WORKDIR/output.txt" "running: echo finally"

assert_file_contains "$WORKDIR"/output.txt "Failed tasks: 1"
assert_file_contains "$WORKDIR"/output.txt "Command cat baz/bar/foo has failed with exit status: 1"

cleanup

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
assert_file_contains "$WORKDIR/output.txt" "command: echo before"
assert_file_contains "$WORKDIR/output.txt" "command: exit 1"
assert_file_contains "$WORKDIR/output.txt" "This task command has failed exit status: 1"
assert_file_contains "$WORKDIR/output.txt" "command: cat foo/bar/baz"
assert_file_contains "$WORKDIR/output.txt" "command: exit 125"
assert_file_contains "$WORKDIR/output.txt" "This task command has failed exit status: 125"
assert_file_contains "$WORKDIR/output.txt" "command: echo after"

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
assert_file_contains "$WORKDIR/output.txt" "command: echo before"
assert_file_contains "$WORKDIR/output.txt" "command: cat baz/bar/foo"
assert_file_contains "$WORKDIR/output.txt" "This task command has failed exit status: 1"
assert_file_contains "$WORKDIR/output.txt" "command: echo finally"

cleanup

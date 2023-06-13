#!/usr/bin/env bash
source "$HELPERS"

test "it allows a list of commands for the same task (on init)"

echo "
- name: run complex command
  run: ['echo first', 'echo second', 'echo complex | sed s/complex/third/g']
  change: \"$WORKDIR/**\"
  run_on_init: true
" > $WORKDIR/.oninit.yaml

$TEST_DIR/funzzy --config $WORKDIR/.oninit.yaml > $WORKDIR/output.log &
FUNZZY_PID=$!

assert_file_content_at "$WORKDIR/output.log" "Running on init commands" 1
assert_file_content_at "$WORKDIR/output.log" "first" 2
assert_file_content_at "$WORKDIR/output.log" "second" 3
assert_file_content_at "$WORKDIR/output.log" "third" 4
assert_file_contains "$WORKDIR/output.log" "Watching..."

cleanup

test "it allows a list of commands for the same task (on change)"

echo "
- name: run complex command
  run: ['echo 100', 'echo 200', 'echo 4000 | sed s/4000/3333/g']
  change: \"$WORKDIR/**\"
" > $WORKDIR/.oninit.yaml

$TEST_DIR/funzzy --config $WORKDIR/.oninit.yaml > $WORKDIR/output.log &
FUNZZY_PID=$!

assert_file_contains "$WORKDIR/output.log" "Watching..."
echo "test" >> $WORKDIR/test.txt
sh -c "vi +%s/test/foo/g +wq $WORKDIR/test.txt -u NONE"
assert_file_content_at "$WORKDIR/output.log" "100" 2
assert_file_content_at "$WORKDIR/output.log" "200" 3
assert_file_content_at "$WORKDIR/output.log" "3333" 4

cleanup

#!/usr/bin/env bash
source "$HELPERS"

test "it allows a list of commands for the same task (on init)"

echo "
- name: run complex command
  run: ['echo first', 'echo second', 'echo {{place}} | sed s/place/replace/g']
  change: \"$WORKDIR/*.txt\"
  run_on_init: true
" > $WORKDIR/.oninit.yaml

$TEST_DIR/funzzy --config $WORKDIR/.oninit.yaml > $WORKDIR/output.log &
FUNZZY_PID=$!

assert_file_content_at "$WORKDIR/output.log" "Running on init commands" 1
assert_file_contains "$WORKDIR/output.log" "echo first"
assert_file_contains "$WORKDIR/output.log" "echo second"
assert_file_contains "$WORKDIR/output.log" "echo complex"
assert_file_contains "$WORKDIR/output.log" "{{replace}}"
assert_file_contains "$WORKDIR/output.log" "Watching..."

cleanup

test "it allows a list of commands for the same task on change"

echo "
- name: run complex command
  run: [
    'echo 100',
    'echo 200',
    'echo {{placeholder}} | sed s/placeholder/replace/g'
  ]
  change: \"$WORKDIR/*.txt\"
" > $WORKDIR/.oninit.yaml

$TEST_DIR/funzzy --config $WORKDIR/.oninit.yaml > $WORKDIR/output.log &
FUNZZY_PID=$!

assert_file_contains "$WORKDIR/output.log" "Watching..."
echo "test" >> $WORKDIR/test.txt
vi +%s/test/foo/g +wq $WORKDIR/test.txt -u NONE
assert_file_contains "$WORKDIR/output.log" "echo 100"
assert_file_contains "$WORKDIR/output.log" "echo 200"
assert_file_contains "$WORKDIR/output.log" "echo 4000"
assert_file_contains "$WORKDIR/output.log" "{{replace}}"

cleanup

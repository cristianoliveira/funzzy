#!/usr/bin/env bash
source "$HELPERS"

test "it a list of commands for the same task (on init)"

echo "
- name: run complex command
  run: ['echo first', 'echo second', 'echo complex | sed s/complex/third/g']
  change: \"$WORKDIR/**\"
  run_on_init: true
" > $WORKDIR/.oninit.yaml

funzzy --config $WORKDIR/.oninit.yaml > $WORKDIR/output.txt &
FUNZZY_PID=$!

assert_file_content_at "$WORKDIR/output.txt" "Running on init commands" 1
assert_file_contains "$WORKDIR/output.txt" "running: echo first"
assert_file_contains "$WORKDIR/output.txt" "running: echo second"
assert_file_contains "$WORKDIR/output.txt" "running: echo complex"
assert_file_content_at "$WORKDIR/output.txt" "third" 6
assert_file_contains "$WORKDIR/output.txt" "Watching..."

cleanup

exit 0

if [ -n "$CI" ]; then
  echo "skipping test in CI cuz no trigger is possible"
  exit 0
fi

test "it a list of commands for the same task (on change)"

echo "
- name: run complex command
  run: ['echo 100', 'echo 200', 'echo 4000 | sed s/4000/3333/g']
  change: \"$WORKDIR/**\"
" > $WORKDIR/.oninit.yaml

funzzy --config $WORKDIR/.oninit.yaml > $WORKDIR/output.txt &
FUNZZY_PID=$!

assert_file_contains "$WORKDIR/output.txt" "Watching..."
echo "test" >> $WORKDIR/test.txt
sh -c "vi +%s/test/foo/g +wq $WORKDIR/test.txt -u NONE"
assert_file_contains "$WORKDIR/output.txt" "running: echo 100"
assert_file_contains "$WORKDIR/output.txt" "running: echo 200"
assert_file_contains "$WORKDIR/output.txt" "running: echo 4000"
assert_file_content_at "$WORKDIR/output.txt" "3333" 6

cleanup

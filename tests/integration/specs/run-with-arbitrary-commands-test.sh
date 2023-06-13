#!/usr/bin/env bash
source "$HELPERS"

test "it allows watching a given file list and run an arbitrary command"

touch $WORKDIR/test.txt
touch $WORKDIR/test2.txt
touch $WORKDIR/readme.md
touch $WORKDIR/output.log
# $TEST_DIR/funzzy --config $WORKDIR/.onwatch.yaml &

find . -name '*.txt' | \
  $TEST_DIR/funzzy 'echo arbitrary' -V > $WORKDIR/output.log &
FUNZZY_PID=$!

assert_file_contains "$WORKDIR/output.log" "verbose"
assert_file_contains "$WORKDIR/output.log" "test.txt"
assert_file_contains "$WORKDIR/output.log" "test2.txt"
assert_file_contains "$WORKDIR/output.log" "output.log"
assert_file_not_contains "$WORKDIR/output.log" "readme.md"

echo "test" >> "$WORKDIR"/test.txt
echo "test" >> "$WORKDIR"/test2.txt
wait_for_file "$WORKDIR/output.log"
vi +%s/test/foo/g +wq "$WORKDIR"/test.txt -u NONE
vi +%s/test/foo/g +wq "$WORKDIR"/test2.txt -u NONE
assert_file_contains "$WORKDIR/output.log" "echo arbitrary"

cleanup

test "it does not run when the changed file doesn't match"

touch "$WORKDIR"/test.txt
touch "$WORKDIR"/test2.txt
touch "$WORKDIR"/output.log
# $TEST_DIR/funzzy --config $WORKDIR/.onwatch.yaml &

find . -name '*.txt' | \
  $TEST_DIR/funzzy 'echo arbitrary' > $WORKDIR/output.log &
FUNZZY_PID=$!

echo "test" >> "$WORKDIR"/test.js
echo "test" >> "$WORKDIR"/test2.js
wait_for_file "$WORKDIR/output.log"
vi +%s/test/foo/g +wq $WORKDIR/test.js -u NONE
vi +%s/test/foo/g +wq $WORKDIR/test2.js -u NONE

sleep 5

assert_file_not_contains "$WORKDIR/output.log" "echo arbitrary"

cleanup

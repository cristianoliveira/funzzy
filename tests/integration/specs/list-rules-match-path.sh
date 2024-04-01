#!/usr/bin/env bash
source "$HELPERS"
test "given check path match at one rules"

echo "
- name: 1st task
  run: 'echo before'
  change: [
    \"$WORKDIR/**\",
    \"$WORKDIR/src/**/*.txt\"
  ]

- name: 2nd task
  run: 'echo before'
  change: \"$WORKDIR/src/**/*.log\"
  ignore: \"$WORKDIR/src/test.log\"

- name: 3rd task
  run: 'echo before'
  change: [
    \"$WORKDIR/tmp/**/*.log\"
  ]
" > $WORKDIR/.check.yaml

$TEST_DIR/funzzy rules -m "$WORKDIR/src/foo.txt" -c $WORKDIR/.check.yaml >> $WORKDIR/output.log

wait_for_file "$WORKDIR/output.log"

cat "$WORKDIR"/output.log

assert_file_occurrencies "$WORKDIR/output.log" "1st task" 2 
assert_file_occurrencies "$WORKDIR/output.log" "2nd task" 1
assert_file_occurrencies "$WORKDIR/output.log" "3rd task" 1

echo "" > "$WORKDIR"/output.log

$TEST_DIR/funzzy rules -m "$WORKDIR/src/foo.log" -c $WORKDIR/.check.yaml >> $WORKDIR/output.log

cat "$WORKDIR"/output.log

assert_file_occurrencies "$WORKDIR/output.log" "1st task" 2 
assert_file_occurrencies "$WORKDIR/output.log" "2nd task" 2
assert_file_occurrencies "$WORKDIR/output.log" "3rd task" 1

echo "" > "$WORKDIR"/output.log

$TEST_DIR/funzzy rules -m "$WORKDIR/src/test.log" -c $WORKDIR/.check.yaml >> $WORKDIR/output.log

cat "$WORKDIR"/output.log

assert_file_occurrencies "$WORKDIR/output.log" "1st task" 2 
assert_file_occurrencies "$WORKDIR/output.log" "2nd task" 1
assert_file_occurrencies "$WORKDIR/output.log" "3rd task" 1

cleanup

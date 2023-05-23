#!/usr/bin/env bash
source "$HELPERS"

test "it accepts multiple commands configurations"

echo "
- name: run first command
  run: \"echo '{{test}} command' | sed  s/test/first/g\"
  change: \"$WORKDIR/**\"
  run_on_init: true

- name: run second command
  run: \"echo '{{test}} command' | sed  s/test/second/g\"
  change: \"$WORKDIR/**\"
  run_on_init: true

- name: run third command on change
  run: \"echo '{{not_executed}} command' | sed  s/not_executed/third/g\"
  change: \"another/**\"

- name: run last command on init
  run: \"echo '{{not_executed}} command' | sed  s/not_executed/third/g\"
  change: \"another/**\"
  run_on_init: true
" > $WORKDIR/.oninit.yaml

$TEST_DIR/funzzy --config $WORKDIR/.oninit.yaml > $WORKDIR/output.txt &
FUNZZY_PID=$!

echo "test" >> $WORKDIR/test.txt
# Run vim in ex mode
ex $WORKDIR/test.txt <<EOEX
  :%s/test/foo/g
  :x
EOEX

wait_for_file "$WORKDIR/output.txt"
assert_file_contains "$WORKDIR/output.txt" "{{first}} command"
assert_file_contains "$WORKDIR/output.txt" "{{second}} command"
assert_file_contains "$WORKDIR/output.txt" "{{not_executed}} command"

cleanup

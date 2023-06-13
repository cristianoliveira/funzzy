#!/usr/bin/env bash
source "$HELPERS"

test "allows to run a target task"
target="second command"
config_file="$WORKDIR/target.yaml"

echo "
- name: run first command
  run: \"echo '{{test}} command' | sed  s/test/first/g\"
  change: \"$WORKDIR/**\"
  run_on_init: true

- name: run second command
  run: \"echo '{{test}} command' | sed  s/test/second/g\"
  change: \"$WORKDIR/**\"
  run_on_init: true

- name: run third command
  run: \"echo '{{test}} command' | sed  s/test/third/g\"
  change: \"$WORKDIR/**\"
  run_on_init: true
" > "$config_file"

echo "target: $target";

$TEST_DIR/funzzy --config="$config_file" \
  --target="$target" > "$WORKDIR/output.log" &
FUNZZY_PID=$!

wait_for_file "$WORKDIR/output.log"
assert_file_contains "$WORKDIR/output.log" "{{second}} command"
assert_file_not_contains "$WORKDIR/output.log" "{{first}} command"
assert_file_not_contains "$WORKDIR/output.log" "{{third}} command"

cleanup

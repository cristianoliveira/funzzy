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
  --target="$target" > "$WORKDIR/output.txt" &
FUNZZY_PID=$!

wait_for_file "$WORKDIR/output.txt"
assert_file_contains "$WORKDIR/output.txt" "{{second}} command"
assert_file_not_contains "$WORKDIR/output.txt" "{{first}} command"
assert_file_not_contains "$WORKDIR/output.txt" "{{third}} command"

cleanup

test "shows the available tasks when target doesn't match"
target="unknown"
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
  --target="$target" > "$WORKDIR/output.txt" &
FUNZZY_PID=$!

wait_for_file "$WORKDIR/output.txt"
assert_file_contains "$WORKDIR/output.txt" "No target found for '$target'"
assert_file_contains "$WORKDIR/output.txt" "Available targets:"
assert_file_contains "$WORKDIR/output.txt" "run first command"
assert_file_contains "$WORKDIR/output.txt" "run second command"
assert_file_contains "$WORKDIR/output.txt" "run third command"

cleanup

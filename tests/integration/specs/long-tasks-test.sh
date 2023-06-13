#!/usr/bin/env bash
source "$HELPERS"
if [ -n "$CI" ]; then
  echo "skipping long tasks test on CI"
  exit 0
fi

test "process does not die when a one or more commands fail (list)"

"$TEST_DIR"/funzzy \
  --config "$TEST_DIR"/examples/long-task.yaml \
  --non-block  > "$WORKDIR"/output.log \
  &
FUNZZY_PID=$!

echo "test" > "$WORKDIR"/temp.txt

wait_for_file "$WORKDIR"/output.log

assert_file_not_contains "$WORKDIR"/output.log "All tasks finished successfully."
assert_file_contains "$WORKDIR"/output.log "Watching..."

vi +wq tests/integration/workdir/temp.txt -u NONE
assert_file_occurrencies "$WORKDIR/output.log" "longtask.sh list 4" 2

sleep 1
vi +wq tests/integration/workdir/temp.txt -u NONE
assert_file_occurrencies "$WORKDIR/output.log" "longtask.sh list 4" 3

sleep 1
vi +wq tests/integration/workdir/temp.txt -u NONE
assert_file_occurrencies "$WORKDIR"/output.log "longtask.sh list 4" 4

# Check if there are any zombie processes
leaks=$(ps -A -ostat,pid,ppid | grep -e '[zZ]')
if [ -n "$leaks" ]; then
  echo "Zombie processes found: $leaks"
  exit 1
fi

cleanup

#!/usr/bin/env bash
source "$HELPERS"
if [ -n "$CI" ]; then
  echo "skipping long tasks test on CI"
  exit 0
fi

test "process does not die when a one or more commands fail (list)"

"$TEST_DIR"/funzzy \
  --config "$TEST_DIR"/examples/long-task.yaml \
  --non-block  > "$WORKDIR"/output.txt &
FUNZZY_PID=$!

echo "test" > "$WORKDIR"/temp.txt

wait_for_file "$WORKDIR"/output.txt

assert_file_contains "$WORKDIR"/output.txt "Watching..."
assert_file_contains "$WORKDIR/output.txt" "Task short finished"
vi +wq tests/integration/workdir/temp.txt -u NONE
assert_file_occurrencies "$WORKDIR/output.txt" "longtask.sh short 5" 2

vi +wq tests/integration/workdir/temp.txt -u NONE
assert_file_occurrencies "$WORKDIR/output.txt" "longtask.sh short 5" 3

vi +wq tests/integration/workdir/temp.txt -u NONE
assert_file_occurrencies "$WORKDIR"/output.txt "longtask.sh short 5" 4

cleanup

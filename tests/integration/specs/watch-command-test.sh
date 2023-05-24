#!/usr/bin/env bash
source "$HELPERS"

# skip if in CI
if [ -n "$CI" ]; then
  echo "skipping watch-command-test.sh in CI no trigger is possible"
  exit 0
fi

test "it watches the configured rules"

echo "
- name: run simple command
  run: echo 'test1'
  change: \"$WORKDIR/**\"
" > $WORKDIR/.onwatch.yaml

touch $WORKDIR/test.txt
touch $WORKDIR/output.txt
# $TEST_DIR/funzzy --config $WORKDIR/.onwatch.yaml &
$TEST_DIR/funzzy watch --config $WORKDIR/.onwatch.yaml >> $WORKDIR/output.txt &
FUNZZY_PID=$!

echo "test" >> $WORKDIR/test.txt
wait_for_file "$WORKDIR/output.txt"
sh -c "vi +%s/test/foo/g +wq $WORKDIR/test.txt -u NONE"
assert_file_contains "$WORKDIR/output.txt" "echo 'test1'"

cleanup

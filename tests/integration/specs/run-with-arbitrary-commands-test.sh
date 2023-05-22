#!/usr/bin/env bash
source "$HELPERS"

# skip if in CI
# if [ -n "$CI" ]; then
#   echo "skipping watch-command-test.sh in CI no trigger is possible"
#   exit 0
# fi

test "it allows watching a given file list and run an arbitrary command"

touch $WORKDIR/test.txt
touch $WORKDIR/test2.txt
touch $WORKDIR/output.txt
# $TEST_DIR/funzzy --config $WORKDIR/.onwatch.yaml &

find . -name '*.txt' | \
  $TEST_DIR/funzzy 'echo arbitrary' > $WORKDIR/output.txt &
FUNZZY_PID=$!

echo "test" >> $WORKDIR/test.txt
echo "test" >> $WORKDIR/test2.txt
wait_for_file "$WORKDIR/output.txt"
sh -c "vi +%s/test/foo/g +wq $WORKDIR/test.txt -u NONE"
sh -c "vi +%s/test/foo/g +wq $WORKDIR/test2.txt -u NONE"
assert_file_contains "$WORKDIR/output.txt" "echo arbitrary"

cleanup

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

- name: run different command
  run: echo '__placeholder__' | sed s/placeholder/second_commmand/g
  change: \"$WORKDIR/**\"
" > "$WORKDIR"/.onwatch.yaml

touch "$WORKDIR"/test.txt
touch "$WORKDIR"/output.log
# $TEST_DIR/funzzy --config $WORKDIR/.onwatch.yaml &
"$TEST_DIR"/funzzy watch \
  --config "$WORKDIR"/.onwatch.yaml >> "$WORKDIR"/output.log &
FUNZZY_PID=$!

wait_for_file "$WORKDIR/output.log"

echo "some content" > "$WORKDIR"/test.txt

touch "$WORKDIR"/test.txt
assert_file_occurrencies "$WORKDIR/output.log" "__second_commmand__" 1

touch "$WORKDIR"/test.txt
assert_file_occurrencies "$WORKDIR/output.log" "__second_commmand__" 2

cleanup

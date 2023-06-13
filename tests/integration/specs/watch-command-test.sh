#!/usr/bin/env bash
source "$HELPERS"

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
touch "$WORKDIR"/output.txt
$TEST_DIR/funzzy watch --config $WORKDIR/.onwatch.yaml >> $WORKDIR/output.txt &
FUNZZY_PID=$!

wait_for_file "$WORKDIR/output.txt"

echo "some content" > "$WORKDIR"/test.txt

touch "$WORKDIR"/test.txt
assert_file_occurrencies "$WORKDIR/output.txt" "__second_commmand__" 1

touch "$WORKDIR"/test.txt
assert_file_occurrencies "$WORKDIR/output.txt" "__second_commmand__" 2

cleanup

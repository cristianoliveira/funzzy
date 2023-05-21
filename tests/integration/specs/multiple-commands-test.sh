#!/usr/bin/env bash
source "$HELPERS"

test "it accespts multiple commands configurations"

echo '
- name: run first command
  run: "echo '{{test}} command' | sed  s/test/first/g"
  change: "workdir/**"
  run_on_init: true

- name: run second command
  run: "echo '{{test}} command' | sed  s/test/second/g"
  change: "workdir/**"
  run_on_init: true

- name: run third command on change
  run: "echo '{{not_executed}} command' | sed  s/not_executed/third/g"
  change: "another/**"

- name: run last command on init
  run: "echo '{{not_executed}} command' | sed  s/not_executed/third/g"
  change: "another/**"
  run_on_init: true
' > workdir/.oninit.yaml

funzzy --config workdir/.oninit.yaml > workdir/output.txt &
FUNZZY_PID=$!

echo "test" >> workdir/test.txt
# Run vim in ex mode
ex workdir/test.txt <<EOEX
  :%s/test/foo/g
  :x
EOEX

wait_for_file "workdir/output.txt"
assert_file_contains "workdir/output.txt" "{{first}} command"
assert_file_contains "workdir/output.txt" "{{second}} command"
assert_file_contains "workdir/output.txt" "{{not_executed}} command"

cleanup

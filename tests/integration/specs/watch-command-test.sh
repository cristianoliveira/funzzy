#!/usr/bin/env bash
source "$HELPERS"

test "simple watch"

echo '
- name: run simple command
  run: echo 'test1'
  change: "workdir/**"
  run_on_init: true
' > workdir/.watch.yaml

touch workdir/test.txt
funzzy --config ./workdir/.watch.yaml > workdir/output.txt &
FUNZZY_PID=$!

echo "test" >> workdir/test.txt
# Run vim in ex mode
ex workdir/test.txt <<EOEX
  :%s/test/foo/g
  :x
EOEX

wait_for_file workdir/output.txt
assert_file_contains "echo test1" workdir/output.txt

cleanup

#!/usr/bin/env bash
source "$HELPERS"

test "multiple commands"

echo '
- name: run first command
  run: "echo '{{test}} command' | sed  s/test/first/g"
  change: "workdir/**"
  run_on_init: true

- name: run second command
  run: "echo '{{test}} command' | sed  s/test/second/g"
  change: "workdir/**"
  run_on_init: true
' > workdir/.oninit.yaml

funzzy --config workdir/.oninit.yaml > workdir/output.txt &
FUNZZY_PID=$!

wait_for_file "workdir/output.txt"
assert_file_contains "{{first}} command" "workdir/output.txt"
assert_file_contains "{{second}} command" "workdir/output.txt"

cleanup

#!/usr/bin/env bash
source "$HELPERS"

test "complex commands"

echo '
- name: run complex command
  run: "echo 'runnning command' | sed  s/command/foobar/g | sed  s/runnning/blabla/g"
  change: "workdir/**"
  run_on_init: true
' > workdir/.oninit.yaml

funzzy --config workdir/.oninit.yaml > workdir/output.txt &
FUNZZY_PID=$!

wait_for_file "workdir/output.txt"
assert_file_contains "workdir/output.txt" "blabla foobar"

cleanup

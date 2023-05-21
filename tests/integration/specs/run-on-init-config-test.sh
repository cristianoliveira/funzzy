#!/usr/bin/env bash
source "$HELPERS"
test "run on init"

echo '
- name: run complex command
  run: "echo 'runnnig command on init'"
  change: "workdir/**"
  run_on_init: true
' > workdir/.oninit.yaml

funzzy --config workdir/.oninit.yaml > workdir/output.txt &
FUNZZY_PID=$!

wait_for_file workdir/output.txt
assert_file_contains "runnnig command on init" workdir/output.txt

cleanup

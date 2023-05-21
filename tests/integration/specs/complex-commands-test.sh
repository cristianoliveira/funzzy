#!/usr/bin/env bash
ls
ls -ls $PWD
source "functions.sh"
source "$TEST_DIR/functions.sh"

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
assert_file_contains "blabla foobar" "workdir/output.txt"

cleanup

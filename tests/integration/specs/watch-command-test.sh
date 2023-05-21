#!/usr/bin/env bash
source "$HELPERS"

if [ -z "$CI" ]; then
  echo "skipping test, only run on CI"
  return;
fi

test "it watches the configured rules"

echo '
- name: run simple command
  run: echo 'test1'
  change: "workdir/**"
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

cat workdir/test.txt

wait_for_file "workdir/output.txt"
assert_file_contains "workdir/output.txt" "echo test1"

cleanup

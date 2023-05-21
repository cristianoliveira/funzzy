#!/usr/bin/env bash
source "$HELPERS"

# skip if on CI
if [ -n "$CI" ]; then
  echo "skipping test, only run on CI"
  exit 0
fi

test "it watches the configured rules"

echo '
- name: run simple command
  run: echo 'test1'
  change: "workdir/**"
' > workdir/.onwatch.yaml

touch workdir/test.txt
touch workdir/output.txt
funzzy --config workdir/.onwatch.yaml >> workdir/output.txt &
FUNZZY_PID=$!

echo "test" >> workdir/test.txt
# Run vim in ex mode
ex workdir/test.txt <<EOEX
  :%s/test/foo/g
  :x
EOEX

cat workdir/test.txt

wait_for_file "workdir/output.txt"
cat workdir/output.txt
assert_file_contains "workdir/output.txt" "echo test1"
cat workdir/output.txt

cleanup

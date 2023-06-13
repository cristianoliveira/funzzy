#!/usr/bin/env bash
source "$HELPERS"

if [ -n "$CI" ]; then
  echo "skipping ignoring-folders-tests.sh in CI no trigger is possible"
  exit 0
fi

test "it allows setting up ignored paths"

echo "
- name: run ignored
  run: ['echo {{placeholder}} | sed s/placeholder/ignored/g']
  change: '$WORKDIR/**'
  ignore: [
    '$WORKDIR/ignored/**',
    '$WORKDIR/output.log',
    '$WORKDIR/file-to-ignore.txt',

    # Vim backup files
    '$WORKDIR/file-to-ignore.txt~',
    '$WORKDIR/.file-to-ignore.txt.swp',
    '$WORKDIR/.file-to-ignore.txt.swx'
  ]

- name: run not ignored
  run: ['echo {{placeholder}} | sed s/placeholder/changed/g']
  change: '$WORKDIR/**'
" > $WORKDIR/.oninit.yaml


mkdir -p "$WORKDIR/ignored"

$TEST_DIR/funzzy --config $WORKDIR/.oninit.yaml > $WORKDIR/output.log &
FUNZZY_PID=$!

assert_file_contains "$WORKDIR/output.log" "Watching..."

echo "test" >> $WORKDIR/file-to-ignore.txt
sh -c "vi +%s/test/foo/g +wq $WORKDIR/file-to-ignore.txt -u NONE"

echo "test" >> $WORKDIR/ignored/test.txt
sh -c "vi +%s/test/foo/g +wq $WORKDIR/ignored/text.txt -u NONE"

assert_file_contains "$WORKDIR/output.log" "{{changed}}"
assert_file_not_contains "$WORKDIR/output.log" "{{ignored}}"

echo "test" >> $WORKDIR/test.txt
sh -c "vi +%s/test/foo/g +wq $WORKDIR/test.txt -u NONE"

assert_file_contains "$WORKDIR/output.log" "{{changed}}"
assert_file_contains "$WORKDIR/output.log" "{{ignored}}"

cleanup

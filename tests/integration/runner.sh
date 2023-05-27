#!/usr/bin/env bash

set -e

export TEST_DIR="$PWD"/tests/integration
export HELPERS="$TEST_DIR"/functions.sh
export WORKDIR="$TEST_DIR"/workdir

echo "Building funzzy"

rm -f "$TEST_DIR"/funzzy
# if CI build with --release flag else build with debug flag
if [ -n "$CI" ]; then
  cargo build --release --target-dir "$TEST_DIR"
  cp "$TEST_DIR"/release/funzzy "$TEST_DIR"/funzzy
else
  cargo build --target-dir "$TEST_DIR"
  cp "$TEST_DIR"/debug/funzzy "$TEST_DIR"/funzzy
fi

"$TEST_DIR"/funzzy --version
"$TEST_DIR"/funzzy --help


rm -f $TEST_DIR/failed.log

## if path received as argument, run only that test
if [ -n "$1" ]; then
  echo "Running only $1"
  bash "$1" && echo "result: passed" || exit 1
  echo "----------------------------"
  exit 0
fi

for spec in "$PWD"/tests/integration/specs/*; do
  echo "Running $spec"
  bash "$spec" && echo "result: passed" || exit 1
  echo "----------------------------"
done

if [ -f "$TEST_DIR"/workdir/output.txt ]; then
  echo "output:"
  cat "$TEST_DIR"/workdir/output.txt
fi

if [ -f $TEST_DIR/failed.log ] && [ -s $TEST_DIR/failed.log ]; then
  echo "Specs failed:"
  cat $TEST_DIR/failed.log
  rm -f $TEST_DIR/failed.log
  exit 1
else
  echo "All tests passed"
  exit 0
fi

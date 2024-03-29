#!/usr/bin/env bash

set -e

export TEST_DIR="$PWD"/tests/integration
export HELPERS="$TEST_DIR"/functions.sh
export WORKDIR="$TEST_DIR"/workdir

echo "Building funzzy"

rm -f "$TEST_DIR"/funzzy

cargo build --release --target-dir "$TEST_DIR"
cp "$TEST_DIR"/release/funzzy "$TEST_DIR"/funzzy

"$TEST_DIR"/funzzy --version
"$TEST_DIR"/funzzy --help


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

if [ -f "$TEST_DIR"/workdir/output.log ]; then
  echo "output:"
  cat "$TEST_DIR"/workdir/output.log
fi

echo "All tests passed"
exit 0

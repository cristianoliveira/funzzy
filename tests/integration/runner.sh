#!/usr/bin/env bash

set -e

export TEST_DIR="$PWD/tests/integration"
export HELPERS="$TEST_DIR/functions.sh"
export WORKDIR="$TEST_DIR/workdir"

echo "Building funzzy"
cargo build --release
cp target/release/funzzy $TEST_DIR/funzzy

PATH=$PATH:tests/integration

$TEST_DIR/funzzy --version
$TEST_DIR/funzzy --help

## if path received as argument, run only that test
if [ -n "$1" ]; then
  echo "Running only $1"
  bash "$1" && echo "result: passed" || exit 1
  echo "----------------------------"
  exit 0
fi

for spec in $TEST_DIR/specs/*; do
  echo "Running $spec"
  bash "$spec" && echo "result: passed" || exit 1
  echo "----------------------------"
done

if [ -f $TEST_DIR/workdir/output.txt ]; then
  echo "output:"
  cat $TEST_DIR/workdir/output.txt
fi

echo "All integration tests passed"
exit 0

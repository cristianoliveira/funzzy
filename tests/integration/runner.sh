#!/usr/bin/env bash

set -e

export TEST_DIR="$PWD/tests/integration"
export HELPERS="$TEST_DIR/functions.sh"
cargo build --release

cp target/release/funzzy $TEST_DIR/funzzy

PATH=$PATH:tests/integration

for spec in $TEST_DIR/specs/*; do
  echo "Running $spec"
  bash "$spec" && echo "result: passed" || exit 1
  echo "----------------------------"
done

echo "All integration tests passed"
exit 0

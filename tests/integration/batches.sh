#!/usr/bin/env bash

export TEST_DIR="$PWD"/tests/integration

START_INDEX=${1:-5}

for item in $(
  find "$TEST_DIR"/specs -name '*.sh' \
    | head -n "$START_INDEX" \
    | tail -n 5
); do
  bash "$TEST_DIR"/runner.sh "$item"
done

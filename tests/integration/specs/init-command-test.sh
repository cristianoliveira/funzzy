#!/usr/bin/env bash
source "$HELPERS"
test "it allows creating a new configuration file"

cd $WORKDIR
$TEST_DIR/funzzy init
ls -la
assert_file_contains "$WORKDIR/.watch.yaml" "Funzzy events file"
cd $TEST_DIR

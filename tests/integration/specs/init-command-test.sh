#!/usr/bin/env bash
source "$HELPERS"
test "it allows creating a new configuration file"

rm -f .watch.yaml

cd $WORKDIR
funzzy init
assert_file_contains "$WORKDIR/.watch.yaml" "Funzzy events file"
cd $TESTDIR

#!/usr/bin/env bash
source "$HELPERS"
test "init command"

rm -f .watch.yaml

funzzy init

wait_for_file .watch.yaml

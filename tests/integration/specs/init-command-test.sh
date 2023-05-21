#!/usr/bin/env bash
source "$HELPERS"
test "it allows creating a new configuration file"

rm -f .watch.yaml

funzzy init

wait_for_file .watch.yaml

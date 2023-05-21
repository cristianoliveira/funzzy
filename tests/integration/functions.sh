#!/usr/bin/env bash
#

function test() {
  echo "test: $1"
  mkdir workdir
}

function cleanup() {
  rm -rf workdir
  echo "kill funzzy $FUNZZY_PID"
  kill "$FUNZZY_PID"
}

function assert_equal() {
  if [ "$1" != "$2" ]; then
    echo "ERROR: $1 != $2"
    exit 1
  fi
}

function assert_file_contains() {
  local passed=0
  for i in {1..10}
  do
    if grep -q "$2" "$1"; then
      passed=1
      break
    fi
    echo "Attempt $i..."
    sleep 1
  done

  if [ $passed -eq 0 ]; then
    echo "ERROR: file $1 does not contain $2"
    echo "file content:"
    echo "$(cat $1)"
    exit 1
  fi
}

function wait_for_file() {
  local file_exists=0
  for i in {1..10}
  do
    if [ -s "$1" ]
    then
      file_exists=1
      break
    fi
    echo "Waiting for $1..."
    sleep 1
  done

  if [ $file_exists -eq 0 ]; then
    echo "ERROR: file $1 does not exist"
    exit 1
  fi
}

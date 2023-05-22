#!/usr/bin/env bash
#

function test() {
  rm -rf "$TEST_DIR/workdir"
  echo "test: $1"
  mkdir "$TEST_DIR/workdir"
}

function cleanup() {
  echo "kill funzzy $FUNZZY_PID"
  kill "$FUNZZY_PID"
}

function assert_equal() {
  if [ "$1" != "$2" ]; then
    echo "ERROR: $1 != $2"
    exit 1
  fi
}

function assert_file_content_at() {
  local success=0
  for i in {1..10}
  do
    if sed "$3!d" $1 | grep -q "$2"; then
      success=1
      break
    else
      echo "Expected: $2"
      echo "Content:"
      sed "$3!d" "$1"
      echo "Attempt failed: file $1 does not contain $2 at line $3"
      echo "Attempt $i..."
      sleep 5
    fi
  done

  if [ $success -eq 0 ]; then
    echo "ERROR: file $1 does not contain $2"
    echo "file content:"
    echo cat $1
    exit 1
  fi
}

function assert_file_contains() {
  local success=0
  for i in {1..10}
  do
    if grep -q "$2" "$1"; then
      success=1
      break
    else
      echo "Attempt failed: file $1 does not contain $2"
      echo "Attempt $i..."
      sleep 5
    fi
  done

  if [ $success -eq 0 ]; then
    echo "ERROR: file $1 does not contain $2"
    echo "file content:"
    echo "$(cat $1)"
    exit 1
  fi
}

function assert_file_not_contains() {
  local success=0
  for i in {1..10}
  do
    if grep -q "$2" "$1"; then
      echo "Attempt failed: file $1 does contain $2"
      echo "Attempt $i..."
      sleep 5
    else
      success=1
      break
    fi
  done

  if [ $success -eq 0 ]; then
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
    sleep 5
  done

  if [ $file_exists -eq 0 ]; then
    echo "ERROR: file $1 does not exist"
    exit 1
  fi
}

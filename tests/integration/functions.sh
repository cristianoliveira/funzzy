#!/usr/bin/env bash
#

TEST_REFERENCE=""

function test() {
  rm -rf "$TEST_DIR/workdir"
  TEST_REFERENCE="$1"
  echo "test: $1"
  mkdir "$TEST_DIR/workdir"
}

function cleanup() {
  echo "kill funzzy $FUNZZY_PID"
  # https://github.com/drbeco/killgracefully
  for SIG in 2 9 ; do echo $SIG ; kill -$SIG $FUNZZY_PID || break ; sleep 2 ; done
}

function assert_equal() {
  if [ "$1" != "$2" ]; then
    echo "ERROR: $1 != $2"
    report_failure
  fi
}

function report_failure() {
  echo "Failed: $TEST_REFERENCE"
  cleanup
  exit 1
}

function assert_file_content_at() {
  local success=0
  for i in {1..5}
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
    echo "ERROR: file $1 does not contain $2" echo "file content:"
    echo "file content:"
    cat $1
    report_failure
  fi
}

function assert_file_occurrencies() {
  local success=0
  for i in {1..5}
  do
    occurrencies=$(grep -o "$2" "$1" | wc -l)
    echo  "occurrencies of '$2': $occurrencies"
    if [ $occurrencies -eq $3 ]; then
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
    echo "final output content:"
    cat $1
    report_failure
  fi
}

function assert_file_contains() {
  local success=0
  for i in {1..5}
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
    echo "final output content:"
    cat $1
    report_failure
  fi
}

function assert_file_not_contains() {
  local success=0
  for i in {1..5}
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
    echo "final output content:"
    cat "$1"
    report_failure
  fi
}

function wait_for_file() {
  local file_exists=0
  for i in {1..5}
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
    report_failure
  fi
}

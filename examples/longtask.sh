#!/usr/bin/env bash

# Simulate a long task and count time
count=0
echo "Started task $1 $2"
while true; do
    echo "Long task running... $count"
    count=$((count+1))
    sleep 3

    if [ $count -eq "$2" ]; then
        echo "Task $1 $2 finished"
        break
    fi
done

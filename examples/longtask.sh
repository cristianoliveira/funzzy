#!/usr/bin/env bash

# Check if a signal is received
trap "echo '>> SIGINT received'; exit" SIGINT
trap "echo '>> SIGTERM received'; exit" SIGTERM

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

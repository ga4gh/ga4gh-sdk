#!/bin/sh
# TES site check for the ga4gh-cli image (see README.md in this directory).
#
# Submits one disposable TES task, waits until it ends and prints its FULL
# view. The task is submitted exactly once; the script never resubmits it.
#
# stdout: "task_id=<id>", then the FULL view of the task as JSON, then
#         "result=<state>".
# stderr: progress ("state=<state>") and errors.
# Exit status: 0 if the task is COMPLETE; 1 if a request fails, the task ends
# in another state or the time limit is reached (the task is then canceled).
#
# Environment:
#   TASK_FILE        TES task JSON (default: task.json next to this script)
#   TIMEOUT_SECONDS  time limit for the task to end (default: 600)
#   POLL_SECONDS     interval between status requests (default: 5)
set -eu

task_file=${TASK_FILE:-$(dirname "$0")/task.json}
timeout_seconds=${TIMEOUT_SECONDS:-600}
poll_seconds=${POLL_SECONDS:-5}

for value in "$timeout_seconds" "$poll_seconds"; do
    case $value in
        '' | *[!0-9]*)
            echo "TIMEOUT_SECONDS and POLL_SECONDS must be whole numbers, not '$value'" >&2
            exit 1
            ;;
    esac
done

task_id=$(ga4gh-cli tes create "$task_file")
printf 'task_id=%s\n' "$task_id"

deadline=$(($(date +%s) + timeout_seconds))
while :; do
    state=$(ga4gh-cli tes status "$task_id")
    echo "state=$state" >&2
    case $state in
        QUEUED | INITIALIZING | RUNNING | PAUSED) ;;
        *) break ;;
    esac
    if [ "$(date +%s)" -ge "$deadline" ]; then
        echo "Task $task_id did not end within ${timeout_seconds}s; canceling it" >&2
        ga4gh-cli tes cancel "$task_id" >&2 || true
        state=TIMEOUT
        break
    fi
    sleep "$poll_seconds"
done

# Captured, so that the JSON always ends with a newline
full_view=$(ga4gh-cli tes get "$task_id" FULL)
printf '%s\n' "$full_view"
printf 'result=%s\n' "$state"
[ "$state" = COMPLETE ]

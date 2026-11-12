#!/bin/sh
# TES site check for the ga4gh-cli image (see README.md in this directory).
#
# Submits one disposable TES task, waits until it ends and prints its FULL
# view. The task is submitted exactly once; the script never resubmits it.
#
# stdout: "task_id=<id>", then the FULL view of the task as JSON, then
#         "result=<state>". Instead of a TES state, the result is TIMEOUT
#         after the time limit, REQUEST_FAILED if a request after the
#         submission failed and INTERRUPTED after SIGINT or SIGTERM.
# stderr: progress ("state=<state>") and errors.
# Exit status: 0 if the task is COMPLETE, otherwise 1: the submission or a
# later request failed, the task ended in another state, or the time limit
# was reached. If the script stops waiting before the task ends, it cancels
# the task.
#
# Environment:
#   TASK_FILE        TES task JSON (default: task.json next to this script)
#   TIMEOUT_SECONDS  time limit for the task to end (default: 600)
#   POLL_SECONDS     interval between status requests (default: 5)
set -u

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

if ! task_id=$(ga4gh-cli tes create "$task_file"); then
    echo 'Could not submit the task' >&2
    exit 1
fi
printf 'task_id=%s\n' "$task_id"

# Cancels the task; prints the result given as $1 and exits 1
give_up() {
    echo "$2; canceling task $task_id" >&2
    ga4gh-cli tes cancel "$task_id" >&2 || echo "Could not cancel task $task_id" >&2
    printf 'result=%s\n' "$1"
    exit 1
}
trap 'give_up INTERRUPTED "Stopped by a signal"' INT TERM

deadline=$(($(date +%s) + timeout_seconds))
while :; do
    if ! state=$(ga4gh-cli tes status "$task_id"); then
        give_up REQUEST_FAILED 'Could not get the task state'
    fi
    echo "state=$state" >&2
    case $state in
        QUEUED | INITIALIZING | RUNNING | PAUSED) ;;
        *) break ;;
    esac
    if [ "$(date +%s)" -ge "$deadline" ]; then
        echo "Task $task_id did not end within ${timeout_seconds}s; canceling it" >&2
        ga4gh-cli tes cancel "$task_id" >&2 || echo "Could not cancel task $task_id" >&2
        state=TIMEOUT
        break
    fi
    sleep "$poll_seconds"
done
trap - INT TERM

# Captured, so that the JSON always ends with a newline
if full_view=$(ga4gh-cli tes get "$task_id" FULL); then
    printf '%s\n' "$full_view"
else
    echo "Could not get the FULL view of task $task_id" >&2
    if [ "$state" = COMPLETE ]; then
        state=REQUEST_FAILED
    fi
fi
printf 'result=%s\n' "$state"
[ "$state" = COMPLETE ]

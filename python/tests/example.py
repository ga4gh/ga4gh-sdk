"""Create, poll, and inspect a TES task with Python objects.

Run from ``python/`` with Funnel and Docker, or set ``GA4GH_TES_URL`` to
another TES API root.
"""

import os
import time

from GA4GH import Configuration, ListTasksParams, TES, TesState

TERMINAL_STATES = {
    TesState.COMPLETE,
    TesState.EXECUTOR_ERROR,
    TesState.SYSTEM_ERROR,
    TesState.CANCELED,
    TesState.PREEMPTED,
}


def main() -> None:
    config = Configuration(
        os.environ.get("GA4GH_TES_URL", "http://localhost:8000/ga4gh/tes/v1")
    )
    tes = TES(config)
    task = tes.create(
        {
            "name": "hello",
            "executors": [{"image": "alpine", "command": ["echo", "hello"]}],
        }
    )
    print("TASK ID:", task.id)

    state = task.status()
    for _ in range(5):
        print("STATE:  ", state)
        if state in TERMINAL_STATES:
            break
        time.sleep(1)
        state = task.status()

    if state not in TERMINAL_STATES:
        print("Canceling task...")
        task.cancel()

    details = tes.get("FULL", task.id)
    print("TASK:   ", details.to_dict())

    print(f"\n{'TASK ID':<25} {'STATE':<15}")
    for listed in tes.list(ListTasksParams(page_size=10)).tasks:
        print(f"{listed.id or '':<25} {listed.state:<15}")


if __name__ == "__main__":
    main()

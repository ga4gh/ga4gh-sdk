"""Run a workflow file on a WES service and print the run's details.

Run from ``python/``:
``python examples/wes_run.py <wes-api-root> <workflow-file> [<type> <version>]``.
The workflow file is uploaded as a workflow attachment; the type and version
default to CWL v1.2.
"""

import json
import sys
import time
from pathlib import Path

from GA4GH import WES, Configuration, WesState

TERMINAL_STATES = {
    WesState.COMPLETE,
    WesState.EXECUTOR_ERROR,
    WesState.SYSTEM_ERROR,
    WesState.CANCELED,
    WesState.PREEMPTED,
}

USAGE = "usage: python examples/wes_run.py <wes-api-root> <workflow-file> [<type> <version>]"


def main(argv: list[str]) -> None:
    if len(argv) not in (3, 5):
        raise SystemExit(USAGE)
    workflow = Path(argv[2])
    workflow_type, workflow_type_version = argv[3:5] or ("CWL", "v1.2")
    wes = WES(Configuration(argv[1]))
    run = wes.run(
        {
            "workflow_type": workflow_type,
            "workflow_type_version": workflow_type_version,
            "workflow_url": workflow.name,
            "workflow_params": {},
        },
        {workflow.name: workflow},
    )
    print("RUN ID:", run.id)

    state = run.status()
    for _ in range(10):
        print("STATE: ", state)
        if state in TERMINAL_STATES:
            break
        time.sleep(1)
        state = run.status()

    print(json.dumps(run.log().to_dict(), indent=2))


if __name__ == "__main__":
    main(sys.argv)

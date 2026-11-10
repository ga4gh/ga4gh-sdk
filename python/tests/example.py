"""Submit the checked-in TES task to a local Funnel server."""

import os
from pathlib import Path

from GA4GH import Configuration, TES


config = Configuration(os.environ.get("GA4GH_TES_URL", "http://localhost:8000/ga4gh/tes/v1"))
tes = TES(config)

task_file = Path(__file__).resolve().parents[2] / "examples" / "tes-hello.json"
task = tes.create(str(task_file))
print(f"Created task {task.id}: {task.status()}")

print("Tasks:")
for listed_task in tes.list().tasks:
    print(f"{listed_task.id}: {listed_task.status}")

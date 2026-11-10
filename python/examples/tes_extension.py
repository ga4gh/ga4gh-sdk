"""Example of extending TES for a service-specific provenance endpoint.

Run from ``python/`` with a service that implements the extra endpoint:
``python examples/tes_extension.py <tes-api-root> <task-id>``.
"""

import json
import sys
from urllib.parse import quote

from GA4GH import Configuration, TES


class ProvenanceTES(TES):
    """Add one endpoint while reusing the SDK's configuration and transport."""

    def provenance(self, task_id):
        segment = quote(task_id, safe="")
        body = self.transport.get(f"tasks/{segment}/provenance")
        return json.loads(body)


if __name__ == "__main__":
    if len(sys.argv) != 3:
        raise SystemExit("usage: python examples/tes_extension.py <tes-api-root> <task-id>")
    client = ProvenanceTES(Configuration(sys.argv[1]))
    print(json.dumps(client.provenance(sys.argv[2]), indent=2))

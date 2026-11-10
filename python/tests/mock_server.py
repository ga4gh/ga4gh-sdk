"""A recording TES- and WES-like HTTP mock in a separate process.

It prints its port on the first line of stdout. ``GET /__requests__`` returns
the requests received so far. The process keeps tests isolated from Python
runtime and GIL changes in the bindings.
"""

import base64
import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlsplit

SERVICE_INFO = {
    "id": "org.example.tes",
    "name": "Example TES",
    "type": {"group": "org.ga4gh", "artifact": "tes", "version": "1.1.0"},
    "organization": {"name": "Example", "url": "https://example.org"},
    "version": "1.0.0",
}

TASK = {
    "id": "task-1",
    "state": "COMPLETE",
    "name": "hello",
    "executors": [{"image": "alpine", "command": ["echo", "hello"]}],
    "tags": {"gateway.plugin": "example"},
    "extension_field": {"nested": [1, 2, 3]},
}

ROUTES = {
    ("GET", "/api/service-info"): (200, SERVICE_INFO),
    ("GET", "/api/tasks/task-1"): (200, TASK),
    ("GET", "/api/tasks"): (200, {"tasks": [TASK], "next_page_token": "next"}),
    ("GET", "/api/tasks/task-1/provenance"): (200, {"events": ["created"]}),
    ("GET", "/api/tasks/bad"): (200, {"id": "bad", "state": "NOT_A_STATE"}),
    ("POST", "/api/tasks"): (200, {"id": "task-1"}),
    ("POST", "/api/runs"): (200, {"run_id": "run-1"}),
    ("POST", "/api/tasks/task-1:cancel"): (200, {}),
    ("PUT", "/api/plugins/p1"): (200, {"updated": True}),
    ("DELETE", "/api/plugins/p1"): (200, {"deleted": True}),
}

REQUESTS = []


class Handler(BaseHTTPRequestHandler):
    """Records each request and answers it from `ROUTES`."""

    def _reply(self, status, payload):
        data = json.dumps(payload).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def _handle(self):
        url = urlsplit(self.path)
        if url.path == "/__requests__":
            self._reply(200, REQUESTS)
            return
        length = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(length) if length else b""
        REQUESTS.append(
            {
                "method": self.command,
                "path": url.path,
                "query": parse_qs(url.query, keep_blank_values=True),
                "headers": dict(self.headers),
                "body": base64.b64encode(body).decode(),
            }
        )
        self._reply(*ROUTES.get((self.command, url.path), (404, {"msg": "not found"})))

    do_GET = do_POST = do_PUT = do_DELETE = _handle

    def log_message(self, *args):
        pass


# A WES below `/wes`.
WES_SERVICE_INFO = {
    **SERVICE_INFO,
    "id": "org.example.wes",
    "name": "Example WES",
    "type": {"group": "org.ga4gh", "artifact": "wes", "version": "1.1.0"},
}

RUN_REQUEST = {
    "workflow_type": "CWL",
    "workflow_type_version": "v1.2",
    "workflow_url": "main.cwl",
    "workflow_params": {"message": "hello"},
    "tags": {"project": "demo"},
}

RUN_LOG = {
    "run_id": "run-1",
    "state": "COMPLETE",
    "request": RUN_REQUEST,
    "run_log": {"name": "hello", "exit_code": 0},
    "task_logs_url": "runs/run-1/tasks",
    "outputs": {"out": "hello"},
    "extension_field": {"nested": [1, 2, 3]},
}

RUN_SUMMARY = {
    "run_id": "run-1",
    "state": "COMPLETE",
    "start_time": "2026-01-01T00:00:00Z",
    "tags": {"project": "demo"},
    "extension_field": True,
}

TASK_LOG = {
    "id": "task-1",
    "name": "echo",
    "cmd": ["echo", "hello"],
    "exit_code": 0,
    "tes_uri": "https://tes.example/v1/tasks/1",
    "extension_field": "kept",
}

ROUTES.update(
    {
        ("GET", "/wes/service-info"): (200, WES_SERVICE_INFO),
        ("POST", "/wes/runs"): (200, {"run_id": "run-1"}),
        ("GET", "/wes/runs"): (200, {"runs": [RUN_SUMMARY], "next_page_token": "next"}),
        ("GET", "/wes/runs/run-1"): (200, RUN_LOG),
        ("GET", "/wes/runs/run-1/status"): (200, {"run_id": "run-1", "state": "COMPLETE"}),
        ("POST", "/wes/runs/run-1/cancel"): (200, {"run_id": "run-1"}),
        ("GET", "/wes/runs/run-1/tasks"): (200, {"task_logs": [TASK_LOG], "next_page_token": "next"}),
        ("GET", "/wes/runs/run-1/tasks/task-1"): (200, TASK_LOG),
        ("GET", "/wes/runs/no-state"): (200, {"run_id": "no-state"}),
        ("GET", "/wes/runs/bad"): (200, {"run_id": "bad", "state": "NOT_A_STATE"}),
        # A WES 0.2.0 server (GA4GH Starter Kit) lists runs as a bare array.
        ("GET", "/starter-kit/service-info"): (200, WES_SERVICE_INFO),
        ("GET", "/starter-kit/runs"): (200, [RUN_SUMMARY]),
    }
)

if __name__ == "__main__":
    httpd = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    print(httpd.server_address[1], flush=True)
    httpd.serve_forever()

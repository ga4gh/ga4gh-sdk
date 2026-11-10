"""A recording TES-like HTTP mock in a separate process.

It prints its port on the first line of stdout. ``GET /__requests__`` returns
the requests received so far. The process keeps tests isolated from Python
runtime and GIL changes in the bindings. ``GET /api/tasks/slow-<seconds>``
answers with a running task of that ID after the given delay, for tests of
concurrent and cancelled calls, as does ``GET /slow-<seconds>/service-info``
with the service info; if the client closes the connection before,
``GET /__disconnects__`` lists the request's path.
"""

import base64
import json
import select
import socket
import time
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
    # A TES whose other endpoints all fail.
    ("GET", "/broken/service-info"): (200, SERVICE_INFO),
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
DISCONNECTS = []


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
        if url.path == "/__disconnects__":
            self._reply(200, DISCONNECTS)
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
        if self.command == "GET" and url.path.startswith("/api/tasks/slow-"):
            task_id = url.path.rsplit("/", 1)[1]
            if self._client_left_within(float(task_id.removeprefix("slow-"))):
                DISCONNECTS.append(url.path)
                return
            self._reply(200, {**TASK, "id": task_id, "state": "RUNNING"})
            return
        if url.path.startswith("/slow-") and url.path.endswith("/service-info"):
            if self._client_left_within(float(url.path.split("/")[1].removeprefix("slow-"))):
                DISCONNECTS.append(url.path)
                return
            self._reply(200, SERVICE_INFO)
            return
        self._reply(*ROUTES.get((self.command, url.path), (404, {"msg": "not found"})))

    def _client_left_within(self, seconds):
        """Waits up to `seconds`; tells whether the client closed the connection meanwhile."""
        deadline = time.monotonic() + seconds
        while (remaining := deadline - time.monotonic()) > 0:
            readable, _, _ = select.select([self.connection], [], [], min(remaining, 0.05))
            if readable:
                try:
                    if not self.connection.recv(1, socket.MSG_PEEK):
                        return True
                except ConnectionResetError:
                    return True
                time.sleep(min(remaining, 0.05))  # Pipelined data, not a close.
        return False

    do_GET = do_POST = do_PUT = do_DELETE = _handle

    def log_message(self, *args):
        pass


if __name__ == "__main__":
    httpd = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    print(httpd.server_address[1], flush=True)
    httpd.serve_forever()

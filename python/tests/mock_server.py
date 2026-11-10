"""A recording HTTP mock of a TES-like service, run as a separate process.

The bindings hold the GIL while a request runs, so the server can't run in a thread of the
test process. It prints its port on the first line of stdout. `GET /__requests__` returns
the requests received so far.
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


if __name__ == "__main__":
    httpd = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    print(httpd.server_address[1], flush=True)
    httpd.serve_forever()

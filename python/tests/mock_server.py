"""A recording TES-like HTTP mock in a separate process.

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

# A DRS below `/drs`. Replies may carry headers as a third item.
DRS_SERVICE_INFO = {
    **SERVICE_INFO,
    "id": "org.example.drs",
    "name": "Example DRS",
    "type": {"group": "org.ga4gh", "artifact": "drs", "version": "1.4.0"},
}

DRS_OBJECT = {
    "id": "obj-1",
    "name": "reads.bam",
    "self_uri": "drs://drs.example/obj-1",
    "size": 1024,
    "created_time": "2026-01-01T00:00:00Z",
    "mime_type": "application/octet-stream",
    "checksums": [{"checksum": "abc", "type": "sha-256"}],
    "access_methods": [
        {"type": "s3", "access_url": {"url": "s3://bucket/obj-1"}, "region": "us-east-1"},
        {"type": "https", "access_id": "https-1", "region": "eu-west-1"},
    ],
    "aliases": ["reads"],
    "extension_field": {"nested": [1, 2, 3]},
}

ACCESS_URL = {
    "url": "https://store.example/obj-1?signature=SECRET",
    "headers": ["Authorization: Bearer SECRET"],
    "extension_field": "kept",
}


class Sequence:
    """Answers with each reply in turn, then keeps answering with the last one."""

    def __init__(self, *replies):
        self.replies = list(replies)

    def __iter__(self):
        reply = self.replies.pop(0) if len(self.replies) > 1 else self.replies[0]
        return iter(reply)


PENDING = (202, {}, {"Retry-After": "1"})

ROUTES.update(
    {
        ("GET", "/drs/service-info"): (200, DRS_SERVICE_INFO),
        ("GET", "/drs/objects/obj-1"): (200, DRS_OBJECT),
        ("GET", "/drs/objects/obj-1/access/https-1"): (200, ACCESS_URL),
        ("GET", "/drs/objects/a%2Fb%20c"): (200, {**DRS_OBJECT, "id": "a/b c"}),
        ("GET", "/drs/objects/pending"): PENDING,
        ("GET", "/drs/objects/obj-1/access/pending"): PENDING,
        ("GET", "/drs/objects/later"): Sequence(PENDING, (200, {**DRS_OBJECT, "id": "later"})),
        ("GET", "/drs/objects/no-retry"): (202, {}),
        ("GET", "/drs/objects/bad"): (200, {"id": "bad"}),
        ("GET", "/drs/objects/obj-1/access/bad"): (200, ["not", "an", "object"]),
        ("POST", "/drs/objects"): (
            200,
            {
                "resolved_drs_object": [DRS_OBJECT, {"id": "invalid"}],
                "unresolved_drs_objects": [{"error_code": 404, "object_ids": ["missing"]}],
            },
        ),
        ("POST", "/drs/objects/access"): (
            200,
            {
                "resolved_drs_object_access_urls": [
                    {"drs_object_id": "obj-1", "drs_access_id": "https-1", **ACCESS_URL}
                ],
                "unresolved_drs_objects": [{"error_code": 403, "object_ids": ["obj-2"]}],
            },
        ),
        # A DRS whose bulk requests are still pending.
        ("GET", "/drs-pending/service-info"): (200, DRS_SERVICE_INFO),
        ("POST", "/drs-pending/objects"): (202, {}, {"Retry-After": "5"}),
    }
)

REQUESTS = []


class Handler(BaseHTTPRequestHandler):
    """Records each request and answers it from `ROUTES`."""

    def _reply(self, status, payload, headers=None):
        data = json.dumps(payload).encode()
        self.send_response(status)
        for name, value in (headers or {}).items():
            self.send_header(name, value)
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

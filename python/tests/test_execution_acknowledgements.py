"""Run the SDK in a bounded child; the HTTP server has an independent GIL."""
import json
import subprocess
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


def test_creation_acknowledgements_fail_once_and_preserve_opaque_id():
    opaque = '"opaque / % id"'
    replies = ['{}', '{"id":null}', '{"id":4}', '{"id":""}', 'invalid', json.dumps({'id': opaque})]
    posts = []
    gets = []

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def send(self, body, status=200):
            encoded = body.encode()
            self.send_response(status)
            self.send_header('Content-Length', str(len(encoded)))
            self.end_headers()
            self.wfile.write(encoded)

        def do_GET(self):
            gets.append(self.path)
            if self.path == '/service-info':
                self.send(json.dumps({'id': 'test', 'name': 'test', 'version': '1',
                    'organization': {'name': 'test', 'url': 'https://example.org'},
                    'type': {'group': 'org.ga4gh', 'artifact': 'tes', 'version': '1.1'}}))
            elif self.path == '/tasks/%22opaque%20%2F%20%25%20id%22?view=MINIMAL':
                self.send('{"state":"COMPLETE","executors":[]}')
            else:
                self.send('{}', 400)

        def do_POST(self):
            self.rfile.read(int(self.headers.get('Content-Length', 0)))
            posts.append(self.path)
            self.send(replies[len(posts) - 1] if len(posts) <= len(replies) else '{}')

    server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    server.daemon_threads = True
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    code = r'''
import sys
from ga4gh_sdk import Configuration, TES
client = TES(Configuration(sys.argv[1]))
for _ in range(5):
    try:
        client.create({'executors': []})
        raise AssertionError('accepted malformed acknowledgement')
    except RuntimeError as error:
        assert 'may have succeeded' in str(error), str(error)
task = client.create({'executors': []})
assert task.id == '"opaque / % id"', task.id
assert task.status() == 'COMPLETE'
'''
    try:
        result = subprocess.run([sys.executable, '-c', code, f'http://127.0.0.1:{server.server_port}'],
                                capture_output=True, text=True, timeout=15)
        assert result.returncode == 0, result.stdout + result.stderr
        assert posts == ['/tasks'] * len(replies)
        assert gets == ['/service-info', '/tasks/%22opaque%20%2F%20%25%20id%22?view=MINIMAL']
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=2)

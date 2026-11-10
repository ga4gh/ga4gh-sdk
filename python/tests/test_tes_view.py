"""TES views are validated before a request, and sent with their TES spelling."""
import json
import subprocess
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


def test_get_validates_and_sends_the_view():
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
            elif self.path.startswith('/tasks/task-1?view='):
                self.send('{"id":"task-1","executors":[]}')
            else:
                self.send('{}', 400)

    server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    server.daemon_threads = True
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    code = r'''
import sys
from GA4GH import Configuration, TES
client = TES(Configuration(sys.argv[1]))
for view in ['EVERYTHING', '', 'FULL VIEW']:
    try:
        client.get(view, 'task-1')
        raise AssertionError(f'accepted view {view!r}')
    except ValueError as error:
        assert 'MINIMAL, BASIC, or FULL' in str(error), str(error)
for view in ['MINIMAL', 'basic', ' Full ']:
    assert client.get(view, 'task-1').id == 'task-1'
'''
    try:
        result = subprocess.run([sys.executable, '-c', code, f'http://127.0.0.1:{server.server_port}'],
                                capture_output=True, text=True, timeout=15)
        assert result.returncode == 0, result.stdout + result.stderr
        # Invalid views never reach the server.
        assert gets == ['/service-info'] + [f'/tasks/task-1?view={v}' for v in ['MINIMAL', 'BASIC', 'FULL']]
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=2)

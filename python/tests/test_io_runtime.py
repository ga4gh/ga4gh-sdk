"""Bound each regression in a child: a GIL deadlock must not hang pytest."""
import json
import subprocess
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import pytest


@pytest.mark.parametrize('scenario', ['transport', 'clients', 'errors'])
def test_synchronous_io(scenario):
    result = subprocess.run(
        [sys.executable, str(Path(__file__).resolve()), scenario],
        capture_output=True, text=True, timeout=15,
    )
    assert result.returncode == 0, result.stdout + result.stderr


def exercise(scenario):
    from GA4GH import Configuration, ServiceInfo, TES, Task, Transport
    ports = []
    requests = []
    ticks = []
    stopped = threading.Event()

    class Handler(BaseHTTPRequestHandler):
        protocol_version = 'HTTP/1.1'

        def log_message(self, *args):
            pass

        def reply(self):
            ports.append(self.client_address[1])
            requests.append((self.command, self.path))
            self.rfile.read(int(self.headers.get('Content-Length', 0)))
            status = 200
            body = {}
            if self.path == '/slow':
                time.sleep(0.2)
            elif self.path == '/error':
                status = 503
            elif self.path == '/service-info':
                body = {'id': 'test', 'name': 'Test TES', 'type': {
                    'group': 'org.ga4gh', 'artifact': 'tes', 'version': '1.1.0'},
                    'organization': {'name': 'Test', 'url': 'https://example.org'},
                    'version': '1.0'}
            elif self.path == '/tasks' and self.command == 'POST':
                body = {'id': 'task-1'}
            elif self.path == '/tasks':
                body = {'tasks': []}
            elif self.path.endswith(':cancel'):
                body = {'status': 'CANCELED'}
            elif self.path.startswith('/tasks/'):
                body = {'id': 'task-1', 'state': 'COMPLETE', 'executors': []}
            payload = json.dumps(body).encode()
            self.send_response(status)
            self.send_header('Content-Type', 'application/json')
            self.send_header('Content-Length', str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)

        do_GET = do_POST = do_PUT = do_DELETE = reply

    server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    server.daemon_threads = True
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()

    def heartbeat():
        while not stopped.wait(0.005):
            ticks.append(1)

    pulse = threading.Thread(target=heartbeat, daemon=True)
    pulse.start()
    config = Configuration(f'http://127.0.0.1:{server.server_port}')
    transport = Transport(config)
    try:
        if scenario == 'transport':
            before = len(ticks)
            assert json.loads(transport.get('slow')) == {}
            assert len(ticks) - before >= 10
            for _ in range(3):
                assert json.loads(transport.get('ok')) == {}
            assert len(set(ports)) == 1, ports
            assert json.loads(transport.post('ok', '{}')) == {}
            assert json.loads(transport.put('ok', '{}')) == {}
            assert json.loads(transport.delete('ok')) == {}
            assert len(set(ports)) == 1, ports
        elif scenario == 'clients':
            assert 'Test TES' in ServiceInfo(config).get_service_info()
            tes = TES(config)
            task = tes.create('{"executors": []}')
            assert task.id == 'task-1'
            assert task.status() == 'Complete'
            assert json.loads(task.cancel())['status'] == 'CANCELED'
            assert Task('task-1', transport).status() == 'Complete'
            assert tes.get('FULL', 'task-1').id == 'task-1'
            assert tes.list().tasks == []
            # A client created on one Python thread remains usable on another.
            cross_thread = []

            def list_from_other_thread():
                try:
                    cross_thread.append(tes.list().tasks)
                except BaseException as error:
                    cross_thread.append(error)

            worker = threading.Thread(target=list_from_other_thread)
            worker.start()
            worker.join(timeout=2)
            assert not worker.is_alive()
            assert cross_thread == [[]], cross_thread
            assert requests.count(('POST', '/tasks')) == 1
            # TES create/get/list and its Task share their live transport pool.
            assert len(set(ports[2:5] + ports[6:8])) == 1, ports
        else:
            try:
                transport.get('error')
                raise AssertionError('HTTP error was accepted')
            except RuntimeError as error:
                assert '503' in str(error)
            try:
                transport.post('ok', '{')
                raise AssertionError('malformed JSON was accepted')
            except ValueError:
                pass
            assert json.loads(transport.get('ok')) == {}
            assert len(requests) == 2
    finally:
        stopped.set()
        server.shutdown()
        server.server_close()
        thread.join(timeout=2)
        pulse.join(timeout=2)


if __name__ == '__main__':
    exercise(sys.argv[1])

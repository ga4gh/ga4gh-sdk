"""Bound each regression in a child: a GIL deadlock must not hang pytest."""
import json
import subprocess
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import pytest


@pytest.mark.parametrize('scenario', ['transport', 'clients', 'errors', 'wes'])
def test_synchronous_io(scenario):
    result = subprocess.run(
        [sys.executable, str(Path(__file__).resolve()), scenario],
        capture_output=True, text=True, timeout=15,
    )
    assert result.returncode == 0, result.stdout + result.stderr


def wes_body(command, path):
    """The answer of the mock WES below `/wes`; starting a run takes 0.2 s."""
    # Inline rather than imported from mock_server.py: this child runs on its own.
    if path == '/wes/service-info':
        return {'id': 'test', 'name': 'Test WES', 'type': {
            'group': 'org.ga4gh', 'artifact': 'wes', 'version': '1.1.0'},
            'organization': {'name': 'Test', 'url': 'https://example.org'},
            'version': '1.0'}
    if path == '/wes/runs' and command == 'POST':
        time.sleep(0.2)
        return {'run_id': 'run-1'}
    if path == '/wes/runs':
        return {'runs': []}
    if path.endswith('/cancel'):
        return {'run_id': 'run-1'}
    if path.endswith('/status'):
        return {'run_id': 'run-1', 'state': 'COMPLETE'}
    if path.endswith('/tasks'):
        return {'task_logs': []}
    if '/tasks/' in path:
        return {'id': path.rsplit('/', 1)[1], 'name': 'task'}
    return {'run_id': 'run-1', 'state': 'COMPLETE'}


def exercise(scenario):
    from GA4GH import Configuration, ServiceInfo, TES, Task, TesState, Transport, WES, WesState
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
            if self.path.startswith('/drs/'):
                drs_reply(self)
                return
            if self.path == '/slow':
                time.sleep(0.2)
            elif self.path == '/error':
                status = 503
            elif self.path.startswith('/wes/'):
                body = wes_body(self.command, self.path)
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
        if scenario == 'drs':
            exercise_drs(server.server_port, ports, requests, ticks)
        elif scenario == 'transport':
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
            task = tes.create({'executors': []})
            assert task.id == 'task-1'
            assert task.status() is TesState.COMPLETE
            assert task.cancel() is None
            assert Task('task-1', transport).status() is TesState.COMPLETE
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
        elif scenario == 'wes':
            wes = WES(Configuration(f'http://127.0.0.1:{server.server_port}/wes'))
            before = len(ticks)
            run = wes.run({'workflow_type': 'CWL', 'workflow_type_version': 'v1.2',
                           'workflow_url': 'main.cwl', 'workflow_params': {}},
                          {'main.cwl': b'cwlVersion: v1.2'})
            assert len(ticks) - before >= 10
            assert run.id == 'run-1'
            assert run.status() is WesState.COMPLETE
            assert run.cancel() is None
            assert run.log().run_id == 'run-1'
            assert run.tasks().task_logs == []
            assert run.task('t1').id == 't1'
            assert wes.get('run-1').state is WesState.COMPLETE
            assert wes.list().runs == []
            assert requests.count(('POST', '/wes/runs')) == 1
            # The client and its run share their live transport pool.
            assert len(set(ports)) == 1, ports
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


def test_drs_synchronous_io():
    test_synchronous_io('drs')


DRS_PENDING = []


def drs_reply(handler):
    """Answers as a DRS below `/drs`; slow paths take 0.2 s, `later` is pending once."""
    # Inline rather than imported from mock_server.py: this child runs on its own.
    path = handler.path
    status, headers = 200, {}
    if path == '/drs/service-info':
        body = {'id': 'test', 'name': 'Test DRS', 'type': {
            'group': 'org.ga4gh', 'artifact': 'drs', 'version': '1.4.0'},
            'organization': {'name': 'Test', 'url': 'https://example.org'},
            'version': '1.0'}
    elif '/access/' in path:
        body = {'url': 'https://store.example/data'}
    elif path == '/drs/objects/later' and not DRS_PENDING:
        DRS_PENDING.append(path)
        status, headers, body = 202, {'Retry-After': '1'}, {}
    elif path == '/drs/objects' and handler.command == 'POST':
        time.sleep(0.2)
        body = {'resolved_drs_object': [drs_object('slow')]}
    else:
        if path == '/drs/objects/slow':
            time.sleep(0.2)
        body = drs_object(path.rsplit('/', 1)[1])
    payload = json.dumps(body).encode()
    handler.send_response(status)
    for name, value in headers.items():
        handler.send_header(name, value)
    handler.send_header('Content-Type', 'application/json')
    handler.send_header('Content-Length', str(len(payload)))
    handler.end_headers()
    handler.wfile.write(payload)


def drs_object(object_id):
    return {'id': object_id, 'self_uri': f'drs://drs.example/{object_id}', 'size': 1,
            'created_time': '2026-01-01T00:00:00Z', 'checksums': [],
            'access_methods': [{'type': 'https', 'access_id': 'a1'}]}


def exercise_drs(port, ports, requests, ticks):
    from GA4GH import Configuration, DRS, DrsResolver
    drs = DRS(Configuration(f'http://127.0.0.1:{port}/drs'))
    before = len(ticks)
    assert drs.get_object('slow').id == 'slow'
    assert len(ticks) - before >= 10
    before = len(ticks)
    assert drs.get_bulk_objects(['slow']).outcomes[0].is_success
    assert len(ticks) - before >= 10
    before = len(ticks)
    assert drs.resolve_object('later', timeout=10).id == 'later'
    # The wait for `Retry-After` releases the GIL, too.
    assert len(ticks) - before >= 50
    assert drs.get_access_url('obj-1', 'a1').url == 'https://store.example/data'
    resolver = DrsResolver().with_service('drs.example', drs.transport)
    assert resolver.access_plan('drs://drs.example/obj-1').method == 'https'
    assert requests.count(('POST', '/drs/objects')) == 1
    # After service-info discovery, the client and the resolver share their connection
    # pool. The client doesn't read the body of a pending answer, so it opens a new
    # connection after one.
    assert len(set(ports[1:4])) == 1, ports
    assert len(set(ports[4:])) == 1, ports


if __name__ == '__main__':
    exercise(sys.argv[1])

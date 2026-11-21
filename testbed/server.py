#!/usr/bin/env python3
"""Synthetic, nonexecuting GA4GH scenario API. Python standard library only."""
import argparse
import base64
import hashlib
import hmac
import json
import secrets
import socket
import threading
import time
from email import policy
from email.parser import BytesParser
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, unquote, urlsplit

ROOT = Path(__file__).resolve().parent
BINARY = bytes(range(256)) + b'\x00GA4GH\xff\n'
TERMINAL = {'COMPLETE', 'EXECUTOR_ERROR', 'SYSTEM_ERROR', 'CANCELED', 'CANCELED'}
VERSION = '0.1.1'
MAX_BODY = 1024 * 1024

def digest(data):
    return hashlib.sha256(data).hexdigest()

def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':')).encode()

class Problem(Exception):
    def __init__(self, status, message):
        self.status, self.message = status, message

class Session:
    def __init__(self, seed, ttl):
        self.token = secrets.token_urlsafe(32)
        self.seed = seed
        self.expires = time.monotonic() + ttl
        self.lock = threading.RLock()
        self.reset()

    def reset(self):
        self.tick = 0
        self.tasks, self.runs = {}, {}
        self.events, self.inputs = [], []
        self.faults = {}
        self.artifact_expires = 10
        self.journal('reset', {'seed': self.seed})

    def journal(self, event, data):
        self.events.append({'sequence': len(self.events), 'tick': self.tick, 'event': event, 'data': data})

    def new(self, kind, request):
        records = self.tasks if kind == 'tes' else self.runs
        if len(self.tasks) + len(self.runs) >= 32:
            raise Problem(429, 'session task quota reached')
        ident = digest(f'{self.seed}:{kind}:{len(records)}'.encode())[:20]
        state = 'QUEUED'
        records[ident] = {'id': ident, 'state': state, 'request': request, 'cancel_pending': False}
        self.inputs.append({'service': kind, 'request': request})
        self.journal('accepted', {'service': kind, 'id': ident})
        return ident

class Server(ThreadingHTTPServer):
    daemon_threads = True
    def __init__(self, address, cors=(), public_base_url=None):
        super().__init__(address, Handler)
        if public_base_url is None:
            host = self.server_address[0]
            if host in ('', '0.0.0.0', '::'):
                host = '127.0.0.1'
            if ':' in host:
                host = '[' + host + ']'
            public_base_url = f'http://{host}:{self.server_port}'
        try:
            parsed = urlsplit(public_base_url)
            parsed.port  # Reject an invalid explicit port before serving requests.
            valid = (parsed.scheme in ('http', 'https') and parsed.hostname
                     and parsed.username is None and parsed.password is None
                     and parsed.path in ('', '/') and '?' not in public_base_url
                     and '#' not in public_base_url
                     and not any(char.isspace() or ord(char) < 32 for char in public_base_url))
        except ValueError:
            valid = False
        if not valid:
            self.server_close()
            raise ValueError('public base URL must be an HTTP(S) origin without credentials or a path')
        self.public_base_url = public_base_url.rstrip('/')
        self.sessions = {}
        self.guard = threading.Lock()
        self.cors = set(cors)

class Handler(BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'
    server_version = 'GA4GHScenario/' + VERSION
    def log_message(self, *_):
        pass  # URL paths, tokens and submitted inputs never enter access logs.

    def respond(self, status, value=None, headers=None, binary=None):
        body = binary if binary is not None else canonical(value)
        self.send_response(status)
        self.send_header('Content-Type', 'application/octet-stream' if binary is not None else 'application/json')
        self.send_header('Content-Length', str(len(body)))
        self.send_header('Cache-Control', 'no-store')
        self.send_header('Connection', 'close')
        origin = self.headers.get('Origin')
        if origin in self.server.cors:
            self.send_header('Access-Control-Allow-Origin', origin)
            self.send_header('Vary', 'Origin')
            self.send_header('Access-Control-Allow-Headers', 'Authorization, Content-Type')
            self.send_header('Access-Control-Allow-Methods', 'GET, POST, DELETE, OPTIONS')
        for k, v in (headers or {}).items():
            self.send_header(k, str(v))
        self.end_headers()
        try:
            self.wfile.write(body)
        except (BrokenPipeError, ConnectionResetError):
            pass

    def body(self):
        if self.headers.get('Transfer-Encoding'):
            raise Problem(400, 'use Content-Length for bounded synthetic requests')
        try:
            size = int(self.headers.get('Content-Length', '0'))
        except ValueError:
            raise Problem(400, 'invalid Content-Length')
        if size < 0 or size > MAX_BODY:
            raise Problem(413, 'synthetic request exceeds quota')
        data = self.rfile.read(size)
        if len(data) != size:
            raise Problem(400, 'truncated body')
        return data

    def json_body(self):
        try:
            value = json.loads(self.body() or b'{}')
        except (ValueError, UnicodeDecodeError):
            raise Problem(400, 'invalid JSON')
        if not isinstance(value, dict):
            raise Problem(400, 'expected object')
        return value

    def session(self, ident):
        with self.server.guard:
            session = self.server.sessions.get(ident)
        if not session or not hmac.compare_digest(self.headers.get('Authorization', ''), 'Bearer ' + session.token):
            raise Problem(403, 'session access denied')
        if time.monotonic() >= session.expires:
            raise Problem(410, 'session expired')
        return session

    def fault(self, s, key):
        self.injected_name = None
        fault = s.faults.get(key)
        if fault and fault['remaining']:
            fault['remaining'] -= 1
            name = fault['name']
            self.injected_name = name
            s.journal('fault', {'operation': key, 'name': name})
            if name in ('429', '503'):
                self.respond(int(name), {'msg': 'bounded injected fault'}, {'Retry-After': '1'})
                return True
            if name == 'malformed-json':
                self.respond(200, binary=b'{invalid-json')
                return True
        return False

    def multipart(self):
        content_type = self.headers.get('Content-Type', '')
        if not content_type.startswith('multipart/form-data;'):
            raise Problem(400, 'WES requires multipart/form-data')
        msg = BytesParser(policy=policy.default).parsebytes(b'Content-Type: ' + content_type.encode() + b'\r\nMIME-Version: 1.0\r\n\r\n' + self.body())
        if not msg.is_multipart():
            raise Problem(400, 'invalid multipart boundary')
        fields, files = {}, []
        for part in msg.iter_parts():
            name = part.get_param('name', header='content-disposition')
            data = part.get_payload(decode=True) or b''
            if name == 'workflow_attachment':
                filename = part.get_filename() or 'unnamed'
                if filename != 'fixture.cwl':
                    raise Problem(422, 'only the pack attachment filename is accepted')
                files.append({'filename': filename, 'size': len(data), 'sha256': digest(data), 'bytes': base64.b64encode(data).decode()})
            elif name in fields:
                raise Problem(400, 'duplicate multipart field')
            else:
                fields[name] = data.decode('utf-8')
        required = ('workflow_params', 'workflow_type', 'workflow_type_version', 'workflow_url')
        if any(k not in fields for k in required):
            raise Problem(400, 'missing required multipart field')
        for k in ('workflow_params', 'workflow_engine_parameters', 'tags'):
            if k in fields:
                fields[k] = json.loads(fields[k])
        if fields['workflow_url'] != 'fixture.cwl' or fields['workflow_type'] != 'CWL' or fields['workflow_type_version'] != 'v1.2' or fields['workflow_params'] != {'message': 'synthetic'}:
            raise Problem(422, 'only the pack synthetic workflow is accepted')
        fixture = (ROOT / 'pack/fixture.cwl').read_bytes()
        if len(files)>1 or any(base64.b64decode(f['bytes']) != fixture for f in files):
            raise Problem(422, 'only the pack fixture attachment is accepted')
        # Never persist caller-provided metadata or credentials in replay artifacts.
        return {k: fields[k] for k in required} | {'attachments': files}

    def service_info(self, kind):
        result = {'id': 'org.ga4gh.synthetic.' + kind, 'name': 'Synthetic scenario ' + kind, 'type': {'group': 'org.ga4gh', 'artifact': kind, 'version': {'tes':'1.1.0','wes':'1.1.0','drs':'1.4.0','trs':'2.0.1'}.get(kind,'1.0.0')}, 'organization': {'name': 'GA4GH scenario prototype', 'url': 'https://github.com/ga4gh/ga4gh-sdk'}, 'version': VERSION, 'environment': 'test'}
        if kind == 'wes':
            result.update(workflow_type_versions={'CWL': {'workflow_type_version': ['v1.2']}}, supported_wes_versions=['1.1.0'], supported_filesystem_protocols=['https'], workflow_engine_versions={'synthetic': {'workflow_engine_version': ['0.1.0']}}, default_workflow_engine_parameters=[], system_state_counts={}, auth_instructions_url='https://github.com/ga4gh/ga4gh-sdk', tags={})
        return result

    def control(self, parts, method, query):
        if parts == ['control', 'v1', 'sessions'] and method == 'POST':
            data = self.json_body()
            seed, ttl = data.get('seed', 1), data.get('ttl_seconds', 600)
            if type(seed) is not int or type(ttl) not in (int, float) or not 0 < ttl <= 3600:
                raise Problem(400, 'integer seed and TTL in (0,3600] required')
            with self.server.guard:
                for key in list(self.server.sessions):
                    if self.server.sessions[key].expires < time.monotonic():
                        del self.server.sessions[key]
                if len(self.server.sessions) >= 32:
                    raise Problem(429, 'server session quota reached')
                ident = secrets.token_hex(16)
                s = Session(seed, ttl)
                self.server.sessions[ident] = s
            self.respond(201, {'session_id': ident, 'token': s.token, 'seed': seed, 'ttl_seconds': ttl, 'base_path': '/sessions/' + ident})
            return
        if len(parts) < 4:
            raise Problem(404, 'control route missing')
        s = self.session(parts[3])
        with s.lock:
            action = parts[4:] or ['reset']
            if action == ['reset'] and method in ('POST', 'DELETE'):
                self.body()
                s.reset()
                self.respond(200, {'reset': True})
            elif action == ['journal'] and method == 'GET':
                self.respond(200, {'events': s.events, 'accepted_tasks': len(s.tasks), 'accepted_runs': len(s.runs)})
            elif action == ['replay'] and method == 'GET':
                pack = (ROOT / 'pack/scenarios.json').read_bytes()
                pins = json.loads((ROOT / 'pack/spec-pins.json').read_text())
                self.respond(200, {'format': 'ga4gh-scenario-replay-v1', 'server_version': VERSION, 'image_digest': self.server.image_digest, 'specs': pins, 'pack_sha256': digest(pack), 'seed': s.seed, 'inputs': s.inputs, 'events': s.events, 'accepted_tasks': len(s.tasks), 'accepted_runs': len(s.runs)})
            elif action == ['events'] and method == 'POST':
                data = self.json_body()
                kind = data.get('action')
                if kind == 'advance':
                    ticks = data.get('ticks', 1)
                    if type(ticks) is not int or not 0 <= ticks <= 10000:
                        raise Problem(400, 'invalid logical ticks')
                    s.tick += ticks
                    for service, records in [('tes', s.tasks), ('wes', s.runs)]:
                        for r in records.values():
                            if r['state'] in TERMINAL:
                                continue
                            if r['cancel_pending']:
                                r['state'] = 'CANCELED' if service == 'tes' else 'CANCELED'
                            elif r['state'] == 'QUEUED':
                                r['state'] = 'RUNNING'
                            else:
                                r['state'] = 'COMPLETE'
                    s.journal('advance', {'ticks': ticks})
                elif kind == 'fault':
                    name, operation, count = data.get('name'), data.get('operation'), data.get('count', 1)
                    if name not in ('429', '503', 'lost-response', 'cancel-completion-race', 'malformed-json') or operation not in ('tes.create', 'tes.list', 'wes.create', 'wes.list', 'tes.cancel', 'wes.cancel', 'drs.get') or type(count) is not int or not 1 <= count <= 5:
                        raise Problem(400, 'unknown or unbounded fault')
                    if name == 'lost-response' and operation not in ('tes.create', 'wes.create') or name == 'cancel-completion-race' and operation not in ('tes.cancel', 'wes.cancel'):
                        raise Problem(400, 'fault unsupported for this operation')
                    s.faults[operation] = {'name': name, 'remaining': count}
                    s.journal('configure_fault', {'operation': operation, 'name': name, 'count': count})
                else:
                    raise Problem(400, 'unknown control action')
                self.respond(200, {'tick': s.tick})
            else:
                raise Problem(404, 'unknown control operation')

    def ga4gh(self, parts, method, query):
        if len(parts) < 5 or parts[3] != ('v2' if parts[2] == 'trs' else 'v1'):
            raise Problem(404, 'API root missing')
        s, service = self.session(parts[1]), parts[2]
        route = parts[4:]
        with s.lock:
            if route == ['service-info']:
                self.respond(200, self.service_info(service))
                return
            if service in ('tes', 'wes'):
                records = s.tasks if service == 'tes' else s.runs
                collection = 'tasks' if service == 'tes' else 'runs'
                if not route or route[0] != collection:
                    raise Problem(404, 'unknown collection')
                op = service + ('.create' if len(route) == 1 and method == 'POST' else '.list' if len(route) == 1 else '.cancel' if method == 'POST' else '.observe')
                if self.fault(s, op):
                    self.body() if method == 'POST' else None
                    return
                if len(route) == 1 and method == 'POST':
                    if service == 'tes':
                        request = self.json_body()
                        if request != {'executors': [{'image': 'synthetic', 'command': ['fixture']}]}:
                            raise Problem(422, 'only the synthetic pack task is accepted')
                    else:
                        request = self.multipart()
                    ident = s.new(service, request)
                    if self.injected_name == 'lost-response':
                        self.close_connection = True
                        self.connection.shutdown(socket.SHUT_RDWR)
                        self.connection.close()
                        return
                    self.respond(200, {'id' if service == 'tes' else 'run_id': ident})
                    return
                if len(route) == 1 and method == 'GET':
                    ids = list(records)
                    start = 0
                    token = query.get('page_token', [''])[0]
                    if token:
                        tokens = {digest(f'{s.seed}:{service}:page:{i}'.encode()): i for i in range(len(ids) + 1)}
                        if token not in tokens:
                            raise Problem(400, 'unknown opaque page token')
                        start = tokens[token]
                    try:
                        size = int(query.get('page_size', ['1'])[0])
                    except ValueError:
                        raise Problem(400, 'invalid page size')
                    if not 1 <= size <= 32:
                        raise Problem(400, 'page size out of bounds')
                    chosen = ids[start:start+size]
                    key = 'tasks' if service == 'tes' else 'runs'
                    data = {key: [{'id' if service == 'tes' else 'run_id': i, 'state': records[i]['state']} for i in chosen]}
                    if start + size < len(ids):
                        data['next_page_token'] = digest(f'{s.seed}:{service}:page:{start+size}'.encode())
                    self.respond(200, data)
                    return
                ident = route[1].removesuffix(':cancel')
                if ident not in records:
                    raise Problem(404, 'unknown execution')
                r = records[ident]
                cancel = route[1].endswith(':cancel') or route[2:] == ['cancel']
                if cancel and method == 'POST':
                    self.body()
                    if self.injected_name == 'cancel-completion-race':
                        r['state'] = 'COMPLETE'
                    elif r['state'] not in TERMINAL:
                        r['cancel_pending'] = True
                        if service == 'wes':
                            r['state'] = 'CANCELING'
                    s.journal('cancel', {'service': service, 'id': ident, 'observed_state': r['state']})
                    self.respond(200, {} if service == 'tes' else {'run_id': ident})
                    return
                if method != 'GET':
                    raise Problem(405, 'method not allowed')
                if service == 'tes':
                    self.respond(200, {'id': ident, 'state': r['state']})
                elif route[2:] == ['status']:
                    self.respond(200, {'run_id': ident, 'state': r['state']})
                else:
                    self.respond(200, {'run_id': ident, 'state': r['state'], 'request': {k: v for k,v in r['request'].items() if k != 'attachments'}, 'run_log': {'name': 'synthetic', 'cmd': ['fixture']}, 'task_logs': [], 'outputs': {'artifact': 'fixture.bin'}})
                return
            if service == 'drs' and route[0] == 'objects':
                if self.fault(s, 'drs.get'):
                    return
                if s.tick == 0:
                    self.respond(202, {'msg': 'fixture pending'}, {'Retry-After': '1'})
                elif route[1:] == ['fixture']:
                    self.respond(200, {'id': 'fixture', 'self_uri': 'drs://synthetic/fixture', 'size': len(BINARY), 'created_time': '2026-01-01T00:00:00Z', 'checksums': [{'type': 'sha-256', 'checksum': digest(BINARY)}], 'access_methods': [{'type': 'https', 'access_id': 'fixture-access'}]})
                elif route[1:] == ['fixture', 'access', 'fixture-access']:
                    if s.tick >= s.artifact_expires:
                        raise Problem(403, 'artifact access expired')
                    self.respond(200, {'url': self.server.public_base_url + '/sessions/' + parts[1] + '/drs/v1/artifacts/fixture.bin', 'headers': ['Authorization: Bearer ' + s.token]})
                else:
                    raise Problem(404, 'unknown fixture')
                return
            if service == 'drs' and route == ['artifacts', 'fixture.bin']:
                if s.tick >= s.artifact_expires:
                    raise Problem(403, 'artifact access expired')
                self.respond(200, binary=BINARY)
                return
            if service == 'trs' and route == ['tools', 'fixture', 'versions', '1', 'CWL', 'descriptor']:
                self.respond(200, {'content': (ROOT / 'pack/fixture.cwl').read_text(), 'checksum': [{'type':'sha-256','checksum':digest((ROOT / 'pack/fixture.cwl').read_bytes())}]})
                return
            raise Problem(404, 'unknown GA4GH fixture route')

    def handle_request(self):
        self.connection.settimeout(10)
        try:
            parsed = urlsplit(self.path)
            parts = [unquote(p) for p in parsed.path.split('/') if p]
            if self.command == 'OPTIONS':
                self.respond(204, binary=b'')
            elif parsed.path in ('/', '/health'):
                self.respond(200, {'service': 'GA4GH synthetic scenario API', 'version': VERSION, 'executes_containers': False, 'session_creation': 'POST /control/v1/sessions', 'pack': '/pack'})
            elif parsed.path == '/pack':
                self.respond(200, json.loads((ROOT / 'pack/scenarios.json').read_text()))
            elif parts[:2] == ['control', 'v1']:
                self.control(parts, self.command, parse_qs(parsed.query))
            elif parts[:1] == ['sessions']:
                self.ga4gh(parts, self.command, parse_qs(parsed.query))
            else:
                raise Problem(404, 'unknown route')
        except Problem as e:
            self.respond(e.status, {'msg': e.message})
        except (ValueError, UnicodeDecodeError, IndexError, KeyError, TypeError):
            self.respond(400, {'msg': 'malformed synthetic request'})
        self.close_connection = True

    do_GET = do_POST = do_DELETE = do_OPTIONS = handle_request

def make_server(host='127.0.0.1', port=0, cors=(), image_digest='unpackaged-source', public_base_url=None):
    server = Server((host, port), cors, public_base_url)
    server.image_digest = image_digest
    return server

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--host', default='127.0.0.1')
    parser.add_argument('--port', type=int, default=8815)
    parser.add_argument('--cors-origin', action='append', default=[])
    parser.add_argument('--image-digest', default='unpackaged-source')
    parser.add_argument('--public-base-url', help='trusted external HTTP(S) origin for DRS artifact URLs')
    args = parser.parse_args()
    server = make_server(args.host, args.port, args.cors_origin, args.image_digest, args.public_base_url)
    print(json.dumps({'listening': list(server.server_address), 'version': VERSION}), flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()

"""A synthetic native-WES profile fixture; never executes a submitted workflow."""
import email.policy
from email.parser import BytesParser
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import hashlib
import json
import threading


class Fixture:
    def __init__(self, workflow, public_hello=False, failure_body=None, failure_headers=None):
        self.workflow = workflow
        self.state = 'QUEUED'
        self.requests = []
        self.accepted = 0
        self.error = None
        fixture = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def reply(self, body, status=200):
                self.reply_bytes(json.dumps(body).encode(), status)

            def reply_bytes(self, data, status=200, headers=None):
                self.send_response(status)
                self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(data)))
                for key, value in (headers or {}).items():
                    self.send_header(key, value)
                self.end_headers()
                try:
                    self.wfile.write(data)
                except (BrokenPipeError, ConnectionResetError):
                    pass

            def authorized(self):
                if self.headers.get('Authorization') != 'Bearer synthetic-profile-token':
                    self.reply({'msg': 'unauthorized'}, 401)
                    return False
                fixture.requests.append([self.command, self.path])
                return True

            def do_GET(self):
                if not self.authorized():
                    return
                if self.path == '/ga4gh/wes/v1/service-info':
                    self.reply({'supported_wes_versions': ['1.0'],
                                'workflow_type_versions': {'NFL': {'workflow_type_version': ['DSL2']}},
                                'workflow_engine_versions': {'Nextflow': '26.10.5'}})
                elif self.path.endswith('/status'):
                    if fixture.state == 'QUEUED':
                        fixture.state = 'RUNNING'
                    self.reply({'run_id': 'synthetic-run', 'state': fixture.state})
                elif self.path.endswith('/synthetic-run'):
                    self.reply({'run_id': 'synthetic-run', 'state': fixture.state,
                                'run_log': {'name': 'synthetic-log', 'stdout': 'fixture-only'},
                                'outputs': {}})
                else:
                    self.reply({'msg': 'missing'}, 404)

            def do_POST(self):
                if not self.authorized():
                    return
                if self.path.endswith('/cancel'):
                    fixture.state = 'CANCELED'
                    self.reply({'run_id': 'synthetic-run'})
                    return
                if self.path != '/ga4gh/wes/v1/runs':
                    self.reply({'msg': 'missing'}, 404)
                    return
                body = self.rfile.read(int(self.headers['Content-Length']))
                content_type = self.headers.get('Content-Type', '')
                parsed = BytesParser(policy=email.policy.default).parsebytes(
                    ('Content-Type: '+content_type+'\r\nMIME-Version: 1.0\r\n\r\n').encode()+body)
                try:
                    assert parsed.is_multipart(), 'request must be real multipart'
                    parts = {}
                    attachment = None
                    for part in parsed.iter_parts():
                        name = part.get_param('name', header='content-disposition')
                        if name == 'workflow_attachment':
                            assert part.get_filename() == 'seqera.nf'
                            attachment = part.get_payload(decode=True)
                        else:
                            parts[name] = part.get_payload(decode=True).decode()
                    assert json.loads(parts['workflow_engine_parameters']) == {
                        'workspaceId': 'synthetic-workspace', 'computeEnvId': 'synthetic-compute'}
                    assert parts['workflow_type'] == 'NFL'
                    assert parts['workflow_type_version'] == 'DSL2'
                    if public_hello:
                        assert parts['workflow_url'] == 'https://github.com/nextflow-io/hello'
                        assert attachment is None
                        assert json.loads(parts['workflow_params']) == {}
                    else:
                        assert parts['workflow_url'] == 'seqera.nf'
                        assert attachment == fixture.workflow
                        assert json.loads(parts['workflow_params']) == {'delay': 30}
                    if failure_body is not None:
                        self.reply_bytes(failure_body, 400, failure_headers)
                    else:
                        fixture.accepted += 1
                        self.reply({'run_id': 'synthetic-run'})
                except (AssertionError, KeyError, ValueError) as exc:
                    fixture.error = str(exc)
                    self.reply({'msg': 'fixture assertion failed'}, 400)

        self.server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, *args):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join()

    @property
    def url(self):
        return f'http://127.0.0.1:{self.server.server_port}/ga4gh/wes/v1/'

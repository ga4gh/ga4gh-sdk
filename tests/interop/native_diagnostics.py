"""Opt-in loopback regressions for the actual native SDK example; never contacts cloud.

Run with `python3 tests/interop/native_diagnostics.py`; Rust is required unless
SEQERA_PROFILE_SDK_BIN selects an already built example.
"""
import json
import os
from pathlib import Path
import stat
import subprocess
import tempfile
import unittest

from offline_seqera import Fixture
import run


class NativeDiagnostics(unittest.TestCase):
    def execute(self, fixture, directory):
        token = directory/'token'
        token.write_text('synthetic-profile-token\n')
        token.chmod(0o600)
        env = os.environ.copy()
        env.update(SEQERA_PROFILE_API_ROOT=fixture.url,
                   SEQERA_PROFILE_TOKEN_FILE=str(token),
                   SEQERA_PROFILE_ACTION='launch-cancel',
                   SEQERA_PROFILE_ALLOW_LAUNCH='one-bounded-run',
                   SEQERA_PROFILE_WORKSPACE_ID='synthetic-workspace',
                   SEQERA_PROFILE_COMPUTE_ENV_ID='synthetic-compute',
                   SEQERA_PROFILE_DESCRIPTOR_VERSION='DSL2',
                   SEQERA_PROFILE_WORKFLOW=str(run.ROOT/'fixtures/seqera.nf'),
                   SEQERA_PROFILE_LEDGER=str(directory/'ledger.json'),
                   SEQERA_PROFILE_TIMEOUT_SECONDS='1')
        env.pop('SEQERA_PROFILE_WORKFLOW_MODE', None)
        result = subprocess.run(run.sdk_command(), cwd=run.REPO, env=env,
                                capture_output=True, text=True, timeout=180)
        self.assertEqual(result.returncode, 0, 'native example failed to build/execute')
        public = json.loads(result.stdout)
        return public, result.stdout+result.stderr

    def exercise_error(self, response, expected_status=400):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        path = Path(directory.name)
        fixture = Fixture((run.ROOT/'fixtures/seqera.nf').read_bytes(),
                          failure_body=response,
                          failure_headers={'X-Request-Id':'HEADER_PRIVATE_SENTINEL'})
        self.addCleanup(fixture.__exit__)
        fixture.__enter__()
        public, output = self.execute(fixture, path)
        ledger_path = path/'ledger.json'
        ledger = json.loads(ledger_path.read_text())
        self.assertEqual(ledger['state'], 'SUBMISSION_OUTCOME_UNKNOWN')
        if os.name == 'posix':
            self.assertEqual(stat.S_IMODE(ledger_path.stat().st_mode), 0o600)
        check = next(c for c in public['checks'] if c['id']=='launch')
        self.assertEqual(check['observed'], {'http_status':expected_status})
        junit = run.junit({'profiles':[{'id':'synthetic','evidence':'simulated_protocol',
                                      'checks':public['checks']}]})
        for marker in ['BODY_PRIVATE_SENTINEL','ACCOUNT_PRIVATE_SENTINEL',
                       'REQUEST_PRIVATE_SENTINEL','HEADER_PRIVATE_SENTINEL',
                       'synthetic-profile-token']:
            self.assertNotIn(marker, output+junit)
        return fixture, path, ledger, ledger_path.read_bytes()

    def test_http400_private_body_request_id_and_existing_ledger(self):
        body = json.dumps({'requestId':'REQUEST_PRIVATE_SENTINEL',
                           'message':'BODY_PRIVATE_SENTINEL ACCOUNT_PRIVATE_SENTINEL'}).encode()
        fixture, path, ledger, before = self.exercise_error(body)
        diagnostic = ledger['diagnostic']
        self.assertEqual(diagnostic['http_status'],400)
        self.assertEqual(diagnostic['body'],body.decode())
        self.assertFalse(diagnostic['body_truncated'])
        self.assertEqual(diagnostic['request_id'],'REQUEST_PRIVATE_SENTINEL')
        self.assertEqual(diagnostic['request_id_source'],'response_body')
        self.assertFalse(diagnostic['response_headers_available'])
        self.assertNotIn('synthetic-profile-token', json.dumps(diagnostic))
        public, _ = self.execute(fixture, path)
        self.assertTrue(any(c['outcome']=='harness_error' for c in public['checks']))
        self.assertEqual((path/'ledger.json').read_bytes(),before)
        self.assertEqual(sum(method=='POST' for method, _ in fixture.requests),1)

    def test_private_body_is_utf8_and_size_bounded(self):
        request_id = 'REQUEST_PRIVATE_SENTINEL' + 'q'*1000
        body = json.dumps({'request_id':request_id,
                           'message':'BODY_PRIVATE_SENTINEL '+('💚'*2500)},ensure_ascii=False).encode()
        _, _, ledger, serialized = self.exercise_error(body)
        diagnostic = ledger['diagnostic']
        self.assertEqual(diagnostic['body_bytes_available'],len(body))
        self.assertLessEqual(len(diagnostic['body'].encode()),4096)
        self.assertEqual(diagnostic['body_bytes_saved'],len(diagnostic['body'].encode()))
        self.assertTrue(body.decode().startswith(diagnostic['body']))
        self.assertTrue(diagnostic['body_truncated'])
        self.assertEqual(diagnostic['request_id'],request_id[:256])
        self.assertLessEqual(len(diagnostic['request_id'].encode()),256)
        self.assertLess(len(serialized),32768)

    def test_transport_oversize_retains_ambiguity_without_inventing_status_or_body(self):
        _, _, ledger, serialized = self.exercise_error(b'BODY_PRIVATE_SENTINEL'+b'x'*20000,
                                                      expected_status=None)
        diagnostic = ledger['diagnostic']
        self.assertEqual(diagnostic['kind'],'ambiguous_submission_error')
        self.assertIsNone(diagnostic['http_status'])
        self.assertIsNone(diagnostic['body'])
        self.assertIn('body limit',diagnostic['reason'])
        self.assertLessEqual(len(diagnostic['reason'].encode()),4096)
        self.assertLess(len(serialized),32768)


if __name__ == '__main__':
    unittest.main()

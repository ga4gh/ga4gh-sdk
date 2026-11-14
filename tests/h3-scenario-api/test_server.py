"""Exercise actual HTTP, sockets, multipart, replay, expiry and session boundaries."""
import concurrent.futures
import hashlib
import http.client
import importlib.util
import json
import sys
import threading
import time
import unittest
from pathlib import Path
from urllib.error import HTTPError
from urllib.request import Request, urlopen
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('scenario_server', ROOT / 'testbed/server.py')
api = importlib.util.module_from_spec(spec)
spec.loader.exec_module(api)

class ScenarioAPI(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.server = api.make_server(cors=['http://client.example'])
        cls.thread = threading.Thread(target=cls.server.serve_forever, daemon=True)
        cls.thread.start()
        cls.base = 'http://127.0.0.1:' + str(cls.server.server_port)

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()
        cls.server.server_close()
        cls.thread.join()

    def call(self, path, data=None, token=None, method=None, headers=None):
        h = headers or {}
        if token:
            h['Authorization'] = 'Bearer ' + token
        if isinstance(data, dict):
            data = json.dumps(data).encode()
            h['Content-Type'] = 'application/json'
        req = Request(self.base + path, data=data, headers=h, method=method)
        try:
            res = urlopen(req, timeout=5)
        except HTTPError as e:
            res = e
        with res:
            raw = res.read()
            value = json.loads(raw) if raw and res.headers.get('Content-Type') == 'application/json' else raw
            return res.status, value, res.headers

    def session(self, seed=123, ttl=600):
        status, s, _ = self.call('/control/v1/sessions', {'seed': seed, 'ttl_seconds': ttl})
        self.assertEqual(status, 201)
        return s

    def control(self, s, action):
        return self.call('/control/v1/sessions/' + s['session_id'] + '/events', action, s['token'])

    def journal(self, s, replay=False):
        return self.call('/control/v1/sessions/' + s['session_id'] + ('/replay' if replay else '/journal'), token=s['token'])[1]

    def tes_create(self, s):
        task = {'executors': [{'image': 'synthetic', 'command': ['fixture']}]}
        return self.call(s['base_path'] + '/tes/v1/tasks', task, s['token'])

    def multipart(self, s, version="v1.2", filename="fixture.cwl"):
        boundary = 'real-client-boundary'
        fields = json.loads((ROOT / 'testbed/pack/scenarios.json').read_text())['wes_request']
        fields['workflow_type_version']=version
        body = b''
        for name, value in fields.items():
            value = json.dumps(value) if isinstance(value, dict) else value
            body += f'--{boundary}\r\nContent-Disposition: form-data; name="{name}"\r\n\r\n{value}\r\n'.encode()
        body += f'--{boundary}\r\nContent-Disposition: form-data; name="workflow_attachment"; filename="{filename}"\r\nContent-Type: application/octet-stream\r\n\r\n'.encode()
        body += (ROOT / 'testbed/pack/fixture.cwl').read_bytes() + f'\r\n--{boundary}--\r\n'.encode()
        return self.call(s['base_path'] + '/wes/v1/runs', body, s['token'], headers={'Content-Type': 'multipart/form-data; boundary=' + boundary})

    def test_lifecycle_binary_paging_and_pending_access(self):
        s = self.session()
        ids = [self.tes_create(s)[1]['id'] for _ in range(3)]
        root = s['base_path']
        status, page, _ = self.call(root + '/tes/v1/tasks', token=s['token'])
        self.assertEqual(status, 200)
        self.assertEqual(page['tasks'][0]['id'], ids[0])
        self.assertNotEqual(page['next_page_token'], '1')
        second = self.call(root + '/tes/v1/tasks?page_token=' + page['next_page_token'], token=s['token'])[1]
        self.assertEqual(second['tasks'][0]['id'], ids[1])
        pending = self.call(root + '/drs/v1/objects/fixture', token=s['token'])
        self.assertEqual(pending[0], 202)
        self.assertEqual(pending[2]['Retry-After'], '1')
        self.control(s, {'action': 'advance'})
        self.assertEqual(self.call(root + '/tes/v1/tasks/' + ids[0], token=s['token'])[1]['state'], 'RUNNING')
        self.control(s, {'action': 'advance'})
        self.assertEqual(self.call(root + '/tes/v1/tasks/' + ids[0], token=s['token'])[1]['state'], 'COMPLETE')
        obj = self.call(root + '/drs/v1/objects/fixture', token=s['token'])[1]
        access = self.call(root + '/drs/v1/objects/fixture/access/fixture-access', token=s['token'])[1]
        absolute = urlsplit(access['url'])
        self.assertEqual(absolute.scheme, 'http')
        self.assertEqual(absolute.netloc, urlsplit(self.base).netloc)
        artifact_headers = dict(h.split(': ',1) for h in access['headers'])
        data = self.call(absolute.path, headers=artifact_headers)[1]
        self.assertEqual(data, api.BINARY)
        self.assertEqual(api.digest(data), obj['checksums'][0]['checksum'])
        self.control(s, {'action': 'advance', 'ticks': 8})
        self.assertEqual(self.call(absolute.path, headers=artifact_headers)[0], 403)

    def test_wes_real_multipart_and_cancellation_race(self):
        s = self.session()
        status, run, _ = self.multipart(s)
        self.assertEqual(status, 200)
        ident = run['run_id']
        self.control(s, {'action': 'fault', 'operation': 'wes.cancel', 'name': 'cancel-completion-race'})
        root = s['base_path'] + '/wes/v1/runs/' + ident
        self.assertEqual(self.call(root + '/status', token=s['token'])[1]['state'], 'QUEUED')
        self.assertEqual(self.call(root + '/cancel', b'', s['token'])[0], 200)
        self.assertEqual(self.call(root + '/status', token=s['token'])[1]['state'], 'COMPLETE')
        record = self.journal(s, True)['inputs'][0]['request']
        self.assertEqual(record['attachments'][0]['sha256'], api.digest((ROOT / 'testbed/pack/fixture.cwl').read_bytes()))
        self.assertEqual(self.call(s['base_path'] + '/wes/v1/runs', {'workflow_url':'not-multipart'}, s['token'])[0], 400)

    def test_trs_wrapper_content_and_version(self):
        s = self.session()
        root = s['base_path'] + '/trs/v2'
        info = self.call(root + '/service-info', token=s['token'])[1]
        self.assertEqual(info['type']['version'], '2.0.1')
        wrapper = self.call(root + '/tools/fixture/versions/1/CWL/descriptor', token=s['token'])[1]
        self.assertEqual(wrapper['content'].encode(), (ROOT/'testbed/pack/fixture.cwl').read_bytes())
        self.assertEqual(wrapper['checksum'][0]['checksum'],api.digest(wrapper['content'].encode()))
        self.assertNotIn('url', wrapper)

    def test_drs_access_url_ignores_untrusted_host(self):
        s = self.session()
        self.control(s, {'action': 'advance'})
        status, access, _ = self.call(
            s['base_path'] + '/drs/v1/objects/fixture/access/fixture-access',
            token=s['token'], headers={'Host': 'attacker.invalid'},
        )
        self.assertEqual(status, 200)
        self.assertEqual(urlsplit(access['url']).netloc, urlsplit(self.base).netloc)
        self.assertNotIn('attacker.invalid', json.dumps(access))

    def test_public_origin_configuration_and_wildcard_default(self):
        server = api.make_server(host='0.0.0.0')
        try:
            self.assertEqual(server.public_base_url, 'http://127.0.0.1:' + str(server.server_port))
        finally:
            server.server_close()
        server = api.make_server(public_base_url='https://api.example.org/')
        try:
            self.assertEqual(server.public_base_url, 'https://api.example.org')
        finally:
            server.server_close()
        for invalid in ('https://user:password@example.org/private',
                        'https://api.example.org:invalid', 'https://api.example.org?'):
            with self.assertRaises(ValueError):
                api.make_server(public_base_url=invalid)

    def test_unsupported_versions_and_arbitrary_replay_text_rejected(self):
        s=self.session()
        self.assertEqual(self.multipart(s,version="unsupported-secret-text")[0],422)
        self.assertEqual(self.multipart(s,filename="private-name-secret.cwl")[0],422)
        replay=self.journal(s,True)
        self.assertEqual(replay['accepted_runs'],0)
        text=json.dumps(replay)
        self.assertNotIn('unsupported-secret-text',text)
        self.assertNotIn('private-name-secret',text)

    def test_pinned_schema_dependencies_resolve_offline(self):
        import re
        pack=ROOT/'testbed/pack'
        remotes=json.loads((pack/'reference-map.json').read_text())
        pins=json.loads((pack/'spec-pins.json').read_text())
        expected={f'{service}.openapi.yaml': pins[service]['sha256'] for service in ('tes','wes','trs')}
        expected.update(pins['drs']['files'])
        expected[pins['service-info']['local_path']]=pins['service-info']['sha256']
        snapshot_files=(*pack.rglob('*.yaml'),*pack.rglob('*.md'))
        self.assertEqual({str(path.relative_to(pack)) for path in snapshot_files},set(expected))
        for name,sha256 in expected.items():
            self.assertEqual(hashlib.sha256((pack/name).read_bytes()).hexdigest(),sha256,name)
        for source in pack.rglob('*.yaml'):
            for ref in re.findall(r"['\"]?\$ref['\"]?\s*:\s*['\"]?([^'\"\s]+)",source.read_text()):
                path=ref.split('#')[0]
                if not path:continue
                target=pack/remotes[path] if path.startswith('http') else source.parent/path
                self.assertTrue(target.is_file(),str(target))

    def test_cancel_pending_then_terminal(self):
        s = self.session()
        ident = self.multipart(s)[1]['run_id']
        path = s['base_path'] + '/wes/v1/runs/' + ident
        self.call(path + '/cancel', b'', s['token'])
        self.assertEqual(self.call(path + '/status', token=s['token'])[1]['state'], 'CANCELING')
        self.control(s, {'action': 'advance'})
        self.assertEqual(self.call(path + '/status', token=s['token'])[1]['state'], 'CANCELED')

    def test_lost_response_count_and_replay_determinism(self):
        bundles=[]
        for _ in range(2):
            s=self.session(seed=5)
            self.control(s, {'action':'fault','operation':'tes.create','name':'lost-response','count':2})
            for _ in range(2):
                with self.assertRaises((http.client.RemoteDisconnected, ConnectionError)):
                    self.tes_create(s)
            self.assertEqual(self.tes_create(s)[0],200)
            self.assertEqual(self.journal(s)['accepted_tasks'],3)
            bundles.append(self.journal(s,True))
        self.assertEqual(bundles[0],bundles[1])
        encoded=json.dumps(bundles[0])
        self.assertNotIn(s['token'],encoded)
        self.assertNotIn(s['session_id'],encoded)
        self.assertNotIn('Bearer ' + s['token'],encoded)

    def test_concurrent_session_isolation_reset_and_expiry(self):
        a,b=self.session(),self.session()
        with concurrent.futures.ThreadPoolExecutor(max_workers=2)as pool:
            self.assertEqual([v[0] for v in pool.map(self.tes_create,[a,b])],[200,200])
        self.assertEqual(self.call(a['base_path']+'/tes/v1/tasks',token=b['token'])[0],403)
        self.assertEqual(self.call('/control/v1/sessions/'+a['session_id'],b'',b['token'])[0],403)
        self.assertEqual(self.journal(a)['accepted_tasks'],1)
        self.call('/control/v1/sessions/'+a['session_id'],b'',a['token'])
        self.assertEqual(self.journal(a)['accepted_tasks'],0)
        self.assertEqual(self.journal(b)['accepted_tasks'],1)
        short=self.session(ttl=0.01)
        time.sleep(0.03)
        self.assertEqual(self.call(short['base_path']+'/tes/v1/service-info',token=short['token'])[0],410)

    def test_bounded_faults_cors_and_input_boundary(self):
        s=self.session()
        self.control(s,{'action':'fault','operation':'wes.list','name':'503','count':2})
        root=s['base_path']+'/wes/v1/runs'
        self.assertEqual([self.call(root,token=s['token'])[0]for _ in range(3)],[503,503,200])
        status,_,headers=self.call('/health',headers={'Origin':'http://client.example'})
        self.assertEqual(headers['Access-Control-Allow-Origin'],'http://client.example')
        self.assertEqual(headers['Connection'],'close')
        self.assertNotIn('Access-Control-Allow-Origin',self.call('/health',headers={'Origin':'http://untrusted.example'})[2])
        self.assertEqual(self.control(s,{'action':'fault','operation':'tes.create','name':'503','count':6})[0],400)
        self.assertEqual(self.call(s['base_path']+'/tes/v1/tasks',{'executors':[{'command':['curl','https://private.invalid']}]},s['token'])[0],422)
        self.assertEqual(self.journal(s)['accepted_tasks'],0)

if __name__=='__main__':
    unittest.main(verbosity=2)

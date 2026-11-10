#!/usr/bin/env python3
"""Wrap pinned existing suites; classify evidence without asserting certification."""
import argparse
import collections
import contextlib
from datetime import datetime, timezone
import hashlib
import json
import os
import re
import signal
import time
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import urllib.request
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parent.parent
OUTCOMES = {'passed', 'version_mismatch', 'missing_fixture', 'harness_error',
            'unsupported', 'not_run', 'observed_failure'}
EVIDENCE = {'real_execution', 'real_service', 'simulated_protocol', 'not_run'}


def item(id, outcome, detail, **extra):
    if outcome not in OUTCOMES:
        raise ValueError('invalid outcome')
    return dict(id=id, outcome=outcome, detail=detail, **extra)


def seed_fixtures(destination):
    """Verify content before making a provider-visible, controlled fixture copy."""
    manifest = json.loads((ROOT/'fixtures/manifest.json').read_text())
    for fixture in manifest['fixtures']:
        source = ROOT/'fixtures'/fixture['path']
        if not source.is_file():
            return [item('fixtures', 'missing_fixture', 'Required fixture is absent')]
        if hashlib.sha256(source.read_bytes()).hexdigest() != fixture['sha256']:
            return [item('fixtures', 'missing_fixture', 'Fixture identity differs from manifest')]
    destination.mkdir(parents=True, exist_ok=True)
    for fixture in manifest['fixtures']:
        shutil.copyfile(ROOT/'fixtures'/fixture['path'], destination/fixture['path'])
    return [item('fixtures', 'passed', 'Public fixture bytes verified and seeded',
                 observed=manifest)]


def command(args, env=None, cwd=REPO, timeout=600):
    # Raw output can contain bearer tokens, workspace IDs or signed URLs. The
    # report uses structured allowlisted evidence, never captured raw output.
    return subprocess.run(args, cwd=cwd, env=env, text=True, capture_output=True,
                          timeout=timeout)


def sdk_command():
    binary = os.environ.get('SEQERA_PROFILE_SDK_BIN')
    if binary:
        return [str(Path(binary).resolve())]
    return ['cargo', 'run', '--locked', '--quiet', '-p', 'ga4gh-lib',
            '--example', 'seqera_wes_profile']


def seqera(profile, dest, offline, launch):
    env = os.environ.copy()
    if offline:
        from offline_seqera import Fixture
        token = dest/'synthetic-token'
        token.write_text('synthetic-profile-token\n')
        token.chmod(0o600)
        with Fixture((dest/'seqera.nf').read_bytes()) as fixture:
            env.update(SEQERA_PROFILE_API_ROOT=fixture.url,
                       SEQERA_PROFILE_TOKEN_FILE=str(token),
                       SEQERA_PROFILE_ACTION='launch-cancel',
                       SEQERA_PROFILE_ALLOW_LAUNCH='one-bounded-run',
                       SEQERA_PROFILE_WORKSPACE_ID='synthetic-workspace',
                       SEQERA_PROFILE_COMPUTE_ENV_ID='synthetic-compute',
                       SEQERA_PROFILE_DESCRIPTOR_VERSION='DSL2',
                       SEQERA_PROFILE_WORKFLOW=str(dest/'seqera.nf'),
                       SEQERA_PROFILE_LEDGER=str(dest/'run-ledger.json'),
                       SEQERA_PROFILE_TIMEOUT_SECONDS='3')
            result = command(sdk_command(), env=env)
            checks = decode_sdk(result)
            checks.append(item('fixture_multipart_assertions',
                               'passed' if fixture.accepted == 1 and not fixture.error else 'observed_failure',
                               'Synthetic server parses MIME, context fields and exact uploaded workflow bytes',
                               observed={'accepted': fixture.accepted, 'requests': fixture.requests}))
            return checks
    for setting in ['SEQERA_PROFILE_API_ROOT', 'SEQERA_PROFILE_TOKEN_FILE']:
        if not env.get(setting):
            return [item('prerequisite', 'not_run', f'{setting} is required')]
    env['SEQERA_PROFILE_ACTION'] = 'launch-cancel' if launch else 'probe'
    if launch:
        for setting in ['SEQERA_PROFILE_WORKSPACE_ID', 'SEQERA_PROFILE_COMPUTE_ENV_ID',
                        'SEQERA_PROFILE_DESCRIPTOR_VERSION', 'SEQERA_PROFILE_LEDGER']:
            if not env.get(setting):
                return [item('prerequisite', 'not_run', f'{setting} is required')]
        if env.get('SEQERA_PROFILE_ALLOW_LAUNCH') != 'one-bounded-run':
            return [item('prerequisite', 'not_run', 'Explicit one-bounded-run operator opt-in is required')]
        env['SEQERA_PROFILE_WORKFLOW'] = str(dest/'seqera.nf')
    return decode_sdk(command(sdk_command(), env=env))


def decode_sdk(result):
    if result.returncode:
        return [item('sdk_command', 'harness_error', 'SDK example could not build or execute; inspect private local output')]
    try:
        value = json.loads(result.stdout)
        checks = value['checks']
        for check in checks:
            if check['outcome'] not in OUTCOMES:
                raise ValueError('unknown SDK outcome')
        return checks
    except (ValueError, KeyError, TypeError):
        return [item('sdk_command', 'harness_error', 'SDK example did not emit the expected JSON report')]


def get_json(url):
    with urllib.request.urlopen(url, timeout=15) as response:
        return json.load(response)


def tes_suite(profile, dest):
    required = ['TES_PROFILE_API_ROOT', 'TES_PROFILE_BACKEND_URL', 'PROTES_SOURCE', 'PROTES_COMPOSE_FILE']
    for setting in required:
        if not os.environ.get(setting):
            return [item('prerequisite', 'not_run', f'{setting} is required')]
    source = Path(os.environ['PROTES_SOURCE'])
    suite = source/'tests/integration/check.sh'
    if not suite.is_file():
        return [item('upstream_suite', 'missing_fixture', 'Pinned upstream real-backend integration script is absent')]
    digest = hashlib.sha256(suite.read_bytes()).hexdigest()
    if digest != profile['client']['suite_sha256']:
        return [item('upstream_suite', 'version_mismatch', 'Upstream suite checksum differs from the pinned profile')]
    info = get_json(os.environ['TES_PROFILE_API_ROOT'].rstrip('/')+'/service-info')
    version = info.get('type', {}).get('version')
    if version != profile['spec']['tested']:
        return [item('discovery', 'version_mismatch', 'Advertised TES spec differs from the tested pin', observed={'spec': version})]
    if info.get('version') != profile['implementation']['advertised_version']:
        return [item('implementation_version', 'version_mismatch', 'Advertised proTES implementation differs from profile')]
    checks = [item('discovery', 'passed', 'TES advertised spec matches profile',
                   observed={'spec':version,'implementation':info.get('version')})]
    env = os.environ.copy()
    env['COMPOSE_FILE'] = str(Path(env['PROTES_COMPOSE_FILE']).resolve())
    result = command(['bash', str(suite), env['TES_PROFILE_API_ROOT'].rstrip('/'),
                      env['TES_PROFILE_BACKEND_URL'], env.get('TES_PROFILE_BACKEND_API_PATH', '')],
                     env=env, cwd=source)
    (dest/'upstream-output.private.log').write_text(result.stdout+result.stderr)
    checks.append(item('upstream_real_backend_suite', 'passed' if result.returncode == 0 else 'harness_error' if result.returncode in {126,127} else 'observed_failure',
                       'Existing proTES integration/check.sh: completion, input staging, stdout/stderr, exit failure and cancellation',
                       observed={'suite_sha256':digest, 'exit_code':result.returncode}))
    checks.append(item('physical_worker_cleanup', 'unsupported',
                       'Upstream suite observes backend TES state; a separate operator/container observation is required'))
    return checks


def direct_sdk(profile, dest):
    if not os.environ.get('TES_PROFILE_API_ROOT'):
        return [item('prerequisite', 'not_run', 'TES_PROFILE_API_ROOT is required')]
    info = get_json(os.environ['TES_PROFILE_API_ROOT'].rstrip('/')+'/service-info')
    if info.get('type', {}).get('version') != profile['spec']['tested']:
        return [item('discovery', 'version_mismatch', 'Advertised TES spec differs from the tested pin')]
    if info.get('version') != profile['implementation']['version']:
        return [item('implementation_version', 'version_mismatch', 'Advertised Funnel implementation differs from profile')]
    result = command(['cargo', 'test', '--locked', '-p', 'ga4gh-lib', '--test',
                      'provider_profiles', '--', '--ignored'], timeout=300)
    (dest/'direct-output.private.log').write_text(result.stdout+result.stderr)
    return [item('direct_sdk_complete_failure_cancel',
                 'passed' if result.returncode == 0 else 'observed_failure' if 'running 1 test' in result.stdout else 'harness_error',
                 'SDK real TES discovery/create/status/full logs/failed exit/cancel',
                 observed={'exit_code':result.returncode,'spec':info['type']['version'],
                           'implementation':info.get('version')}),
            item('physical_worker_cleanup', 'unsupported',
                 'TES CANCELED does not by itself prove physical container termination')]


def nf_api(endpoint, token, path, method='GET'):
    request = urllib.request.Request(endpoint+'/v1/'+path,
                                     headers={'Authorization':'Bearer '+token}, method=method)
    with urllib.request.urlopen(request, timeout=15) as response:
        return json.load(response)


def nf_cancel(executable, config, dest, endpoint, token):
    work = dest/'cancel-work'
    output = dest/'cancel-output.private.log'
    task_id = None
    state = None
    interrupted = False
    with output.open('w') as log:
        process = subprocess.Popen([executable, '-C', str(config), 'run', str(dest/'main.nf'),
                                    '-work-dir', str(work), '--seed', str(dest/'seed.txt'),
                                    '--delay', '90'], cwd=dest, stdout=log, stderr=subprocess.STDOUT,
                                   start_new_session=True)
        try:
            deadline = time.monotonic()+90
            while time.monotonic() < deadline and process.poll() is None:
                match = re.search(r'Created task with ID: ([^,\s]+)', output.read_text())
                if match:
                    task_id = match.group(1)
                    state = nf_api(endpoint, token, 'tasks/'+task_id+'?view=MINIMAL').get('state')
                    if state == 'RUNNING':
                        interrupted = True
                        os.killpg(process.pid, signal.SIGINT)
                        break
                time.sleep(1)
            if not interrupted and process.poll() is None:
                os.killpg(process.pid, signal.SIGINT)
            try:
                process.wait(timeout=45)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=10)
            if task_id:
                deadline = time.monotonic()+45
                while time.monotonic() < deadline:
                    state = nf_api(endpoint, token, 'tasks/'+task_id+'?view=MINIMAL').get('state')
                    if state in {'CANCELED','COMPLETE','EXECUTOR_ERROR','SYSTEM_ERROR'}:
                        break
                    time.sleep(1)
            result = item('nextflow_cancellation',
                          'passed' if interrupted and state == 'CANCELED' else 'observed_failure',
                          'Interrupt acknowledged RUNNING workflow and observe TES cancellation through nf-ga4gh',
                          observed={'running_before_interrupt':interrupted,'terminal_state':state,
                                    'submission_outcome_unknown':task_id is None})
            return result
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=10)
            # Retain the distinction if SDK/harness cleanup, rather than the
            # Nextflow signal handler, had to cancel the known execution.
            if task_id and state not in {'CANCELED','COMPLETE','EXECUTOR_ERROR','SYSTEM_ERROR'}:
                nf_api(endpoint, token, 'tasks/'+task_id+':cancel', method='POST')


def nf_suite(profile, dest):
    for setting in ['NEXTFLOW_PROFILE_BIN', 'NEXTFLOW_PROFILE_ENDPOINT', 'NEXTFLOW_PROFILE_TOKEN_FILE']:
        if not os.environ.get(setting):
            return [item('prerequisite', 'not_run', f'{setting} is required')]
    executable = str(Path(os.environ['NEXTFLOW_PROFILE_BIN']).resolve())
    if hashlib.sha256(Path(executable).read_bytes()).hexdigest() != profile['implementation']['nextflow_dist_sha256']:
        return [item('client_distribution', 'version_mismatch', 'Nextflow distribution checksum differs from profile')]
    result = command([executable, '-version'], timeout=30)
    if result.returncode or '26.10.5' not in result.stdout+result.stderr:
        return [item('client_version', 'version_mismatch', 'Expected Nextflow 26.10.5')]
    endpoint = os.environ['NEXTFLOW_PROFILE_ENDPOINT'].rstrip('/')
    if endpoint.endswith('/v1'):
        return [item('endpoint_prefix', 'harness_error', 'nf-ga4gh appends /v1; configure the parent TES path, not the full API root')]
    token = Path(os.environ['NEXTFLOW_PROFILE_TOKEN_FILE']).read_text().strip()
    # JSON string literals are valid Groovy strings, including escaped tokens.
    config = dest/'nextflow.config'
    image = profile['implementation']['task_image']
    config.write_text("plugins { id 'nf-ga4gh@1.5.0' }\n"+
                      "process.executor = 'tes'\nprocess.container = "+json.dumps(image)+"\n"+
                      "tes.endpoint = "+json.dumps(endpoint)+"\n"+
                      "tes.oauthToken = "+json.dumps(token)+"\n"+
                      "tes.pollInterval = '1s'\n")
    config.chmod(0o600)
    work = dest/'shared-work'
    work.mkdir()
    run = command([executable, '-C', str(config), 'run', str(dest/'main.nf'),
                   '-work-dir', str(work), '--seed', str(dest/'seed.txt'), '--delay', '0'], cwd=dest, timeout=240)
    (dest/'nextflow-output.private.log').write_text(run.stdout+run.stderr)
    plugin_root = Path(os.environ.get('NXF_HOME', str(Path.home()/'.nextflow')))/'plugins/nf-ga4gh-1.5.0'
    plugin_hash = hashlib.sha256()
    for path in sorted(plugin_root.rglob('*')):
        if path.is_file():
            plugin_hash.update(path.relative_to(plugin_root).as_posix().encode()+b'\0'+hashlib.sha256(path.read_bytes()).digest())
    client_identity = item('plugin_artifact_identity',
        'passed' if plugin_hash.hexdigest() == profile['implementation']['plugin_tree_sha256'] else 'version_mismatch',
        'Installed published nf-ga4gh artifact tree matches the pinned profile',
        observed={'plugin_tree_sha256':plugin_hash.hexdigest()})
    checks = [client_identity, item('nextflow_protes_execution', 'passed' if run.returncode == 0 else 'observed_failure' if 'Created task with ID:' in run.stdout+run.stderr else 'harness_error',
                   'Pinned Nextflow nf-ga4gh submits to configured proTES with a host-visible shared work directory',
                   observed={'exit_code':run.returncode})]
    outputs = list(work.glob('**/nested/result.txt'))
    correct = bool(outputs) and all(p.read_bytes() == (dest/'seed.txt').read_bytes() for p in outputs)
    checks.append(item('shared_storage_directory_glob_output', 'passed' if correct else 'observed_failure',
                       'Verify staged seed and nested directory/glob output bytes in the shared work path'))
    checks.append(item('log_selection', 'passed' if any(p.read_text().strip() == 'profile-stdout' for p in work.glob('**/.command.out')) and any(p.read_text().strip() == 'profile-stderr' for p in work.glob('**/.command.err')) else 'observed_failure',
                       'Select stdout and stderr command files separately; upstream issue #7'))
    match = re.search(r'Created task with ID: ([^,\s]+)', run.stdout+run.stderr)
    if match:
        task = nf_api(endpoint, token, 'tasks/'+match.group(1)+'?view=FULL')
        mappings = task.get('outputs', [])
        prefixes = [o.get('path_prefix') for o in mappings if '*' in o.get('path','') or '?' in o.get('path','')]
        checks.append(item('path_prefix', 'passed' if prefixes and all(prefixes) and correct else 'observed_failure',
                           'Inspect submitted glob path_prefix and verify nested relative output destination; upstream issue #12',
                           observed={'glob_mappings':len(prefixes),'nonempty_prefixes':sum(bool(p) for p in prefixes),
                                     'directory_mappings':sum(o.get('type') == 'DIRECTORY' for o in mappings)}))
    else:
        checks.append(item('path_prefix', 'not_run', 'No acknowledged Nextflow TES task available to inspect'))
    if run.returncode == 0:
        try:
            checks.append(nf_cancel(executable, config, dest, endpoint, token))
        except (OSError, ValueError, subprocess.TimeoutExpired):
            checks.append(item('nextflow_cancellation', 'harness_error',
                               'Cancellation observation failed; private work and known IDs remain for operator cleanup'))
    else:
        checks.append(item('nextflow_cancellation', 'not_run', 'Fix the completion fixture before submitting a separate cancellation task'))
    checks.append(item('physical_worker_cleanup', 'unsupported', 'Requires independent backend worker observation'))
    return checks


def junit(report):
    all_checks = [(p, c) for p in report['profiles'] for c in p['checks']]
    failure = {'version_mismatch', 'missing_fixture', 'observed_failure'}
    root = ET.Element('testsuite', name='ga4gh-provider-profiles', tests=str(len(all_checks)),
                      failures=str(sum(c['outcome'] in failure for _, c in all_checks)),
                      errors=str(sum(c['outcome'] == 'harness_error' for _, c in all_checks)),
                      skipped=str(sum(c['outcome'] in {'not_run','unsupported'} for _, c in all_checks)))
    for profile, check in all_checks:
        case = ET.SubElement(root, 'testcase', classname=profile['id'], name=check['id'])
        properties = ET.SubElement(case, 'properties')
        for key, value in [('outcome', check['outcome']), ('evidence', profile['evidence'])]:
            ET.SubElement(properties, 'property', name=key, value=value)
        kind = ('failure' if check['outcome'] in failure else
                'error' if check['outcome'] == 'harness_error' else
                'skipped' if check['outcome'] in {'not_run','unsupported'} else None)
        if kind:
            ET.SubElement(case, kind, type=check['outcome'], message=check['detail'])
    return ET.tostring(root, encoding='unicode')+'\n'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--profile', default='all')
    parser.add_argument('--execute', action='store_true', help='execute configured existing suites or read-only native WES probe')
    parser.add_argument('--offline-seqera', action='store_true', help='execute synthetic multipart/status/log/cancel fixture through SDK')
    parser.add_argument('--live-launch', action='store_true', help='allow the additionally gated single native WES launch/cancel')
    parser.add_argument('--out', type=Path, default=ROOT/'reports/latest')
    args = parser.parse_args()
    profiles = [json.loads(p.read_text()) for p in sorted((ROOT/'profiles').glob('*.json'))]
    if args.profile != 'all':
        profiles = [p for p in profiles if p['id'] == args.profile]
    if not profiles:
        parser.error('unknown profile')
    if args.live_launch and (not args.execute or args.profile != 'seqera-native-wes'):
        parser.error('--live-launch requires --execute --profile seqera-native-wes')
    report = {'schema_version':1, 'created_at':datetime.now(timezone.utc).isoformat(),
              'sdk_revision':command(['git','rev-parse','HEAD']).stdout.strip(),
              'sdk_dirty':bool(command(['git','status','--porcelain']).stdout.strip()),
              'harness_sha256':hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
              'sdk_example_sha256':hashlib.sha256((REPO/'lib/examples/seqera_wes_profile.rs').read_bytes()).hexdigest(),
              'command':sys.argv, 'profiles':[]}
    args.out.mkdir(parents=True, exist_ok=True)
    work_root = ROOT/'.local/run-work'
    work_root.mkdir(parents=True, exist_ok=True)
    work_root.chmod(0o700)
    for profile in profiles:
        evidence = ('simulated_protocol' if args.offline_seqera and profile['kind']=='seqera' else
                    'real_service' if args.execute and profile['kind']=='seqera' and not args.live_launch else
                    'real_execution' if args.execute else 'not_run')
        result = {'id':profile['id'], 'profile':profile, 'evidence':evidence, 'checks':[]}
        with contextlib.nullcontext(tempfile.mkdtemp(prefix='private-seed-', dir=work_root)) as temp:
            dest = Path(temp)
            dest.chmod(0o700)
            result['checks'] += seed_fixtures(dest)
            if result['checks'][-1]['outcome'] == 'passed':
                try:
                    if profile['kind']=='seqera' and (args.execute or args.offline_seqera):
                        checks = seqera(profile, dest, args.offline_seqera, args.live_launch)
                    elif args.execute and profile['kind']=='direct-sdk':
                        checks = direct_sdk(profile, dest)
                    elif args.execute and profile['kind']=='tes-suite':
                        checks = tes_suite(profile, dest)
                    elif args.execute and profile['kind']=='nf':
                        checks = nf_suite(profile, dest)
                    else:
                        checks = [item('execution', 'not_run', 'Explicit execution opt-in is absent')]
                    result['checks'] += checks
                except subprocess.TimeoutExpired:
                    result['checks'].append(item('harness', 'harness_error', 'Command exceeded harness deadline; owned jobs may remain unresolved'))
                except (OSError, ValueError, KeyError, TypeError):
                    result['checks'].append(item('harness', 'harness_error', 'Setup, transport or report decoding failed; inspect private operator context'))
            docker_context = os.environ.get('INTEROP_DOCKER_CONTEXT')
            if args.execute and docker_context and profile['kind'] != 'seqera':
                images = []
                for service in ['funnel','protes','protes-worker']:
                    inspect = command(['docker','--context',docker_context,'inspect',
                        'ga4gh-h3-interop-'+service+'-1','--format',
                        '{"configured":{{json .Config.Image}},"resolved":{{json .Image}}}'], timeout=15)
                    if inspect.returncode == 0:
                        images.append(dict(service=service, **json.loads(inspect.stdout)))
                result['runtime_images'] = images
                snapshot = command(['docker','--context',docker_context,'ps','-q','--filter',
                                    'label=ga4gh-h3-interop=task'], timeout=15)
                initial_count = len(snapshot.stdout.split()) if snapshot.returncode == 0 else None
                started = time.monotonic()
                while snapshot.returncode == 0 and snapshot.stdout.strip() and time.monotonic()-started < 15:
                    time.sleep(0.5)
                    snapshot = command(['docker','--context',docker_context,'ps','-q','--filter',
                                        'label=ga4gh-h3-interop=task'], timeout=15)
                result['checks'].append(item('operator_container_termination',
                    'harness_error' if snapshot.returncode else 'passed' if not snapshot.stdout.strip() else 'observed_failure',
                    'Independent bounded Docker observation after API checks; container stop may lag CANCELED; no whole-worker cleanup guarantee',
                    observed={'initial_running_task_containers':initial_count,
                              'final_running_task_containers':len(snapshot.stdout.split()) if snapshot.returncode == 0 else None,
                              'observation_seconds':round(time.monotonic()-started,3)}))
            provider_checks = [c for c in result['checks'] if c['id'] != 'fixtures']
            if provider_checks and all(c['outcome'] in {'not_run', 'unsupported'} for c in provider_checks):
                result['evidence'] = 'not_run'
            if result['evidence'] == 'real_execution':
                execution_checks = {'direct_sdk_complete_failure_cancel', 'upstream_real_backend_suite',
                                    'nextflow_protes_execution', 'multipart_launch'}
                if not any(c['id'] in execution_checks and c['outcome'] == 'passed' for c in provider_checks):
                    result['evidence'] = 'real_service'
            report['profiles'].append(result)
    (args.out/'report.json').write_text(json.dumps(report, indent=2)+'\n')
    (args.out/'junit.xml').write_text(junit(report))
    counts = collections.Counter(c['outcome'] for p in report['profiles'] for c in p['checks'])
    print(json.dumps(dict(counts), sort_keys=True))
    return int(any(counts[k] for k in ['version_mismatch','missing_fixture','harness_error','observed_failure']))


if __name__ == '__main__':
    sys.exit(main())

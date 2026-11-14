#!/usr/bin/env python3
"""Run the same scenario API against the SDK and pinned maintained WES provider."""
import argparse
import hashlib
import importlib.util
import json
import os
import shutil
import subprocess
import threading
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]

def run(*args,env):
    subprocess.run(args,cwd=ROOT,env=env,check=True)

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--python-only',action='store_true')
    parser.add_argument('--url',help='test an already running source or OCI deployment')
    args=parser.parse_args()
    env=os.environ.copy()
    output=ROOT/'target/h3-scenarios';output.mkdir(parents=True,exist_ok=True)
    run('python3',str(ROOT/'tests/h3-scenario-api/test_server.py'),env=env)
    if args.python_only:return
    spec=importlib.util.spec_from_file_location('scenario_api',ROOT/'testbed/server.py')
    api=importlib.util.module_from_spec(spec);spec.loader.exec_module(api)
    server=None;thread=None
    if not args.url:
        server=api.make_server();thread=threading.Thread(target=server.serve_forever,daemon=True);thread.start()
    env.update(GA4GH_SCENARIO_API=args.url or 'http://127.0.0.1:'+str(server.server_port),GA4GH_SCENARIO_SDK_EVIDENCE=str(output/'sdk.json'),GA4GH_SCENARIO_BROWSER_EVIDENCE=str(output/'browser.json'))
    try:
        run('cargo','test','-p','ga4gh-lib','--test','scenario_api','--','--ignored','--nocapture',env=env)
        pins=json.loads((ROOT/'testbed/vendor/provenance.json').read_text())
        for filename,expected in pins['sha256'].items():
            if hashlib.sha256((ROOT/'testbed/vendor'/filename).read_bytes()).hexdigest()!=expected:raise RuntimeError('vendor checksum mismatch: '+filename)
        build=output/'browser';build.mkdir(exist_ok=True)
        run('npm','--cache','/tmp/ga4gh-h3/npm-cache','exec','--yes','--package','typescript@5.9.3','--','tsc','--target','ES2022','--module','NodeNext','--moduleResolution','NodeNext','--lib','ES2022,DOM','--outDir',str(build),str(ROOT/'testbed/vendor/rest-wes-provider.ts'),str(ROOT/'testbed/vendor/wes-provider.ts'),env=env)
        env['GA4GH_WES_PROVIDER_JS']=str(build/'rest-wes-provider.js')
        run('node',str(ROOT/'tests/h3-scenario-api/browser.mjs'),env=env)
        results=[json.loads((output/f).read_text())for f in ['sdk.json','browser.json']]
        # WES history must agree across clients, including the lost-response acceptance.
        wes=lambda r:[e for e in r['replay']['events']if e['data'].get('service')=='wes' or e['data'].get('operation','').startswith('wes.')]
        if wes(results[0])!=wes(results[1]):raise RuntimeError('cross-client WES histories differ')
        (output/'summary.json').write_text(json.dumps({'status':'passed','evidence_class':'simulated-protocol','client_results':results},indent=2)+'\n')
        print('SDK and maintained RestWesProvider shared WES histories match')
    finally:
        if server:
            server.shutdown();server.server_close();thread.join()

if __name__=='__main__':main()

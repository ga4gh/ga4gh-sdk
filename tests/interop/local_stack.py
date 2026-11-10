#!/usr/bin/env python3
"""Disposable reference stack from upstream proTES real-integration config.

Uses already cached exact images; never changes another Compose project.
Requires Docker Compose, a Linux Docker CLI path on its daemon host, and proTES
source for its existing synthetic identity provider (not used as a TES backend).
"""
import argparse
import hashlib
import os
from pathlib import Path
import subprocess
import tarfile
import urllib.request

ROOT = Path(__file__).resolve().parent
LOCAL = ROOT/'.local'
FUNNEL_SHA = '01c57611a8627781400cab6a17ce84fc2b5e782c9c39b3a14fd62dfcf1fe9184'
IDP_SHA = 'ecb63ef21fd772b257c5d71c2f6b24d5c240e313e76ade4bfbb6bb06ea8803cd'
IMAGES = ['python@sha256:f3fa41d74a768c2fce8016b98c191ae8c1bacd8f1152870a3f9f87d350920b7c',
          'h3-protes@sha256:e11689face55b807e23d36030ec5f427ce8976be618e1720a4fc8d62015c8593',
          'mongo@sha256:8bb4442761a63a8dda452744082cdac9644b429807eb977b42324a63cef59ca4',
          'rabbitmq@sha256:2afee120d039c78a16c930036ac47568ed7fc45dabdc56fc3d890d2d9273ac49']


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=['up','down'])
    parser.add_argument('--context', required=True, help='explicit operator-owned Docker context')
    parser.add_argument('--protes-source', type=Path)
    parser.add_argument('--docker-cli-vm', default='/usr/bin/docker', help='Linux Docker CLI path on daemon host')
    args = parser.parse_args()
    docker = ['docker', '--context', args.context]
    compose = (docker+['compose'] if subprocess.run(docker+['compose','version'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0
               else ['docker-compose','--context',args.context])+['-f', str(LOCAL/'compose.yaml')]
    if args.action == 'down':
        subprocess.run(compose+['down','--volumes'], check=True)
        return
    if not args.protes_source:
        parser.error('--protes-source is required for up')
    LOCAL.mkdir(exist_ok=True)
    (LOCAL/'bin').mkdir(exist_ok=True)
    (LOCAL/'funnel-work').mkdir(exist_ok=True)
    for image in IMAGES:
        subprocess.run(docker+['image','inspect',image], stdout=subprocess.DEVNULL,check=True)
    archive = LOCAL/'funnel-linux.tar.gz'
    if not archive.exists():
        urllib.request.urlretrieve('https://github.com/calypr/funnel/releases/download/v0.12.3/funnel-linux-arm64-v0.12.3.tar.gz',archive)
    if hashlib.sha256(archive.read_bytes()).hexdigest() != FUNNEL_SHA:
        raise ValueError('Funnel archive identity mismatch')
    with tarfile.open(archive) as package:
        data = package.extractfile('funnel').read()
    (LOCAL/'bin/funnel-linux').write_bytes(data)
    (LOCAL/'bin/funnel-linux').chmod(0o755)
    idp = args.protes_source/'tests/smoke/fake_tes.py'
    if hashlib.sha256(idp.read_bytes()).hexdigest() != IDP_SHA:
        raise ValueError('upstream identity fixture identity mismatch')
    (LOCAL/'integration-idp').mkdir(exist_ok=True)
    (LOCAL/'integration-idp/fake_tes.py').write_bytes(idp.read_bytes())
    for name in ['compose.yaml','funnel.yaml','protes.yaml']:
        text = (ROOT/(name+'.template')).read_text()
        text = text.replace('{{LOCAL}}',str(LOCAL)).replace('{{DOCKER_CLI_VM}}',args.docker_cli_vm)
        (LOCAL/name).write_text(text)
    subprocess.run(compose+['up','-d','--wait','--wait-timeout','120'], check=True)
    print('Funnel: http://127.0.0.1:18910/ga4gh/tes/v1')
    print('proTES: http://127.0.0.1:18920/ga4gh/tes/v1')
    print('nf-ga4gh endpoint: http://127.0.0.1:18920/ga4gh/tes (plugin appends /v1)')


if __name__ == '__main__':
    main()

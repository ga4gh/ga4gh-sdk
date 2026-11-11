#!/usr/bin/env python3
"""Exercise QC on two independent local WES processes. This is not a federation test."""
import json, os, pathlib, socket, subprocess, sys, tempfile, time, urllib.request
root=pathlib.Path(__file__).resolve().parents[1]
env_dir=pathlib.Path(os.environ.get('GA4GH_TOIL_ENV_DIR',str(root/'target/toil-qc-venv')))
if not (env_dir/'bin/toil').exists():
 subprocess.run([sys.executable,'-m','venv',str(env_dir)],check=True)
 subprocess.run([str(env_dir/'bin/python'),'-m','pip','install','toil[server,cwl,aws]==9.3.0','ipython==9.6.0'],check=True)
if subprocess.run([str(env_dir/'bin/python'),'-c','import IPython'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL).returncode:
 subprocess.run([str(env_dir/'bin/python'),'-m','pip','install','ipython==9.6.0'],check=True)
work=root/'target/phase2-qc';work.mkdir(parents=True,exist_ok=True)
servers=[];logs=[];urls=[]
try:
 for index in range(2):
  with socket.socket() as sock:sock.bind(('127.0.0.1',0));port=sock.getsockname()[1]
  node=work/f'node-{index}';node.mkdir(exist_ok=True);(node/'tmp').mkdir(exist_ok=True)
  env=os.environ.copy();env['PATH']=str(env_dir/'bin')+os.pathsep+env['PATH'];env['TMPDIR']=str(node/'tmp');env['AWS_EC2_METADATA_DISABLED']='true'
  logs.append(open(node/'server.log','w'))
  cmd=[str(env_dir/'bin/toil'),'server','--host','127.0.0.1','--port',str(port),'--workers','1','--bypass_celery','--work_dir',str(node/'workflows')]
  if os.environ.get('GA4GH_QC_NO_CONTAINER')=='1':cmd+=['--opt=--no-container']
  servers.append(subprocess.Popen(cmd,env=env,stdout=logs[-1],stderr=subprocess.STDOUT));url=f'http://127.0.0.1:{port}/ga4gh/wes/v1';urls.append(url)
  for _ in range(240):
   try:
    with urllib.request.urlopen(url+'/service-info',timeout=1):break
   except OSError:
    if servers[-1].poll() is not None:raise RuntimeError('Toil exited; inspect '+str(node/'server.log'))
    time.sleep(.25)
  else:raise RuntimeError('Toil readiness timeout')
 evidence=work/'evidence.json';env=os.environ.copy();env['GA4GH_QC_WES_URLS']=','.join(urls);env['GA4GH_QC_EVIDENCE']=str(evidence)
 subprocess.run(['cargo','test','-p','ga4gh-lib','--features','workflow-contracts','--test','qc_workflow','real_qc_on_two_wes_instances','--locked','--','--ignored','--nocapture'],env=env,cwd=root,check=True)
 subprocess.run([str(env_dir/'bin/python'),str(root/'examples/qc-demo/view_report.py'),str(evidence)],check=True)
 print('QC evidence:',evidence)
finally:
 for server in servers:server.terminate()
 for server in servers:
  try:server.wait(timeout=10)
  except subprocess.TimeoutExpired:server.kill();server.wait()
 for log in logs:log.close()

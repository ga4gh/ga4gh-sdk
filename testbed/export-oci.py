#!/usr/bin/env python3
"""Convert a docker image-save tar into a content-addressed OCI image archive."""
import argparse,gzip,hashlib,io,json,tarfile
from pathlib import Path

def encoded(v):return json.dumps(v,sort_keys=True,separators=(',',':')).encode()
def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('source',type=Path);p.add_argument('output',type=Path);args=p.parse_args()
 blobs={}
 def descriptor(data,kind):
  sha=hashlib.sha256(data).hexdigest();blobs['blobs/sha256/'+sha]=data
  return {'mediaType':kind,'digest':'sha256:'+sha,'size':len(data)}
 with tarfile.open(args.source)as archive:
  manifests=json.load(archive.extractfile('manifest.json'))
  if len(manifests)!=1:raise ValueError('export exactly one image')
  record=manifests[0];raw=archive.extractfile(record['Config']).read();config=json.loads(raw)
  config_ref=descriptor(raw,'application/vnd.oci.image.config.v1+json')
  layers=[];actual=[]
  for name in record['Layers']:
   data=archive.extractfile(name).read();compressed=data.startswith(b'\x1f\x8b')
   actual.append('sha256:'+hashlib.sha256(gzip.decompress(data)if compressed else data).hexdigest())
   layers.append(descriptor(data,'application/vnd.oci.image.layer.v1.tar'+('+gzip'if compressed else '')))
  if actual!=config['rootfs']['diff_ids']:raise ValueError('uncompressed layer diff_ids mismatch')
  manifest=descriptor(encoded({'schemaVersion':2,'mediaType':'application/vnd.oci.image.manifest.v1+json','config':config_ref,'layers':layers}),'application/vnd.oci.image.manifest.v1+json')
  manifest['annotations']={'org.opencontainers.image.ref.name':record.get('RepoTags',['ga4gh-scenario-api:0.1.0'])[0]}
  manifest['platform']={'architecture':config['architecture'],'os':config['os']}
  blobs['index.json']=encoded({'schemaVersion':2,'manifests':[manifest]})
  blobs['oci-layout']=encoded({'imageLayoutVersion':'1.0.0'})
 args.output.parent.mkdir(parents=True,exist_ok=True)
 with tarfile.open(args.output,'w')as output:
  for name,data in sorted(blobs.items()):
   info=tarfile.TarInfo(name);info.size=len(data);info.mode=0o644;info.mtime=0;output.addfile(info,io.BytesIO(data))
 result={'format':'oci-image-layout','image_manifest_digest':manifest['digest'],'architecture':config['architecture'],'os':config['os'],'archive_sha256':hashlib.sha256(args.output.read_bytes()).hexdigest(),'archive_size':args.output.stat().st_size}
 args.output.with_suffix('.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result))
if __name__=='__main__':main()

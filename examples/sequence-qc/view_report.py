#!/usr/bin/env python3
"""Adapt declared html-viewer metadata to IPython's existing HTML viewer."""
import json,sys,urllib.parse,pathlib
from IPython.display import HTML
record=json.loads(pathlib.Path(sys.argv[1]).read_text())
for execution in record['executions']:
 for item in execution['viewer_metadata']:
  assert item['widget']=='html-viewer'
  path=urllib.parse.urlparse(item['value']['location'])
  assert path.scheme=='file', 'This local public-data demo accepts only explicit local report paths'
  viewer=HTML(filename=urllib.parse.unquote(path.path))
  assert 'Sequence QC' in viewer.data and 'sequence_count' in viewer.data
print('IPython HTML viewer consumed both declared public QC reports')

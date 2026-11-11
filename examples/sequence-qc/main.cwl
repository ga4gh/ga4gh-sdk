cwlVersion: v1.2
class: Workflow
requirements:
  InlineJavascriptRequirement: {}
inputs:
  sequences: string[]
outputs:
  metrics:
    type: Any
    outputSource: qc/metrics
  report:
    type: File
    outputSource: qc/report
steps:
  qc:
    in:
      sequences: sequences
    out: [metrics, report]
    run:
      class: CommandLineTool
      requirements:
        DockerRequirement:
          dockerPull: python@sha256:6771159cd4fa5d9bba1258caf0b82e6b73458c694d178ad97c5e925c2d0e1a91
        InitialWorkDirRequirement:
          listing:
            - entryname: qc.py
              entry: |
                import json, sys, html
                sequences = json.loads(sys.argv[1])
                lengths = [len(s) for s in sequences]
                metrics = dict(sequence_count=len(lengths), total_length=sum(lengths), mean_length=sum(lengths)/len(lengths))
                with open('metrics.json', 'w') as out:
                    json.dump(metrics, out, sort_keys=True)
                with open('report.html', 'w') as out:
                    out.write('<!doctype html><title>Sequence QC</title><h1>Sequence QC</h1><pre>'+html.escape(json.dumps(metrics, sort_keys=True))+'</pre>')
      baseCommand: [python3, qc.py]
      inputs:
        sequences:
          type: string[]
          inputBinding:
            position: 1
            valueFrom: $(JSON.stringify(self))
      outputs:
        metrics:
          type: Any
          outputBinding:
            glob: metrics.json
            loadContents: true
            outputEval: $(JSON.parse(self[0].contents))
        report:
          type: File
          outputBinding:
            glob: report.html

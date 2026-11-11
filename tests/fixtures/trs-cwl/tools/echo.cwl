cwlVersion: v1.2
class: CommandLineTool
baseCommand: echo
inputs:
  message:
    type: string
    inputBinding:
      position: 1
stdout: greeting.txt
outputs:
  out:
    type: File
    outputBinding:
      glob: greeting.txt

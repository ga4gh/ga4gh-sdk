cwlVersion: v1.2
class: Workflow
inputs:
  message: string
  secret: ["null", string]
  run:
    type: string
    default: unused
outputs:
  greeting:
    type: File
    outputSource: echo/out
steps:
  echo:
    run: tools/echo.cwl
    in:
      message: message
    out: [out]

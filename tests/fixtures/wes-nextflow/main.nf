#!/usr/bin/env nextflow

params.sleep_seconds = 0
params.message = 'sdk-integration'

messages = Channel.from(params.message)

process writeResult {
  input:
  val message from messages

  output:
  file 'result.txt'

  script:
  """
  sleep ${params.sleep_seconds}
  echo '${message}' > result.txt
  """
}

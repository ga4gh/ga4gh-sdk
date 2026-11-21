nextflow.enable.dsl=2
params.delay = 30
process PROFILE {
    cpus 1
    memory '512 MB'
    time '2 min'
    output:
    stdout
    script:
    """
    echo ga4gh-sdk-native-wes-profile
    sleep ${params.delay}
    """
}
workflow { PROFILE() }

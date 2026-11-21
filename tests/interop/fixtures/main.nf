nextflow.enable.dsl=2
params.delay = 0
process PROFILE {
    input:
    path seed
    output:
    path 'nested/*.txt', emit: files
    path 'nested', type: 'dir', emit: directory
    script:
    """
    mkdir -p nested
    cat ${seed} > nested/result.txt
    echo profile-stdout
    echo profile-stderr >&2
    sleep ${params.delay}
    """
}
workflow {
    PROFILE(file(params.seed))
}

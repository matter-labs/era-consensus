#!/usr/bin/env bash
#
# Check a state invariant in parallel with randomized symbolic execution.
# Igor Konnov, 2024

if [ "$#" -lt 5 ]; then
    echo "Use: $0 spec.qnt invariant max-steps max-runs max-parallel-jobs [init] [step]"
    echo ""
    echo "  - spec.qnt is the specification to check"
    echo "  - invariant is the invariant to check"
    echo "  - max-steps is the maximal number of steps every run may have"
    echo "  - max-runs is the maximal number of symbolic runs in total to try across all jobs"
    echo "  - max-parallel-jobs is the maximum of jobs to run in parallel"
    echo "  - init is the initialization action"
    echo "  - step is the step action"
    exit 1
fi

# https://stackoverflow.com/questions/242538/unix-shell-script-find-out-which-directory-the-script-file-resides
SCRIPT=$(readlink -f "$0")
# Absolute path this script is in, thus /home/user/bin
BASEDIR=$(dirname "$SCRIPT")

spec=$1
invariant=$2
max_steps=$3
max_runs=$4
max_jobs=$5
init=${6:-"init"}
step=${7:-"step"}

csv=${invariant}_${max_steps}_${max_runs}.csv

# since `quint verify --random-transitions=true` tries 100 symbolic runs, divide `max_runs` by 100 and compensate for rounding
${BASEDIR}/gen-inputs.sh $((max_runs / 100 + 1)) ${invariant} ${csv}
${BASEDIR}/test-invariants.sh ${spec} ${csv} ${max_steps} ${max_jobs} 1 ${init} ${step}
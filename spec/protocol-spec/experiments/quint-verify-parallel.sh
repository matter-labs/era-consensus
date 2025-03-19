#!/usr/bin/env bash
#
# Check a state invariant in parallel with randomized symbolic execution.
# Igor Konnov, 2024 (for Matter Labs)

if [ "$#" -lt 5 ]; then
    echo "Use: $0 spec.qnt invariant max-steps max-runs parallel-jobs [init] [step]"
    echo ""
    echo "  - spec.qnt is the specification to check"
    echo "  - invariant is the invariant to check"
    echo "  - max-steps is the maximal number of steps every run may have"
    echo "  - max-runs is the maximal number of symbolic runs per job"
    echo "  - parallel-jobs is the number of jobs to run in parallel"
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

seq 18001 $((18000+max_jobs)) | \
  parallel -j ${max_jobs} --bar --progress --delay 1 --halt now,fail=1 --results out \
    quint verify --random-transitions=true --max-steps=${max_steps} \
      --init=${init} --step=${step} --invariant=${invariant} \
      --server-endpoint=localhost:{1} ${spec}
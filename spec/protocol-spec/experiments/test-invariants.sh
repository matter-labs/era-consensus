#!/usr/bin/env bash

if [ "$#" -lt 5 ]; then
    echo "Use: $0 spec.qnt input.csv max-steps max-jobs max-failing-jobs [init] [step]"
    echo "  - spec.qnt is the specification to check"
    echo "  - input.csv is the experiments file that contains one pair per line: invariant,port"
    echo "  - max-steps is the maximal number of protocol steps to check, e.g., 30"
    echo "  - max-jobs is the maximal number of jobs to run in parallel, e.g., 16"
    echo "  - max-failing-jobs is the maximal number of jobs to fail"
    echo "  - init it the initial action, by default: init"
    echo "  - step it the step action, by default: step"
    exit 1
fi

spec=$1
input=$2
max_steps=$3
max_jobs=$4
max_failing_jobs=$5
init=${6:-"init"}
step=${7:-"step"}

# https://lists.defectivebydesign.org/archive/html/bug-parallel/2017-04/msg00000.html
export LANG= LC_ALL= LC_CTYPE= 

# set -j <cpus> to the number of CPUs - 1
parallel -j ${max_jobs} -v --delay 1 --halt now,fail=${max_failing_jobs} --results out --colsep=, -a ${input} \
  quint verify --max-steps=${max_steps} --init=${init} --step=${step} --random-transitions=true \
    --apalache-config=apalache.json \
    --server-endpoint=localhost:{2} --invariant={1} --verbosity=3 ${spec}
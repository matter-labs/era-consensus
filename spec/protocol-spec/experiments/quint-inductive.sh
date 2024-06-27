#!/usr/bin/env bash

if [ "$#" -lt 4 ]; then
    echo "Use: $0 spec.qnt max-transitions max-jobs max-failing-jobs [invariant] [init] [step]"
    echo "  - spec.qnt is the specification to check"
    echo "  - max-transitions is the maximal number of protocol transitions (from step)"
    echo "  - max-jobs is the maximal number of jobs to run in parallel, e.g., 16"
    echo "  - max-failing-jobs is the maximal number of jobs to fail"
    echo "  - invariant is the invariant to check, by default: inv"
    echo "  - init is the initial action, by default: init"
    echo "  - step is the step action, by default: step"
    exit 1
fi

spec=$1
max_trans=$2
max_jobs=$3
max_failing_jobs=$4
inv=${5:-"inv"}
init=${6:-"init"}
step=${7:-"step"}

# https://lists.defectivebydesign.org/archive/html/bug-parallel/2017-04/msg00000.html
export LANG= LC_ALL= LC_CTYPE= 

for i in `seq 0 $((max_trans-1))`; do
  cat >$TMPDIR/apalache-inductive${i}.json <<EOF
  {
    "checker": {
      "discard-disabled": false,
      "no-deadlocks": true,
      "write-intermediate": true,
      "tuning": {
        "search.invariant.mode": "after",
        "search.invariantFilter": "(1->.*)",
        "search.transitionFilter": "(0->.*|1->$i)"
      }
    },
    "common": {
      "debug": true
    }
  }
EOF
done

# set -j <cpus> to the number of CPUs - 1
seq 0 $((max_trans-1)) \
  | parallel -j ${max_jobs} -v --delay 1 --halt now,fail=${max_failing_jobs} --results out \
  JVM_ARGS=-Xmx16G quint verify --max-steps=1 --init=${init} --step=${step} \
    --apalache-config=$TMPDIR/apalache-inductive{}.json \
    --invariant=${inv} ${spec}

# --server-endpoint=localhost:200{#}
#!/usr/bin/env bash
#
# Complete parallelization in transitions and inductive lemmas for proving
# an inductive step.
#
# Igor Konnov, 2024 (for Matter Labs)

if [ "$#" -lt 5 ]; then
    echo "Use: $0 spec.qnt max-transitions max-lemmas max-jobs max-failing-jobs [invariant] [init] [step] [memsize]"
    echo "  - spec.qnt is the specification to check"
    echo "  - max-transitions is the maximal number of protocol transitions (from step), use 1 to disable"
    echo "  - max-lemmas is the maximal number of conjuncts in the invariant, use 1 to disable"
    echo "  - max-jobs is the maximal number of jobs to run in parallel, e.g., 16"
    echo "  - max-failing-jobs is the maximal number of jobs to fail"
    echo "  - invariant is the invariant to check, by default: inv"
    echo "  - init is the initial action, by default: init"
    echo "  - step is the step action, by default: step"
    echo "  - memfree is the minimum memory to start a job, man parallel --memfree, by default: 1G"
    echo ""
    echo "Additionally, you can pass an environment variable PARGS to parallel"
    exit 1
fi

spec=$1
max_trans=$2
max_lemmas=$3
max_jobs=$4
max_failing_jobs=$5
inv=${6:-"inv"}
init=${7:-"init"}
step=${8:-"step"}
memfree=${9:-"1G"}

# protect the root!
TMPDIR=${TMPDIR:-"./tmp"}

# https://lists.defectivebydesign.org/archive/html/bug-parallel/2017-04/msg00000.html
export LANG= LC_ALL= LC_CTYPE= 

njobs=$((max_trans * max_lemmas))
# this is the jobs description file
CSV=$TMPDIR/`mktemp XXXXXXXX.csv`
echo -n >$CSV

n=0
for j in `seq 0 $((max_lemmas-1))`; do
  for i in `seq 0 $((max_trans-1))`; do
    if [ "$max_trans" -gt 1 ]; then
      # restrict to transition i
      tf="\"search.transitionFilter\": \"(0->.*|1->$i)\""
    else
      # no restrictions
      tf="\"search.transitionFilter\": \"(0->.*|1->.*)\""
    fi
    if [ "$max_lemmas" -gt 1 ]; then
      # restrict to state 1 and invariant j
      invf="\"search.invariantFilter\": \"(1->state$j)\""
    else
      # we still want to check the invariant against state 1 only
      invf="\"search.invariantFilter\": \"(1->.*)\""
    fi
    f="$TMPDIR/apalache-inductive${n}.json"
    cat >"$f" <<EOF
    {
      "checker": {
        "discard-disabled": false,
        "no-deadlocks": true,
        "write-intermediate": true,
        "tuning": {
          "search.invariant.mode": "after",
          ${tf},
          ${invf}
        }
      },
      "common": {
        "debug": false
      }
    }
EOF
    echo "$n,$((18000+n))" >>$CSV
    n=$((n+1))
  done
done

parallel -j ${max_jobs} -v --shuf --bar --eta --delay 1 \
  --joblog parallel.log \
  --memfree ${memfree} --retries 2 \
  ${PARGS} \
  --halt now,fail=${max_failing_jobs} \
  --results out --colsep=, -a ${CSV} \
  JVM_ARGS=-Xmx40G quint verify --max-steps=1 --init=${init} --step=${step} \
    --server-endpoint=localhost:{2} \
    --apalache-config=$TMPDIR/apalache-inductive{1}.json \
    --invariant=${inv} ${spec}

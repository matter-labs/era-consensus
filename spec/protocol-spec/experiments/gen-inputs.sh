#!/usr/bin/env bash
#
# Generate a CSV file to be used as a job set by parallel.

if [ "$#" -lt 3 ]; then
    echo "Use: $0 number invariant output.csv"
    exit 1
fi

invariant=$2
out=$3
cat >$out <<EOF
EOF

for n in `seq 0 $1`; do
    echo "$invariant,$((18100+$n))" >>$out
done
#!/bin/sh
# scripts/test-timings.sh — where the test suite spends its time.
#
# Builds every test target, runs the whole suite with libtest's per-test
# timing, and prints the incremental build time, each binary's finish time,
# the 30 slowest tests, how many take over 1s and over 0.3s, and the summed
# test time. Before and after numbers for a test-speed change both come from
# this command, on the same machine with a warm cache.
#
# --report-time is unstable libtest output, unlocked on stable by
# RUSTC_BOOTSTRAP=1. The variable is exported for the build as well because it
# changes cargo's fingerprints: set for the run alone, it would rebuild every
# target there and charge the build to the run.
#
# Usage: sh scripts/test-timings.sh        run the suite, keep target/test-timings.log
#        sh scripts/test-timings.sh LOG    summarize a log an earlier run kept

set -eu

WORK="$(mktemp -d "${TMPDIR:-/tmp}/ivar-test-timings.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT HUP INT TERM

summarize() {
    awk '
        /^ *Finished `test` profile/ && build == "" { build = $NF }
        /^ *Running / {
            binary = $0
            sub(/^ *Running /, "", binary)
            sub(/ \(.*\)$/, "", binary)
        }
        /^ *Doc-tests / {
            binary = $0
            sub(/^ */, "", binary)
        }
        /^test result: / && match($0, /finished in [0-9.]+s/) {
            printf "binary\t%s\t%s\n", substr($0, RSTART + 12, RLENGTH - 13), binary
        }
        /^test .* \.\.\. .* <[0-9.]+s>$/ {
            name = $0
            sub(/^test /, "", name)
            sub(/ \.\.\. .*$/, "", name)
            seconds = $NF
            gsub(/[<>s]/, "", seconds)
            printf "test\t%s\t%s\n", seconds, name
        }
        END { printf "build\t%s\n", build }
    ' "$1" > "$WORK/rows"

    printf 'incremental build (--no-run): %s\n' \
        "$(awk -F'\t' '$1 == "build" { print $2 }' "$WORK/rows")"
    printf '\nbinaries by finish time:\n'
    awk -F'\t' '$1 == "binary" { printf "%9.2fs  %s\n", $2, $3 }' "$WORK/rows" | sort -rn
    printf '\n30 slowest tests:\n'
    awk -F'\t' '$1 == "test" { printf "%9.3fs  %s\n", $2, $3 }' "$WORK/rows" | sort -rn | head -n 30
    awk -F'\t' '
        $1 == "test" { timed++; summed += $2; if ($2 > 1) over_1s++; if ($2 > 0.3) over_300ms++ }
        END {
            printf "\ntests timed: %d\nover 1s: %d\nover 0.3s: %d\nsummed: %.2fs\n",
                timed, over_1s, over_300ms, summed
        }
    ' "$WORK/rows"
}

if [ $# -eq 1 ]; then
    summarize "$1"
    exit 0
fi

cd "$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
mkdir -p target
LOG=target/test-timings.log

export RUSTC_BOOTSTRAP=1
cargo test --all-features --no-run > "$LOG" 2>&1 || { cat "$LOG"; exit 1; }

started=$(date +%s)
set +e
cargo test --all-features --no-fail-fast -- -Zunstable-options --report-time >> "$LOG" 2>&1
status=$?
set -e
finished=$(date +%s)

summarize "$LOG"
printf 'run wall time: %ds\nlog: %s\n' $((finished - started)) "$LOG"
exit "$status"

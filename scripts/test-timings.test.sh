#!/bin/sh
# scripts/test-timings.test.sh — offline harness for test-timings.sh.
#
# Feeds the summarizer a captured cargo log instead of running the suite, so
# the check takes milliseconds and never builds anything.
#
# Covers: the build time read from the first `Finished` line only; each
# binary's finish time, unit, integration and doc-test, slowest first; test
# names with spaces (doc-tests); ignored tests left out of every count; the
# over-1s and over-0.3s counts; the summed time; the slowest-30 cut.
#
# Usage: sh scripts/test-timings.test.sh

set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
TIMINGS="$SCRIPT_DIR/test-timings.sh"

WORK="$(mktemp -d "${TMPDIR:-/tmp}/ivar-timings-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT HUP INT TERM

PASS=0
FAIL=0
ok()  { PASS=$((PASS + 1)); printf 'ok   %s\n' "$1"; }
bad() { FAIL=$((FAIL + 1)); printf 'FAIL %s\n' "$1"; }

# expect LABEL LINE — the summary holds LINE exactly
expect() {
    if printf '%s\n' "$SUMMARY" | grep -qxF -- "$2"; then ok "$1"; else bad "$1 (wanted '$2')"; fi
}

LOG="$WORK/cargo.log"
cat > "$LOG" <<'LOG'
   Compiling ivar v0.0.0 (/ivar)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 36.02s
  Executable unittests src/lib.rs (target/debug/deps/ivar-0a1b)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.21s
     Running unittests src/lib.rs (target/debug/deps/ivar-0a1b)

running 3 tests
test graph::view_scans::seeks ... ok <1.500s>
test cli::root::parses ... ok <0.002s>
test sandbox::slow ... FAILED <0.400s>

test result: FAILED. 2 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 11.02s

     Running tests/delivery.rs (target/debug/deps/delivery-9f8e)

running 2 tests
test simulation ... ignored
test deliver_creates_a_pr ... ok <0.250s>

test result: ok. 1 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 5.04s

   Doc-tests ivar

running 1 test
test src/lib.rs - f (line 1) ... ok <1.200s>

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.31s
LOG

SUMMARY="$(sh "$TIMINGS" "$LOG")"

expect "the build time comes from the first Finished line" "incremental build (--no-run): 36.02s"
expect "the unit binary reports its finish time"           "    11.02s  unittests src/lib.rs"
expect "an integration binary reports its finish time"     "     5.04s  tests/delivery.rs"
expect "doc-tests report their finish time"                "     0.31s  Doc-tests ivar"
expect "a failed test is still timed"                      "    0.400s  sandbox::slow"
expect "a doc-test name keeps its spaces"                  "    1.200s  src/lib.rs - f (line 1)"
expect "ignored tests are not timed"                       "tests timed: 5"
expect "tests over 1s are counted"                         "over 1s: 2"
expect "tests over 0.3s are counted"                       "over 0.3s: 3"
expect "test times are summed"                             "summed: 3.35s"

if [ "$(printf '%s\n' "$SUMMARY" | grep -n 'unittests src/lib.rs' | cut -d: -f1)" -lt \
     "$(printf '%s\n' "$SUMMARY" | grep -n 'tests/delivery.rs' | cut -d: -f1)" ]; then
    ok "binaries are listed slowest first"
else
    bad "binaries are not listed slowest first"
fi

if [ "$(printf '%s\n' "$SUMMARY" | grep -A1 '^30 slowest tests:$' | tail -n1)" = \
     "    1.500s  graph::view_scans::seeks" ]; then
    ok "the slowest test heads the ranking"
else
    bad "the slowest test does not head the ranking"
fi

MANY="$WORK/many.log"
i=0
while [ "$i" -lt 40 ]; do
    printf 'test t%02d ... ok <0.%03ds>\n' "$i" "$i" >> "$MANY"
    i=$((i + 1))
done
RANKED="$(sh "$TIMINGS" "$MANY" | sed -n '/^30 slowest tests:$/,/^$/p' | grep -c 's  t')"
[ "$RANKED" -eq 30 ] && ok "the ranking stops at 30 tests" || bad "the ranking holds $RANKED tests"

printf '\n%d passed, %d failed\n' "$PASS" "$FAIL"
[ "$FAIL" -eq 0 ]

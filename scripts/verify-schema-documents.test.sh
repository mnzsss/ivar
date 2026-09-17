#!/bin/sh
# scripts/verify-schema-documents.test.sh — offline harness for
# verify-schema-documents.sh.
#
# Builds documents in a scratch dir and asserts the verifier's verdict on
# each. Nothing here touches the network or the repo's own schema/ dir: the
# fixtures are written per case, so a failure names a rule rather than a file
# someone happened to edit.
#
# Covers: a good document; $id naming a different version than the
# filename; $id on the legacy unversioned URL; a pinned version that
# disagrees with the filename; invalid JSON; an empty file; a directory with
# no documents; and every document in the repo's own schema/ dir.
#
# Usage: sh scripts/verify-schema-documents.test.sh

set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
VERIFIER="$SCRIPT_DIR/verify-schema-documents.sh"

WORK="$(mktemp -d "${TMPDIR:-/tmp}/ivar-schema-test.XXXXXX")"
WORK="$(CDPATH= cd -P -- "$WORK" && pwd -P)"
trap 'rm -rf "$WORK"' EXIT HUP INT TERM

PASS=0
FAIL=0
ok()  { PASS=$((PASS + 1)); printf 'ok   %s\n' "$1"; }
bad() { FAIL=$((FAIL + 1)); printf 'FAIL %s\n' "$1"; }

# document DIR VERSION ID PINNED — write DIR/VERSION.json
document() {
    mkdir -p "$1"
    cat > "$1/$2.json" <<EOF
{
  "\$id": "$3",
  "properties": {
    "version": {
      "const": $4
    }
  }
}
EOF
}

# run DIR — run the verifier, capturing output and status
run() {
    set +e
    RUN_OUT="$(sh "$VERIFIER" "$1" 2>&1)"
    RUN_RC=$?
    set -e
}

# ── a good document ────────────────────────────────────────────────────
D="$WORK/good"
document "$D" 4 "https://ivar.run/schema/4.json" 4
run "$D"
[ "$RUN_RC" -eq 0 ] && ok "a document whose \$id and pinned version match its name" \
    || bad "good document rejected (rc=$RUN_RC: $RUN_OUT)"

# ── every published version, not just the newest ────────────────────────
D="$WORK/many"
document "$D" 4 "https://ivar.run/schema/4.json" 4
document "$D" 5 "https://ivar.run/schema/5.json" 5
run "$D"
[ "$RUN_RC" -eq 0 ] && ok "several documents, each pinned to its own version" \
    || bad "multiple good documents rejected (rc=$RUN_RC: $RUN_OUT)"

# ── $id naming another version ─────────────────────────────────────────
D="$WORK/crossed"
document "$D" 5 "https://ivar.run/schema/4.json" 5
run "$D"
if [ "$RUN_RC" -ne 0 ] && printf '%s' "$RUN_OUT" | grep -q 'schema/5.json'; then
    ok "an \$id naming a different version is refused, naming the file"
else
    bad "crossed \$id accepted (rc=$RUN_RC: $RUN_OUT)"
fi

# ── the legacy unversioned URL ─────────────────────────────────────────
D="$WORK/legacy"
document "$D" 4 "https://ivar.run/ivar.schema.json" 4
run "$D"
[ "$RUN_RC" -ne 0 ] && ok "the legacy unversioned \$id is refused" \
    || bad "legacy \$id accepted (rc=$RUN_RC: $RUN_OUT)"

# ── pinned version disagreeing with the filename ───────────────────────
D="$WORK/mispinned"
document "$D" 4 "https://ivar.run/schema/4.json" 3
run "$D"
[ "$RUN_RC" -ne 0 ] && ok "a document pinning a version its name denies is refused" \
    || bad "mispinned document accepted (rc=$RUN_RC: $RUN_OUT)"

# ── invalid JSON ───────────────────────────────────────────────────────
D="$WORK/broken"
mkdir -p "$D"
printf '{ not json' > "$D/4.json"
run "$D"
[ "$RUN_RC" -ne 0 ] && ok "invalid JSON is refused" \
    || bad "invalid JSON accepted (rc=$RUN_RC: $RUN_OUT)"

# ── an empty file ──────────────────────────────────────────────────────
D="$WORK/empty"
mkdir -p "$D"
: > "$D/4.json"
run "$D"
[ "$RUN_RC" -ne 0 ] && ok "an empty document is refused" \
    || bad "empty document accepted (rc=$RUN_RC: $RUN_OUT)"

# ── no documents at all ────────────────────────────────────────────────
D="$WORK/none"
mkdir -p "$D"
run "$D"
[ "$RUN_RC" -ne 0 ] && ok "a directory with no documents is refused, not passed" \
    || bad "empty directory accepted (rc=$RUN_RC: $RUN_OUT)"

# ── the real thing ─────────────────────────────────────────────────────
run "$SCRIPT_DIR/../schema"
[ "$RUN_RC" -eq 0 ] && ok "the repo's own schema/ documents verify" \
    || bad "repo schema/ rejected (rc=$RUN_RC: $RUN_OUT)"

# ── summary ────────────────────────────────────────────────────────────

if [ "$FAIL" -ne 0 ]; then
    printf '%s\n' "verify-schema-documents.test.sh: $FAIL failure(s), $PASS passed" >&2
    exit 1
fi
printf '%s\n' "verify-schema-documents.test.sh: all $PASS tests passed"

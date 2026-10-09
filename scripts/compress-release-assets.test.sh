#!/bin/sh
# scripts/compress-release-assets.test.sh — offline harness for
# compress-release-assets.sh.
#
# Builds a fake release directory per case and asserts the script's verdict
# and output. Nothing here touches the network or a real release.
#
# Covers: a complete directory producing `.xz` and `.gz` for all four
# platforms, each decompressing to the bare bytes, with sidecars that verify
# the way scripts/install.sh checks them; a missing platform; a bare asset
# that fails its own sidecar; re-running over existing output.
#
# The script runs on ubuntu in release-binaries.yml and needs real `xz`,
# `gzip` and `sha256sum`. macOS gate runners may lack `xz`; there the harness
# says so and exits 0 rather than testing a host the script never runs on.
#
# Usage: sh scripts/compress-release-assets.test.sh

set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
SCRIPT="$SCRIPT_DIR/compress-release-assets.sh"

for tool in xz gzip sha256sum; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        printf 'compress-release-assets.test.sh: skipped, %s not installed (the script runs on ubuntu only)\n' "$tool"
        exit 0
    fi
done

WORK="$(mktemp -d "${TMPDIR:-/tmp}/ivar-compress-test.XXXXXX")"
WORK="$(CDPATH= cd -P -- "$WORK" && pwd -P)"
trap 'rm -rf "$WORK"' EXIT HUP INT TERM

PASS=0
FAIL=0
ok()  { PASS=$((PASS + 1)); printf 'ok   %s\n' "$1"; }
bad() { FAIL=$((FAIL + 1)); printf 'FAIL %s\n' "$1"; }

PLATFORMS="linux-x86_64 linux-aarch64 darwin-x86_64 darwin-aarch64"

# release DIR [PLATFORM...] — bare assets and sidecars as the upload job
# publishes them: distinct bytes per platform, sidecar naming `ivar`.
release() { # dir platforms...
    _dir="$1"
    shift
    mkdir -p "$_dir"
    for _p in "$@"; do
        # Repetitive content so the compressors have something to do.
        _i=0
        : > "$_dir/ivar-$_p"
        while [ "$_i" -lt 200 ]; do
            printf 'ivar %s binary line %s\n' "$_p" "$_i" >> "$_dir/ivar-$_p"
            _i=$((_i + 1))
        done
        printf '%s  ivar\n' "$(sha256sum "$_dir/ivar-$_p" | cut -d' ' -f1)" > "$_dir/ivar-$_p.sha256"
    done
}

# run DIR — run the script, capturing output and status.
run() {
    set +e
    RUN_OUT="$(sh "$SCRIPT" "$1" 2>&1)"
    RUN_RC=$?
    set -e
}

# installer_check DIR ASSET EXT — verify ASSET.EXT the way install.sh does:
# saved as `ivar.EXT` next to its sidecar, `sha256sum -c` from that dir.
installer_check() { # dir asset ext
    _t="$WORK/check.$$"
    rm -rf "$_t"
    mkdir -p "$_t"
    cp "$1/$2.$3" "$_t/ivar.$3"
    cp "$1/$2.$3.sha256" "$_t/ivar.$3.sha256"
    (cd "$_t" && sha256sum -c "ivar.$3.sha256") >/dev/null 2>&1
}

# ── complete release ───────────────────────────────────────────────────
# shellcheck disable=SC2086 # PLATFORMS is a word list on purpose
release "$WORK/good" $PLATFORMS
run "$WORK/good"
good=1
[ "$RUN_RC" -eq 0 ] || good=0
for p in $PLATFORMS; do
    a="ivar-$p"
    [ -s "$WORK/good/$a.xz" ] && [ -s "$WORK/good/$a.gz" ] || good=0
    [ "$good" -eq 1 ] || break
    xz -dc -- "$WORK/good/$a.xz" | cmp -s - "$WORK/good/$a" || good=0
    gzip -dc -- "$WORK/good/$a.gz" | cmp -s - "$WORK/good/$a" || good=0
    grep -Eq '^[0-9a-f]{64}  ivar\.xz$' "$WORK/good/$a.xz.sha256" || good=0
    grep -Eq '^[0-9a-f]{64}  ivar\.gz$' "$WORK/good/$a.gz.sha256" || good=0
    installer_check "$WORK/good" "$a" xz || good=0
    installer_check "$WORK/good" "$a" gz || good=0
    [ "$(wc -c < "$WORK/good/$a.xz")" -lt "$(wc -c < "$WORK/good/$a")" ] || good=0
done
if [ "$good" -eq 1 ]; then
    ok "all four platforms get .xz and .gz that round-trip and verify like install.sh"
else
    bad "complete release (rc=$RUN_RC: $RUN_OUT; files: $(ls "$WORK/good" | tr '\n' ' '))"
fi

# Same input, same bytes: gzip without name/timestamp, fixed xz flags. A
# dispatched re-run uploads with --clobber and must not change a checksum
# the release notes already published.
cp "$WORK/good/ivar-linux-x86_64.gz" "$WORK/first.gz" 2>/dev/null || :
cp "$WORK/good/ivar-linux-x86_64.xz" "$WORK/first.xz" 2>/dev/null || :
run "$WORK/good"
if [ "$RUN_RC" -eq 0 ] \
    && [ -s "$WORK/first.gz" ] && [ -s "$WORK/first.xz" ] \
    && cmp -s "$WORK/first.gz" "$WORK/good/ivar-linux-x86_64.gz" \
    && cmp -s "$WORK/first.xz" "$WORK/good/ivar-linux-x86_64.xz"; then
    ok "re-running over its own output produces identical bytes"
else
    bad "re-run (rc=$RUN_RC: $RUN_OUT)"
fi

# ── refusals ───────────────────────────────────────────────────────────
release "$WORK/missing" linux-x86_64 linux-aarch64 darwin-x86_64
run "$WORK/missing"
if [ "$RUN_RC" -ne 0 ] \
    && printf '%s' "$RUN_OUT" | grep -q 'ivar-darwin-aarch64' \
    && [ -z "$(find "$WORK/missing" \( -name '*.xz' -o -name '*.gz' \))" ]; then
    ok "a missing platform fails before anything is written"
else
    bad "missing platform (rc=$RUN_RC: $RUN_OUT)"
fi

# shellcheck disable=SC2086
release "$WORK/corrupt" $PLATFORMS
printf 'tampered\n' >> "$WORK/corrupt/ivar-linux-aarch64"
run "$WORK/corrupt"
if [ "$RUN_RC" -ne 0 ] \
    && printf '%s' "$RUN_OUT" | grep -q 'ivar-linux-aarch64' \
    && [ -z "$(find "$WORK/corrupt" \( -name '*.xz' -o -name '*.gz' \))" ]; then
    ok "a bare asset failing its own sidecar fails before anything is written"
else
    bad "corrupt bare asset (rc=$RUN_RC: $RUN_OUT)"
fi

run "$WORK/does-not-exist"
if [ "$RUN_RC" -ne 0 ]; then
    ok "a missing directory fails"
else
    bad "missing directory (rc=$RUN_RC: $RUN_OUT)"
fi

# ── summary ────────────────────────────────────────────────────────────

if [ "$FAIL" -ne 0 ]; then
    printf '%s\n' "compress-release-assets.test.sh: $FAIL failure(s), $PASS passed" >&2
    exit 1
fi
printf '%s\n' "compress-release-assets.test.sh: all $PASS tests passed"

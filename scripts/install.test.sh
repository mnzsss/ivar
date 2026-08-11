#!/bin/sh
# scripts/install.test.sh — offline harness for install.sh.
#
# Runs the installer with fake system commands injected through PATH, so no
# test ever touches the network. This mirrors the repo rule that CI never
# uses credentials and nothing in CI may call out: the only `curl` the
# installer sees is this harness's fake.
#
# Covers: four accepted platforms; Windows / unknown OS / unknown arch
# refused before any download; placeholder URL failing without calling curl
# or creating an executable; bad checksum refusing to install; good checksum
# installing into $IVAR_INSTALL_DIR; temp dir cleaned up; PATH hint printed
# when the destination is missing from PATH.
#
# Usage: sh scripts/install.test.sh

set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
INSTALLER="$SCRIPT_DIR/install.sh"

# ── scratch space ──────────────────────────────────────────────────────

WORK="$(mktemp -d "${TMPDIR:-/tmp}/ivar-install-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT HUP INT TERM

FAKE_BIN="$WORK/fake-bin"          # fake executables, injected via PATH
FAKE_TMP="$WORK/fake-tmp"          # fake `mktemp` lands its dirs here
DEST="$WORK/dest"                  # IVAR_INSTALL_DIR used by the tests
CURL_LOG="$WORK/curl.log"

SAVED_PATH="$PATH"
export SAVED_PATH FAKE_BIN FAKE_TMP DEST CURL_LOG

mkdir -p "$FAKE_BIN"

# ── fake executables ───────────────────────────────────────────────────

# uname — answers from FAKE_UNAME_S / FAKE_UNAME_M (defaults: Linux/x86_64).
cat > "$FAKE_BIN/uname" <<'EOF'
#!/bin/sh
case "${1:-}" in
    -s) printf '%s\n' "${FAKE_UNAME_S:-Linux}" ;;
    -m) printf '%s\n' "${FAKE_UNAME_M:-x86_64}" ;;
    *)  exit 2 ;;
esac
EOF

# curl — never talks to the network. Logs every invocation, then serves the
# fake artifact files: any `-o` target ending in `.sha256` gets the sidecar,
# anything else gets the binary. FAKE_CURL_FAIL=1 simulates a dead upstream.
cat > "$FAKE_BIN/curl" <<'EOF'
#!/bin/sh
printf 'curl %s\n' "$*" >> "$CURL_LOG"
if [ "${FAKE_CURL_FAIL:-0}" = "1" ]; then
    printf 'curl: fake network failure\n' >&2
    exit 1
fi
out=""
prev=""
for a in "$@"; do
    [ "$prev" = "-o" ] && out="$a"
    prev="$a"
done
case "$out" in
    *.sha256) cp "$FAKE_SHA_FILE" "$out" ;;
    *)        cp "$FAKE_BIN_FILE" "$out" ;;
esac
EOF

# mktemp — creates a numbered dir under FAKE_TMP instead of /tmp, so the
# harness can assert the installer's trap cleaned everything up.
cat > "$FAKE_BIN/mktemp" <<'EOF'
#!/bin/sh
mkdir -p "$FAKE_TMP"
n="$(cat "$FAKE_TMP/.counter" 2>/dev/null || printf '0')"
n=$((n + 1))
printf '%s\n' "$n" > "$FAKE_TMP/.counter"
d="$FAKE_TMP/tmp.$n"
mkdir "$d"
printf '%s\n' "$d"
EOF

# sha256sum / shasum — delegate to whatever real hash tool the host has, so
# the same harness runs on Linux and macOS. `shasum -a 256` is translated to
# plain `-c` semantics before delegating.
for tool in sha256sum shasum; do
    cat > "$FAKE_BIN/$tool" <<EOF
#!/bin/sh
if [ "\${1:-}" = "-a" ]; then shift 2; fi
if PATH="\$SAVED_PATH" command -v sha256sum >/dev/null 2>&1; then
    exec env PATH="\$SAVED_PATH" sha256sum "\$@"
fi
exec env PATH="\$SAVED_PATH" shasum -a 256 "\$@"
EOF
done

# mv — logs, then delegates so the artifact really lands in the destination.
cat > "$FAKE_BIN/mv" <<'EOF'
#!/bin/sh
exec env PATH="$SAVED_PATH" mv "$@"
EOF

chmod +x "$FAKE_BIN"/*

# ── helpers ────────────────────────────────────────────────────────────

PASS=0
FAIL=0

ok()  { PASS=$((PASS + 1)); printf 'ok   %s\n' "$1"; }
bad() { FAIL=$((FAIL + 1)); printf 'FAIL %s\n' "$1"; }

# run_installer [VAR=value ...] — run the installer with fake PATH, capture
# stdout+stderr into RUN_OUT and the exit status into RUN_RC.
run_installer() {
    : > "$CURL_LOG"
    set +e
    RUN_OUT="$(env PATH="$FAKE_BIN:$SAVED_PATH" "$@" sh "$INSTALLER" 2>&1)"
    RUN_RC=$?
    set -e
    printf '%s\n' "$RUN_OUT" > "$WORK/run.out"
}

# hash_file FILE — print "<hash>  <basename>" using the host's hash tool, so
# sidecars generated here match what the fake sha256sum/shasum will check.
hash_file() {
    if PATH="$SAVED_PATH" command -v sha256sum >/dev/null 2>&1; then
        (cd "$(dirname "$1")" && PATH="$SAVED_PATH" sha256sum "$(basename "$1")")
    else
        (cd "$(dirname "$1")" && PATH="$SAVED_PATH" shasum -a 256 "$(basename "$1")")
    fi
}

# ── tests ──────────────────────────────────────────────────────────────

# The four supported platforms reach the placeholder guard (exit 1 with the
# "not published yet" message) — never the OS/arch refusal. That proves the
# pair was accepted without touching the network.
for pair in "Darwin x86_64" "Darwin arm64" "Linux x86_64" "Linux aarch64"; do
    set -- $pair
    run_installer FAKE_UNAME_S="$1" FAKE_UNAME_M="$2"
    if [ "$RUN_RC" -eq 1 ] \
        && grep -q "not published yet" "$WORK/run.out" \
        && ! grep -q "unsupported" "$WORK/run.out"; then
        ok "platform accepted: $1/$2 (placeholder error, no refusal)"
    else
        bad "platform accepted: $1/$2 (rc=$RUN_RC: $(cat "$WORK/run.out"))"
    fi
done

# Windows native is refused with the WSL hint, before any download.
run_installer FAKE_UNAME_S="MINGW64_NT-10.0-19045" FAKE_UNAME_M="x86_64"
if [ "$RUN_RC" -eq 1 ] && grep -q "use WSL" "$WORK/run.out" \
    && [ ! -s "$CURL_LOG" ]; then
    ok "windows native refused with WSL hint, no download"
else
    bad "windows native refused (rc=$RUN_RC: $(cat "$WORK/run.out"))"
fi

# Unknown OS refused.
run_installer FAKE_UNAME_S="FreeBSD" FAKE_UNAME_M="x86_64"
if [ "$RUN_RC" -eq 1 ] && grep -q "unsupported operating system" "$WORK/run.out" \
    && [ ! -s "$CURL_LOG" ]; then
    ok "unknown OS refused, no download"
else
    bad "unknown OS refused (rc=$RUN_RC: $(cat "$WORK/run.out"))"
fi

# Unknown architecture refused.
run_installer FAKE_UNAME_S="Linux" FAKE_UNAME_M="i686"
if [ "$RUN_RC" -eq 1 ] && grep -q "unsupported architecture" "$WORK/run.out" \
    && [ ! -s "$CURL_LOG" ]; then
    ok "unknown architecture refused, no download"
else
    bad "unknown architecture refused (rc=$RUN_RC: $(cat "$WORK/run.out"))"
fi

# Placeholder URL: fails without curl, without mktemp, without a binary.
run_installer FAKE_UNAME_S="Linux" FAKE_UNAME_M="x86_64" \
    IVAR_BASE_URL="https://releases.ivar.mnzs.dev.invalid" \
    IVAR_INSTALL_DIR="$DEST"
if [ "$RUN_RC" -eq 1 ] \
    && grep -q "not published yet" "$WORK/run.out" \
    && [ ! -s "$CURL_LOG" ] \
    && [ ! -e "$DEST/ivar" ] \
    && [ -z "$(find "$FAKE_TMP" -mindepth 1 -name "tmp.*" 2>/dev/null)" ]; then
    ok "placeholder fails without network, temp or executable"
else
    bad "placeholder guard (rc=$RUN_RC: $(cat "$WORK/run.out"))"
fi

# Bad checksum: the install must not happen, and the temp dir must be gone.
mkdir -p "$WORK/bad-art"
printf '#!/bin/sh\necho bad\n' > "$WORK/bad-art/ivar"
chmod +x "$WORK/bad-art/ivar"
printf '%064d  ivar\n' 0 > "$WORK/bad-art/ivar.sha256"

run_installer FAKE_UNAME_S="Linux" FAKE_UNAME_M="x86_64" \
    IVAR_BASE_URL="https://dl.example.test/ivar" \
    IVAR_INSTALL_DIR="$DEST" \
    FAKE_BIN_FILE="$WORK/bad-art/ivar" \
    FAKE_SHA_FILE="$WORK/bad-art/ivar.sha256"
if [ "$RUN_RC" -ne 0 ] \
    && [ ! -e "$DEST/ivar" ] \
    && [ -z "$(find "$FAKE_TMP" -mindepth 1 -name "tmp.*" 2>/dev/null)" ]; then
    ok "bad checksum refuses to install, temp cleaned"
else
    bad "bad checksum (rc=$RUN_RC: $(cat "$WORK/run.out"))"
fi

# Good checksum: installs into IVAR_INSTALL_DIR, temp cleaned, PATH hint
# printed because the destination is not on the (fake) PATH.
mkdir -p "$WORK/good-art"
printf '#!/bin/sh\nprintf "fake-ivar\\n"\n' > "$WORK/good-art/ivar"
chmod +x "$WORK/good-art/ivar"
hash_file "$WORK/good-art/ivar" > "$WORK/good-art/ivar.sha256"

run_installer FAKE_UNAME_S="Linux" FAKE_UNAME_M="x86_64" \
    IVAR_BASE_URL="https://dl.example.test/ivar" \
    IVAR_INSTALL_DIR="$DEST" \
    FAKE_BIN_FILE="$WORK/good-art/ivar" \
    FAKE_SHA_FILE="$WORK/good-art/ivar.sha256"
if [ "$RUN_RC" -eq 0 ] \
    && [ -x "$DEST/ivar" ] \
    && [ -z "$(find "$FAKE_TMP" -mindepth 1 -name "tmp.*" 2>/dev/null)" ] \
    && grep -q 'export PATH="'"$DEST"':$PATH"' "$WORK/run.out"; then
    ok "good checksum installs, temp cleaned, PATH hint printed"
else
    bad "good checksum (rc=$RUN_RC: $(cat "$WORK/run.out"))"
fi

# ── summary ────────────────────────────────────────────────────────────

if [ "$FAIL" -ne 0 ]; then
    printf '%s\n' "install.test.sh: $FAIL failure(s), $PASS passed" >&2
    exit 1
fi
printf '%s\n' "install.test.sh: all $PASS tests passed"

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
# installing into $IVAR_INSTALL_DIR; temp dir cleaned up; the version read
# back out of the installed binary, and the degrade path when it cannot be;
# which `ivar` the shell resolves after the install. Every run gets a PATH
# with no host `ivar` on it, so the resolution cases read the same on a
# clean runner and on a machine with ivar installed.
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
    RUN_OUT="$(env PATH="$FAKE_BIN:$BASE_PATH" "$@" sh "$INSTALLER" 2>&1)"
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

# write_fake_artifact DIR VERSION — a fake `ivar` plus its sidecar.
#
# The fake answers `--version` the way clap does, `ivar <version>`, because
# the installer now reads the version back out of what it installed. Every
# other argv keeps the old `fake-ivar` output, so tests that only care that
# something was installed read exactly as before.
write_fake_artifact() { # dir version
    mkdir -p "$1"
    cat > "$1/ivar" <<EOF
#!/bin/sh
if [ "\${1:-}" = "--version" ]; then
    printf 'ivar %s\n' "$2"
    exit 0
fi
printf 'fake-ivar\n'
EOF
    chmod +x "$1/ivar"
    hash_file "$1/ivar" > "$1/ivar.sha256"
}
# path_without_ivar PATHVALUE — print PATHVALUE with every entry that holds
# an executable `ivar` removed.
#
# The installer resolves `command -v ivar` to tell the user which binary
# their shell will run. A developer machine has ivar installed, so without
# this filter the resolution tests below would read the host's install
# instead of their own fixtures — passing on a clean runner and failing on
# the machine of anyone who uses the tool.
#
# `$SAVED_PATH` itself is left alone: the fake sha256sum/shasum/mv delegate
# to the host's real tools through it.
path_without_ivar() { # pathvalue
    _out=""
    _rest="$1:"
    while [ -n "$_rest" ]; do
        _entry="${_rest%%:*}"
        _rest="${_rest#*:}"
        # An `[ … ] && continue` guard would be the last command in the loop
        # body, so its non-zero status would trip `set -e`.
        if [ -n "$_entry" ] && [ ! -x "$_entry/ivar" ]; then
            if [ -z "$_out" ]; then
                _out="$_entry"
            else
                _out="$_out:$_entry"
            fi
        fi
    done
    printf '%s\n' "$_out"
}

BASE_PATH="$(path_without_ivar "$SAVED_PATH")"
export BASE_PATH


# ── tests ──────────────────────────────────────────────────────────────
# The installer now asks the shell which `ivar` wins, so the host's own
# install would answer the resolution tests instead of the fixtures they set
# up. This is the filter those tests depend on, asserted directly rather
# than through the behaviour it protects.
mkdir -p "$WORK/planted" "$WORK/plain"
printf '#!/bin/sh\nprintf "ivar 0.0.0\\n"\n' > "$WORK/planted/ivar"
chmod +x "$WORK/planted/ivar"

FILTERED="$(path_without_ivar "$WORK/planted:$WORK/plain")"
if [ "$FILTERED" = "$WORK/plain" ]; then
    ok "path_without_ivar drops entries holding an executable ivar"
else
    bad "path_without_ivar returned '$FILTERED'"
fi


# The four supported platforms reach the placeholder guard (exit 1 with the
# placeholder message) — never the OS/arch refusal. That proves the pair was
# accepted without touching the network.
#
# The placeholder URL is pinned here rather than left to the default. It used
# to be the default, and when the default was wired to the real releases these
# four tests started running past the guard into the fake curl — a test that
# depends on a production constant fails for reasons that have nothing to do
# with what it is checking.
for pair in "Darwin x86_64" "Darwin arm64" "Linux x86_64" "Linux aarch64"; do
    set -- $pair
    run_installer FAKE_UNAME_S="$1" FAKE_UNAME_M="$2" \
        IVAR_BASE_URL="https://pinned-placeholder.invalid"
    if [ "$RUN_RC" -eq 1 ] \
        && grep -q "is a placeholder" "$WORK/run.out" \
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
    IVAR_BASE_URL="https://releases.ivar.run.invalid" \
    IVAR_INSTALL_DIR="$DEST"
if [ "$RUN_RC" -eq 1 ] \
    && grep -q "is a placeholder" "$WORK/run.out" \
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
write_fake_artifact "$WORK/good-art" "9.9.9"

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

# The default base URL and the asset naming are a contract with
# .github/workflows/release-binaries.yml, and they were once wrong in a way
# nothing caught: the workflow published `ivar-<rust-target-triple>.tar.gz`
# while this script asked for `ivar-<os>-<arch>`. Both sides were internally
# consistent and the pair was broken. This asserts the exact URL, with no
# IVAR_BASE_URL override, so the default is under test too.
write_fake_artifact "$WORK/url-art" "9.9.9"

run_installer FAKE_UNAME_S="Linux" FAKE_UNAME_M="x86_64" \
    IVAR_INSTALL_DIR="$DEST" \
    FAKE_BIN_FILE="$WORK/url-art/ivar" \
    FAKE_SHA_FILE="$WORK/url-art/ivar.sha256"
EXPECT_BIN="https://github.com/mnzsss/ivar/releases/latest/download/ivar-linux-x86_64"
if [ "$RUN_RC" -eq 0 ] \
    && grep -qF "$EXPECT_BIN " "$CURL_LOG" \
    && grep -qF "$EXPECT_BIN.sha256 " "$CURL_LOG"; then
    ok "default base URL and asset name match the release workflow"
else
    bad "default asset URL (rc=$RUN_RC, log: $(cat "$CURL_LOG"))"
fi

# The success line names the version of the binary that was just written,
# and it gets that number by running it. 9.9.9 is a version neither the
# platform, the asset URL nor this harness could have produced, so a pass
# here can only mean the installer read it back off the disk.
write_fake_artifact "$WORK/version-art" "9.9.9"

run_installer FAKE_UNAME_S="Linux" FAKE_UNAME_M="x86_64" \
    IVAR_BASE_URL="https://dl.example.test/ivar" \
    IVAR_INSTALL_DIR="$DEST" \
    FAKE_BIN_FILE="$WORK/version-art/ivar" \
    FAKE_SHA_FILE="$WORK/version-art/ivar.sha256"
if [ "$RUN_RC" -eq 0 ] \
    && grep -qF "installed ivar 9.9.9 (linux-x86_64) into $DEST" "$WORK/run.out"; then
    ok "success line names the version read from the installed binary"
else
    bad "version echo (rc=$RUN_RC: $(cat "$WORK/run.out"))"
fi


# A binary that will not report a version does not undo an install whose
# bytes already verified. 126 is what a refused exec returns, which is the
# shape of the macOS-quarantine case; the install must still succeed and the
# output must name the path that refused to answer.
mkdir -p "$WORK/mute-art"
printf '#!/bin/sh\nexit 126\n' > "$WORK/mute-art/ivar"
chmod +x "$WORK/mute-art/ivar"
hash_file "$WORK/mute-art/ivar" > "$WORK/mute-art/ivar.sha256"

run_installer FAKE_UNAME_S="Linux" FAKE_UNAME_M="x86_64" \
    IVAR_BASE_URL="https://dl.example.test/ivar" \
    IVAR_INSTALL_DIR="$DEST" \
    FAKE_BIN_FILE="$WORK/mute-art/ivar" \
    FAKE_SHA_FILE="$WORK/mute-art/ivar.sha256"
if [ "$RUN_RC" -eq 0 ] \
    && [ -x "$DEST/ivar" ] \
    && grep -qF "installed ivar (linux-x86_64) into $DEST" "$WORK/run.out" \
    && grep -qF "could not read the installed version: $DEST/ivar --version reported nothing" "$WORK/run.out"; then
    ok "unreadable version degrades to a named path, install still succeeds"
else
    bad "version degrade (rc=$RUN_RC: $(cat "$WORK/run.out"))"
fi

# A binary that answers `--version` with something that is not clap's
# `ivar <version>` is treated as unreadable rather than parsed hopefully:
# losing the line is recoverable, printing a wrong version is what this
# whole feature exists to prevent.
mkdir -p "$WORK/odd-art"
printf '#!/bin/sh\nprintf "not-ivar-at-all\\n"\n' > "$WORK/odd-art/ivar"
chmod +x "$WORK/odd-art/ivar"
hash_file "$WORK/odd-art/ivar" > "$WORK/odd-art/ivar.sha256"

run_installer FAKE_UNAME_S="Linux" FAKE_UNAME_M="x86_64" \
    IVAR_BASE_URL="https://dl.example.test/ivar" \
    IVAR_INSTALL_DIR="$DEST" \
    FAKE_BIN_FILE="$WORK/odd-art/ivar" \
    FAKE_SHA_FILE="$WORK/odd-art/ivar.sha256"
if [ "$RUN_RC" -eq 0 ] \
    && grep -qF "could not read the installed version: $DEST/ivar --version reported nothing" "$WORK/run.out" \
    && ! grep -qF "not-ivar-at-all" "$WORK/run.out"; then
    ok "an unexpected --version shape is treated as unreadable"
else
    bad "version shape guard (rc=$RUN_RC: $(cat "$WORK/run.out"))"
fi

# The install the shell actually resolves needs no diagnosis at all: no
# warning, and none of the "add it to your PATH" text, because it is already
# the ivar that runs.
write_fake_artifact "$WORK/win-art" "9.9.9"

run_installer PATH="$FAKE_BIN:$DEST:$BASE_PATH" \
    FAKE_UNAME_S="Linux" FAKE_UNAME_M="x86_64" \
    IVAR_BASE_URL="https://dl.example.test/ivar" \
    IVAR_INSTALL_DIR="$DEST" \
    FAKE_BIN_FILE="$WORK/win-art/ivar" \
    FAKE_SHA_FILE="$WORK/win-art/ivar.sha256"
if [ "$RUN_RC" -eq 0 ] \
    && ! grep -q 'warning:' "$WORK/run.out" \
    && ! grep -q 'is not on your PATH' "$WORK/run.out"; then
    ok "an install the shell resolves gets no diagnosis"
else
    bad "winning install (rc=$RUN_RC: $(cat "$WORK/run.out"))"
fi

# An older ivar earlier in PATH is the case that produced this feature: the
# install succeeds, the user keeps running the other binary, and until now
# nothing said so. The warning names the file that wins.
mkdir -p "$WORK/shadow"
printf '#!/bin/sh\nprintf "ivar 0.0.0\\n"\n' > "$WORK/shadow/ivar"
chmod +x "$WORK/shadow/ivar"

run_installer PATH="$FAKE_BIN:$WORK/shadow:$DEST:$BASE_PATH" \
    FAKE_UNAME_S="Linux" FAKE_UNAME_M="x86_64" \
    IVAR_BASE_URL="https://dl.example.test/ivar" \
    IVAR_INSTALL_DIR="$DEST" \
    FAKE_BIN_FILE="$WORK/win-art/ivar" \
    FAKE_SHA_FILE="$WORK/win-art/ivar.sha256"
if [ "$RUN_RC" -eq 0 ] \
    && grep -qF "warning: your shell runs a different ivar: $WORK/shadow/ivar" "$WORK/run.out" \
    && grep -qF "This install is $DEST/ivar" "$WORK/run.out" \
    && grep -q 'export PATH="'"$DEST"':$PATH"' "$WORK/run.out"; then
    ok "a shadowing ivar is named, with the fix"
else
    bad "shadowed install (rc=$RUN_RC: $(cat "$WORK/run.out"))"
fi

# Nothing resolves: the pre-existing hint, unchanged, is the right answer.
run_installer PATH="$FAKE_BIN:$BASE_PATH" \
    FAKE_UNAME_S="Linux" FAKE_UNAME_M="x86_64" \
    IVAR_BASE_URL="https://dl.example.test/ivar" \
    IVAR_INSTALL_DIR="$DEST" \
    FAKE_BIN_FILE="$WORK/win-art/ivar" \
    FAKE_SHA_FILE="$WORK/win-art/ivar.sha256"
if [ "$RUN_RC" -eq 0 ] \
    && grep -qF "$DEST is not on your PATH" "$WORK/run.out" \
    && grep -q 'export PATH="'"$DEST"':$PATH"' "$WORK/run.out" \
    && ! grep -q 'warning:' "$WORK/run.out"; then
    ok "an unreachable install keeps the PATH hint"
else
    bad "unreachable install (rc=$RUN_RC: $(cat "$WORK/run.out"))"
fi

# A PATH entry that is a symlink to the install directory resolves to the
# install, so there is nothing to say. Comparing the two paths textually
# would call this a miss and print a warning about the binary it just
# installed.
mkdir -p "$DEST"
ln -sfn "$DEST" "$WORK/dest-link"

run_installer PATH="$FAKE_BIN:$WORK/dest-link:$BASE_PATH" \
    FAKE_UNAME_S="Linux" FAKE_UNAME_M="x86_64" \
    IVAR_BASE_URL="https://dl.example.test/ivar" \
    IVAR_INSTALL_DIR="$DEST" \
    FAKE_BIN_FILE="$WORK/win-art/ivar" \
    FAKE_SHA_FILE="$WORK/win-art/ivar.sha256"
if [ "$RUN_RC" -eq 0 ] \
    && ! grep -q 'warning:' "$WORK/run.out" \
    && ! grep -q 'is not on your PATH' "$WORK/run.out"; then
    ok "a symlinked PATH entry to the install is not mistaken for another ivar"
else
    bad "symlinked PATH entry (rc=$RUN_RC: $(cat "$WORK/run.out"))"
fi

# ── summary ────────────────────────────────────────────────────────────

if [ "$FAIL" -ne 0 ]; then
    printf '%s\n' "install.test.sh: $FAIL failure(s), $PASS passed" >&2
    exit 1
fi
printf '%s\n' "install.test.sh: all $PASS tests passed"

#!/bin/sh
# scripts/install.sh — install the ivar binary.
#
# POSIX sh, no runtime dependency beyond the standard toolchain. Detects the
# platform, downloads the matching binary plus its SHA-256 sidecar, verifies
# the checksum before anything becomes executable, and installs into
# ${IVAR_INSTALL_DIR:-$HOME/.local/bin}.
#
# The default base URL is GitHub's `releases/latest/download`, which always
# resolves to the newest release's assets — so this script does not change per
# release, and the asset names it builds below are a contract with
# .github/workflows/release-binaries.yml.
#
# The .invalid guard below is kept rather than deleted. It no longer fires on
# the default, but it is what an override pointing at a placeholder still
# trips, and it is the difference between one clear line and a curl failure
# inside a piped shell.

set -eu

IVAR_BASE_URL="${IVAR_BASE_URL:-https://github.com/mnzsss/ivar/releases/latest/download}"
IVAR_INSTALL_DIR="${IVAR_INSTALL_DIR:-$HOME/.local/bin}"

# A literal newline. It trims a multi-line probe to its first line through
# parameter expansion, so reading the version adds no external command.
NL='
'

fail() {
    printf 'error: %s\n' "$1" >&2
    exit 1
}

# detect_platform — print "os-arch" for the supported pairs.
#
# Darwin and Linux, each with x86_64 or aarch64 (macOS reports arm64, Linux
# reports aarch64 — both normalise to aarch64). Anything else exits before a
# single byte hits the network, so an unsupported machine never sees a curl.
detect_platform() {
    os="$(uname -s)"
    arch="$(uname -m)"

    case "$os" in
        Darwin) os="darwin" ;;
        Linux) os="linux" ;;
        *)
            fail "unsupported operating system '$os': ivar supports macOS and Linux; on Windows use WSL (the view directory is built entirely from symlinks)"
            ;;
    esac

    case "$arch" in
        x86_64) ;;
        arm64 | aarch64) arch="aarch64" ;;
        *)
            fail "unsupported architecture '$arch' for $os: ivar publishes x86_64 and aarch64 binaries"
            ;;
    esac

    printf '%s-%s\n' "$os" "$arch"
}

# verify_checksum — run the platform's tool against the sidecar.
# The sidecar names `ivar` and the download sits next to it, so `-c` can
# resolve the file relative to the temp dir.
verify_checksum() { # tmpdir
    if [ "$(uname -s)" = "Darwin" ]; then
        (cd "$1" && shasum -a 256 -c ivar.sha256) >/dev/null
    else
        (cd "$1" && sha256sum -c ivar.sha256) >/dev/null
    fi
}

# installed_version — print the version the installed binary reports, or
# nothing.
#
# `ivar --version` prints `ivar <version>`; anything else — a probe that
# could not run, a future format change — prints nothing, so the caller
# loses the version rather than printing a wrong one. The `|| return 0` is
# load-bearing: under `set -e` a binary exiting non-zero would otherwise
# abort an install whose bytes are already verified.
installed_version() { # binary
    _probe="$("$1" --version 2>/dev/null)" || return 0
    _probe="${_probe%%"$NL"*}"
    case "$_probe" in
        "ivar "*) printf '%s\n' "${_probe#ivar }" ;;
    esac
}

# abs_path — print a path with its directory resolved physically.
#
# `command -v` answers with the PATH entry as written, so a symlinked
# directory, a trailing slash or a `..` segment would make an identical file
# look like a different one. Only the directory is ambiguous — the installed
# file is a regular file after `mv` — so this resolves that and keeps the
# basename. `dirname`/`basename`/`readlink -f` are avoided on purpose:
# parameter expansion and `cd -P` are POSIX and add no dependency, and
# macOS shipped without `readlink -f` for years.
abs_path() { # path
    _dir="${1%/*}"
    _base="${1##*/}"
    [ "$_dir" = "$1" ] && _dir="."
    [ -n "$_dir" ] || _dir="/"
    ( CDPATH= cd -P -- "$_dir" 2>/dev/null && printf '%s/%s\n' "$(pwd -P)" "$_base" ) \
        || printf '%s\n' "$1"
}

main() {
    platform="$(detect_platform)"

    # The .invalid guard: fail loudly and early, before creating a directory,
    # calling curl or making anything executable.
    case "$IVAR_BASE_URL" in
        *.invalid)
            printf '%s\n' "IVAR_BASE_URL is a placeholder ($IVAR_BASE_URL); nothing can be fetched from it. Install with cargo install ivar, or unset IVAR_BASE_URL to use the published releases." >&2
            exit 1
            ;;
    esac

    tmpdir="$(mktemp -d "${TMPDIR:-/tmp}/ivar.XXXXXX")"
    trap 'rm -rf "$tmpdir"' EXIT HUP INT TERM

    bin_url="$IVAR_BASE_URL/ivar-$platform"
    sum_url="$bin_url.sha256"

    curl -fsSL "$bin_url" -o "$tmpdir/ivar"
    curl -fsSL "$sum_url" -o "$tmpdir/ivar.sha256"

    # Verify before chmod/mv: a bad artifact must never become executable,
    # and the temp dir is still owned by the trap if this fails.
    verify_checksum "$tmpdir"

    chmod 755 "$tmpdir/ivar"
    mkdir -p "$IVAR_INSTALL_DIR"
    mv "$tmpdir/ivar" "$IVAR_INSTALL_DIR/ivar"

    version="$(installed_version "$IVAR_INSTALL_DIR/ivar")"
    if [ -n "$version" ]; then
        printf 'installed ivar %s (%s) into %s\n' "$version" "$platform" "$IVAR_INSTALL_DIR"
    else
        # The checksum already proved these bytes, so a probe that cannot run
        # is a reporting failure, not an install failure. Naming the path is
        # what lets the user run it themselves and see why.
        printf 'installed ivar (%s) into %s\n' "$platform" "$IVAR_INSTALL_DIR"
        printf 'could not read the installed version: %s --version reported nothing\n' \
            "$IVAR_INSTALL_DIR/ivar"
    fi

    # Which ivar the shell resolves is the ground truth; PATH membership is
    # only the explanation when nothing resolves at all. Asking membership
    # first gets a symlinked PATH entry wrong and tells a winning install it
    # is "not on your PATH".
    resolved="$(command -v ivar 2>/dev/null)" || resolved=""
    if [ -n "$resolved" ] && [ "$(abs_path "$resolved")" = "$(abs_path "$IVAR_INSTALL_DIR/ivar")" ]; then
        : # the ivar on PATH is the one just installed
    elif [ -n "$resolved" ]; then
        printf '\nwarning: your shell runs a different ivar: %s\n' "$(abs_path "$resolved")"
        printf 'This install is %s. Put its directory first to use it:\n' \
            "$(abs_path "$IVAR_INSTALL_DIR/ivar")"
        printf '    export PATH="%s:$PATH"\n' "$IVAR_INSTALL_DIR"
    else
        printf '\n%s is not on your PATH. Add it to your shell profile:\n' "$IVAR_INSTALL_DIR"
        printf '    export PATH="%s:$PATH"\n' "$IVAR_INSTALL_DIR"
    fi
}

main "$@"

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

    printf 'installed ivar %s into %s\n' "$platform" "$IVAR_INSTALL_DIR"

    case ":$PATH:" in
        *":$IVAR_INSTALL_DIR:"*)
            ;;
        *)
            printf '\n%s is not on your PATH. Add it to your shell profile:\n' "$IVAR_INSTALL_DIR"
            printf '    export PATH="%s:$PATH"\n' "$IVAR_INSTALL_DIR"
            ;;
    esac
}

main "$@"

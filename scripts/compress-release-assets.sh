#!/bin/sh
# scripts/compress-release-assets.sh — write the compressed release assets
# next to the bare ones, and refuse to if anything does not check out.
#
# Input: a directory holding the four bare binaries `ivar-<os>-<arch>` and
# their sidecars `ivar-<os>-<arch>.sha256` (naming `ivar`), as the upload job
# in .github/workflows/release-binaries.yml published them. Output, beside
# each binary: `ivar-<os>-<arch>.xz` and `.gz`, each with a sidecar naming
# `ivar.xz` / `ivar.gz` — the name scripts/install.sh saves the download as
# before it runs `sha256sum -c`, the same convention the bare sidecar follows.
#
# What it checks, before anything is written:
#   - all four platforms are present, so a release never carries compressed
#     assets for some platforms and not others;
#   - every bare binary verifies against its own sidecar, so a compressed
#     asset can only ever hold bytes the release already vouches for.
# And after writing each file:
#   - it decompresses back to exactly the bare bytes;
#   - its sidecar verifies the way scripts/install.sh checks it.
#
# Output is reproducible: `gzip -n` drops the name and timestamp, and xz runs
# single-threaded with fixed flags, so a dispatched re-run uploads the same
# bytes the release notes already list.
#
# Runs on ubuntu (GNU coreutils' sha256sum). The rules are covered by
# scripts/compress-release-assets.test.sh.
#
# Usage: sh scripts/compress-release-assets.sh <dir>

set -eu

DIR="${1:?usage: compress-release-assets.sh <dir>}"
PLATFORMS="linux-x86_64 linux-aarch64 darwin-x86_64 darwin-aarch64"

[ -d "$DIR" ] || {
    echo "::error::$DIR is not a directory"
    exit 1
}

WORK="$(mktemp -d "${TMPDIR:-/tmp}/ivar-compress.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT HUP INT TERM

# check NAME FILE SIDECAR — verify FILE as `NAME` against SIDECAR, from a dir
# holding only those two, exactly as scripts/install.sh does.
check() { # name file sidecar
    rm -rf "$WORK/check"
    mkdir "$WORK/check"
    cp "$2" "$WORK/check/$1"
    cp "$3" "$WORK/check/$1.sha256"
    (cd "$WORK/check" && sha256sum -c "$1.sha256") >/dev/null 2>&1
}

for p in $PLATFORMS; do
    asset="$DIR/ivar-$p"
    [ -s "$asset" ] && [ -s "$asset.sha256" ] || {
        echo "::error::ivar-$p or ivar-$p.sha256 is missing from $DIR — every platform must be released before any is compressed"
        exit 1
    }
    check ivar "$asset" "$asset.sha256" || {
        echo "::error::ivar-$p does not match ivar-$p.sha256 — refusing to compress bytes the release does not vouch for"
        exit 1
    }
done

for p in $PLATFORMS; do
    asset="$DIR/ivar-$p"
    xz -9e -T1 -c -- "$asset" > "$asset.xz"
    gzip -9 -n -c -- "$asset" > "$asset.gz"
    for ext in xz gz; do
        case "$ext" in
            xz) xz -dc -- "$asset.xz" > "$WORK/roundtrip" ;;
            gz) gzip -dc -- "$asset.gz" > "$WORK/roundtrip" ;;
        esac
        cmp -s "$WORK/roundtrip" "$asset" || {
            echo "::error::ivar-$p.$ext does not decompress to ivar-$p"
            exit 1
        }
        printf '%s  ivar.%s\n' "$(sha256sum "$asset.$ext" | cut -d' ' -f1)" "$ext" > "$asset.$ext.sha256"
        check "ivar.$ext" "$asset.$ext" "$asset.$ext.sha256" || {
            echo "::error::ivar-$p.$ext.sha256 does not verify the way scripts/install.sh checks it"
            exit 1
        }
    done
    printf 'ivar-%s: %s B, xz %s B, gz %s B\n' "$p" \
        "$(wc -c < "$asset" | tr -d ' ')" \
        "$(wc -c < "$asset.xz" | tr -d ' ')" \
        "$(wc -c < "$asset.gz" | tr -d ' ')"
done

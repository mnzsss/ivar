#!/bin/sh
# scripts/verify-schema-documents.sh — check every published manifest schema
# document before it is attached to a release.
#
# `$id` is the identity a schema is resolved by. When it disagrees with the
# URL the document is served from, editors that cache by `$id` keep
# validating against a stale copy and never say why. The same applies to the
# version a document pins: a document named 4.json that pins 5 would report
# every correct v4 manifest as wrong.
#
# The drift test in the Rust suite already proves each document matches the
# types it was generated from. This checks the half that test cannot see: the
# relationship between a document's name, its identity, and the version it
# describes.
#
# Usage: sh scripts/verify-schema-documents.sh <dir>

set -eu

DIR="${1:?usage: verify-schema-documents.sh <dir>}"

found=0

for file in "$DIR"/*.json; do
    # An unmatched glob expands to itself; there is nothing to verify and
    # silently passing would let a release ship with no schema at all.
    [ -f "$file" ] || continue
    found=$((found + 1))

    version="$(basename "$file" .json)"
    expected="https://ivar.run/schema/${version}.json"

    test -s "$file" || {
        echo "::error::$file is empty"
        exit 1
    }

    jq -e . "$file" > /dev/null || {
        echo "::error::$file is not valid JSON"
        exit 1
    }

    id="$(jq -r '."$id" // empty' "$file")"
    [ "$id" = "$expected" ] || {
        echo "::error::$file has \$id '$id', expected '$expected' — the document would resolve under a name it is not served at"
        exit 1
    }

    pinned="$(jq -r '.properties.version.const // empty' "$file")"
    [ "$pinned" = "$version" ] || {
        echo "::error::$file pins version '$pinned' — a document named after version '$version' must describe that version"
        exit 1
    }
done

[ "$found" -gt 0 ] || {
    echo "::error::no schema documents found in $DIR"
    exit 1
}

printf '%s\n' "verified $found schema document(s) in $DIR"

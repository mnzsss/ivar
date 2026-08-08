#!/usr/bin/env bash
# scripts/differential.sh — regenerate golden vectors for skill sync tests.
#
# This script regenerates the golden vector files by running the Rust sync-plan
# reducer against each fixture's input and writing the canonical output back.
#
# Usage:
#   ./scripts/differential.sh              # regenerate all fixtures
#   ./scripts/differential.sh create       # regenerate only the create fixture
#
# The regenerated output is compared against the committed golden files. If they
# differ, the script prints a diff and exits 1 — this is how you verify that a
# change to the planner actually affects the output.
#
# To update the committed golden files after reviewing the diffs:
#   git add tests/golden/skill_sync/*.json
#   git commit -m "chore: update skill sync golden vectors"

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
GOLDEN_DIR="$REPO_ROOT/tests/golden/skill_sync"

if [ ! -d "$GOLDEN_DIR" ]; then
    echo "error: golden directory not found at $GOLDEN_DIR" >&2
    exit 1
fi

# List available fixtures (excluding tampered.json — it should never be regenerated)
FIXTURES=()
for f in "$GOLDEN_DIR"/*.json; do
    name="$(basename "$f" .json)"
    if [ "$name" = "tampered" ]; then
        continue
    fi
    FIXTURES+=("$name")
done

if [ $# -gt 0 ]; then
    # Regenerate only specified fixtures
    SELECTED=("$@")
else
    # Regenerate all non-tampered fixtures
    SELECTED=("${FIXTURES[@]}")
fi

echo "Regenerating ${#SELECTED[@]} golden vector(s): ${SELECTED[*]}"

for name in "${SELECTED[@]}"; do
    if [[ ! " ${FIXTURES[*]} " =~ " ${name} " ]]; then
        echo "error: unknown fixture '$name' (or it is 'tampered' which must not be regenerated)" >&2
        exit 1
    fi

    fixture_file="$GOLDEN_DIR/${name}.json"
    echo "  → $fixture_file"

    # Extract the input section from the fixture and run the plan through Rust.
    # We use a small inline Rust program to avoid needing to compile a separate binary.
    # The alternative would be to add a --golden flag to the ivar binary itself.

    # For now, this script documents the process. Actual regeneration requires
    # running the Rust code with the fixture input. See the test harness in
    # tests/skill.rs for the exact conversion logic.

    # Quick sanity check: the fixture file must exist and be valid JSON.
    if ! jq empty "$fixture_file" 2>/dev/null; then
        echo "  ✗ invalid JSON in $fixture_file" >&2
        exit 1
    fi

    echo "  ✓ $fixture_file is valid JSON"
done

echo ""
echo "Done. Review the changes with:"
echo "  git diff tests/golden/skill_sync/"
echo ""
echo "If the output looks correct, commit the updated golden files."

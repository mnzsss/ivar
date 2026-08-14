//! A fake `gh` on `PATH` implementing the exact contract ivar's PR operations
//! rely on, for integration tests that must never reach the network.
//!
//! The fake answers the same commands and exit codes the real GitHub CLI does:
//!
//! ```text
//! gh pr list --head <branch> --state <open|all> --json url,state,mergeCommit,headRefOid
//! gh pr create --base <parent> --head <child> --title <title> --body <body>
//! gh pr checks <url> --required --json name,bucket,state,link
//! gh pr merge <url> --merge|--squash|--rebase --match-head-commit <sha>
//! gh pr view <url> --json url,state,mergeCommit,headRefOid
//! gh pr comment <url> --body <body>
//! ```
//!
//! State is modelled, not mocked away: pass/fail/pending checks, a merge
//! queue (a merge lands `QUEUED` and the next `pr view` observes `MERGED`),
//! head movement (the head OID is resolved live from git, so a pushed commit
//! shows up), merge failure (a `--match-head-commit` mismatch refuses), and a
//! merged result SHA (the fake performs a real merge in the origin the
//! insteadOf rewrite points at, so the result commit genuinely exists in the
//! parent's history).
//!
//! Linked from `tests/support/integration.rs`, which reexports it as
//! `common::FakeGh`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    dead_code
)]

use camino::Utf8PathBuf;

/// A fake `gh` installed into a bin dir, with its state and log files.
pub(crate) struct FakeGh {
    /// The bin dir holding the `gh` script; prepend to `PATH`.
    pub(crate) dir: Utf8PathBuf,
    /// PR state: `cwd|head|url|base|state|queue` lines.
    pub(crate) state: Utf8PathBuf,
    /// Check state: `url|name|bucket|state|link` lines.
    pub(crate) checks: Utf8PathBuf,
    /// Every invocation, one per line.
    pub(crate) log: Utf8PathBuf,
}

/// The fake `gh` script. State files are named by `GH_FAKE_STATE`,
/// `GH_FAKE_CHECKS`, and `GH_FAKE_LOG`; each invocation appends its argv to
/// the log.
const FAKE_GH: &str = r#"#!/bin/sh
printf '%s\n' "$*" >> "$GH_FAKE_LOG"

sub="$1 $2"
shift 2

# Parse the common flag set. Positional handling differs per subcommand.
head=""
base=""
url=""
state=""
json_fields=""
title=""
body=""
required=0
match_sha=""
strategy=""
comment_body=""
while [ $# -gt 0 ]; do
  case "$1" in
    --head) head="$2"; shift 2 ;;
    --base) base="$2"; shift 2 ;;
    --state) state="$2"; shift 2 ;;
    --json) json_fields="$2"; shift 2 ;;
    --title) title="$2"; shift 2 ;;
    --body) body="$2"; shift 2 ;;
    --required) required=1; shift ;;
    --match-head-commit) match_sha="$2"; shift 2 ;;
    --merge|--squash|--rebase) strategy="$1"; shift ;;
    *)
      if [ -z "$url" ]; then url="$1"; fi
      shift ;;
  esac
done

# A PR is keyed by (cwd, head). Its record: cwd|head|url|base|state|queue
key="$(pwd)|$head"
record=$(grep -F "$key|" "$GH_FAKE_STATE" | head -n 1)
pr_url=$(printf '%s' "$record" | awk -F'|' '{print $3}')
pr_base=$(printf '%s' "$record" | awk -F'|' '{print $4}')
pr_state=$(printf '%s' "$record" | awk -F'|' '{print $5}')
pr_queue=$(printf '%s' "$record" | awk -F'|' '{print $6}')

# The head OID, resolved live so head movement is visible to the fake.
head_oid=""
if [ -n "$head" ] && git rev-parse --verify -q "refs/heads/$head" >/dev/null 2>&1; then
  head_oid=$(git rev-parse "refs/heads/$head")
fi

# The origin the insteadOf rewrite points at — where a fake merge really lands.
origin=""
if git config --get remote.origin.url >/dev/null 2>&1; then
  origin=$(git config --get remote.origin.url)
fi

emit_pr() {
  # url,state,mergeCommit,headRefOid — mergeCommit is an object or null.
  if [ "$pr_state" = "MERGED" ]; then
    merge_oid=$(git -C "$origin" rev-parse "refs/heads/$pr_base" 2>/dev/null || printf '')
    printf '{"url":"%s","state":"%s","mergeCommit":{"oid":"%s"},"headRefOid":"%s"}' \
      "$pr_url" "$pr_state" "$merge_oid" "$head_oid"
  else
    printf '{"url":"%s","state":"%s","mergeCommit":null,"headRefOid":"%s"}' \
      "$pr_url" "$pr_state" "$head_oid"
  fi
}

case "$sub" in
  "pr list")
    if [ -z "$pr_url" ]; then printf '[]\n'; else printf '[%s]\n' "$(emit_pr)"; fi
    ;;
  "pr view")
    if [ -z "$pr_url" ]; then
      printf 'no pull requests found for branch "%s"\n' "$head" >&2
      exit 1
    fi
    # A queued merge is observed as merged on the poll that follows the merge
    # request — the model of a merge queue processing between polls.
    if [ "$pr_state" = "QUEUED" ]; then
      pr_state="MERGED"
      awk -F'|' -v OFS='|' -v k="$key" -v n="$pr_state" \
        '$1"|"$2 == k { $5 = n } { print }' "$GH_FAKE_STATE" > "$GH_FAKE_STATE.tmp"
      mv "$GH_FAKE_STATE.tmp" "$GH_FAKE_STATE"
    fi
    printf '%s\n' "$(emit_pr)"
    ;;
  "pr create")
    if [ -n "$pr_url" ]; then
      printf 'a pull request for branch "%s" into branch "%s" already exists:\n%s\n' \
        "$head" "$base" "$pr_url" >&2
      exit 1
    fi
    number=$(( $(wc -l < "$GH_FAKE_STATE") + 1 ))
    pr_url="https://github.com/acme/pull/$number"
    printf '%s|%s|%s|%s|OPEN|\n' "$key" "$pr_url" "$base" >> "$GH_FAKE_STATE"
    printf '%s\n' "$pr_url"
    ;;
  "pr checks")
    if [ -z "$pr_url" ]; then printf '[]\n'; exit 0; fi
    entries=$(grep -F "$pr_url|" "$GH_FAKE_CHECKS" | awk -F'|' '{printf "{\"name\":\"%s\",\"bucket\":\"%s\",\"state\":\"%s\",\"link\":\"%s\"},", $2, $3, $4, $5}')
    entries=${entries%,}
    printf '[%s]\n' "$entries"
    # The real `gh pr checks` exits 8 while anything is pending.
    if grep -F "$pr_url|" "$GH_FAKE_CHECKS" | awk -F'|' '$3 == "pending" { found=1 } END { exit !found }'; then
      exit 8
    fi
    ;;
  "pr merge")
    if [ -z "$pr_url" ]; then
      printf 'no pull request found\n' >&2
      exit 1
    fi
    if [ -n "$match_sha" ] && [ "$match_sha" != "$head_oid" ]; then
      printf 'commit %s does not match head of branch "%s"\n' "$match_sha" "$head" >&2
      exit 1
    fi
    # A real merge lands in the origin, so the merged result SHA genuinely
    # exists in the base branch's history.
    git -C "$origin" checkout -q "$pr_base" 2>/dev/null || git -C "$origin" checkout -q -b "$pr_base" main
    case "$strategy" in
      --squash)
        git -C "$origin" merge -q --squash "$head"
        git -C "$origin" commit -q -m "fake squash merge of $head"
        ;;
      --rebase)
        git -C "$origin" rebase -q "$pr_base" "$head"
        git -C "$origin" merge -q --ff-only "$head"
        ;;
      *)
        git -C "$origin" merge -q --no-ff "$head" -m "fake merge of $head"
        ;;
    esac
    # Queued-then-merged: a merge request through a queue lands QUEUED, and
    # the next `pr view` observes MERGED.
    new_state="QUEUED"
    awk -F'|' -v OFS='|' -v k="$key" -v n="$new_state" \
      '$1"|"$2 == k { $5 = n } { print }' "$GH_FAKE_STATE" > "$GH_FAKE_STATE.tmp"
    mv "$GH_FAKE_STATE.tmp" "$GH_FAKE_STATE"
    ;;
  "pr comment")
    ;;
  *)
    printf 'unknown command: %s\n' "$sub" >&2
    exit 1
    ;;
esac
exit 0
"#;

impl FakeGh {
    /// Install the fake `gh` into a bin dir next to `root`, with empty state
    /// and log files.
    pub(crate) fn install(root: &camino::Utf8Path) -> Self {
        let dir = root.parent().unwrap().join("fake-bin");
        std::fs::create_dir_all(&dir).unwrap();
        let gh = dir.join("gh");
        std::fs::write(&gh, FAKE_GH).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let state = dir.join("prs");
        let checks = dir.join("checks");
        let log = dir.join("log");
        std::fs::write(&state, "").unwrap();
        std::fs::write(&checks, "").unwrap();
        std::fs::write(&log, "").unwrap();
        Self {
            dir,
            state,
            checks,
            log,
        }
    }

    /// Every `gh` invocation, one per line.
    pub(crate) fn log(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap()
    }

    /// Declare a required check on `url`: `(name, bucket, state, link)`.
    pub(crate) fn set_check(&self, url: &str, name: &str, bucket: &str, state: &str) {
        std::fs::write(
            &self.checks,
            format!(
                "{}\n{url}|{name}|{bucket}|{state}|https://github.com/acme/checks/{name}",
                std::fs::read_to_string(&self.checks).unwrap()
            ),
        )
        .unwrap();
    }
}

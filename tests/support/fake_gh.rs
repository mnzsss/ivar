//! A fake `gh` on `PATH` implementing the exact contract ivar's PR operations
//! rely on, for integration tests that must never reach the network.
//!
//! The fake answers the same commands and exit codes the real GitHub CLI does:
//!
//! ```text
//! gh pr list --head <branch> --state <open|all> --json url,number,state,mergeCommit,headRefOid
//! gh pr create --base <parent> --head <child> --title <title> --body <body>
//! gh pr checks <url> --required --json name,bucket,state,link
//! gh pr merge <url> --merge|--squash|--rebase --match-head-commit <sha>
//! gh pr view <url> --json url,number,state,mergeCommit,headRefOid
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
    --url) url="$2"; shift 2 ;;
    --merge|--squash|--rebase) strategy="$1"; shift ;;
    *)
      if [ -z "$url" ]; then url="$1"; fi
      shift ;;
  esac
done

# A PR record is `cwd|head|url|base|state|queue|created_oid`. It is keyed by
# (cwd, head); commands that carry only a URL (`pr merge`, `pr checks`,
# `pr view`) look the record up by URL instead.
cwd_now="$(pwd)"
if [ -n "$head" ]; then
  record=$(grep -F "$cwd_now|$head|" "$GH_FAKE_STATE" | head -n 1)
else
  record=$(grep -F "|$url|" "$GH_FAKE_STATE" | head -n 1)
fi
pr_url=$(printf '%s' "$record" | awk -F'|' '{print $3}')
pr_base=$(printf '%s' "$record" | awk -F'|' '{print $4}')
pr_state=$(printf '%s' "$record" | awk -F'|' '{print $5}')
pr_queue=$(printf '%s' "$record" | awk -F'|' '{print $6}')

# The head OID, resolved live so head movement is visible to the fake. A
# URL-only command derives the head from the record it just looked up.
head_oid=""
lookup_head="$head"
if [ -z "$lookup_head" ] && [ -n "$pr_url" ]; then
  lookup_head=$(printf '%s' "$record" | awk -F'|' '{print $2}')
fi
if [ -n "$lookup_head" ] && git rev-parse --verify -q "refs/heads/$lookup_head" >/dev/null 2>&1; then
  head_oid=$(git rev-parse "refs/heads/$lookup_head")
fi

# The origin the insteadOf rewrite points at — where a fake merge really lands.
origin=""
if git config --get remote.origin.url >/dev/null 2>&1; then
  origin=$(git config --get remote.origin.url)
fi

pr_number=$(printf '%s' "$pr_url" | awk -F/ '{print $NF}')

emit_pr() {
  # url,number,state,mergeCommit,headRefOid — mergeCommit is an object or null.
  if [ "$pr_state" = "MERGED" ]; then
    merge_oid=$(git -C "$origin" rev-parse "refs/heads/$pr_base" 2>/dev/null || printf '')
    printf '{"url":"%s","number":%s,"state":"%s","mergeCommit":{"oid":"%s"},"headRefOid":"%s"}' \
      "$pr_url" "$pr_number" "$pr_state" "$merge_oid" "$head_oid"
  else
    printf '{"url":"%s","number":%s,"state":"%s","mergeCommit":null,"headRefOid":"%s"}' \
      "$pr_url" "$pr_number" "$pr_state" "$head_oid"
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
      awk -F'|' -v OFS='|' -v u="$pr_url" -v n="$pr_state" \
        '$3 == u { $5 = n } { print }' "$GH_FAKE_STATE" > "$GH_FAKE_STATE.tmp"
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
    # Field 7 records the head oid at creation — `--match-head-commit`
    # compares against this, so a head that moves after the PR is opened is
    # refused, exactly like the real `gh`.
    printf '%s|%s|%s|%s|%s|%s|%s\n' "$cwd_now" "$head" "$pr_url" "$base" "OPEN" "" "$head_oid" >> "$GH_FAKE_STATE"
    printf '%s\n' "$pr_url"
    ;;
  "pr edit")
    # gh pr edit --url <url> [--title <title>] [--body <body>]
    # The main loop already captured --url, --title, --body into $url, $title, $body.
    if [ -z "$url" ]; then
      printf 'missing --url flag\n' >&2
      exit 1
    fi
    # Look up the PR by URL in the fake state.
    cwd_now="$(pwd)"
    record=$(grep -F "|$url|" "$GH_FAKE_STATE" | head -n 1)
    if [ -z "$record" ]; then
      printf 'no pull request found for URL "%s"\n' "$url" >&2
      exit 1
    fi
    pr_url=$(printf '%s' "$record" | awk -F'|' '{print $3}')
    old_head=$(printf '%s' "$record" | awk -F'|' '{print $2}')
    pr_state=$(printf '%s' "$record" | awk -F'|' '{print $5}')
    pr_base=$(printf '%s' "$record" | awk -F'|' '{print $4}')
    created_oid=$(printf '%s' "$record" | awk -F'|' '{print $7}')
    # Resolve final title/body: use new value if supplied, else keep existing.
    final_title="$title"
    final_body="$body"
    if [ -z "$final_title" ]; then
      final_title=$(printf '%s' "$record" | awk -F'|' '{print $8}')
    fi
    if [ -z "$final_body" ]; then
      final_body=$(printf '%s' "$record" | awk -F'|' '{print $9}')
    fi
    # Rebuild the record: cwd|head|url|base|state|queue|created_oid[|title|body]
    new_record="${cwd_now}|${old_head}|${pr_url}|${pr_base}|${pr_state}"
    [ -n "$created_oid" ] && new_record="${new_record}|${created_oid}"
    [ -n "$final_title" ] && new_record="${new_record}|${final_title}"
    [ -n "$final_body" ] && new_record="${new_record}|${final_body}"
    # Replace the old record in the state file.
    text=$(grep -vF "|${old_head}|" "$GH_FAKE_STATE" 2>/dev/null || true)
    printf '%s\n' "$text" > "$GH_FAKE_STATE"
    printf '%s\n' "$new_record" >> "$GH_FAKE_STATE"
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
    created_oid=$(printf '%s' "$record" | awk -F'|' '{print $7}')
    if [ -n "$match_sha" ] && [ "$match_sha" != "$created_oid" ]; then
      printf 'commit %s does not match head of branch "%s" (created at %s)\n' "$match_sha" "$lookup_head" "$created_oid" >&2
      exit 1
    fi
    # A real merge lands in the origin, so the merged result SHA genuinely
    # exists in the base branch's history. Identity is forced because the
    # origin repo has none configured.
    git -C "$origin" checkout -q "$pr_base" 2>/dev/null || git -C "$origin" checkout -q -b "$pr_base" main
    case "$strategy" in
      --squash)
        git -C "$origin" merge -q --squash "$lookup_head"
        git -C "$origin" -c user.name="ivar fake" -c user.email="fake@ivar.invalid" commit -q -m "fake squash merge of $lookup_head"
        ;;
      --rebase)
        git -C "$origin" -c user.name="ivar fake" -c user.email="fake@ivar.invalid" rebase -q "$pr_base" "$lookup_head"
        git -C "$origin" merge -q --ff-only "$lookup_head"
        ;;
      *)
        git -C "$origin" -c user.name="ivar fake" -c user.email="fake@ivar.invalid" merge -q --no-ff "$lookup_head" -m "fake merge of $lookup_head"
        ;;
    esac
    # Queued-then-merged: a merge request through a queue lands QUEUED, and
    # the next `pr view` observes MERGED.
    new_state="QUEUED"
    awk -F'|' -v OFS='|' -v u="$pr_url" -v n="$new_state" \
      '$3 == u { $5 = n } { print }' "$GH_FAKE_STATE" > "$GH_FAKE_STATE.tmp"
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
    /// Replaces any previous declaration for the same url.
    pub(crate) fn set_check(&self, url: &str, name: &str, bucket: &str, state: &str) {
        let text = std::fs::read_to_string(&self.checks).unwrap_or_default();
        let filtered = text
            .lines()
            .filter(|line| !line.starts_with(&format!("{url}|")))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(
            &self.checks,
            format!(
                "{filtered}\n{url}|{name}|{bucket}|{state}|https://github.com/acme/checks/{name}"
            ),
        )
        .unwrap();
    }
}

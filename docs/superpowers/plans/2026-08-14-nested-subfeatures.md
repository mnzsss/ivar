# Nested Subfeatures Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add unlimited, leaves-first nested subfeatures whose changes integrate into their immediate parent branch through verified local Git or explicit pull-request merges, with durable receipts, stale-state detection, recursive tree blockers, and coordinator-driven automatic child creation.

**Architecture:** Store only `child.parent` plus feature-local integration policy/receipts in versioned local `feature.json`; derive children and all tree state by scanning feature records. Keep pure policy/receipt/state types in `domain::feature`, traversal, pristine reparenting, mutation guards, and orchestration in focused `action::feature` modules, Git/process mechanics behind `git::Git`, and shared PR behavior in one feature-owned helper. Hall defaults and ordered per-repo checks live in committed `ivar.json` v2, whose migration requires a small generic `Store` change so a supported chain may begin at v1 while v0 remains unreachable.

**Tech Stack:** Rust 2024, Clap 4 derive, Serde/versioned canonical JSON, `git2` for local reads, Git CLI for mutations and remote refs, GitHub CLI (`gh`) for PR/check/merge-queue operations, blocking subprocesses, `tempfile`, `assert_cmd`, and real Git repositories in tests.

---

## Resolved behavior

1. A feature has at most one `parent`; children are always inferred by scanning `Feature.parent`. No parent stores a child list.
2. Nesting is unlimited. Parent existence and acyclicity are validated whenever the tree is read. A missing parent or cycle is a hard, non-mutating refusal.
3. `feature create <child> --parent <parent>` derives the child's base from the immediate parent's branch and conflicts with `--base`. `ivar feature reparent <child> --parent <new-parent>` may change lineage only while the child is pristine: no promotions or receipts, no plan/execution/session/close progress beyond creation, and no descendants. It validates the target exists and the new edge is acyclic, then changes `child.parent` and `child.base=new_parent.branch` atomically through one canonical `feature.json` write. Parent is immutable after work starts.
4. A child integrates into its **immediate parent's branch**. It never targets an ancestor or a default branch, and it never collapses through already-integrated ancestors.
5. Integration is leaves-first. `feature integrate <feature>` is valid only for a child and is blocked by every direct/recursive descendant that is active, failed, stale, or not fully integrated/verified. Abandoned descendants do not block but remain in tree/history.
6. `feature deliver` is valid only for a root (`parent=None`). Delivery of a child blocks with the exact fix command `ivar feature integrate <child>`. Root delivery is blocked by every active, failed, stale, or unintegrated descendant.
7. Integration policy precedence is resolved independently per field: CLI override > feature override > hall default > embedded default. Embedded defaults are `via=local` and `strategy=squash`.
8. The public vocabulary is `via=pr|local`; `github` is not accepted as an enum or CLI value. PR implementation uses `gh` internally.
9. Feature overrides are persisted at creation through `feature create --via <pr|local> --strategy <squash|merge|rebase>`. Omitting either field leaves it inheritable. There is no policy-configure command; after the first receipt, relationship, feature base, feature policy, and promotion membership are frozen. Pristine lineage changes use the explicit reparent command in decision 3.
10. Both vias support `squash` (default), `merge` (`--no-ff` semantics), and `rebase` (rebase source onto immediate parent then fast-forward parent).
11. PR integration may create, observe, and explicitly merge a PR. It checks required PR checks before merge, never uses `--admin`, passes `--match-head-commit`, lets `gh` respect protection/auto-merge/merge queues, and observes until the PR is merged or returns a resumable pending/failure result.
12. If a child promoted a repo the immediate parent did not, interactive mode asks to promote it into the parent. An explicit `y` composes existing `feature promote`; refusal, `--json`, CI, and non-TTY runs block without mutation and include the exact safe fix command `ivar feature promote <parent> <repo>`.
13. Each manifest repo has an ordered `checks: Vec<String>`. Commands run via `bash -lc` in order, in the relevant worktree, and stop on first failure. They are executable policy, not preview-only text.
14. Child checks pass before integration. Parent checks run after each per-repo child result is applied. Local integration first builds and checks a temporary candidate without moving the parent; only a passing candidate may update the parent. It checks the real parent again after update.
15. PR integration checks the child before PR creation/merge, verifies required GitHub checks before requesting merge, updates the local parent branch after the observed merge, then runs parent checks. A merged PR followed by failing parent checks records failed evidence, blocks the tree, prints orientation, and never auto-reverts.
16. Multi-repo integration is explicitly partial, durable, and resumable—not atomic. Each repo result is persisted immediately. A recorded successful receipt locks only that promotion; unreceipted promotions and promotions carrying failed evidence remain repairable/resumable. A rerun validates/reuses fresh successes and resumes unfinished repos without exposing successfully receipted repos to feature mutations or executor write contracts.
17. A receipt records source SHA, immediate-parent target branch, result SHA, via, strategy, optional PR URL, verification-command fingerprint, ordered child/parent/PR-check results, and verification time.
18. Receipt freshness is derived live. A receipt is stale when the child branch tip differs from `source_sha`, its verification fingerprint differs from current manifest checks, any recorded verification failed, or `result_sha` is no longer in the immediate parent's branch history.
19. Unexpected recreation, deletion, or movement of a retained child branch is a violation. Failures offer unsafe restoration to the recorded source/result reference and a safe new-child path; ivar never rewrites user branches automatically.
20. Child refs/worktrees are retained after integration so receipt validation remains conservative and exact. Cleanup remains explicit `feature delete`/`feature prune`; integration performs no immediate local/remote branch deletion.
21. A fully fresh child closes with new outcome `integrated`. Root closure remains explicit and uses outcome `delivered`; `feature deliver` itself keeps its existing non-closing behavior. Manual `close --outcome delivered` does not fabricate child integration receipts.
22. After the first receipt of any kind, relationship/base/policy and promotion membership are frozen. A promotion with recorded successful evidence is individually immutable even if later freshness becomes stale; an unreceipted or failed-evidence promotion remains eligible for scoped repair. Feature-wide rebase or any other multi-promotion mutation preflights every affected promotion and refuses before touching any when one is locked.
23. Planning/execution metadata may continue during a partial integration, but every workstream contract is checked before prepare/approval/replan and again immediately before launch. In partial state each contract path must identify one literal promoted repo; paths targeting a successfully receipted repo and ambiguous repo prefixes (`*`, `**`, `?`, or character classes) are refused before any session/view/spawn. Unrestricted feature sessions are refused once a successful receipt exists. Integration refuses to create the first successful receipt while an unrestricted feature session is live.
24. A fully fresh child closes with outcome `integrated`; that outcome freezes the whole child and cannot reopen. Status/list/review/delete/close remain available, and `feature integrate` remains idempotent only for receipt validation/reporting.
25. Parent deletion is blocked by every direct/recursive descendant, including abandoned/integrated history. Descendants must be deleted leaves-first.
26. List/status expose parent, depth, derived integration state, receipt freshness, and blockers. `feature status <name> --recursive` renders the subtree in deterministic pre-order on human and JSON surfaces.
27. Coordinator instructions automatically create a child for an isolatable request outside the approved plan, announce the new child, and do not ask permission. Corrections to the approved plan use replan; implementation-only local divergence uses reconcile.
28. Executor children never create/reparent/promote/integrate features or otherwise mutate shared hall feature state. They stop and report the isolatable request to the coordinator, which performs child creation.

## Current code map and extension points

### CLI/action conventions

- `src/cli/root.rs:209-267,409-451,835-999` owns the full feature Clap model and exhaustive `From<Args> for Input` conversions. Add `FeatureCommand::{Reparent,Integrate}`, `FeatureCreateArgs.parent/via/strategy`, `FeatureReparentArgs`, `FeatureIntegrateArgs`, and `FeatureStatusArgs.recursive` here. Keep exhaustive destructuring.
- `src/bin/ivar.rs:31-34,132-249` stays a thin import/dispatch/render entrypoint.
- `src/action/feature/mod.rs` declares one module per feature verb. Add `reparent` and `integrate`, plus private focused `relations`, `lifecycle`, `mutation`, `verification`, and shared `pull_requests` modules.
- Every leaf remains `pub fn verb(ctx: &Ctx, input: Input) -> Outcome<VerbOutcome>` and returns one serializable value for JSON/human rendering (`src/action/mod.rs:1-17`).

### Persisted feature state

- `src/domain/feature/feature.rs` currently defines `Feature`/`Promotion`, feature schema v2, `base`, and `PromotionOutcome::{Delivered,Abandoned}`.
- `src/store/feature.rs` owns local `feature.json`, uses `Policy::Local`, and already has contiguous v0→v1→v2 migrations. Nested state becomes v3 and auto-persists on read.
- `src/action/feature/close.rs` privately reads/writes plan frontmatter. Extract the minimal frontmatter read/write seam to `action/feature/lifecycle.rs` so tree classification and close share one interpretation.
- `src/action/feature/create.rs`, `list.rs`, `status.rs`, `rebase.rs`, `delete.rs`, `promote.rs`, and `demote.rs` are the existing mutation/status surfaces that need lineage or per-promotion immutability gates. The new `reparent.rs` owns the one allowed pristine lineage transition.

### Delivery/PR capabilities

- `src/action/feature/deliver/mod.rs` owns preview/fingerprint/push/PR apply and currently permits any feature.
- `src/action/feature/deliver/pull_requests.rs` currently implements best-effort open-PR lookup, creation, URL parsing, and sibling comments. Move it to `src/action/feature/pull_requests.rs` for integration reuse.
- `tests/delivery.rs:560-647` has a fake `gh` supporting only `pr list/create/comment`; extend a shared test fixture for `pr checks/merge/view` and merge-queue state.
- Current official GitHub CLI behavior (verified against the current manual): `gh pr checks` exposes pass/fail/pending buckets and exit 8 for pending; `gh pr merge` supports `--merge|--squash|--rebase`, `--match-head-commit`, protection/auto-merge, and merge queues. Never use `--admin`.

### Git seam

Existing relevant `git::Git` methods (`src/git/mod.rs:71-277`) include worktree add/remove, dirty checks, head SHA, ancestry, push, fetch/fast-forward, rebase/abort, and remote tip. Missing focused primitives are:

```rust
fn revision_commit(&self, git_dir: &Utf8Path, revision: &str) -> Result<String, Error>;
fn add_detached_worktree(&self, git_dir: &Utf8Path, dest: &Utf8Path, revision: &str) -> Result<(), Error>;
fn create_branch(&self, git_dir: &Utf8Path, branch: &str, revision: &str) -> Result<(), Error>;
fn delete_branch(&self, git_dir: &Utf8Path, branch: &str) -> Result<(), Error>;
fn merge_no_ff(&self, worktree: &Utf8Path, source: &str) -> Result<(), Error>;
fn squash_merge(&self, worktree: &Utf8Path, source: &str, message: &str) -> Result<(), Error>;
fn fast_forward_to(&self, worktree: &Utf8Path, revision: &str) -> Result<(), Error>;
```

They belong in `src/git/read.rs`/`exec.rs`, delegate through `Git`, and use real-Git tests. No integration branch cleanup API is needed.

### Committed manifest/store constraint

- `src/store/manifest/model.rs` is strict v1 and `Repo` currently has name/url/default branch only.
- `src/store/manifest/persistence.rs` currently constructs an empty migration chain at v1.
- `src/store/versioned/mod.rs:211-239,441-471` currently asserts every non-empty chain starts at v0 and treats any non-empty chain as reaching every older version. A manifest chain containing only v1→v2 would panic; adding a fake v0→v1 step would incorrectly adopt unversioned files.
- Correct generic behavior: permit a chain to start at its earliest supported version; `has_migration_path(detected)` is true only when `detected >= first.from_version` (or already current/newer). Therefore `[1→2]` migrates v1 and refuses v0.
- Manifest v2 adds both hall integration defaults and ordered per-repo checks while retaining `Manifest::new(...)` and `Repo::new(...)` compatibility through embedded defaults/builders.

### Coordinator/executor instruction surfaces

- `src/harness/commands/feature-create.md`, `plan.md`, and `execute.md` are provider-neutral shipped instructions.
- `src/action/execute/prompt.rs:356-409` constructs every executor prompt and is the enforcement point for “stop/report; coordinator creates the child.”
- `src/harness/commands/catalog.rs` embeds the 15 commands. Content-only changes keep IDs/count/fingerprints stable.
- Semantic assertions belong in `tests/unit/action/execute/prompt.rs`, `tests/unit/harness/commands.rs`, and `tests/shipped_commands.rs`.

## File responsibility map

### New production files

- `src/domain/feature/integration.rs` — pure via/strategy/override/receipt/evidence/derived-state types and policy resolution.
- `src/action/feature/relations.rs` — feature scanning, parent/child traversal, cycle validation, descendant blockers, receipt freshness facts, and subtree projection.
- `src/action/feature/lifecycle.rs` — shared plan-frontmatter outcome read/write and fully-integrated outcome detection.
- `src/action/feature/mutation.rs` — structure, per-promotion, unrestricted-session, and executor write-contract guards for fresh/partial/fully-integrated states.
- `src/action/feature/reparent.rs` — pristine-child validation and one-write parent/base replacement.
- `src/action/feature/verification.rs` — ordered command execution, fingerprinting, and evidence construction.
- `src/action/feature/integrate.rs` — child integration orchestration and per-repo resume logic.
- `src/action/feature/pull_requests.rs` — moved/extended shared PR operations.
- `src/action/confirm.rs` — reusable confirmation seam aware of JSON/noninteractive execution.

### New tests

- `tests/unit/domain/feature/integration.rs`
- `tests/unit/action/feature/relations.rs`
- `tests/unit/action/feature/lifecycle.rs`
- `tests/unit/action/feature/mutation.rs`
- `tests/unit/action/feature/reparent.rs`
- `tests/unit/action/feature/verification.rs`
- `tests/unit/action/feature/integrate.rs`
- `tests/unit/action/confirm.rs`
- `tests/support/fake_gh.rs`
- `tests/nested_subfeatures.rs`

### Modified areas

- Domain/store: `src/domain/feature/{mod.rs,feature.rs,delivery.rs}`, `src/store/{feature.rs,layout.rs,versioned/mod.rs,manifest/*}`.
- CLI/actions: `src/cli/root.rs`, `src/bin/ivar.rs`, `src/action/mod.rs`, relevant feature/plan/execute/session mutations.
- Git/PR: `src/git/{mod.rs,read.rs,exec.rs}`, delivery module imports.
- Instructions: shipped `feature-create.md`, `plan.md`, `execute.md`; executor prompt.
- Docs/tests: mirrored unit tests, `tests/delivery.rs`, `tests/shipped_commands.rs`, generated command reference, architecture/concepts/glossary/day-to-day/on-disk format.

---

### Task 1: Define pure nested-integration vocabulary

**Files:**
- Create: `src/domain/feature/integration.rs`
- Create: `tests/unit/domain/feature/integration.rs`
- Modify: `src/domain/feature/mod.rs`
- Modify: `src/domain/feature/feature.rs`
- Modify: `tests/unit/domain/feature/{mod.rs,feature.rs}`

- [ ] **Step 1: Write failing default/parser/policy tests**

```rust
#[test]
fn embedded_policy_is_local_squash() {
    assert_eq!(IntegrationPolicy::default().via, IntegrationVia::Local);
    assert_eq!(IntegrationPolicy::default().strategy, IntegrationStrategy::Squash);
}

#[rstest]
#[case("pr", IntegrationVia::Pr)]
#[case("local", IntegrationVia::Local)]
fn via_accepts_only_public_spellings(#[case] raw: &str, #[case] expected: IntegrationVia) {
    assert_eq!(IntegrationVia::parse(raw).unwrap(), expected);
    assert!(IntegrationVia::parse("github").is_err());
}

#[test]
fn policy_resolves_each_field_cli_then_feature_then_hall_then_embedded() {
    let resolved = IntegrationPolicy::resolve(
        IntegrationOverride { via: Some(IntegrationVia::Pr), strategy: None },
        IntegrationOverride { via: Some(IntegrationVia::Local), strategy: Some(IntegrationStrategy::Merge) },
        IntegrationPolicy { via: IntegrationVia::Pr, strategy: IntegrationStrategy::Rebase },
    );
    assert_eq!(resolved, IntegrationPolicy { via: IntegrationVia::Pr, strategy: IntegrationStrategy::Merge });
}
```

- [ ] **Step 2: Write failing receipt/evidence/state tests**

```rust
let receipt = IntegrationReceipt {
    source_sha: "111".to_owned(),
    target_branch: BranchName::new("parent").unwrap(),
    result_sha: "222".to_owned(),
    via: IntegrationVia::Pr,
    strategy: IntegrationStrategy::Squash,
    pr_url: Some("https://github.com/acme/api/pull/7".to_owned()),
    verification: VerificationEvidence {
        command_fingerprint: "checks-v1".to_owned(),
        child: vec![VerificationResult::passed("cargo test", Some(0), "")],
        parent: vec![VerificationResult::passed("cargo test", Some(0), "")],
        pr_checks: vec![PrCheckResult::passed("ci")],
        verified_at: "2026-08-14T12:00:00Z".to_owned(),
    },
};
assert!(receipt.verification.passed());
assert_eq!(serde_json::from_str::<IntegrationReceipt>(&serde_json::to_string(&receipt).unwrap()).unwrap(), receipt);
```

Test derived classifications for `Active`, `Integrated`, `Failed`, `Stale`, `Abandoned`, and root `Delivered` from explicit facts; no lifecycle field is serialized.

- [ ] **Step 3: Run focused tests and verify compile failure**

Run: `rtk cargo test --lib domain::feature::integration`

Expected: integration types are undefined.

- [ ] **Step 4: Implement the pure types**

Define exact core shapes:

```rust
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationVia { Pr, #[default] Local }

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationStrategy { Merge, #[default] Squash, Rebase }

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntegrationOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<IntegrationVia>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy: Option<IntegrationStrategy>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntegrationPolicy { pub via: IntegrationVia, pub strategy: IntegrationStrategy }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationResult {
    pub command: String,
    pub success: bool,
    pub exit_code: Option<i32>,
    pub diagnostic: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrCheckResult { pub name: String, pub bucket: String }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationEvidence {
    pub command_fingerprint: String,
    pub child: Vec<VerificationResult>,
    pub parent: Vec<VerificationResult>,
    pub pr_checks: Vec<PrCheckResult>,
    pub verified_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntegrationReceipt {
    pub source_sha: String,
    pub target_branch: BranchName,
    pub result_sha: String,
    pub via: IntegrationVia,
    pub strategy: IntegrationStrategy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr_url: Option<String>,
    pub verification: VerificationEvidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureIntegrationState { Active, Integrated, Failed, Stale, Abandoned, Delivered }
```

Implement strict parsers/errors, `Display`, `IntegrationPolicy::resolve`, `VerificationEvidence::passed`, and a pure classifier accepting outcome/receipt/freshness facts.

- [ ] **Step 5: Extend feature-local data without reverse edges**

```rust
pub struct Feature {
    version: u32,
    pub name: FeatureName,
    pub branch: BranchName,
    pub promotions: BTreeMap<RepoName, Promotion>,
    #[serde(default)] pub base: Option<BranchName>,
    #[serde(default)] pub parent: Option<FeatureName>,
    #[serde(default)] pub integration: IntegrationOverride,
}

pub struct Promotion {
    pub worktree: WorktreeState,
    #[serde(default)] pub base: Option<BranchName>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub integration_receipt: Option<IntegrationReceipt>,
}
```

Set defaults in `Feature::new`/`promote`; add `has_any_receipt`, `promotion_has_successful_receipt(&RepoName)`, and `all_promotions_have_passing_receipts`. “Successful” is based on recorded passing evidence, not current freshness, so later source/check/history drift never unlocks a pinned promotion. Do not add children or stored lifecycle.

- [ ] **Step 6: Reexport explicitly and run domain tests**

Run: `rtk cargo test --lib domain::feature`

Expected: all domain tests pass while the feature value still reports schema version 2; Task 2 advances the persisted schema and its assertions together.

- [ ] **Step 7: Commit**

```bash
git add src/domain/feature tests/unit/domain/feature
git commit -m "feat(feature): model nested integration evidence"
```

### Task 2: Migrate local feature state and add `integrated` close outcome

**Files:**
- Modify: `src/domain/feature/feature.rs`
- Modify: `src/store/feature.rs`
- Modify: `src/action/feature/{close.rs,mod.rs}`
- Create: `src/action/feature/lifecycle.rs`
- Create: `tests/unit/action/feature/lifecycle.rs`
- Modify: `tests/unit/{store/feature.rs,domain/feature/feature.rs,action/feature/close.rs}`

- [ ] **Step 1: Write failing v2→v3 migration test**

Use a real v2 fixture and assert v3 adds `parent=None`, empty feature override, and no promotion receipt, then persists because policy is local:

```rust
assert_eq!(migrated.version(), 3);
assert_eq!(migrated.parent, None);
assert_eq!(migrated.integration, IntegrationOverride::default());
assert_eq!(migrated.promotions[&api].integration_receipt, None);
assert!(fs::read_text(&path).unwrap().unwrap().contains("\"version\": 3"));
```

- [ ] **Step 2: Write failing outcome/lifecycle tests**

Assert `PromotionOutcome::parse("integrated")`, display/serde spelling, close human/JSON output, frontmatter round-trip, and that a second close cannot replace `integrated` with `delivered` or reopen an integrated child.

- [ ] **Step 3: Run tests and verify failures**

Run:

```bash
rtk cargo test --lib store::feature
rtk cargo test --lib action::feature::close
```

Expected: no v3 migration and unknown `integrated` outcome.

- [ ] **Step 4: Implement v3 migration**

Set feature constants to 3 and add `feature_v2_to_v3` inserting `parent: null`, `integration: {}`, and `integration_receipt: null` under each promotion. Register `Migration::new(2, 3, feature_v2_to_v3)` after existing steps.

- [ ] **Step 5: Add `PromotionOutcome::Integrated`**

Extend parse/display/serde and failure guidance to accept exactly `delivered`, `integrated`, `abandoned`. Root delivery continues using delivered; nested integration uses integrated.

- [ ] **Step 6: Extract shared lifecycle access without broad refactor**

Move only the frontmatter shape/read/replace operations from close into:

```rust
pub(crate) struct CloseRecord { pub outcome: String, pub closed_at: String }
pub(crate) fn read_close(layout: &Layout, feature: &FeatureName) -> Result<Option<CloseRecord>, Failure>;
pub(crate) fn write_close(layout: &Layout, feature: &FeatureName, outcome: PromotionOutcome) -> Result<CloseRecord, Failure>;
pub(crate) fn is_fully_integrated(layout: &Layout, feature: &Feature) -> Result<bool, Failure>;
```

Keep the read shape's outcome as a string so an outcome written by another tool still reads as “already closed,” preserving current idempotency. Add `CloseRecord::known_outcome() -> Option<PromotionOutcome>` for tree classification. `is_fully_integrated` is true only for known outcome `Integrated`; partial-receipt policy belongs in Task 8's focused mutation module, not in a blanket lifecycle guard. `close` composes these functions; a requested `Integrated` outcome additionally requires a child (`parent.is_some()`) with a fresh passing receipt for every promotion, so a direct close cannot fabricate integration evidence or freeze a stale partial result.

- [ ] **Step 7: Run focused tests and commit**

```bash
rtk cargo test --lib store::feature
rtk cargo test --lib action::feature::lifecycle
rtk cargo test --lib action::feature::close
git add src/domain/feature/feature.rs src/store/feature.rs src/action/feature tests/unit
git commit -m "feat(feature): persist nested lifecycle evidence"
```

### Task 3: Let a generic migration chain start at v1 while v0 remains unreachable

**Files:**
- Modify: `src/store/versioned/mod.rs`
- Modify: `tests/unit/store/versioned.rs`

- [ ] **Step 1: Replace the obsolete start-at-zero constructor test**

Add tests proving a v1→v2-only committed chain constructs, reads v1 in memory, refuses v0, reports v0 unreachable, and explicitly migrates v1:

```rust
let store = Store::<Widget>::new(
    path.clone(),
    vec![Migration::new(1, 2, v1_to_v2_rename_label_to_name)],
    2,
    Policy::Committed,
);
assert!(store.has_migration_path(1));
assert!(!store.has_migration_path(0));
```

Keep gap/overlap/terminus panic tests. Remove `chain_not_starting_at_zero_panics_at_construction`; replace it with `chain_may_start_at_earliest_supported_version`.

- [ ] **Step 2: Run focused tests and verify constructor panic**

Run: `rtk cargo test --lib store::versioned`

Expected: the v1-starting chain panics under current invariant.

- [ ] **Step 3: Change only chain-boundary semantics**

Update `assert_chain_valid` to require: every step advances (`from < to`), steps are contiguous, and last lands on current. Do not require first.from=0.

Implement reachability exactly:

```rust
pub fn has_migration_path(&self, detected: u32) -> bool {
    if detected >= self.current {
        return true;
    }
    self.migrations
        .first()
        .is_some_and(|first| detected >= first.from_version)
}
```

`guard_has_migration_path` remains the single refusal. Update module docs from “always starts at v0” to “starts at the earliest schema this format supports.”

- [ ] **Step 4: Run all versioned-store tests and commit**

```bash
rtk cargo test --lib store::versioned
git add src/store/versioned/mod.rs tests/unit/store/versioned.rs
git commit -m "fix(store): support versioned migration baselines"
```

### Task 4: Add hall policy and executable repo checks in manifest v2

**Files:**
- Modify: `src/store/manifest/{model.rs,persistence.rs,mod.rs,error.rs}`
- Modify: `src/action/provider/add.rs`
- Modify: `tests/unit/store/manifest/mod.rs`
- Modify manifest/action fixtures under `tests/unit/action/{hall.rs,provider/add.rs,repo/add.rs,repo/remove.rs}`

- [ ] **Step 1: Write failing compatibility/default tests**

Assert unchanged constructor calls produce local/squash and empty checks:

```rust
let repo = Repo::new(api, "git@github.com:acme/api.git", main);
assert!(repo.checks().is_empty());
let manifest = Manifest::new(name, providers, vec![repo], None).unwrap();
assert_eq!(manifest.integration(), IntegrationPolicy::default());
```

Add builder tests:

```rust
let repo = Repo::new(api, url, main).with_checks(vec![
    "cargo fmt --check".to_owned(),
    "cargo test --all-features".to_owned(),
]);
let configured = manifest.with_integration(IntegrationPolicy {
    via: IntegrationVia::Pr,
    strategy: IntegrationStrategy::Rebase,
});
```

Prove repo add/remove, provider add, and MCP updates preserve both fields.

- [ ] **Step 2: Write failing v1→v2 committed migration tests**

Write v1 with two repos, call `Manifest::read`, and assert in-memory local/squash plus empty checks without byte changes. Assert `plan=Available {1,2}`, unversioned plan is `Unreachable {0,2}`, plain write refuses v1, and explicit migrate writes canonical v2.

- [ ] **Step 3: Run tests and verify failures**

Run: `rtk cargo test --lib store::manifest`

Expected: missing fields/builders and current version 1.

- [ ] **Step 4: Extend model compatibly**

Add non-optional `integration: IntegrationPolicy` to `Manifest`, initialized by `Manifest::new`. Add `checks: Vec<String>` to `Repo` with `#[serde(default, skip_serializing_if = "Vec::is_empty")]`; `Repo::new` uses empty checks and `with_checks` returns an updated value. Extend manifest validation with `Error::EmptyRepoCheck { name, index }`: a blank command is refused on read/build with a fix to remove it or provide an executable command.

Add `Manifest::with_integration`, `with_providers`, and a private rebuild that always preserves integration/MCP/skills/repos. Rewrite existing `with_*` methods through it; update provider add to use `with_providers`.

- [ ] **Step 5: Add v1→v2 migration using the new baseline support**

```rust
fn v1_to_v2(mut value: serde_json::Value) -> Result<serde_json::Value, String> {
    let root = value.as_object_mut().ok_or("manifest must be an object")?;
    root.entry("integration").or_insert_with(|| serde_json::json!({
        "strategy": "squash",
        "via": "local"
    }));
    let repos = root.get_mut("repos").and_then(serde_json::Value::as_array_mut)
        .ok_or("manifest is missing repos")?;
    for repo in repos {
        repo.as_object_mut().ok_or("repo must be an object")?
            .entry("checks").or_insert_with(|| serde_json::json!([]));
    }
    Ok(value)
}
```

Set manifest current=2 and build `Store::new(..., vec![Migration::new(1,2,v1_to_v2)], 2, Policy::Committed)`. V0 remains unreachable through Task 3 semantics and still maps to `manifest.missing_version`.

- [ ] **Step 6: Run manifest/mutation tests and commit**

```bash
rtk cargo test --lib store::manifest
rtk cargo test --lib action::hall
rtk cargo test --lib action::provider::add
rtk cargo test --lib action::repo
git add src/store/manifest src/action/provider/add.rs tests/unit
git commit -m "feat(manifest): configure verified integration"
```

### Task 5: Add JSON-aware confirmation and ordered verification runner

**Files:**
- Create: `src/action/confirm.rs`
- Create: `tests/unit/action/confirm.rs`
- Create: `src/action/feature/verification.rs`
- Create: `tests/unit/action/feature/verification.rs`
- Modify: `src/action/{mod.rs,hall/mod.rs,hall/cleanup.rs,hall/migrate.rs}`
- Modify: `src/bin/ivar.rs`
- Modify: `tests/unit/action/hall.rs`

- [ ] **Step 1: Write failing confirmation seam tests**

Test `NonInteractive` always answers false without reading; `Fixed(true)` enables unit tests; and the binary builds a noninteractive confirmer for `--json`, `$CI`, or a non-TTY. Preserve cleanup/migrate non-TTY behavior.

- [ ] **Step 2: Implement confirmation on `Ctx`**

Define:

```rust
pub trait Confirm: std::fmt::Debug + Send + Sync {
    fn confirm(&self, question: &str, caveat: Option<&str>) -> Result<bool, Failure>;
}
pub fn reporter(enabled: bool) -> Arc<dyn Confirm>;
```

Add `confirm: Arc<dyn Confirm>` to `Ctx`, default `NonInteractive`, `with_confirm`, and `ctx.confirm(...)`. In `bin/ivar.rs`, use `reporter(!json && std::env::var_os("CI").is_none() && term::is_tty(Stream::Stderr))`. Migrate hall cleanup/migrate from `hall::ask` and remove that helper.

- [ ] **Step 3: Write failing verification tests**

Cover ordered execution, stop-on-first-failure, exact cwd, empty-list success, spawn failure, diagnostic capture, and deterministic fingerprint:

```rust
let commands = vec!["printf one >> order".to_owned(), "printf two >> order".to_owned()];
let report = run(&commands, &worktree).unwrap();
assert_eq!(report.results.iter().map(|r| r.command.as_str()).collect::<Vec<_>>(), [
    "printf one >> order", "printf two >> order"
]);
assert_eq!(fingerprint(&commands).unwrap(), fingerprint(&commands).unwrap());
```

- [ ] **Step 4: Implement executable checks**

```rust
pub(crate) struct VerificationRun {
    pub command_fingerprint: String,
    pub results: Vec<VerificationResult>,
}
pub(crate) fn fingerprint(commands: &[String]) -> Result<String, Failure>;
pub(crate) fn run(commands: &[String], cwd: &Utf8Path) -> Result<VerificationRun, Failure>;
```

Fingerprint canonical JSON with `infra::json` + `infra::hash`. Execute each command as `bash -lc <command>` through `infra::proc::capture` with cwd; append result and stop after first nonzero/spawn failure. Do not render without running.

- [ ] **Step 5: Run tests and commit**

```bash
rtk cargo test --lib action::confirm
rtk cargo test --lib action::feature::verification
rtk cargo test --lib action::hall
git add src/action src/bin/ivar.rs tests/unit/action
git commit -m "feat(action): run confirmed integration checks"
```

### Task 6: Build one child-derived tree/freshness projection

**Files:**
- Create: `src/action/feature/relations.rs`
- Create: `tests/unit/action/feature/relations.rs`
- Modify: `src/action/feature/mod.rs`

- [ ] **Step 1: Write failing unlimited-tree tests**

Create `root <- parent <- child <- leaf`; assert children are inferred, parent chain is unlimited, descendants are deterministic pre-order, and only each child stores one parent. Add missing-parent and hand-edited-cycle failures.

- [ ] **Step 2: Write failing blocker/freshness tests**

Assert:

- leaf has no descendant blockers;
- child is blocked by active leaf;
- root is blocked by active/stale/failed/unintegrated descendants at any depth;
- an abandoned node is ignored but its active child still blocks;
- an integrated fresh verified descendant does not block;
- source tip movement, missing/recreated source branch, check-fingerprint drift, failed evidence, and result no longer ancestor of immediate parent each classify stale/failed and block;
- target is always the immediate parent's branch.

- [ ] **Step 3: Run and verify missing module**

Run: `rtk cargo test --lib action::feature::relations`

Expected: compile failure.

- [ ] **Step 4: Implement focused APIs**

```rust
pub(crate) fn read_feature(layout: &Layout, name: &FeatureName) -> Result<Feature, Failure>;
pub(crate) fn read_all(layout: &Layout) -> Result<Vec<Feature>, Failure>;
pub(crate) fn parent(layout: &Layout, feature: &Feature) -> Result<Option<Feature>, Failure>;
pub(crate) fn descendants(layout: &Layout, name: &FeatureName) -> Result<Vec<Feature>, Failure>;
pub(crate) fn subtree_status(
    git: &impl Git,
    layout: &Layout,
    manifest: &Manifest,
    root: &FeatureName,
) -> Result<Vec<TreeEntry>, Failure>;
pub(crate) fn blocking_descendants(
    git: &impl Git,
    layout: &Layout,
    manifest: &Manifest,
    feature: &Feature,
) -> Result<Vec<TreeEntry>, Failure>;
pub(crate) fn receipt_freshness(
    git: &impl Git,
    layout: &Layout,
    manifest: &Manifest,
    child: &Feature,
    parent: &Feature,
    repo: &RepoName,
    receipt: &IntegrationReceipt,
) -> Result<ReceiptFreshness, Failure>;
```

Use `git.revision_commit(bare, child.branch)` for source equality, current checks fingerprint, recorded evidence success, and `git.is_ancestor(bare, result_sha, parent.branch)` for result membership. A missing revision is stale, not “not integrated.”

- [ ] **Step 5: Add exact orientation failures**

Tree-block failures include every blocker in details. Stale source fixes include unsafe `git -C <child-worktree> reset --hard <source_sha>` and safe `ivar feature create <new-child> --parent <parent>`. Stale parent-history fixes include unsafe `git -C <parent-worktree> merge --ff-only <result_sha>` and the safe new-child route. Never execute either automatically.

- [ ] **Step 6: Run and commit**

```bash
rtk cargo test --lib action::feature::relations
git add src/action/feature/relations.rs src/action/feature/mod.rs tests/unit/action/feature/relations.rs
git commit -m "feat(feature): derive nested tree health"
```

### Task 7: Wire parent creation, pristine reparenting, and recursive status/delete surfaces

**Files:**
- Modify: `src/cli/root.rs`
- Modify: `src/bin/ivar.rs`
- Create: `src/action/feature/reparent.rs`
- Modify: `src/action/feature/{mod.rs,create.rs,list.rs,status.rs,delete.rs}`
- Modify: `tests/unit/cli/root.rs`
- Create: `tests/unit/action/feature/reparent.rs`
- Modify: `tests/unit/action/feature/{mod.rs,create.rs,list.rs,status.rs,delete.rs}`

- [ ] **Step 1: Write failing Clap tests**

Parse/convert:

```bash
ivar feature create child --parent parent --via pr --strategy rebase
ivar feature reparent child --parent new-parent
ivar feature status parent --recursive
```

Assert create `--parent` conflicts with `--base`; invalid public values reach typed action errors; reparent requires exactly a child positional and `--parent`; no policy-configure subcommand exists. Add the exhaustive conversion:

```rust
pub struct FeatureReparentArgs {
    pub child: String,
    #[arg(long)] pub parent: String,
}

pub struct ReparentInput { pub child: String, pub parent: String }
```

- [ ] **Step 2: Write failing action tests**

Cover existing-parent requirement, immediate-parent base derivation, self/cycle refusal, optional feature overrides, list depth/parent/state, status receipts/freshness/blockers, recursive pre-order, and delete refusal containing all descendants including abandoned ones.

For reparent, add named tests proving:

- `reparent_updates_parent_and_base_in_one_feature_write`: the child starts with no promotions/artifacts/descendants, the target exists, and the resulting canonical `feature.json` contains both the new `parent` and `base=new_parent.branch`; the action must reach only the single `Feature::write` shown in Step 5 and must never persist the fields separately;
- missing target, self-parent, and a target below the child each refuse with original bytes unchanged;
- any promotion (including `WorktreeState::Failed`), any receipt, any plan-directory entry, execution board/journal, live or detached feature session, close record, or direct/recursive descendant means work started and refuses with `feature.reparent_work_started` before writing;
- a just-created child may be reparented repeatedly until one of those work facts appears, after which parent/base are immutable.

- [ ] **Step 3: Add exact CLI fields/conversions**

```rust
pub struct FeatureCreateArgs {
    pub name: String,
    #[arg(long)] pub branch: Option<String>,
    #[arg(long, conflicts_with = "parent")] pub base: Option<String>,
    #[arg(long, conflicts_with = "base")] pub parent: Option<String>,
    #[arg(long)] pub via: Option<String>,
    #[arg(long)] pub strategy: Option<String>,
}
pub struct FeatureStatusArgs { pub feature: String, #[arg(long)] pub recursive: bool }
```

Pass raw strings to actions; domain validation stays below CLI.

- [ ] **Step 4: Implement create declarations**

Read/validate parent before writing child; set `base=parent.branch`; parse feature override fields independently; persist only the child-side `parent` and `base` relation facts and no reverse child list. With no parent, preserve existing explicit base/default behavior.

- [ ] **Step 5: Implement pristine reparent as one atomic record replacement**

In `reparent.rs`, read all features through `relations`, validate both nodes and the proposed edge before mutation, and define pristine as: `promotions.is_empty()`, no receipt in any promotion, no entries under the feature's plan/execution/session paths, no close record, and `relations::descendants(...).is_empty()`. Then perform exactly one persisted mutation:

```rust
child.parent = Some(new_parent.name.clone());
child.base = Some(new_parent.branch.clone());
child.write(&layout)?;
```

Do not create, delete, or move a branch/worktree: a pristine feature has none. Return `ReparentOutcome { root, child, old_parent, new_parent, base }`. All validation failures occur before this single canonical/atomic `Feature::write`.

- [ ] **Step 6: Extend list/status outcomes**

Add `parent`, `depth`, `state`, and `blockers` to list summaries. Add `TreeEntry { feature,parent,depth,state,repos,blockers }` to recursive status. Human output uses indentation; JSON emits a flat deterministic pre-order with depth so scripts need no recursive schema parser.

- [ ] **Step 7: Gate delete before teardown**

Call `relations::descendants`; if nonempty return `feature.has_descendants` with every name and fix “delete leaves first.” This runs before permission preflight/worktree removal.

- [ ] **Step 8: Run and commit**

```bash
rtk cargo test --lib cli::root
rtk cargo test --lib action::feature::create
rtk cargo test --lib action::feature::reparent
rtk cargo test --lib action::feature::list
rtk cargo test --lib action::feature::status
rtk cargo test --lib action::feature::delete
git add src/cli/root.rs src/bin/ivar.rs src/action/feature tests/unit/cli/root.rs tests/unit/action/feature
git commit -m "feat(feature): expose and reparent nested subfeature trees"
```

### Task 8: Enforce whole-child and per-promotion mutation boundaries

**Files:**
- Create: `src/action/feature/mutation.rs`
- Create: `tests/unit/action/feature/mutation.rs`
- Modify feature mutations: `src/action/feature/{mod.rs,reparent.rs,promote.rs,demote.rs,rebase.rs}`
- Modify plan mutations: `src/action/plan/{create.rs,approve.rs}`
- Modify execute mutations: `src/action/execute/{prepare.rs,replan.rs,ack.rs,reconcile.rs,approve.rs,tick/mod.rs,reply.rs}`
- Modify session bindings: `src/action/session/{start.rs,connect.rs,conversion.rs}` (`relay` composes `start`)
- Modify corresponding mirrored tests under `tests/unit/action/`

- [ ] **Step 1: Write failing fresh/partial/full guard tests**

Use a two-repo child where repo A has recorded passing evidence and repo B is first unreceipted, then carries failed evidence. Assert:

- any `integrated` close record blocks every child mutation before writes/spawns; status/list/review/delete/close and idempotent integrate validation remain available, and close cannot reopen or replace `integrated`;
- after the first receipt, reparent, feature base/policy changes, promote/demote membership changes, and feature-wide rebase all refuse before touching either repo;
- repo A's successful receipt blocks every action or contract that can move/write A, even when its receipt is now stale;
- repo B remains eligible for setup/check repair and an executor workstream whose contract contains only literal `b/...` paths;
- a failed-evidence receipt on B does not trigger A's per-promotion lock and can be reverified/resumed;
- plan/board/inbox/journal-only mutations remain possible during partial integration, but not after outcome `integrated`;
- unrestricted session start/connect/conversion/relay refuses before view creation or provider spawn when any successful receipt exists.

- [ ] **Step 2: Add focused guards instead of one blanket mutable flag**

Define in `action/feature/mutation.rs` and expose only `pub(crate)` wrappers from `action::feature`:

```rust
pub(crate) fn ensure_not_fully_integrated(
    layout: &Layout,
    feature: &Feature,
) -> Result<(), Failure>;
pub(crate) fn ensure_structure_mutable(
    layout: &Layout,
    feature: &Feature,
) -> Result<(), Failure>;
pub(crate) fn ensure_promotion_mutable(
    layout: &Layout,
    feature: &Feature,
    repo: &RepoName,
) -> Result<(), Failure>;
pub(crate) fn ensure_contracts_avoid_locked_promotions(
    layout: &Layout,
    feature: &Feature,
    workstreams: &[WorkstreamDef],
) -> Result<(), Failure>;
pub(crate) fn ensure_unrestricted_session_allowed(
    layout: &Layout,
    feature: &Feature,
) -> Result<(), Failure>;
```

`ensure_structure_mutable` refuses an `integrated` outcome or any receipt because relationship/base/policy and promotion membership are feature-wide facts. `ensure_promotion_mutable` refuses an `integrated` outcome or a successful receipt on exactly `repo`; failed evidence does not lock that promotion. A multi-repo action such as current feature-wide `rebase` calls the per-promotion guard for every target in a complete preflight and performs no mutation if any target is locked. Never replace these scopes with “any receipt means no mutations.”

- [ ] **Step 3: Add the exact path-to-repo contract guard**

When no successful receipt exists, contract behavior is unchanged. Otherwise, inspect every raw `WorkstreamDef.write_contract` entry before converting it to `WriteContract`: normalize separators, reject absolute/parent traversal as today, take the first path component as the repo, and require that component to be a literal promoted `RepoName`. A first component containing `*`, `?`, `[` or `]`, an empty component, or a non-promoted repo is ambiguous and blocks with `feature.partial_contract_ambiguous`. A literal component naming a promotion with successful evidence blocks with `feature.promotion_integration_immutable`; literal paths under unreceipted/failed B pass.

Add table tests for `a/src/lib.rs`, `a/**`, `*/src/lib.rs`, `**/*.rs`, `../a/x`, `/a/x`, `b/src/lib.rs`, and an unknown `c/x`. Test the complete wave, not only the selected workstream: one A path anywhere refuses before materialising an executor view or spawning a process.

- [ ] **Step 4: Apply each guard at the narrow mutation boundary**

- `reparent`, promote/demote membership changes, and feature-wide rebase use structure/per-promotion preflights as above. Retrying setup/checks for an already-present unreceipted/failed promotion is not a membership change and remains allowed.
- `plan create/approve` and execute `ack/reconcile/reply` write only plan/board/inbox/journal state: call `ensure_not_fully_integrated`, not a receipt-wide guard.
- execute `prepare`, graph approval, and `replan` call `ensure_not_fully_integrated` plus `ensure_contracts_avoid_locked_promotions` before persisting a board/graph. `tick` repeats the contract guard against the loaded board immediately before any view/guard/session/spawn; its post-run audit treats every change under a successfully receipted repo as a violation even if a shell bypassed the tool hook.
- session `start`, `connect`, and `conversion` call `ensure_unrestricted_session_allowed` before smart fetch, transition markers, view creation, hooks, or spawn; relay inherits start's gate. The guard permits fresh or failed-evidence-only children, refuses any successful receipt, and refuses all fully integrated children.
- `integrate` is not gated here: it owns receipt validation/resume and the final close. Status/list/review/delete/close remain readable/administrative.

- [ ] **Step 5: Verify orientation and the A-locked/B-repairable boundary**

Use `feature.integration_immutable` only for a fully integrated child and `feature.promotion_integration_immutable` for a successful repo receipt. Both explain the pinned source/result and offer status, unsafe recorded-ref restoration when accidental, and the safe new-child route. Structure failures after any receipt use `feature.integration_structure_frozen`. Contract failures name the exact workstream/path/repo. There is no reopen command.

Add an action-level test that snapshots A's branch/worktree/receipt, repairs B through a B-only contract, reruns integration, and asserts A's SHA/files/receipt bytes never change while B gains a fresh successful receipt. Add refusal tests showing demote/rebase/A-targeting contract fail before writes/spawns, while a failed B verification can be rerun with unchanged source/result and upgraded to passing evidence.

- [ ] **Step 6: Run mutation suites and commit**

```bash
rtk cargo test --lib action::feature
rtk cargo test --lib action::plan
rtk cargo test --lib action::execute
rtk cargo test --lib action::session
git add src/action tests/unit/action
git commit -m "feat(feature): guard partial integration per promotion"
```

### Task 9: Add temporary local-integration Git primitives

**Files:**
- Modify: `src/store/layout.rs`
- Modify: `tests/unit/store/layout.rs`
- Modify: `src/git/{mod.rs,read.rs,exec.rs}`
- Modify: `tests/unit/git/{read.rs,exec.rs,mod.rs}`

- [ ] **Step 1: Write failing layout/Git tests**

Assert deterministic candidate/source paths under `.ivar/features/<child>/integration/<repo>/`, revision SHA reads, detached worktree creation, temporary branch lifecycle, no-ff merge parent count, squash single-parent commit, rebase+ff topology, and refusal on dirty/diverged targets.

- [ ] **Step 2: Add layout-owned paths**

```rust
pub fn integration_candidate(&self, feature: &FeatureName, repo: &RepoName) -> Utf8PathBuf;
pub fn integration_source(&self, feature: &FeatureName, repo: &RepoName) -> Utf8PathBuf;
```

No action computes managed paths manually.

- [ ] **Step 3: Extend/delegate Git trait**

Add the exact signatures listed in the code map. `revision_commit` uses git2; mutations use `git()`/`run()` in exec. Temporary branch names are action-provided validated strings under `ivar-integrate/<feature>/<repo>` and are deleted only after their worktrees are removed.

- [ ] **Step 4: Prove parent is untouched before candidate checks**

Add a test that creates a candidate, performs each strategy there, records parent SHA/files, and asserts parent is byte/ref identical until `fast_forward_to` or the actual merge is explicitly invoked.

- [ ] **Step 5: Run and commit**

```bash
rtk cargo test --lib store::layout
rtk cargo test --lib git
git add src/store/layout.rs src/git tests/unit/store/layout.rs tests/unit/git
git commit -m "feat(git): stage verified local integrations"
```

### Task 10: Share PR operations, required-check observation, and merge queues

**Files:**
- Move: `src/action/feature/deliver/pull_requests.rs` → `src/action/feature/pull_requests.rs`
- Create: `tests/support/fake_gh.rs`
- Modify: `src/action/feature/{mod.rs,deliver/mod.rs,deliver/repos.rs}`
- Modify: `tests/delivery.rs`

- [ ] **Step 1: Extract/extend fake gh first**

Support exact contracts:

```text
gh pr list --head <branch> --state <open|all> --json url,state,mergeCommit,headRefOid
gh pr create --base <parent> --head <child> --title <title> --body <body>
gh pr checks <url> --required --json name,bucket,state,link
gh pr merge <url> --merge|--squash|--rebase --match-head-commit <sha>
gh pr view <url> --json url,state,mergeCommit,headRefOid
```

Model pass/fail/pending, queued-then-merged, head movement, merge failure, and merged result SHA.

- [ ] **Step 2: Define strict shared APIs**

```rust
pub(crate) fn find_pull_request(git_dir: &Utf8Path, head: &str, state: &str) -> Result<Option<PullRequest>, Failure>;
pub(crate) fn create_pull_request(git_dir: &Utf8Path, head: &BranchName, base: &BranchName, feature: &FeatureName) -> Result<PullRequest, Failure>;
pub(crate) fn required_checks(git_dir: &Utf8Path, url: &str) -> Result<Vec<PrCheckResult>, Failure>;
pub(crate) fn request_merge(git_dir: &Utf8Path, url: &str, source_sha: &str, strategy: IntegrationStrategy) -> Result<(), Failure>;
pub(crate) fn observe_merge(git_dir: &Utf8Path, url: &str) -> Result<PullRequest, Failure>;
```

`request_merge` maps strategy to one flag, always passes `--match-head-commit`, never passes admin/delete. `required_checks` accepts only all-pass; pending is a resumable blocked result, failure is failed orientation.

- [ ] **Step 3: Implement bounded merge observation**

After explicit merge request, poll `pr view` every 2 seconds for at most 10 minutes. `MERGED` returns result SHA; `CLOSED` fails; timeout returns `integration.pr_pending` with rerun guidance. Constants live in `pull_requests.rs`; tests use an injected zero-duration observation helper, not sleeps.

- [ ] **Step 4: Preserve delivery behavior through moved helpers**

Delivery preview may still collapse lookup errors to “new PR” only where current behavior intentionally does so; apply uses strict errors. Update imports and remove the old nested module—one command construction site only.

- [ ] **Step 5: Run and commit**

```bash
rtk cargo test --test delivery
rtk cargo test --lib action::feature
git add src/action/feature tests/support/fake_gh.rs tests/delivery.rs
git commit -m "feat(feature): observe protected pull request merges"
```

### Task 11: Implement leaves-first partial/resumable child integration

**Files:**
- Create: `src/action/feature/integrate.rs`
- Create: `tests/unit/action/feature/integrate.rs`
- Modify: `src/action/feature/mod.rs`
- Modify: `src/cli/root.rs`
- Modify: `src/bin/ivar.rs`
- Modify: `tests/unit/cli/root.rs`

- [ ] **Step 1: Write failing CLI/policy tests**

Parse `feature integrate child [--via pr|local] [--strategy ...]`; assert raw overrides and precedence CLI > feature > hall > local/squash.

- [ ] **Step 2: Write failing preflight tests**

Cover root refusal with `ivar feature deliver root`, missing parent, Plan gate, descendant blockers leaves-first, abandoned descendant exception, source/receipt staleness, dirty child/parent, unrestricted live-session refusal before the first successful receipt, and already-fresh idempotent outcome.

- [ ] **Step 3: Write failing parent-promotion interaction tests**

With child repo absent in parent:

- fixed interactive yes calls promote and proceeds;
- interactive no blocks;
- default/noninteractive blocks;
- compiled `--json` never prompts and returns fix command `ivar feature promote parent api`;
- promotion failure returns its typed failure and leaves no receipt.

- [ ] **Step 4: Add command/input/outcomes**

```rust
pub struct IntegrateInput { pub feature: String, pub via: Option<String>, pub strategy: Option<String> }
pub struct RepoIntegration {
    pub repo: RepoName,
    pub source_sha: String,
    pub target_branch: BranchName,
    pub result_sha: Option<String>,
    pub status: RepoIntegrationStatus,
    pub pr_url: Option<String>,
    pub detail: Option<String>,
}
pub struct IntegrateOutcome {
    pub root: Utf8PathBuf,
    pub feature: FeatureName,
    pub parent: FeatureName,
    pub policy: IntegrationPolicy,
    pub repos: Vec<RepoIntegration>,
    pub state: FeatureIntegrationState,
    pub closed_integrated: bool,
}
pub fn integrate(ctx: &Ctx, input: IntegrateInput) -> Outcome<IntegrateOutcome>;
```

Statuses are `reused`, `integrated`, `pending`, `failed`, `stale`.

- [ ] **Step 5: Write failing per-promotion resume tests**

With A and B promoted, make A complete and B fail before parent movement. Assert A's successful receipt is persisted and immediately locks only A; B can be repaired through a B-only execution contract; rerun reuses A without moving its source/result and integrates B. Add a failed-post-parent-check case: when B's failed receipt still has the same source and its result remains in parent history, rerun only repeats parent verification and replaces failed evidence on success—it does not apply B twice. If that failed receipt's source/result moved, classify it stale and use restoration/new-child orientation rather than guessing an incremental patch.

- [ ] **Step 6: Implement shared orchestration order**

1. discover/read manifest/child/immediate parent;
2. refuse root and non-approved Plan;
3. compute blocking descendants (never ancestors);
4. refuse unrestricted live feature sessions before any repo can gain its first successful receipt;
5. resolve policy precedence, then freeze the resolved relationship/base/policy once the first receipt is persisted;
6. for each promoted repo in name order, validate/reuse successful receipt, reverify an unchanged failed post-parent receipt, or resume an unreceipted repo;
7. ensure parent promotion interactively or block with exact command;
8. snapshot `source_sha` and parent SHA; require clean worktrees;
9. run ordered child checks;
10. execute selected via;
11. persist receipt immediately, including failed post-merge parent evidence;
12. continue other repos with warnings without exposing successful repos to mutation;
13. close integrated only when every receipt is fresh/passing.

- [ ] **Step 7: Implement local candidate path**

For merge/squash, create detached candidate at parent SHA, apply source, run parent checks there, verify parent SHA unchanged, then repeat the operation in parent worktree and run parent checks again. For rebase, create temporary branch at source SHA, rebase it onto parent in temp source worktree, run parent checks there, verify parent unchanged, then fast-forward parent to temp result and recheck. Remove only temporary worktrees/refs. Never move/delete child branch.

On pre-candidate/child/candidate failure, persist no receipt and leave parent unchanged. On post-parent failure, persist receipt with result SHA and failed parent result, warn, never revert.

- [ ] **Step 8: Implement PR path**

Push child branch; reuse/create PR with base exactly parent.branch; confirm PR head OID equals source; run required checks; explicitly request selected merge; observe merge; fetch/fast-forward parent local worktree; run parent checks; persist result/PR/check evidence. A merged PR with parent-check failure records failed evidence and blocks future parent integration/delivery.

- [ ] **Step 9: Close only as integrated**

After re-validating every receipt:

```rust
close::close(ctx, CloseInput {
    name: child.name.to_string(),
    outcome: "integrated".to_owned(),
})?;
```

Do not clean child refs/worktrees. A rerun validates and reports reused receipts; it never reopens. After the close record is written, all mutation helpers resolve to the whole-child immutable refusal regardless of individual receipt state.

- [ ] **Step 10: Run focused tests and commit**

```bash
rtk cargo test --lib cli::root
rtk cargo test --lib action::feature::integrate
git add src/cli/root.rs src/bin/ivar.rs src/action/feature tests/unit
git commit -m "feat(feature): integrate nested leaves into parents"
```

### Task 12: Restrict delivery to healthy roots and run root checks

**Files:**
- Modify: `src/domain/feature/delivery.rs`
- Modify: `src/action/feature/deliver/{mod.rs,preview.rs}`
- Modify: `tests/unit/{domain/feature/delivery.rs,action/feature/deliver.rs}`
- Modify: `tests/delivery.rs`

- [ ] **Step 1: Write failing child-delivery/root-tree tests**

Assert child preview/apply both block with `deliver.child_requires_integration` and exact command `ivar feature integrate child`. Assert root preview fingerprints recursive descendant states; apply blocks active/failed/stale/unintegrated descendants, ignores abandoned descendants themselves, and still sees active grandchildren beneath abandoned nodes.

- [ ] **Step 2: Add tree blockers to preview**

```rust
pub struct DeliveryTreeBlocker {
    pub feature: FeatureName,
    pub depth: usize,
    pub state: FeatureIntegrationState,
    pub reason: String,
}
```

Add `tree_blockers: Vec<DeliveryTreeBlocker>` to `DeliveryPreview` and fingerprint. These are descendants only.

- [ ] **Step 3: Enforce root and blockers before push**

Read feature before repo preview. If parent exists, refuse. For root, call `blocking_descendants`; preview reports them; apply refuses before any push/PR. Never inspect or require ancestors.

- [ ] **Step 4: Run root repo checks on apply**

Before pushing each root repo, execute its manifest checks in the root feature worktree. Failed checks warn/fail that repo and skip its push/PR while other repos continue. Include check results in `DeliverOutcome` so actual execution is machine-visible.

- [ ] **Step 5: Run and commit**

```bash
rtk cargo test --lib action::feature::deliver
rtk cargo test --test delivery
git add src/domain/feature/delivery.rs src/action/feature/deliver tests/unit tests/delivery.rs
git commit -m "feat(deliver): deliver only verified feature roots"
```

### Task 13: Encode automatic child creation and executor boundaries

**Files:**
- Modify: `src/harness/commands/{feature-create.md,plan.md,execute.md}`
- Modify: `src/action/execute/prompt.rs`
- Modify: `tests/unit/action/execute/prompt.rs`
- Modify: `tests/unit/harness/commands.rs`
- Modify: `tests/shipped_commands.rs`

- [ ] **Step 1: Write semantic instruction tests first**

Assert shipped bytes state all exact decisions:

- coordinator automatically runs `ivar feature create <child> --parent <current>` for an isolatable request outside the approved plan;
- coordinator announces but does not ask;
- approved-plan structural correction uses `feature execute replan`;
- local implementation divergence uses `feature execute reconcile`;
- executor never creates/promotes/reparents/integrates or mutates shared feature state and stops/reports to coordinator; a partial child's executor may only write literal paths under unreceipted/failed promotions granted by its contract.

Prompt test asserts every rendered executor prompt contains the same stop/report rule.

- [ ] **Step 2: Run tests and verify semantic failures**

Run:

```bash
rtk cargo test --lib action::execute::prompt
rtk cargo test --lib harness::commands
rtk cargo test --test shipped_commands
```

Expected: required phrases/rules absent.

- [ ] **Step 3: Update coordinator instructions**

In `feature-create.md`, define automatic nested creation and announcement. In `plan.md`, add the decision split:

```text
outside approved scope + isolatable -> create child automatically
structural correction to approved plan -> replan
implementation-only local divergence -> reconcile
```

In `execute.md`, identify the invoking agent as coordinator and repeat the same decision tree; no permission question before child creation.

- [ ] **Step 4: Update executor prompt**

Add before Operations:

```text
You are an executor, not the feature coordinator. If you discover an isolatable
request outside the approved operations, stop and report it. Do not create,
reparent, promote, integrate, close, delete, or otherwise mutate hall feature
state; the coordinator creates the child feature and announces it.
```

- [ ] **Step 5: Run/commit semantic tests**

```bash
rtk cargo test --lib action::execute::prompt
rtk cargo test --lib harness::commands
rtk cargo test --test shipped_commands
git add src/harness/commands src/action/execute/prompt.rs tests/unit tests/shipped_commands.rs
git commit -m "docs(workflow): automate nested subfeature coordination"
```

### Task 14: Prove complete nested journeys

**Files:**
- Create: `tests/nested_subfeatures.rs`
- Modify: `tests/support/{integration.rs,fake_gh.rs}`

- [ ] **Step 1: Add manifest/creation/reparent/tree journey**

Through compiled CLI, migrate v1→v2, configure hall local/squash + ordered checks, create two candidate parents and a child with feature override, reparent that still-pristine child, and assert parent/base update together. Then create its leaf and assert subsequent reparent refuses; independently assert refusal after plan creation and after promotion. Verify list/status recursive tree, depth, immediate targets, and precedence.

- [ ] **Step 2: Add all three local strategies**

For squash/merge/rebase, integrate leaf into child and child into root. Assert candidate failure leaves parent SHA unchanged; success updates only immediate parent; receipts contain source/target/result/check evidence; refs remain; children close integrated; root remains deliverable, not integrated.

- [ ] **Step 3: Add PR protection/queue journeys**

Cover create/reuse PR, required checks pass/fail/pending, source movement via `--match-head-commit`, queue then observed merge, all strategy flags, parent fetch/check after merge, failed parent checks recorded without revert, and rerun orientation.

- [ ] **Step 4: Add partial multi-repo resume**

Make repo A succeed and B fail before parent movement. Assert A receipt persists, no atomic claim is emitted, feature does not close, relationship/base/policy/membership freeze, A demote/rebase/session/A-contract attempts refuse before mutation, and a B-only contract can repair B. Rerun must reuse fresh A byte-for-byte, integrate B, and close integrated. Add a failed-evidence B retry that re-runs unchanged result verification, plus successful-receipt source movement becoming stale without unlocking A.

- [ ] **Step 5: Add parent-promotion mode matrix**

Compiled noninteractive and `--json` block with exact promote command; action-level fixed yes promotes parent and continues; fixed no leaves state unchanged.

- [ ] **Step 6: Add descendant/lifecycle/deletion journeys**

Prove leaves-first blockers, abandoned node exception plus active grandchild blocker, stale/failed receipt blocker, child deliver refusal, root deliver success only after all non-abandoned descendants fresh, unrestricted-session refusal during successful partial state, whole-child immutability/no-reopen after `integrated`, and parent deletion blocked until every descendant is deleted.

- [ ] **Step 7: Run and commit**

```bash
rtk cargo test --test nested_subfeatures
git add tests/nested_subfeatures.rs tests/support
git commit -m "test(feature): cover nested subfeature integration"
```

### Task 15: Update architecture, public docs, and generated reference

**Files:**
- Modify: `ARCHITECTURE.md`
- Modify: `docs/{concepts.md,glossary.md,guides/day-to-day.md}`
- Modify: `docs/reference/{on-disk-format.md,commands.md}`

- [ ] **Step 1: Update architecture map/boundaries**

Document new focused modules, immediate-parent direction, child-only parent relation, pristine reparent action, scoped mutation/contract guards, feature.json v3, manifest v2, Store migration baseline semantics, temporary local candidates, PR helper ownership, and retained refs. Correct the stale architecture claim that `infra::github` is a PR trait/fake seam; tests fake the `gh` executable.

- [ ] **Step 2: Update concepts/glossary/day-to-day examples**

Use only leaves-first examples:

```bash
ivar feature create checkout
ivar feature create checkout-v2
ivar feature create checkout-tax --parent checkout
ivar feature reparent checkout-tax --parent checkout-v2
ivar feature create checkout-tax-ui --parent checkout-tax --via pr
ivar feature integrate checkout-tax-ui
ivar feature integrate checkout-tax
ivar feature deliver checkout-v2 --preview
```

Define Parent Feature, Child/Subfeature, Leaf, pristine/reparentable, Integration Via (`pr|local`), Strategy, Verification Check/Evidence, Receipt, Fresh/Stale/Failed, successful-receipt promotion lock, partial resume, and why abandoned history does not block while deletion still does. Explain that reparent is allowed before work starts, relationship/base/policy/membership freeze after the first receipt, successful promotions freeze individually, and `integrated` freezes the whole child.

- [ ] **Step 3: Document exact on-disk v2/v3 shapes**

Show manifest integration defaults and repo checks, feature parent/override/receipt evidence, explicit committed migration, local auto-migration, and no reverse child list. State refs are retained for validation.

- [ ] **Step 4: Regenerate command reference**

Run:

```bash
IVAR_UPDATE_DOCS=1 rtk cargo test --test docs_reference
rtk cargo test --test docs_reference
```

Expected: create parent/via/strategy, reparent, integrate, recursive status, and the expanded close-outcome help are documented from Clap.

- [ ] **Step 5: Run docs tests and commit**

```bash
rtk cargo test --test docs_reference
rtk cargo test --no-fail-fast --test docs_reference
git add ARCHITECTURE.md docs
git commit -m "docs(feature): explain nested subfeatures"
```

### Task 16: Full acceptance and self-audit

**Files:**
- No planned new files; corrections stay in their owning task files.

- [ ] **Step 1: Run formatting/lints/tests**

```bash
rtk cargo fmt --all --check
rtk cargo clippy --all-targets --all-features -- -D warnings
rtk cargo test --all-features
```

Expected: all exit 0.

- [ ] **Step 2: Verify architecture/process boundaries**

```bash
rtk cargo test --test architecture
rtk rg -n 'Command::new\("git"\)|std::process::Command::new\("git"\)' src --glob '*.rs'
rtk rg -n 'Command::new\("gh"\)' src --glob '*.rs'
```

Expected: Git construction only in `git/exec.rs`; PR `gh` construction only in `action/feature/pull_requests.rs` (token lookup remains in `infra/github.rs`); architecture passes.

- [ ] **Step 3: Verify migrations and forbidden vocabulary**

```bash
rtk rg -n 'Migration::new\(1, 2, v1_to_v2\)' src/store/manifest/persistence.rs
rtk rg -n 'Migration::new\(2, 3, feature_v2_to_v3\)' src/store/feature.rs
! rtk rg 'IntegrationVia::Github|--via github|ancestors_not_integrated|integration_target.*default|collapse.*ancestor' src tests docs/reference docs/guides ARCHITECTURE.md
rtk git diff --check
```

Expected: exact migration links exist; obsolete design vocabulary is absent from implementation/public docs; no whitespace errors.

- [ ] **Step 4: Review invariants manually**

Confirm against the diff:

- only child.parent is persisted; children are derived;
- child target is always immediate parent branch;
- only descendants block; leaves go first;
- abandoned nodes alone do not block, active grandchildren still do;
- only roots deliver and only children integrate;
- defaults are local/squash and precedence is per-field CLI > feature > hall > embedded;
- checks execute in order and evidence is durable;
- local parent is unchanged until candidate checks pass;
- PR merges use required checks, head match, no admin, and observed result;
- pristine reparent validates existence/cycles/work-start facts and writes parent/base together once;
- partial repo success is persisted and never described as atomic; structure/membership freeze after the first receipt while only successful promotions lock individually;
- partial execution contracts name literal repos, exclude successful promotions, and are rechecked before spawn; unrestricted sessions cannot coexist with successful partial state;
- repo A can remain byte-for-byte locked while failed/unreceipted repo B is repaired and resumed;
- stale source/result/check evidence blocks with orientation;
- child refs remain for validation;
- integrated outcome is immutable/no reopen;
- coordinator creates children; executor stops/reports;
- parent delete checks descendants before teardown.

- [ ] **Step 5: Commit only real acceptance corrections**

If acceptance exposes scoped defects, return to the owning task, add the exact corrected paths listed there, rerun that task's focused tests, and use that task's commit command. Do not create an empty or catch-all acceptance commit.

## Final acceptance checklist

- [ ] Unlimited acyclic one-parent nesting; no reverse child persistence.
- [ ] `--parent` derives the immediate branch and conflicts with `--base`; `feature reparent <child> --parent <new-parent>` works only before promotions/planning/execution/sessions/receipts/descendants and atomically writes parent+base once; parent is immutable after work starts.
- [ ] Leaves integrate first into immediate parents; no ancestor-first or target-collapse path exists.
- [ ] Child delivery blocks to integrate; only healthy roots deliver.
- [ ] Descendant active/failed/stale/unintegrated states block; abandoned nodes alone do not.
- [ ] Policy precedence is CLI > feature > hall > embedded local/squash, using `pr|local` vocabulary.
- [ ] PR/local each implement squash, no-ff merge, and rebase+fast-forward.
- [ ] Missing parent promotion prompts only interactively; JSON/CI blocks with exact promote command.
- [ ] Ordered checks actually run for child/candidate/parent/root as specified.
- [ ] PR required checks/head/protection/queue are respected and observed.
- [ ] Multi-repo integration is partial, persisted per repo, and resumable.
- [ ] After the first receipt, relationship/base/policy and promotion membership freeze; successful promotion A is immutable while unreceipted/failed B remains repairable and resumable.
- [ ] Partial-state write contracts use literal repo prefixes, cannot name or ambiguously match a successful promotion, and are rechecked before executor spawn; unrestricted sessions are blocked once a successful receipt exists.
- [ ] Receipts contain source/target/result/policy/PR/verification evidence and time.
- [ ] Source movement, check drift/failure, or result-history loss becomes stale/failed and blocks.
- [ ] Child closes `integrated`; root delivery remains non-closing and an explicit root close uses `delivered`; abandoned remains historical.
- [ ] A fully fresh `integrated` outcome freezes the whole child/no reopen; partial state uses scoped structure, promotion, contract, and session guards rather than a blanket feature-wide mutation refusal.
- [ ] Child refs/worktrees remain available for conservative receipt validation.
- [ ] Parent deletion blocks on all descendants; recursive status/list expose tree health.
- [ ] Coordinator auto-creates isolatable children; executors stop/report instead of mutating hall feature state.
- [ ] Store accepts v1→v2 baseline while refusing v0; manifest v2 adds defaults/checks compatibly.
- [ ] Architecture/docs/generated reference/tests all match the final semantics.

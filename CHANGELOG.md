# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.4.0](https://github.com/mnzsss/ivar/compare/v0.3.0...v0.4.0) - 2026-08-25

### Added

- *(plan)* Make SPDD's upstream gates optional by absence ([#39](https://github.com/mnzsss/ivar/pull/39))

### Changed

- *(lib)* [**breaking**] Drop `action` and `cli` from the public API

### Documentation

- *(readme)* Refresh ivar repository landing page ([#37](https://github.com/mnzsss/ivar/pull/37))

### Fixed

- *(doctor)* report orphaned in-flight run receipts ([#36](https://github.com/mnzsss/ivar/pull/36))

## [0.3.0](https://github.com/mnzsss/ivar/compare/v0.2.1...v0.3.0) - 2026-08-22

### Added

- *(execute)* adopt provider-native run receipts ([#34](https://github.com/mnzsss/ivar/pull/34))

## [0.2.1](https://github.com/mnzsss/ivar/compare/v0.2.0...v0.2.1) - 2026-08-19

### Documentation

- *(harness)* Align the shipped commands with what the CLI does
- *(cli)* Say what session stop and prune actually do
- add a trademark policy ([#32](https://github.com/mnzsss/ivar/pull/32))

### Fixed

- *(plan)* Prefix scaffolded write_contract paths with the repo name
- *(feature)* Match promote's behaviour to its documented contract

## [0.2.0](https://github.com/mnzsss/ivar/compare/v0.1.0...v0.2.0) - 2026-08-18

### Added

- *(repo)* Resolve duplicate-only diverged branches with repo pull --resolve
- *(repo)* Add --diagnose to repo pull for diverged branches

### Other

- ivar-run-domain-cutover ([#28](https://github.com/mnzsss/ivar/pull/28))

## [0.1.0](https://github.com/mnzsss/ivar/compare/v0.0.0...v0.1.0) - 2026-08-18

### Added

- *(install)* Point the installer at the published releases

### Documentation

- Document AUR installation and the post-release pipeline

### Fixed

- *(release)* Ship the asset layout the install script fetches

## [0.0.0](https://github.com/mnzsss/ivar/releases/tag/v0.0.0) - 2026-08-18

### Added

- *(install)* add canonical POSIX-sh installer with offline harness
- *(execute)* scope completion evidence to the current plan revision
- *(execute)* adopt a complete revised graph via stable-id replan merge
- *(execute)* carry session provider into preparation
- *(execute)* resolve provider from caller session
- *(execute)* persist workstream targeting in plans
- *(deliver)* deliver only verified feature roots
- *(feature)* integrate nested leaves into parents
- *(feature)* observe protected pull request merges
- *(git)* stage verified local integrations
- *(feature)* guard partial integration per promotion
- *(feature)* expose and reparent nested subfeature trees
- *(feature)* derive nested tree health
- *(action)* run confirmed integration checks
- *(manifest)* configure verified integration
- *(feature)* persist nested lifecycle evidence
- *(feature)* model nested integration evidence
- *(feature)* Make every verb read the base ([#13](https://github.com/mnzsss/ivar/pull/13))
- *(execute)* Earn done by producing something
- *(git)* Read HEAD and the paths a commit range changed
- *(progress)* Say which repo a fetch is waiting on
- *(relations)* add guided repository context workflow
- *(session)* derive instructions from HALL.md
- *(actions)* maintain hall instruction topology
- *(config)* reconcile canonical hall instructions
- *(session)* continue a job across providers
- *(execute)* Audit the write contract against the worktrees
- *(git)* Report which paths in a worktree changed
- *(execution)* Derive board status from its workstreams
- *(execute)* Journal that a harness cannot ask before it launches
- *(proc)* Feed a streamed child on stdin
- *(harness)* Materialise the per-session write guard
- *(harness)* Add the headless execute command and stream parsing
- *(execute)* Render an executor's prompt from the plan
- *(execute)* Choose provider, model and agent from the graph
- *(proc)* Add a streaming spawn for line-protocol children
- *(session)* Start a discovery session when no feature is named
- *(feature)* let create name a branch its feature name cannot spell
- *(promote)* adopt a branch that already exists
- *(setup)* complete the setup-script environment contract
- *(deliver)* gate delivery on the plan approval gate
- *(doctor)* diagnose workflow command drift
- *(commands)* bootstrap workflows during setup
- *(sync)* materialise official workflow commands
- *(commands)* reconcile managed workflow files
- *(commands)* embed official workflows
- *(commands)* add provider command paths
- *(migrate)* Add the `ivar migrate` verb the error text already promised
- *(color)* Apply the colour decision to failures and warnings
- *(proc)* wire port discovery into session connect
- *(closing)* port discovery via /proc, lazygit view-dir test, drop unused deps
- *(plan)* refuse plan approve execution-graph, naming the execute path
- *(verbs)* implement repo setup/upstream, feature prune, plan status, provider add
- *(session)* implement stop, prune, and relay as a verb
- *(deliver)* restore PR creation with part-of sibling linking
- *(execute)* implement approve, tick, guard-check, reply
- *(skill)* implement sync, add, update, remove, detach, status, doctor
- *(git)* clone with auth via the credential helper and test the cascade
- *(git)* add GitHub auth cascade and credential helper
- *(skill)* add skill domain, sync planner, and golden vectors
- *(cli)* declare full v1 surface — all ~20 new CLI variants wired to stub actions
- *(store)* wire v1→v2 board migration into execution board store
- *(ivar-gaps)* implement remaining bifrost gaps ([#1](https://github.com/mnzsss/ivar/pull/1))
- *(hall)* implement status, doctor, cleanup, and provider list
- *(skill)* list and create hall skills
- *(plan)* scaffold, list, and show SPDD artifacts
- *(session)* view dir, harness spawn, and master-detail TUI
- *(feature)* implement create, list, promote, demote, status
- *(feature)* add domain types and store persistence
- *(repo)* implement list, add, remove, pull subcommands
- *(manifest)* add with_repo_added and with_repo_removed mutation helpers
- *(git)* add fetch and list_branches to Git trait
- Implement ivar sync
- *(store)* Add the setup-script receipt, and move the .gitignore writer here
- *(harness)* Keep an ivar-managed block in each harness's instruction file
- *(git)* Add the git module — git2 for reads, the binary for mutations
- *(infra)* Add the subprocess boundary
- Implement the hall data layer and ivar init

### Fixed

- *(execute)* reapprove invalidated graph gates
- *(harness)* scope the OpenCode guard to direct mutation tools
- *(harness)* export the OpenCode guard plugin as a function
- *(harness)* pin the OpenCode executor to its session view dir
- *(infra)* keep a spawned child's PWD in step with its cwd
- *(git)* name the config commands when git has no identity
- *(feature)* validate every repo before promoting any parent
- *(infra)* drain a streamed child's stderr while reading its stdout
- *(feature)* report the pull request url on a missing merge commit
- *(execute)* rewrite targeting only at canonical headings
- *(execute)* persist plan-authored selectors to the board
- *(execute)* complete session provider targeting
- *(execute)* launch the session-selected provider
- *(feature)* round-trip integration state in previews
- *(feature)* classify integrated close outcome
- *(store)* support versioned migration baselines
- *(release)* publish only merged release prs
- *(execute)* Say why a tick could not launch anything
- *(execute)* Hand the executor the whole operation entry
- *(execute)* Refuse a graph the plan cannot back before it is approved
- *(plan)* Tell plan authors the shape the executor parses
- *(execute)* Say when a tick blocked every workstream it found
- *(execute)* Audit a run against the commit it started from
- *(deliver)* Ask the remote whether the work is pushed
- *(git)* Record a push git will not record itself
- *(deliver)* Keep the PR urls the sibling pass reads
- *(deliver)* Ask gh for a PR url the way gh gives it
- *(git)* Give a hall's bare clone remote-tracking refs
- *(git)* Answer the operation git appends to a credential helper
- *(feature)* Reclaim the prefix dir a slashed branch leaves behind
- *(term)* Fall back when a pty answers zero columns
- *(relations)* surface executor instruction warnings
- *(clippy)* satisfy the -D warnings gate
- *(tui)* Scroll with the wheel, and let a dead shell be left
- *(sync)* Lift the read-only guard while a setup script runs
- *(fs)* Restore only the owner write bit when lifting a read-only guard
- *(tui)* Size and resize the shell to the panel it draws in
- *(tui)* Render the shell's colours, and mark a shell that has exited
- *(tui)* Move the navigation prefix off Ctrl+B and make it configurable
- *(tui)* Send the editing keys and control chords a shell needs
- *(tui)* Keep the terminal emulator's state across feeds
- *(tui)* Unfreeze the feature view when a shell falls silent
- *(plan)* accept a plan path projected through a session view dir
- *(execute)* Compare a changed path in the shape a contract names
- *(harness)* Leave no permission prompt nobody can answer
- *(execute)* Give the executor the operation, not its name
- *(execute)* Let a finished wave hand the board to the next tick
- *(harness)* Parse the OpenCode event shape 1.18.16 emits
- *(harness)* Invoke opencode with flags it accepts and a prompt on stdin
- *(execute)* Make tick launch the provider it only pretended to
- *(session)* Give the agent a real .claude dir it actually reads
- *(tests)* Pass session start's feature as a positional
- *(tests)* give the outside-a-hall test a cwd outside a hall
- *(lint)* clear the clippy gate across the v1 surface
- *(git)* remove stale debt comment claiming infra::github does not exist
- *(board)* complete execution board v2 schema bump

### Other

- *(gitignore)* Ignore local agent settings
- *(conduct)* Fix the reporting paragraph and update the address
- *(cargo)* Correct the claim that the crates.io name is reserved
- *(security)* Add a security policy
- *(install)* Document install paths that work today
- *(license)* Consolidate the dual licence into Apache-2.0
- *(execute)* correct the replan merge doc to describe the refusal
- *(cli)* pin replan's expanded input contract
- *(execute)* protect completed workstreams from accidental removal
- *(execute)* extract graph resolution into a shared resolver
- *(feature)* canonicalise the worktree the shell reports back
- *(plan)* restore the drift clause on the fingerprint helper
- *(cli)* name the prepare args for their verb
- *(architecture)* refuse a unit test that compiles nowhere
- *(store)* move the facade tests onto their owners
- *(action)* share the hall and session fixtures
- *(action)* share the worktree environment core
- *(feature)* split integrate into orchestration and plumbing
- *(execute)* move wave planning beside the launcher
- *(action)* let pull own the smart fetch sweep
- *(execute)* let plan_ops own the operation parser
- *(plan)* give the facade the shared artifact helpers
- *(git)* route the captured reads through run
- *(infra)* give the streaming runner its own module
- *(domain)* move the write contract out of the board
- *(tui)* give the scrollback decoder its own module
- *(feature)* collapse the checks lookup and the blocker walk
- *(feature)* drop suppressions that outlived their reason
- *(domain)* compile the integration invariants
- *(execute)* track rewritten plan blocks with a flag
- *(contributing)* drop the Signed-off-by requirement
- *(feature)* fmt nested integration
- *(feature)* polish nested integration acceptance
- *(feature)* cover nested subfeature integration
- *(workflow)* automate nested subfeature coordination
- *(execute)* Ask who runs what before the board is prepared
- simplify session canonical read and sync label
- *(relations)* cover canonical context lifecycle
- apply rustfmt to instruction topology code
- *(layout)* distinguish hall instructions from aliases
- *(tui)* Name what capturing the mouse costs
- Scope the read-only guarantee to the worktree root
- make the lenient path resolution and write check explicit
- teach the workflows the session plan path and continuation
- State what the execution guard cannot see
- *(execute,harness)* split tick and guard into focused modules
- *(unit)* relocate tests from modules added on main
- *(test)* reflow relocated action test modules
- *(architecture)* map the split tree, test layout, and retained exceptions
- *(tui)* separate the concrete PTY adapter from the generic driver
- *(harness)* isolate MCP config materialisation
- *(harness)* separate the shipped-command catalog from reconciliation
- *(infra)* extract Linux port attribution from subprocess execution
- *(infra)* split filesystem capabilities into io, symlink, guard
- *(action)* split delivery into repos, preview, and pull requests
- *(action)* split sync internals behind one verb
- *(action)* split hall verbs into one file per command
- *(git,store)* extract focused error submodules
- *(store)* split manifest into model, persistence, and error
- *(domain)* split feature domain by concept behind a facade
- *(architecture)* enforce centralized test layout and relocated layering
- *(unit)* relocate action test modules under tests/unit
- *(unit)* relocate foundation test modules under tests/unit
- *(support)* centralize unit and integration scaffolding under tests/support
- *(execute)* Describe the graph schema the parser accepts
- *(execute)* Extract the plan's Operations parser
- *(session)* Say how a discovery session is created
- *(session)* Cover discovery start, conversion and the relay refusal
- *(session)* allow indexing in the hook tests
- *(cli)* convert args to inputs by exhaustive destructuring
- *(commands)* simplify slash-name assertion
- *(commands)* name each workflow's slash command
- *(commands)* simplify command reconciliation
- *(commands)* complete workflow bootstrap coverage
- *(commands)* document shipped workflow ownership
- Add concepts, getting started, guides and the glossary
- *(reference)* Generate the command reference from clap
- *(cargo)* Replace two stale rationales with what is actually true
- *(deps)* drop six unused dependencies and dead github_auth_url
- *(board)* cover WriteContract, seq, event_id, and provider fields
- order skill module declarations
- deduplicate worktree-state counting and drop dead driver helpers
- apply rustfmt
- simplify session loop, pull outcome, and doctor probing
- Add the Rust crate skeleton, toolchain pin and lint policy
- Add the offline pull-request gate and the release workflow
- Record the stack decision, architecture and format contracts
- Add the README, licences and contribution rules
- Initial commit

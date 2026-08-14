# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.0.0](https://github.com/mnzsss/ivar/releases/tag/v0.0.0) - 2026-08-14

### Added

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

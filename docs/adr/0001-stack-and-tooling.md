# ADR-0001 — Stack and tooling

- **Status:** proposed
- **Date:** 2026-08-06

## Context

`ivar` is a from-scratch Rust implementation of a multi-repo hall orchestrator
previously written in TypeScript. A prior spike ported ~931 lines of that
TypeScript across three vertical slices — skill sync, terminal emulation, TUI —
and verified 51 of 52 differential cases identical at a 1.08× line ratio, so most
of the stack arrives measured rather than guessed.

This ADR records the whole dependency set: the parts the spike verified, the parts
it never touched, and the four places where its conclusions no longer hold.

## Decision summary

| Concern | Choice | Basis |
| --- | --- | --- |
| CLI parsing | `clap` 4.6, derive | verified by spike |
| Serialization | `serde` 1 + `serde_json` 1 | verified by spike |
| YAML (frontmatter only) | `serde_saphyr` 1 | **changed — see 1** |
| Terminal emulation | `vt100` 0.16, gap documented | **changed — see 2** |
| PTY | `portable-pty` 0.9 | verified by spike |
| TUI | `ratatui` 0.30 + `crossterm` 0.29 | verified by spike (at 0.29/0.28) |
| Git | `git2` 0.21 for reads, `git` CLI for mutations | **refined — see 3** |
| HTTP | `ureq` 3 + `platform-verifier`; no async runtime | **changed — see 4** |
| Concurrency | `std::thread::scope` + `mpsc::sync_channel` | design |
| Errors | `thiserror` 2 per module + one serializable envelope | design |
| Paths | `camino` 1 — `Utf8PathBuf` everywhere | design |
| Config | plain `serde` + `deny_unknown_fields`; no framework | design |
| Layout | one crate, lib + bin | design |
| Release | `release-plz`, GitHub Actions, crates.io Trusted Publishing | decided upstream |

## The four changes

### 1. `serde_yaml` is deprecated — use `serde_saphyr`

The spike substituted `gray-matter` → `serde_yaml`. `serde_yaml` was archived by
its author: newest release `0.9.34+deprecated`, published 2024-03-25, nothing
since. Shipping a public binary on an explicitly abandoned parser is not
defensible when the pitch is "read the source and check".

| crate | latest | assessment |
| --- | --- | --- |
| `serde_yaml` | `0.9.34+deprecated` (2024-03) | rejected — abandoned by author |
| `serde_yaml_ng` | `0.10.0` (2024-05) | drop-in fork, itself ~2 years quiet |
| `serde_norway` | `0.9.42` (2024-12) | drop-in fork, also quiet |
| `gray_matter` | `0.3.2` (2025-07) | wraps the deprecated stack, no clean round-trip |
| **`serde-saphyr`** | **`1.0.1` (2026-08-05)** | **chosen** |

(The crate is `serde-saphyr`; the module it exposes is `serde_saphyr`.)

`serde_saphyr` is actively developed on the `saphyr` parser, exposes both
`from_str` and `to_string` — round-trip is required, since closing a feature
writes `outcome` and `closedAt` back into `plan.md` frontmatter — and advertises
panic-free parsing with useful errors. Its 1.0 being days old was the one real
risk.

**Retired 2026-08-06, with tests rather than optimism.** Round-trip verified for
the case that actually matters: serialize → reparse → equal struct, for LF and
CRLF documents, and an empty frontmatter block deserialising into a struct whose
fields all carry `#[serde(default)]`. Both error types are ordinary
`std::error::Error` implementors, so they compose into the module's `thiserror`
enum with `#[source]` and need no adapter. Default features are what is wanted;
no feature flags.

**The risk is contained by construction.** YAML appears in exactly one place:
splitting and parsing Markdown frontmatter. That lives behind
`infra::frontmatter`, whose surface is two functions, so swapping the parser is a
one-file change — the cheapest decision here to reverse. Frontmatter *splitting*
is hand-rolled (~30 lines); no crate is needed to find a leading `---` fence.

### 2. `wezterm-term` is not installable; the SGR 8/9 gap ships documented

The spike found one divergence in 52 cases: `vt100` does not track SGR 8
(invisible) or SGR 9 (strikethrough), where `@xterm/headless` does. Two
mitigations were proposed — patch `vt100` upstream, or swap to
`wezterm-term`/`termwiz`. Both were re-checked:

- **`wezterm-term` and `wezterm-surface` are not published on crates.io.** They
  exist only inside the wezterm repository. crates.io forbids git dependencies in
  published crates, so depending on them makes `cargo install ivar` impossible —
  and crates.io is one of the four distribution channels. This option is closed,
  not merely expensive.
- **`vt100-ctt` 0.17.1**, a maintained fork updated 2026-02, does *not* fix the
  gap. Its `Cell` exposes the same attributes as upstream — `bold`, `dim`,
  `italic`, `underline`, `inverse`. No `invisible`, no `strikethrough`.

What remains is upstream `vt100` with the gap, or `termwiz` 0.23 — published, by
the same authors as `portable-pty`, models full SGR, but exposes a
`Surface`/`Change` model instead of `vt100`'s ready-made `Parser::screen()`, so it
is a larger port than the 117 lines the spike wrote.

**Decision: ship `vt100` 0.16.2.** The gap costs fidelity in two narrow cases —
password masking, and some linter and diff output rendered inside the session
panel. The limitations page is already a launch deliverable; this is one line on
it. The emulator sits behind `tui::screen::Screen` so `termwiz` stays a later
swap, and a two-bit upstream patch to `vt100` remains worth sending.

`vt100-ctt` is the recorded fallback if upstream goes cold: identical API, so a
dependency-line change.

### 3. Git: `git2` for reads, the `git` binary for mutations

The prior decision was `git2` (libgit2), on the evidence that the closest prior
art — GRM, ~8,800 lines of Rust — used it for a decade without recorded regret,
and that its only real friction was remote authentication, a class of problem a
local-only tool avoids.

That holds for the read side and breaks in three specific places:

1. **`ivar` does touch remotes.** Smart Fetch, Pull, and Promote's pre-branch
   refresh all fetch; delivery pushes. So GRM's worst bug — hardware SSH keys
   with touch confirmation failing under libgit2's SSH transport, still open, the
   maintainer having declined to take it — lands squarely here too. Shelling
   `git fetch`/`git push` inherits the user's working credential setup (agent
   forwarding, hardware keys, `gh` as credential helper) for free.
2. **Worktree layout is a public contract with third-party tools.** GRM broke
   `lazygit` outright by emitting a nonstandard-but-legal libgit2 worktree layout.
   Testing the view dir against a third-party git TUI is already a launch gate;
   `git worktree add` via the CLI produces exactly stock layout, which is the
   cheapest way to pass it.
3. **Rebase.** The required behaviour is `git rebase <default_branch>`, aborting
   on conflict and continuing to the next repo. Shelling that is four lines;
   reimplementing rebase on libgit2 is a known trap.

**The rule, written down so it does not erode:**

> `git2` is read-only and never touches the network. Anything that mutates refs,
> mutates worktree layout, or contacts a remote goes through the `git` binary.

`git2` owns: opening repositories, resolving HEAD and refs, listing worktrees,
`graph_ahead_behind` for the merge and sync gates, status and dirtiness checks,
reading blobs and trees. The CLI owns: `clone --bare`, `worktree add`/`remove`,
`branch`, `fetch`, `push`, `rebase`, `checkout`.

This makes the AUR package's `depends=('git')` correct rather than contradictory,
and keeps `git2` where its prior art is strongest. Both sit behind one `git::Git`
trait, so the split is an implementation detail callers never see — and tests
exercise it against real temporary repositories, never a mock.

### 4. No async runtime; `ureq` instead of `reqwest`

The GitHub authentication cascade was specified as "`gh` preferred →
`GITHUB_TOKEN`/`GH_TOKEN` via `reqwest` → clean failure". `reqwest` pulls
`tokio`, and `tokio` would be the single largest architectural commitment in the
dependency list.

Nothing here wants an executor:

- No server, no daemon, no socket. That is a stated non-goal.
- The PTY read loop is one blocking read per shell on its own thread — what the
  spike did, successfully.
- Batch work over N repos is disk- and CPU-bound, and its live-progress pattern
  is scoped threads over an `mpsc::sync_channel`, not tasks.
- The session view's I/O driver is required to own no executor and spawn no
  background tasks. With no runtime present, that property is free rather than
  something to maintain.
- Startup time is a headline number — ~3 ms wall against Node's ~76 ms. A runtime
  spins up threads before `main` does anything.

**Decision: no async runtime. HTTP via `ureq` 3 with `platform-verifier`.**
`ureq` is blocking and small, and `platform-verifier` makes TLS verify against the
OS trust store rather than a compiled-in Mozilla root bundle. GRM shipped the
compiled-in bundle and got exactly one bug report from it: TLS failures behind
corporate MITM proxies. That is a known-outcome mistake, avoided at line one.

HTTP is confined to `infra::github` behind a trait, which is also how the PR gate
stays offline: a hand-rolled fake, not `wiremock` — which is async and would drag
`tokio` in through the test profile.

## Choices the map left open

### Error model — `thiserror` per module, plus one serializable envelope

Two things, deliberately separate.

**Internal:** one `thiserror` enum per module, composed with
`#[error(transparent)]` + `#[from]`. No `anyhow`, no `eyre`. A binary whose error
output is a machine-readable contract cannot afford a type that erases which
error it is.

**External:** every failure renders through one envelope.

```rust
pub struct Failure {
    pub status: Status,              // Blocked | Failed
    pub code: &'static str,
    pub what: String,
    pub expected: Option<String>,
    pub actual: Option<String>,
    pub fix_actions: Vec<FixAction>, // ordered, most-recommended first
    pub details: Option<serde_json::Value>,
}

pub struct FixAction {
    pub code: &'static str,
    pub what: String,
    pub command: Option<String>,
    pub safe: bool,
}
```

`Blocked` is a precondition refused before anything happened; `Failed` is a
failure in flight. That distinction is what tells an agent whether a retry is
safe. `FixAction.safe` is what lets an agent self-recover without being
authorised to force-push.

`miette` is **rejected**: it is a second renderer competing with the envelope, and
the envelope is the contract — the same bytes must reach `--json` consumers and
the human surface. Human rendering is a `Display` impl over `Failure`.

### Batch operations return warnings as data

Any verb that crosses the hall returns `Result<Report<T>, Error>`, where `Report`
carries `Vec<Warning>`. `Error` means the whole operation is unsalvageable;
`Warning` means one repo had a problem and the rest ran. Warnings are ordinary
data inside `Ok`, never routed through `Result`.

### Config: no config framework

`schematic` was evaluated and **rejected**: 0.19.x, ~400k lifetime downloads,
single-vendor, and its value is multi-format parsing plus `extends` composition
plus env overlays plus JSON Schema generation — four features `ivar` does not
want. `ivar.json` has one format, no inheritance, no env overlay, and a
deliberate policy of never migrating itself.

What is kept is the policy, which costs nothing: `#[serde(deny_unknown_fields)]`
on every config struct, so a typo is a hard parse error rather than silence; and
"genuinely absent" (`Ok(None)`) discriminated from "present but unreadable" (hard
error) by matching `io::ErrorKind::NotFound` specifically.

### Crate layout: one crate, lib + bin

```toml
[lib]   name = "ivar"  path = "src/lib.rs"
[[bin]] name = "ivar"  path = "src/bin/ivar.rs"
```

Not a workspace. The target is ~21,000 lines in a single local binary with no
daemon, no plugin host and no remote cache — the prior art that fragmented into
60 crates was solving all three. One crate means one version, one publish, and
the simplest possible `release-plz` configuration. Module boundaries are enforced
by a test over `use` statements instead of by crate boundaries; see
[`ARCHITECTURE.md`](../../ARCHITECTURE.md).

### Toolchain and MSRV

`rust-toolchain.toml` pins an **exact** version, not `stable`. A floating channel
means a compiler release can turn CI red on an unrelated day, and `-D warnings`
makes every new lint a build break.

`rust-version` in `Cargo.toml` is the MSRV, and is a public contract from the
first crates.io publish. CI builds against exactly it.

**Pin: 1.97.1. MSRV: 1.95.0.** The MSRV is not a free choice — it is set by the
strictest dependency. `sysinfo` 0.39.6 requires 1.95, so 1.94 fails to build.
Verified both ways, not assumed.

### Everything else

| crate | for | why this one |
| --- | --- | --- |
| `camino` | `Utf8Path`/`Utf8PathBuf` for every path | deletes "is this path valid Unicode" from nearly every signature |
| `fs-err` | io errors that name the path | most of what the old `Fs` wrapper provided; still wrapped in `infra::fs` so a missing primitive has one home |
| `sha2` | content fingerprints — tree hash, config drift, plan revision | what the spike hashed with; parity vectors depend on it |
| `walkdir` | directory traversal for tree hashing | `ignore` is only needed where gitignore semantics apply, and here they do not |
| `uuid` | session ids | matches the ids already on disk; nothing wants sortable keys |
| `owo-colors` | colour | honours `NO_COLOR`, no allocation, no global state |
| `comfy-table` | status tables | what the prior art uses for exactly this |
| `indicatif` | progress rendering fed by the reporter channel | pairs with the `mpsc` pattern |
| `etcetera` | home and config directories | more principled about platform conventions than `dirs` |
| `sysinfo` | process tree, for teardown and port attribution | cross-platform, unlike reading `/proc` |
| `netstat2` | listening sockets, for port attribution | see below |
| `clap_complete`, `clap_mangen` | completions, man page, generated reference tables | the docs reference is generated from `clap` |

**Port attribution is not a `/proc` scan.** The incorporation backlog specifies
sweeping `/proc/net/tcp{,6}` and crossing it with the session's process tree.
`/proc` does not exist on macOS, and macOS is a first-class published target —
the primary development machine. So the mechanism is `netstat2` for
cross-platform socket enumeration crossed with `sysinfo` for the process tree.
`netstat2` is small with modest download numbers; it is confined to
`infra::ports`, and the fallback if it disappoints is shelling `lsof -i` on macOS
and reading `/proc` on Linux.

### Testing

- **No mocking of internal code.** Real temporary git repositories via
  `tempfile`, real filesystem, real subprocesses — matching the prior art's
  integration tier and the house convention it came from.
- `assert_cmd` drives the compiled binary; `insta` snapshots stdout, which is the
  cheapest guard on "the human surface and `--json` emit the same bytes".
- `rstest` for parametrised cases. External systems are faked at a trait seam.
- `cargo-llvm-cov` **reports** coverage and does not gate. A per-file threshold is
  good discipline inside a private monorepo and a hostile gate for a stranger
  whose ten-line PR would fail on coverage of a file they did not touch.
- The **differential harness stays**: run the surviving TypeScript and the Rust
  against identical fixtures and require canonicalised equality. It produced
  51/52, it is cheap to keep, and it is applied where a silent reconciliation
  regression actually hurts — sync planning, hall state, session lifecycle.

### Canonical JSON is mandatory, not stylistic

The spike's first differential run failed 13 of 13 cases with *semantically
identical* output. TypeScript emitted object keys in spread order; `serde`
emitted struct field order. These files are written to disk, so a byte comparison
reports churn where there is no change — and two implementations writing the same
file alternately produce noise in the user's git history.

Therefore **every on-disk JSON write goes through one function**,
`infra::json::write_canonical`: sorted keys, two-space indent, LF endings,
trailing newline. No `serde_json::to_writer` calls anywhere else. This is also
the only reason golden vectors shared with the surviving TypeScript package can be
compared byte-for-byte.

## Verification

This ADR's central claim is that these crates co-exist. That was checked, not
assumed — on macOS (aarch64), 2026-08-06:

| check | result |
| --- | --- |
| `cargo check --all-targets` | resolves and compiles, 31 s cold |
| `cargo fmt --all --check` | clean |
| `cargo clippy --all-targets --all-features -- -D warnings` | clean |
| `cargo test --all-features` | passes (the layering test runs) |
| `cargo build --release` | 554 KB binary, no runtime dependency |
| `cargo +1.95.0 build` | passes — MSRV floor confirmed |
| `cargo +1.94.0 build` | fails on `sysinfo` — floor is real, not guessed |
| `tokio` / `async-std` / `smol` in the normal dependency graph | absent |

Two corrections the check produced: the crate is `serde-saphyr`, not
`serde_saphyr`, and `clippy::string_to_string` has been removed from the lint set
upstream. The 554 KB figure is a floor, not a projection — most dependencies are
declared but not yet called, so LTO strips them.

## Consequences

- A dependency tree with no async runtime and no bundled TLS root store: small
  binary, fast start, and the session driver's "owns no executor" property holds
  for free.
- One reversible-by-design seam per uncertain choice — `infra::frontmatter`
  (YAML), `tui::screen::Screen` (emulator), `git::Git` (git backend),
  `infra::github` (HTTP), `infra::ports` (socket enumeration). Each of the five
  choices carrying real residual risk is a one-module swap.
- The SGR 8/9 fidelity gap ships. It must appear on the limitations page before
  launch, not after the first issue.
- `wezterm-term` is off the table while crates.io remains a distribution channel.
  If full SGR becomes necessary, the move is `termwiz`.

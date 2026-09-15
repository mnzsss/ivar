# Task packet template

Generate one packet into `../../tasks/NN-<semantic-task-name>.md` for every
task in `plan.md`. Name files with a two-digit order prefix (e.g.
`01-pin-scaffold.md`). Each task packet must follow this structure:

````
### Task N: [Component Name]
**Files:**
- Create: `exact/path.rs`
- Modify: `exact/path.rs:123-145`
- Test: `tests/exact/path.rs`
**Readers:**
- For each symbol this packet writes, paste the output of
  `git grep -n '<symbol>'`. Run it from the repo root with no pathspec:
  it covers every tracked file and no build artifact, so the scope is a
  fact about the repo rather than a list someone has to derive. A reader
  is anything that asserts something about the symbol — a doc stating a
  count, a fixture, a CI config — not only code that compiles.
- A symbol reached through a re-export answers to a name this grep never
  sees; grep that name too.
- Name the constraint each reader imposes (an assertion, a caller, a config
  consumer)
- When the grep finds nothing outside the declared files, write
  `no readers outside the declared files` and paste the command, so the
  claim is falsifiable
**Interfaces:**
- Consumes: [exact signatures from earlier tasks]
- Produces: [exact function names + types later tasks rely on]
- [ ] **Step 1: Write the failing test** — the test's literal source:

  ```rust
  #[test]
  fn rejects_a_transport_the_schema_does_not_define() {
      let error = McpServerDef::parse(r#"{"type":"sse","url":"http://x"}"#)
          .expect_err("sse is not a canonical transport");
      assert_eq!(error.field(), "type");
  }
  ```
- [ ] **Step 2: Run test to verify it fails**  Run: `...`  Expected: FAIL ...
- [ ] **Step 3: Write minimal implementation**
- [ ] **Step 4: Run test to verify it passes**  Run: `...`  Expected: PASS
- [ ] **Step 5: Commit**
````

Step 1 carries the test's literal source: the assertions themselves, not a
description of them. It is the executable specification and it is short, so
there is never a volume argument for describing it instead. Step 3 carries
literal source at every point that decides behaviour — the exact call,
signature, option set, or type.

Volume that follows mechanically from a decision point may be described
instead, marked `**Sketch:** <reason>` where the reason states why literal
code is inappropriate for that volume. The escape is Step 3's alone, never
Step 1's. A sketch relaxes a body, never an interface: it still names every
symbol it produces with its exact signature, so a later packet's `Consumes`
cites something real.

A packet whose Step 1 shows no source, or whose described step carries no
`**Sketch:**`, is incomplete in the same way a missing Readers section is.

No placeholders anywhere: no `TBD`/`TODO`, no "implement later", no "add error handling",
no "similar to Task N", no step that says what to do without showing how, no reference to a
symbol defined nowhere. Steps 1–2 are Red (write failing test, run to verify FAIL),
Steps 3–4 are Green (minimal code, run to verify PASS). Refactoring is permitted between
Step 4 and Step 5.

## Self-check

The plan-document reviewer judges every packet against these rows; check them
before dispatching the review:

| Category | What to Look For |
|---|---|
| Blast Radius | Every packet has a Readers section holding real `git grep -n` output run without a pathspec; each reader's constraint is named; the Verification checks are at least as wide as those readers, and a reader no command can check is verified by reading it |
| Literal Code | Step 1 shows the test's source, never a description of it; Step 3 shows the exact call or signature at each point that decides behaviour; every described step carries `**Sketch:**` whose reason states why literal code is inappropriate there — a marker with a generic or absent reason is an issue |

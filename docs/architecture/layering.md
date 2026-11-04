# Layering

Part of the [architecture overview](../../ARCHITECTURE.md).

## Layering

Dependencies point downward only. A test over `use` statements enforces this —
a convention nobody remembers is not a boundary.

```
app  ─────────────► cli ─────────────► action ─────────────► domain
                                          │                     ▲
                                          ├──► store ───────────┤
                                          ├──► git ─────────────┤
                                          ├──► harness ─────────┤
                                          ├──► tui ─────────────┤
                                          └──► infra ◄──────────┘   (store/git/harness/tui use infra)
```

| module | may import | may **not** import |
| --- | --- | --- |
| `app` | everything | (nothing may import `app`) |
| `cli` | `action`, `error` | everything else |
| `action` | anything below | `cli` |
| `domain` | `error` only | `store`, `git`, `harness`, `tui`, `infra` |
| `store` | `domain`, `infra`, `error` | `action`, `git`, `harness`, `tui` |
| `git` | `infra`, `error` | `action`, `store`, `domain`, `harness`, `tui` |
| `harness` | `domain`, `infra`, `error` | `action`, `store`, `git` |
| `tui` | `domain`, `infra`, `error` | `action`, `store`, `git`, `harness` |
| `infra` | `error` | everything else |

Two of these earn their strictness:

**`domain` is pure.** It holds the types and the invariants — what a valid
promotion is, how hall health is derived, which branch a repo resolves to. Being
pure is what makes those testable without a temp directory, and what stops the
rules from scattering into the verbs.

`domain::session` is the named clock exception: it calls
`std::time::SystemTime::now()` directly because its timestamp is a plain
`String`, written once at construction and never read back as a clock. Routing
it through `store` and back would add conversion without strengthening an
invariant. `tests/static_checks/architecture.rs`'s layering scan only walks `use` statements,
so a fully-qualified `std::time::SystemTime::now()` call is invisible to it;
this exception is enforced by review, not by the test.

**`tui` cannot reach `action` or `store`.** State is pushed *into* the driver by
the host loop; the driver never fetches. This is what makes `widget.rs` a
referentially transparent projection: two renders of the same snapshot produce
byte-identical cells, so it can be tested headless against a `TestBackend` with no
clock and no I/O.

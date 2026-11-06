macro_rules! kind_definitions {
    () => {
        "\
Page kinds:
- product: what a delivered capability does today, the contract visible to a user or operator.
- update: what a specific delivery changed and what stayed open.
- decision: the option chosen, the alternatives and why.
- gotcha: a trap with its symptom, cause and fix.
- concept: a domain or architecture idea explained.
- procedure: steps to do something.
- session: the consolidated summary of one session (written by consolidation, never by you)."
    };
}

pub const KIND_DEFINITIONS: &str = kind_definitions!();

pub const MEMORY_INSTRUCTIONS: &str = concat!(
    "\
Hall memory: durable pages in docs/memory/<kind>/ and the observations captured from sessions.

- Call memory_query before proposing architecture or when the user refers to earlier work, \
and memory_read_page for the full body of a hit.
- memory_status reports the health and counts of the memory index.
- Record a decision, gotcha, procedure, product or update page during the feature, once the insight \
is settled; never `session`. The update page of a delivery is written before delivery, not at cleanup.
- Before `ivar feature deliver`, call memory_review, give each possibly stale page a verdict and \
confirm; delivery refuses a feature without that review.
- The memory tools do not commit: a written page stays uncommitted until you commit it.

",
    kind_definitions!()
);

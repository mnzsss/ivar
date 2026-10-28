# graph_explore precision and response budget

`graph_explore` returns at most 20 candidates (symbols and files) with line
ranges. This note records how a response that exceeds its byte budget is
assembled, so the text an agent asked for survives and the rest is named.

## Budget priority

A structured MCP answer (`format: "json"`) that does not fit its budget (18,000
bytes, or 24,000 for explicit `paths`) is filled in this fixed order. Each
group only gets what the earlier ones left.

1. `file_matches`
2. `sources`: the requested symbol, file, or `paths` range. It may take all
   the room except a reserve for the symbols below (their cost, at most a
   quarter of the record budget).
3. `primary_symbols`
4. `flows`
5. `call_flows`
6. `entry_points`
7. `direct_relations`
8. `transitive_consumers`
9. `not_shown`

A symbol with hundreds of callers therefore cannot push the requested source
out of the answer. Relations are rows of `file:line` locations, never source,
so they are the part that is shortened.

`format: "compact"` carries no source text. It keeps the same relative order
(`file_matches`, `primary_symbols`, `direct_relations`,
`transitive_consumers`). Markdown reserves the source section first (at least
8,000 characters) and bounds the blast radius to eight symbol lines, so the
source also precedes callers there.

## Truncation and continuation

Rows are omitted whole, except source text. When one source (or, with no
source for its file, one primary symbol) is larger than the room left, the
answer keeps the longest prefix of whole lines that fits and records it in
`output.truncated` (`section`, `repo`, `file_path`, `next_start`, `next_end`).

`output.next` then calls `graph_explore` with `paths: ["file:next_start-next_end"]`:
it starts on the line after the last one returned and covers the rest of what
the answer would have shown. Repeating the call with `next` advances; it never
returns the same prefix. Following `next` until the answer is no longer
truncated rebuilds the file with no gap and no overlap.

A source that does not fit at all (less than a few hundred bytes of room) is
omitted whole and `next` asks for its full range. The markdown continuation
(`…truncated after line N`) ends at the end of the requested range, not at the
end of the file.

## Callers and importers

No list-only mode or tool parameter was added. Structured relations already are
a location list without source, and with the order above they no longer
displace source. For a query that names importers or callers and matches no
symbol, markdown already answers with the matching files and one `Next:` call.
A dedicated parameter, if needed, is a separate change.

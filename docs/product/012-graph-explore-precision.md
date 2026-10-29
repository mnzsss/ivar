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

## Relevance floor and honest misses

A candidate that shares one generic word with a long intent query looks like
an answer and is not. `graph_explore` therefore measures, for each candidate
file, how much of the query it covers, and says so when nothing covers enough.

### Coverage

The query's words are the distinct, lowercase, plural-folded words of its
terms, split at punctuation, camelCase and digits. Generic words (`how`,
`code`, `function`, `where`, …) and the extension of a typed file name
(`reply-comment.ts`) are dropped; a query made only of generic words keeps them.

A file covers a word when the word appears, as a whole word, in its path, in the
name of one of its candidate symbols, or in its indexed content. Content counts
only within one stretch of 40 lines (the stretch that holds the most distinct
words): a workflow or a doc section about the topic keeps its words together,
while a large file holds the same words by chance, far apart. A file is
**strong** when it covers at least half of the words (one of two, two of
three, four of seven), or when the query names it: an exact path, a basename,
or an exact identifier-shaped symbol name. Everything else is **weak**. A query
with no words left to cover (a bare path) has no floor.

Candidates are ordered strong first, then (after exact and pinned evidence) by
how many words they cover, then by the existing score. Coverage comes before the
score so that a file that names five symbols for one word does not outrank a
workflow that holds five of the query's words.

The half is a calibration, not a law: it keeps one shared word out of a seven
word query, and keeps a two-word intent that matches in one file. The
fixtures that calibrate it are synthetic (a workflow YAML against unrelated
application symbols, a symbol that exists next to terms that do not).

### Confidence

| `confidence` | Meaning |
| --- | --- |
| `high` | Every candidate is strong. The field is left out of the answer. |
| `partial` | Some candidates are weak. Strong ones come first with source; weak ones follow, at most five files, as locations. |
| `none` | No candidate is strong. The answer says `No confident match for '<query>'`, names the candidates as weak, and attaches no source, callers or impact. |

A weak candidate carries `weak: true` and never source: a `file_matches` entry
keeps its line and an empty `excerpt`, a `primary_symbols` entry keeps its
span and an empty `code`. Weak candidates are left out of relations, call
flows and the blast radius, so a name shared with an unrelated symbol no longer
adds cross-repo edges to a miss.

The field appears in all three formats:

- markdown: `Confidence: none. No confident match for '…'.` or
  `Confidence: partial.`, then the weak list under `Weak matches`.
- `json`: `"confidence": "none"` (or `"partial"`) after `query`, and
  `"weak": true` on the candidates.
- `compact`: a leading `#SCHEMA: confidence` record, and a trailing `weak`
  column on `file_matches` and `primary_symbols` rows (their schema headers name
  it). With `high` confidence the compact schemas are unchanged.

### Documentation and config

When the query is plain language (no path token and no identifier-shaped term),
the content score of documentation and configuration files (`.md`, `.mdx`,
`.rst`, `.txt`, `.yml`, `.yaml`, `.toml`, `.json`, anything under `.github/`,
`workflows/` or `docs/`) is multiplied by 1.5. "How does CI treat changesets"
is answered by a workflow, not by the code of an app that mentions a request.
The weight orders strong candidates only; it never turns a weak file strong.
A query with a path or an identifier keeps the plain ranking.

### Measuring misses

When the answer is `none` and has candidates, `graph_explore` records a `weak`
miss (`ivar graph misses --kind weak`) with the query and the reason `weak`.
An answer with no candidates at all, a `partial` answer and a confident answer
record nothing.

### Not covered

The pinned directory of a path token still bounds which files are candidates:
`apps/site changeset required` reads only inside `apps/site`, and answers
`none` if nothing there covers the words, even when a workflow elsewhere would.
Path resolution is a separate concern from relevance. Budget and truncation
(above) are unchanged, and so is the index schema.

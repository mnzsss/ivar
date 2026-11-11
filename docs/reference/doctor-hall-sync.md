# `ivar doctor`: hall sync

Part of the [command reference](commands.md). The hall is a git repository that `ivar hall sync`
commits and pushes. When that cannot finish, `ivar doctor` says so.

Each finding is read from git when `ivar doctor` runs. Nothing is recorded in a marker file, so
a finding goes away as soon as the hall is fixed. No network call is made: ahead and behind are
counted against the remote-tracking ref, as of the last fetch.

| Code | Severity | Meaning | Fix |
| --- | --- | --- | --- |
| `hall.sync_diverged` | error | the hall's branch and its upstream each have commits the other lacks, usually a rebase conflict `ivar hall sync` could not settle; the finding gives the count on each side | in the hall, run `git pull --rebase`, resolve the conflicts, then `git push` |
| `hall.sync_unpushed` | warning | the hall has commits its upstream does not | `ivar hall sync`, or `git push` in the hall |
| `hall.sync_in_progress` | error | a merge, rebase or cherry-pick is stopped in the hall | in the hall, `git <operation> --continue` or `git <operation> --abort`, then `ivar hall sync` |

A hall that is not a git repository, or whose branch has no upstream, gets none of these. A
hall that is only behind its upstream gets none either: a pull fixes it and nothing is lost.

## The session brief

`ivar memory brief`, the context injected when a session starts, carries one extra line when
the hall is diverged or has a stopped operation, and nothing when it does not:

```text
Hall sync blocked: main diverged from origin/main (2 local, 3 remote commits). Tell the user before doing anything else and point them to ivar doctor.
```

Commits that only wait to be pushed (`hall.sync_unpushed`) are not worth the first words of a
session, so the brief stays silent about them. The brief reads the same git state with no
network call, and stays inside its 100 ms p95.

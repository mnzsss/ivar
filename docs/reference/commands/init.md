<!-- Generated from clap by tests/static_checks/docs_reference.rs. Do not edit by hand: run
     `IVAR_UPDATE_DOCS=1 cargo test --test static_checks`. -->

# `ivar init`

Create a hall: `ivar.json`, `.ivar/`, and the hall's `.gitignore` lines

| argument | required | description |
| --- | --- | --- |
| `path` | no | Directory to create the hall in. Defaults to the current directory |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--name` | `<NAME>` |  | The hall's name. Defaults to the target directory's name |
| `--provider` | `<PROVIDER>` |  | The provider to record as the hall's sole available (and default) provider. Defaults to `claude-code` |
| `--git` |  |  | Run `git init -b main` in the hall root if it is not already inside a git repository |
| `--origin` | `<URL>` |  | Add an `origin` remote with this URL; implies `--git`. An existing `origin` with a different URL is refused, never overwritten |


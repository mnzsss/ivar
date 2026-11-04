<!-- Generated from clap by tests/static_checks/docs_reference.rs. Do not edit by hand: run
     `IVAR_UPDATE_DOCS=1 cargo test --test static_checks`. -->

# `ivar sync`

Bring the local hall in line with `ivar.json`: clone missing repos, materialise harness config, run setup scripts

| flag | value | default | description |
| --- | --- | --- | --- |
| `--force-setup` |  |  | Run every repo's setup script even if it has already run for this version of the script. For when a script's effect was undone outside `ivar` — a deleted `node_modules`, a dropped database |


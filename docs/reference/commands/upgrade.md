<!-- Generated from clap by tests/static_checks/docs_reference.rs. Do not edit by hand: run
     `IVAR_UPDATE_DOCS=1 cargo test --test static_checks`. -->

# `ivar upgrade`

Upgrade ivar through the channel that installed it. Runs the installer again for an installer install and `cargo install ivar --locked` for a cargo install, printing the command first. A package-manager or unrecognised install gets the command to run instead. Nothing runs when this is already the latest release.

| flag | value | default | description |
| --- | --- | --- | --- |
| `--check` |  |  | Only report the current and latest version; install nothing |


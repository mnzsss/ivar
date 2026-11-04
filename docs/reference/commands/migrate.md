<!-- Generated from clap by tests/static_checks/docs_reference.rs. Do not edit by hand: run
     `IVAR_UPDATE_DOCS=1 cargo test --test static_checks`. -->

# `ivar migrate`

Advance `ivar.json`'s schema version (interactive; shows the change, then asks). Only ever needed after upgrading `ivar` to a build whose format is newer than the one your hall was written with. Local state migrates itself; `ivar.json` is committed, so advancing it is a decision you make and then commit.


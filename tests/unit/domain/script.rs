#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

const SH: &str = "#!/usr/bin/env bash\n# description: Deploy to staging.\n# usage: deploy.sh <build>\n# env: STAGING_TOKEN, STAGING_URL\nset -e\n";

#[test]
fn a_shell_script_with_a_full_header_parses() {
    let script = parse("deploy-staging.sh", SH).unwrap();
    assert_eq!(script.name, "deploy-staging");
    assert_eq!(script.kind, ScriptKind::Shell);
    assert_eq!(script.description, "Deploy to staging.");
    assert_eq!(script.usage, "deploy.sh <build>");
    assert_eq!(script.env, ["STAGING_TOKEN", "STAGING_URL"]);
}

#[test]
fn env_is_optional() {
    let script = parse(
        "a.sh",
        "#!/bin/sh\n# description: d\n# usage: a.sh\necho hi\n",
    )
    .unwrap();
    assert!(script.env.is_empty());
}

#[test]
fn a_python_script_with_a_pep_723_block_parses() {
    let source = "#!/usr/bin/env -S uv run --script\n\
# /// script\n\
# requires-python = \">=3.12\"\n\
# dependencies = [\"requests\"]\n\
# ///\n\
# description: Fetch the report.\n\
# usage: fetch-report.py <date>\n\
import requests\n";
    let script = parse("fetch-report.py", source).unwrap();
    assert_eq!(script.kind, ScriptKind::Python);
    assert_eq!(script.description, "Fetch the report.");
    assert_eq!(script.usage, "fetch-report.py <date>");
}

#[test]
fn fields_inside_the_pep_723_block_are_not_header_fields() {
    let source = "# /// script\n# description: sneaky\n# ///\n# usage: u\n";
    assert_eq!(
        parse("a.py", source).unwrap_err(),
        ScriptProblem::MissingDescription
    );
}

#[test]
fn the_shebang_is_optional() {
    assert!(parse("a.sh", "# description: d\n# usage: u\n").is_ok());
}

#[test]
fn a_missing_description_or_usage_is_rejected() {
    assert_eq!(
        parse("a.sh", "#!/bin/bash\n# usage: u\n").unwrap_err(),
        ScriptProblem::MissingDescription
    );
    assert_eq!(
        parse("a.sh", "#!/bin/bash\n# description: d\n").unwrap_err(),
        ScriptProblem::MissingUsage
    );
    assert_eq!(
        parse("a.sh", "#!/bin/bash\n# description:\n# usage: u\n").unwrap_err(),
        ScriptProblem::MissingDescription
    );
    assert_eq!(
        parse("a.sh", "echo no header\n").unwrap_err(),
        ScriptProblem::MissingDescription
    );
}

#[test]
fn the_header_ends_at_the_first_line_that_is_not_a_comment() {
    let source = "#!/bin/bash\n# description: d\n\n# usage: u\n";
    assert_eq!(
        parse("a.sh", source).unwrap_err(),
        ScriptProblem::MissingUsage
    );
}

#[test]
fn a_repeated_field_is_rejected() {
    let source = "# description: a\n# description: b\n# usage: u\n";
    assert_eq!(
        parse("a.sh", source).unwrap_err(),
        ScriptProblem::DuplicateField("description")
    );
}

#[test]
fn a_bad_env_name_is_rejected() {
    let source = "# description: d\n# usage: u\n# env: GOOD, 1BAD\n";
    assert_eq!(
        parse("a.sh", source).unwrap_err(),
        ScriptProblem::BadEnvName("1BAD".to_owned())
    );
}

#[test]
fn only_sh_and_py_are_supported() {
    for name in ["a.rb", "a", "README.md", "a.sh.bak", "a.SH"] {
        assert_eq!(
            classify(name).unwrap_err(),
            ScriptProblem::UnsupportedExtension,
            "{name}"
        );
    }
}

#[test]
fn the_stem_must_be_kebab_case() {
    for name in [
        "Deploy.sh",
        "deploy_staging.sh",
        "-a.sh",
        "a-.sh",
        "a--b.sh",
        ".sh",
        "a.b.sh",
    ] {
        assert!(
            matches!(classify(name), Err(ScriptProblem::BadName(_))),
            "{name}"
        );
    }
    assert_eq!(classify("a-1-b2.py").unwrap().0, "a-1-b2");
}

#[test]
fn every_problem_has_its_own_code() {
    let codes = [
        ScriptProblem::UnsupportedExtension.code(),
        ScriptProblem::BadName(String::new()).code(),
        ScriptProblem::MissingDescription.code(),
        ScriptProblem::MissingUsage.code(),
        ScriptProblem::DuplicateField("env").code(),
        ScriptProblem::BadEnvName(String::new()).code(),
        ScriptProblem::Unreadable.code(),
    ];
    let unique: std::collections::BTreeSet<_> = codes.iter().collect();
    assert_eq!(unique.len(), codes.len());
}

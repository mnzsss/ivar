use super::*;

#[test]
fn lowercases_and_strips_accents() {
    assert_eq!(normalize("Código ÁGIL Naïve São"), "codigo agil naive sao");
    assert_eq!(normalize("Cre\u{0301}dito"), "credito");
}

#[test]
fn file_paths_become_a_path_placeholder() {
    assert_eq!(
        normalize("fix /home/dev/app/src/main.rs now"),
        "fix <path> now"
    );
    assert_eq!(normalize("edit src/domain/mod.rs"), "edit <path>");
    assert_eq!(normalize("open ~/notes/todo.md"), "open <path>");
    assert_eq!(normalize("run ./scripts/build.sh"), "run <path>");
    assert_eq!(normalize("see (src/lib.rs)"), "see (<path>)");
}

#[test]
fn the_sanitizers_outside_hall_mark_is_a_path() {
    assert_eq!(normalize("fix <outside-hall> now"), "fix <path> now");
}

#[test]
fn a_url_or_a_plain_word_pair_is_not_a_path() {
    assert_eq!(
        normalize("read https://example.com/docs"),
        "read https://example.com/docs"
    );
    assert_eq!(normalize("tests and/or docs"), "tests and/or docs");
}

#[test]
fn uuids_issue_keys_and_shas_become_an_id_placeholder() {
    assert_eq!(
        normalize("trace 123e4567-e89b-12d3-a456-426614174000 failed"),
        "trace <id> failed"
    );
    assert_eq!(normalize("close MNZS-458 and abc-9"), "close <id> <id>");
    assert_eq!(normalize("revert a798422"), "revert <id>");
    assert_eq!(
        normalize("revert 5798422bd9f1c2a3e4d5f60718293a4b5c6d7e8f"),
        "revert <id>"
    );
}

#[test]
fn a_hex_word_or_a_short_hex_run_is_not_an_id() {
    assert_eq!(normalize("a decade ago"), "decade ago");
    assert_eq!(normalize("code abc123"), "code abc123");
    // Only digits is a number, and 6 hex characters is below a short SHA.
    assert_eq!(normalize("ticket 1234567"), "ticket <n>");
}

#[test]
fn numbers_become_a_number_placeholder() {
    assert_eq!(
        normalize("run 3 times, 4.5 and 1,000"),
        "run <n> times, <n> <n>"
    );
    assert_eq!(normalize("last 12"), "last <n>");
    assert_eq!(normalize("line 42."), "line <n>.");
    assert_eq!(normalize("1 2 3"), "<n> <n> <n>");
}

#[test]
fn digits_inside_a_word_are_left_alone() {
    assert_eq!(normalize("use v2 of the 3d api"), "use v2 3d api");
}

#[test]
fn a_high_entropy_token_becomes_a_token_placeholder() {
    let mixed = "k3j9x0q2w8e7r1t5y6u4i";
    let hex = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";
    let base64 = "QWxhZGRpbjpvcGVuIHNlc2FtZQ==";
    assert_eq!(normalize(&format!("key {mixed} end")), "key <token> end");
    assert_eq!(normalize(&format!("sha {hex} end")), "sha <token> end");
    assert_eq!(normalize(&format!("b64 {base64} end")), "b64 <token> end");
}

#[test]
fn a_long_ordinary_word_or_identifier_is_not_a_token() {
    assert_eq!(
        normalize("internationalization of the app"),
        "internationalization app"
    );
    assert_eq!(
        normalize("rename the very-long-kebab-case-feature-name"),
        "rename very-long-kebab-case-feature-name"
    );
}

#[test]
fn stopwords_are_removed_in_portuguese_and_english() {
    assert_eq!(
        normalize("Please fix the login for the user"),
        "fix login user"
    );
    assert_eq!(
        normalize("por favor, corrija o login do usuário"),
        "corrija login usuario"
    );
}

#[test]
fn whitespace_is_collapsed() {
    assert_eq!(normalize("  fix \n\n\t login   now  "), "fix login now");
    assert_eq!(normalize(""), "");
    assert_eq!(normalize("please the"), "");
}

#[test]
fn the_steps_run_in_order() {
    // Paths go before ids and numbers, so the digits inside one never leak.
    assert_eq!(
        normalize("Corrija /srv/app-2/v3/file.rs, MNZS-458 e 7 testes"),
        "corrija <path>, <id> <n> testes"
    );
}

#[test]
fn a_redaction_mark_survives_and_no_secret_does() {
    let secrets = [
        "k3j9x0q2w8e7r1t5y6u4i",
        "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef0",
        "QWxhZGRpbjpvcGVuIHNlc2FtZQ==",
        "5798422bd9f1c2a3e4d5f60718293a4b5c6d7e8f",
    ];
    let body = format!(
        "use [REDACTED:api_key] and {} then {} / {} / {}",
        secrets[0], secrets[1], secrets[2], secrets[3]
    );

    let out = normalize(&body);

    assert!(out.contains("[redacted:api_key]"), "{out}");
    for secret in secrets {
        assert!(!out.contains(secret), "{secret} survived in {out}");
    }
    assert!(out.len() <= body.len());
}

#[test]
fn the_output_is_idempotent() {
    let once = normalize("Fix /a/b/c.rs for MNZS-1 with 42 retries, token k3j9x0q2w8e7r1t5y6u4i");
    assert_eq!(normalize(&once), once);
}

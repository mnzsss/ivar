//! Authentication adapter for OMP (Oh My Pi).
//!
//! Installs MCP credentials through `omp auth-broker import` into the active
//! OMP profile's credential vault.

use camino::Utf8Path;
use serde::{Deserialize, Serialize};

use crate::error::{Failure, FixAction};
use crate::infra::fs::{TempDir, write_sensitive_atomic};
use crate::infra::oauth::Tokens;
use crate::infra::proc::{self, Command};
use crate::providers::{Credential, Provider, launch_contract};

pub(crate) fn credential_binding(server_url: &str) -> String {
    let omp_profile = std::env::var("OMP_PROFILE").ok();
    let pi_profile = std::env::var("PI_PROFILE").ok();
    credential_binding_from(omp_profile.as_deref(), pi_profile.as_deref(), server_url)
}

pub(crate) fn credential_binding_from(
    omp_profile: Option<&str>,
    pi_profile: Option<&str>,
    server_url: &str,
) -> String {
    let profile = omp_profile
        .filter(|p| !p.is_empty())
        .or_else(|| pi_profile.filter(|p| !p.is_empty()))
        .unwrap_or("default");
    format!("mcp_oauth:profile:{profile}:{server_url}")
}

#[derive(Serialize)]
struct CredentialJson<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    access_token: &'a str,
    refresh_token: &'a str,
    expired: String,
}

pub(crate) fn credential_json(tokens: &Tokens) -> Result<String, Failure> {
    let Some(refresh_token) = &tokens.refresh_token else {
        return Err(Failure::blocked(
            "omp_auth.missing_refresh_token",
            "OMP credential store requires a refresh token",
        )
        .fix(FixAction::safe(
            "mcp.reauth",
            "Run `ivar mcp auth` to obtain a fresh token set with a refresh token.",
        )));
    };

    let Some(expires_at) = tokens.expires_at else {
        return Err(Failure::blocked(
            "omp_auth.missing_expiry",
            "OMP credential store requires token expiration timestamp",
        )
        .fix(FixAction::safe(
            "mcp.reauth",
            "Run `ivar mcp auth` to obtain a fresh token set with an expiration timestamp.",
        )));
    };

    let expired = rfc3339_from_epoch_secs(expires_at);

    let cred = CredentialJson {
        kind: "oauth",
        access_token: &tokens.access_token,
        refresh_token,
        expired,
    };

    serde_json::to_string(&cred).map_err(|e| {
        Failure::failed(
            "omp_auth.serialize_failed",
            format!("failed to serialize credential payload: {e}"),
        )
    })
}

fn rfc3339_from_epoch_secs(secs_f64: f64) -> String {
    if secs_f64.is_nan() || secs_f64.is_infinite() || secs_f64 < 0.0 {
        return "1970-01-01T00:00:00.000000000Z".to_owned();
    }
    let secs = secs_f64.floor() as i64;
    let subsec_nanos = ((secs_f64 - secs_f64.floor()) * 1_000_000_000.0).round() as u32;

    let days = secs.div_euclid(86_400);
    let seconds_of_day = secs.rem_euclid(86_400);
    let (hours, minutes, seconds) = (
        seconds_of_day / 3600,
        (seconds_of_day % 3600) / 60,
        seconds_of_day % 60,
    );
    let (year, month, day) = civil_from_days(days);

    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}.{subsec_nanos:09}Z")
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    (
        if month <= 2 { year + 1 } else { year },
        month as u32,
        day as u32,
    )
}

pub(crate) fn logout_command(binding: &str) -> Command {
    let binary = launch_contract(Provider::Omp).binary;
    Command::new(binary)
        .arg("auth-broker")
        .arg("logout")
        .arg("--provider")
        .arg(binding)
        .arg("--json")
}

pub(crate) fn import_command(path: &Utf8Path, binding: &str) -> Command {
    let binary = launch_contract(Provider::Omp).binary;
    Command::new(binary)
        .arg("auth-broker")
        .arg("import")
        .arg(path.as_str())
        .arg("--provider")
        .arg(binding)
        .arg("--json")
}

#[derive(Deserialize)]
struct ImportOutput {
    #[serde(default)]
    imported: Vec<ImportedItem>,
    #[serde(default)]
    skipped: Vec<SkippedItem>,
}

#[derive(Deserialize)]
struct ImportedItem {
    provider: String,
}

#[derive(Deserialize)]
struct SkippedItem {
    provider: String,
    #[serde(default)]
    reason: Option<String>,
}

pub(crate) fn parse_import_result(stdout: &str, binding: &str) -> Result<(), Failure> {
    let parsed: ImportOutput = serde_json::from_str(stdout).map_err(|e| {
        Failure::failed(
            "omp_auth.invalid_json",
            format!("failed to parse `omp auth-broker import --json` output: {e}"),
        )
        .fix(FixAction::safe(
            "mcp.retry_auth",
            "Verify OMP installation and retry `ivar mcp auth`.",
        ))
    })?;

    let is_imported = parsed.imported.iter().any(|item| item.provider == binding);
    if is_imported {
        return Ok(());
    }

    let skip_reason = parsed
        .skipped
        .iter()
        .find(|item| item.provider == binding)
        .and_then(|item| item.reason.as_deref())
        .unwrap_or("binding not in imported list");

    Err(Failure::failed(
        "omp_auth.import_failed",
        format!("omp auth-broker import failed for binding `{binding}`: {skip_reason}"),
    )
    .expected(format!("binding `{binding}` present in `imported[]`"))
    .actual(format!("skip reason: {skip_reason}"))
    .fix(FixAction::safe(
        "mcp.retry_auth",
        "Run `ivar mcp auth` again to re-authenticate with OMP.",
    )))
}

pub(crate) fn install_credentials(_name: &str, credential: &Credential<'_>) -> Result<(), Failure> {
    let binding = credential_binding(credential.server_url);
    let payload = credential_json(credential.tokens)?;

    let temp_dir = TempDir::new()?;
    let cred_path = temp_dir.path().join("credential.json");
    write_sensitive_atomic(&cred_path, payload.as_bytes())?;

    // 1. Logout existing binding (idempotent reset)
    let logout_cmd = logout_command(&binding);
    let _logout_output = proc::capture(&logout_cmd)?;

    // 2. Import new credential
    let import_cmd = import_command(&cred_path, &binding);
    let import_output = proc::capture(&import_cmd)?;

    if import_output.code != Some(0) {
        return Err(Failure::failed(
            "omp_auth.import_command_failed",
            format!(
                "omp auth-broker import exited with code {:?}: {}",
                import_output.code,
                import_output.diagnostic()
            ),
        )
        .fix(FixAction::safe(
            "mcp.retry_auth",
            "Check OMP setup and run `ivar mcp auth` again.",
        )));
    }

    parse_import_result(&import_output.stdout, &binding)
}

pub(crate) fn verify_authenticated(server_url: &str) -> Result<(), Failure> {
    let binding = credential_binding(server_url);
    let binary = launch_contract(Provider::Omp).binary;
    let cmd = Command::new(binary).arg("token").arg(&binding);

    let output = proc::capture(&cmd)?;
    if output.code == Some(0) && !output.stdout.trim().is_empty() {
        return Ok(());
    }

    Err(Failure::failed(
        "mcp.auth_not_verified",
        format!("`omp token {binding}` failed to verify stored token"),
    )
    .expected(format!("an access token for binding `{binding}`"))
    .actual(format!("exit {:?}: {}", output.code, output.diagnostic()))
    .fix(FixAction::safe(
        "mcp.retry_auth",
        "Run `ivar mcp auth` to authenticate.",
    )))
}

#[cfg(test)]
#[path = "../../../tests/unit/providers/omp/auth.rs"]
mod tests;

use serde::Serialize;
use std::collections::BTreeMap;
use std::io;

use super::{resolve_provider, resolve_server};
use crate::action::{Ctx, discover_hall, read_manifest};
use crate::domain::mcp::{AuthRequirement, CredentialState, McpServerDef};
use crate::domain::provider::Provider;
use crate::error::{Outcome, Report, Warning, WriteHuman};
use crate::providers;

#[derive(Debug, Clone)]
pub struct StatusInput {
    pub server: Option<String>,
    pub provider: Option<String>,
    pub live: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StateSource {
    Local,
    Live,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusRow {
    pub server: String,
    pub materialised_name: String,
    pub provider: Provider,
    pub state: CredentialState,
    pub source: StateSource,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusOutcome {
    pub rows: Vec<StatusRow>,
}

impl WriteHuman for StatusOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        if self.rows.is_empty() {
            writeln!(w, "No MCP servers configured.")?;
            return Ok(());
        }

        let mut table = crate::infra::table::new(&["SERVER", "PROVIDER", "STATE (SOURCE)"]);
        for row in &self.rows {
            let state_desc = match row.source {
                StateSource::Local => row.state.as_str().to_owned(),
                StateSource::Live => format!("{} (live)", row.state.as_str()),
            };
            table.add_row(vec![row.server.as_str(), row.provider.id(), &state_desc]);
        }
        crate::infra::table::write(w, &table)
    }
}

pub(crate) fn local_rows(
    hall: &str,
    servers: &[&McpServerDef],
    providers: &[Provider],
    state_of: &dyn Fn(Provider, &str, &str) -> CredentialState,
) -> Vec<StatusRow> {
    let mut rows = Vec::new();
    for server in servers {
        let materialised = format!("{hall}-{}", server.name);
        let req = server.auth_requirement();
        for &provider in providers {
            let state = match req {
                AuthRequirement::NotApplicable => CredentialState::NotApplicable,
                AuthRequirement::NotRequired => CredentialState::NotRequired,
                AuthRequirement::Required => {
                    let url = server.url.as_deref().unwrap_or("");
                    state_of(provider, &materialised, url)
                }
            };
            rows.push(StatusRow {
                server: server.name.clone(),
                materialised_name: materialised.clone(),
                provider,
                state,
                source: StateSource::Local,
            });
        }
    }
    rows
}

pub(crate) fn apply_live(
    rows: &mut [StatusRow],
    live: &BTreeMap<Provider, BTreeMap<String, CredentialState>>,
) {
    for row in rows.iter_mut() {
        if matches!(
            row.state,
            CredentialState::NotApplicable | CredentialState::NotRequired
        ) {
            continue;
        }
        if let Some(provider_entries) = live.get(&row.provider) {
            row.source = StateSource::Live;
            row.state = provider_entries
                .get(&row.materialised_name)
                .copied()
                .unwrap_or(CredentialState::Unknown);
        }
    }
}

pub(crate) fn status_report(rows: Vec<StatusRow>) -> Report<StatusOutcome> {
    let mut warnings = Vec::new();
    for row in &rows {
        if row.state.needs_attention() {
            warnings.push(Warning::new(
                "mcp.auth_needs_attention",
                format!("{}/{}", row.server, row.provider.id()),
                format!(
                    "`{}` is {} for {}; run `ivar mcp auth {} --provider {}`",
                    row.server,
                    row.state.as_str(),
                    row.provider.id(),
                    row.server,
                    row.provider.id()
                ),
            ));
        }
    }
    Report::with_warnings(StatusOutcome { rows }, warnings)
}

pub fn status(ctx: &Ctx, input: &StatusInput) -> Outcome<StatusOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;

    let servers: Vec<&McpServerDef> = if let Some(server_name) = &input.server {
        vec![resolve_server(&manifest, server_name)?]
    } else {
        manifest.mcp_servers().iter().collect()
    };

    let providers: Vec<Provider> = if let Some(provider_raw) = &input.provider {
        vec![resolve_provider(&manifest, Some(provider_raw))?]
    } else {
        manifest.providers().available().to_vec()
    };
    let mut rows = local_rows(
        manifest.name().as_str(),
        &servers,
        &providers,
        &providers::credential_state,
    );

    if input.live {
        let mut live_map = BTreeMap::new();
        let needed_providers: std::collections::BTreeSet<Provider> = rows
            .iter()
            .filter(|r| {
                !matches!(
                    r.state,
                    CredentialState::NotApplicable | CredentialState::NotRequired
                )
            })
            .map(|r| r.provider)
            .collect();

        for provider in needed_providers {
            if let Some(result) = providers::live_states(provider, layout.root()) {
                match result {
                    Ok(map) => {
                        live_map.insert(provider, map);
                    }
                    Err(_) => {
                        // Probe failure is not fatal: map fails closed per row
                        let mut map = BTreeMap::new();
                        for r in rows.iter().filter(|r| r.provider == provider) {
                            map.insert(r.materialised_name.clone(), CredentialState::Unknown);
                        }
                        live_map.insert(provider, map);
                    }
                }
            }
        }
        apply_live(&mut rows, &live_map);
    }

    Ok(status_report(rows))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/mcp/status.rs"]
mod tests;

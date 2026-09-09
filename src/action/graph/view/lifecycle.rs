use std::io;
use std::process::ExitCode;

use crate::action::feature::workspace::OpenAttempt;
use crate::action::graph::input::GraphViewInput;
use crate::action::graph::outcome::GraphViewOutcome;
use crate::action::graph::view::error::ViewError;
use crate::action::graph::view::server::ViewerServer;
use crate::error::WriteHuman;
use crate::infra::proc;
use crate::store::graph::db::GraphDb;

#[derive(Debug)]
pub struct ViewSession {
    pub server: ViewerServer,
    pub outcome: GraphViewOutcome,
    pub no_open: bool,
}

impl ViewSession {
    pub fn url(&self) -> &str {
        &self.outcome.url
    }

    pub fn outcome(&self) -> &GraphViewOutcome {
        &self.outcome
    }

    pub fn serve(self) -> Result<(), ViewError> {
        self.server.serve()
    }
}

pub fn launch_browser(url: &str) -> OpenAttempt {
    #[cfg(target_os = "macos")]
    let command = proc::Command::new("open").arg(url);

    #[cfg(target_os = "windows")]
    let command = proc::Command::new("cmd").args(["/c", "start", "", url]);

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let command = proc::Command::new("xdg-open").arg(url);

    match proc::detach(&command) {
        Ok(()) => OpenAttempt::Opened,
        Err(e) => OpenAttempt::Failed {
            reason: e.to_string(),
        },
    }
}

pub fn prepare_view_session(db: GraphDb, input: GraphViewInput) -> Result<ViewSession, ViewError> {
    let bind_addr = match input.port {
        Some(port) => format!("127.0.0.1:{port}"),
        None => "127.0.0.1:0".to_owned(),
    };
    let server = ViewerServer::bind_loopback(db, input.seed.clone(), &bind_addr)?;
    let url = server.url();

    let open = if input.no_open {
        OpenAttempt::NotRequested
    } else {
        launch_browser(&url)
    };

    let outcome = GraphViewOutcome {
        url,
        seed: input.seed,
        open,
    };

    Ok(ViewSession {
        server,
        outcome,
        no_open: input.no_open,
    })
}

pub fn execute_view_session(
    session: ViewSession,
    json: bool,
    compact: bool,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
) -> ExitCode {
    if json {
        let _ = serde_json::to_writer_pretty(&mut *stdout, &session.outcome);
        let _ = writeln!(stdout);
    } else if compact {
        let _ = writeln!(
            stdout,
            "{}",
            crate::action::graph::compact::ToCompact::to_compact(&session.outcome)
        );
    } else {
        let _ = session.outcome.write_human(stdout);
    }
    let _ = stdout.flush();

    match session.serve() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            let _ = writeln!(stderr, "ivar: graph view: {err}");
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
#[path = "../../../../tests/unit/action/graph/view/lifecycle.rs"]
mod tests;

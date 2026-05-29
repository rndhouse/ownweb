mod annotations;
mod client;
mod content;
mod feedback;
mod output;
mod rules;

use annotations::{run_annotations, AnnotationsCommand};
use clap::{Parser, Subcommand};
use client::{normalize_origin, DaemonClient};
use content::{run_content, ContentCommand};
use feedback::{run_feedback, FeedbackCommand};
use rules::{run_rules, RulesCommand};
use std::{error::Error, fmt};

const DEFAULT_DAEMON_ORIGIN: &str = "http://127.0.0.1:17891";
const DAEMON_ORIGIN_ENV: &str = "WEBLAYER_DAEMON_ORIGIN";
const DEFAULT_SITE: &str = "x.com";

pub type CliResult<T> = Result<T, CliError>;

/// Parsed top-level WebLayer command.
#[derive(Debug, Parser)]
#[command(
    name = "weblayer",
    version,
    about = "Run the WebLayer daemon or talk to a running local daemon"
)]
pub struct Cli {
    /// Run the long-lived local daemon.
    #[arg(long, global = true)]
    pub daemon: bool,

    /// Daemon HTTP origin for client commands.
    #[arg(long, global = true, value_name = "ORIGIN")]
    daemon_origin: Option<String>,

    #[command(subcommand)]
    command: Option<Command>,
}

impl Cli {
    /// Parses command-line arguments.
    pub fn from_args() -> Self {
        Self::parse()
    }

    /// Returns true when this invocation should run the daemon.
    pub fn runs_daemon(&self) -> CliResult<bool> {
        if self.daemon && self.command.is_some() {
            return Err(CliError::message(
                "`--daemon` cannot be combined with another subcommand",
            ));
        }

        Ok(self.daemon || matches!(self.command, Some(Command::Daemon)))
    }

    fn client_command(&self) -> ClientCommand {
        match self.command.clone() {
            Some(Command::Status) | Some(Command::Daemon) | None => ClientCommand::Status,
            Some(Command::Rules(command)) => ClientCommand::Rules(command),
            Some(Command::Content(command)) => ClientCommand::Content(command),
            Some(Command::Feedback(command)) | Some(Command::Dislikes(command)) => {
                ClientCommand::Feedback(command)
            }
            Some(Command::Annotations(command)) => ClientCommand::Annotations(command),
        }
    }

    fn daemon_origin(&self) -> CliResult<String> {
        normalize_origin(
            self.daemon_origin
                .clone()
                .or_else(|| std::env::var(DAEMON_ORIGIN_ENV).ok())
                .unwrap_or_else(|| DEFAULT_DAEMON_ORIGIN.into()),
        )
    }
}

/// Runs the requested client command against the configured daemon origin.
pub async fn run_client(cli: &Cli) -> CliResult<()> {
    let origin = cli.daemon_origin()?;
    let client = DaemonClient::new(origin);

    match cli.client_command() {
        ClientCommand::Status => run_status(&client).await,
        ClientCommand::Rules(command) => run_rules(&client, command).await,
        ClientCommand::Content(command) => run_content(&client, command).await,
        ClientCommand::Feedback(command) => run_feedback(&client, command).await,
        ClientCommand::Annotations(command) => run_annotations(&client, command).await,
    }
}

#[derive(Debug, Clone, Subcommand)]
enum Command {
    /// Run the long-lived local daemon.
    Daemon,
    /// Check whether the local daemon is reachable.
    Status,
    /// Inspect content rules.
    Rules(RulesCommand),
    /// List, search, or summarize stored content.
    Content(ContentCommand),
    /// List stored feedback.
    Feedback(FeedbackCommand),
    #[command(hide = true)]
    Dislikes(FeedbackCommand),
    /// List or upsert content annotations.
    Annotations(AnnotationsCommand),
}

#[derive(Debug, Clone)]
enum ClientCommand {
    Status,
    Rules(RulesCommand),
    Content(ContentCommand),
    Feedback(FeedbackCommand),
    Annotations(AnnotationsCommand),
}

#[derive(Debug)]
pub struct CliError {
    message: String,
}

impl CliError {
    fn message(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for CliError {}

impl From<reqwest::Error> for CliError {
    fn from(error: reqwest::Error) -> Self {
        Self::message(error.to_string())
    }
}

impl From<serde_json::Error> for CliError {
    fn from(error: serde_json::Error) -> Self {
        Self::message(error.to_string())
    }
}

async fn run_status(client: &DaemonClient) -> CliResult<()> {
    match client.get_json("/health", &[]).await {
        Ok(value) => {
            let service = output::value_str(&value, "service").unwrap_or("weblayer-daemon");
            println!("{service} is reachable at {}", client.origin());
            println!("Run `weblayer --help` to see client commands.");
            Ok(())
        }
        Err(error) => Err(CliError::message(format!(
            "daemon is not reachable at {}: {error}\nRun `weblayer daemon` to start it.",
            client.origin()
        ))),
    }
}

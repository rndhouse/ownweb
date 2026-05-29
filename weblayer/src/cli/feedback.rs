use super::{
    client::DaemonClient,
    output::{print_feedback, print_json},
    CliResult, DEFAULT_SITE,
};
use clap::{Args, Subcommand};

#[derive(Debug, Clone, Args)]
pub(super) struct FeedbackCommand {
    #[command(subcommand)]
    command: Option<FeedbackSubcommand>,
}

#[derive(Debug, Clone, Subcommand)]
enum FeedbackSubcommand {
    /// List stored feedback.
    List(FeedbackListArgs),
}

#[derive(Debug, Clone, Args)]
struct FeedbackListArgs {
    /// Site scope to inspect.
    #[arg(long, default_value = DEFAULT_SITE)]
    site: String,
    /// Filter by current active state.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    active: bool,
    /// Maximum rows to return.
    #[arg(long, default_value_t = 20)]
    limit: usize,
    /// Number of matching rows to skip.
    #[arg(long, default_value_t = 0)]
    offset: usize,
    /// Print the raw daemon JSON response.
    #[arg(long)]
    json: bool,
}

pub(super) async fn run_feedback(client: &DaemonClient, command: FeedbackCommand) -> CliResult<()> {
    let args = match command.command {
        Some(FeedbackSubcommand::List(args)) => args,
        None => FeedbackListArgs {
            site: DEFAULT_SITE.into(),
            active: true,
            limit: 20,
            offset: 0,
            json: false,
        },
    };
    let value = client
        .get_json(
            "/v1/feedback",
            &[
                ("site", args.site),
                ("active", args.active.to_string()),
                ("limit", args.limit.to_string()),
                ("offset", args.offset.to_string()),
            ],
        )
        .await?;

    if args.json {
        print_json(&value)?;
    } else {
        print_feedback(&value);
    }

    Ok(())
}

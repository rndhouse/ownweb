use super::{
    client::DaemonClient,
    output::{print_content, print_content_stats, print_json},
    CliResult, DEFAULT_SITE,
};
use clap::{Args, Subcommand};

#[derive(Debug, Clone, Args)]
pub(super) struct ContentCommand {
    #[command(subcommand)]
    command: ContentSubcommand,
}

#[derive(Debug, Clone, Subcommand)]
enum ContentSubcommand {
    /// List recent stored content.
    List(ContentListArgs),
    /// Search stored content.
    Search(ContentSearchArgs),
    /// Show stored content counts.
    Stats(ContentStatsArgs),
}

#[derive(Debug, Clone, Args)]
struct ContentListArgs {
    /// Site scope to inspect.
    #[arg(long, default_value = DEFAULT_SITE)]
    site: String,
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

#[derive(Debug, Clone, Args)]
struct ContentSearchArgs {
    /// Search query.
    q: String,
    /// Site scope to inspect.
    #[arg(long, default_value = DEFAULT_SITE)]
    site: String,
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

#[derive(Debug, Clone, Args)]
struct ContentStatsArgs {
    /// Site scope to inspect.
    #[arg(long, default_value = DEFAULT_SITE)]
    site: String,
    /// Print the raw daemon JSON response.
    #[arg(long)]
    json: bool,
}

pub(super) async fn run_content(client: &DaemonClient, command: ContentCommand) -> CliResult<()> {
    match command.command {
        ContentSubcommand::List(args) => {
            let value = client
                .get_json(
                    "/v1/content",
                    &[
                        ("site", args.site),
                        ("limit", args.limit.to_string()),
                        ("offset", args.offset.to_string()),
                    ],
                )
                .await?;
            if args.json {
                print_json(&value)?;
            } else {
                print_content(&value);
            }
        }
        ContentSubcommand::Search(args) => {
            let value = client
                .get_json(
                    "/v1/content",
                    &[
                        ("site", args.site),
                        ("q", args.q),
                        ("limit", args.limit.to_string()),
                        ("offset", args.offset.to_string()),
                    ],
                )
                .await?;
            if args.json {
                print_json(&value)?;
            } else {
                print_content(&value);
            }
        }
        ContentSubcommand::Stats(args) => {
            let value = client
                .get_json("/v1/content/stats", &[("site", args.site)])
                .await?;
            if args.json {
                print_json(&value)?;
            } else {
                print_content_stats(&value);
            }
        }
    }

    Ok(())
}

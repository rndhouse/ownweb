use super::{
    client::{push_optional_query, DaemonClient},
    output::{
        print_json, print_rule_detail, print_rule_saved, print_rule_suggestions,
        print_rule_validation, print_rules,
    },
    CliResult, DEFAULT_SITE,
};
use clap::{Args, Subcommand};
use serde_json::{json, Value};

#[derive(Debug, Clone, Args)]
pub(super) struct RulesCommand {
    #[command(subcommand)]
    command: Option<RulesSubcommand>,
}

#[derive(Debug, Clone, Subcommand)]
enum RulesSubcommand {
    /// List rules for a site.
    List(RulesListArgs),
    /// Show one rule with audit history.
    Show(RulesShowArgs),
    /// Create a draft rule unless a different status is requested.
    Create(RulesCreateArgs),
    /// Update rule fields or examples.
    Update(RulesUpdateArgs),
    /// Activate a rule.
    Enable(RulesStatusArgs),
    /// Disable a rule without deleting it.
    Disable(RulesStatusArgs),
    /// Archive a rule so it no longer appears in normal active management.
    Archive(RulesStatusArgs),
    /// Change rule priority.
    Reorder(RulesReorderArgs),
    /// Test a rule against stored content.
    Validate(RulesValidateArgs),
    /// Suggest draft rules from active feedback.
    Suggest(RulesSuggestArgs),
}

#[derive(Debug, Clone, Args)]
struct RulesListArgs {
    /// Site scope to inspect.
    #[arg(long, default_value = DEFAULT_SITE)]
    site: String,
    /// Optional rule status filter.
    #[arg(long)]
    status: Option<String>,
    /// Maximum rows to return.
    #[arg(long, default_value_t = 100)]
    limit: usize,
    /// Number of matching rows to skip.
    #[arg(long, default_value_t = 0)]
    offset: usize,
    /// Print the raw daemon JSON response.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Clone, Args)]
struct RulesShowArgs {
    /// Stable rule ID.
    id: String,
    /// Site scope to inspect.
    #[arg(long, default_value = DEFAULT_SITE)]
    site: String,
    /// Print the raw daemon JSON response.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Clone, Args)]
struct RulesCreateArgs {
    /// Optional stable rule ID.
    #[arg(long)]
    id: Option<String>,
    /// Site scope to update.
    #[arg(long, default_value = DEFAULT_SITE)]
    site: String,
    /// Rule lifecycle status. Defaults to draft in the daemon.
    #[arg(long)]
    status: Option<String>,
    /// Lower numbers run earlier.
    #[arg(long)]
    priority: Option<i64>,
    /// Short human-readable rule title.
    #[arg(long)]
    title: String,
    /// Agent-facing instruction text.
    #[arg(long)]
    instruction: String,
    /// Source creating the rule.
    #[arg(long, default_value = "user")]
    source: String,
    /// Example that should match this rule. Can be repeated.
    #[arg(long = "positive-example")]
    positive_examples: Vec<String>,
    /// Example that should not match this rule. Can be repeated.
    #[arg(long = "negative-example")]
    negative_examples: Vec<String>,
    /// Print the raw daemon JSON response.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Clone, Args)]
struct RulesUpdateArgs {
    /// Stable rule ID.
    id: String,
    /// Site scope to update.
    #[arg(long, default_value = DEFAULT_SITE)]
    site: String,
    /// Replacement lifecycle status.
    #[arg(long)]
    status: Option<String>,
    /// Replacement priority.
    #[arg(long)]
    priority: Option<i64>,
    /// Replacement rule title.
    #[arg(long)]
    title: Option<String>,
    /// Replacement instruction text.
    #[arg(long)]
    instruction: Option<String>,
    /// Source updating the rule.
    #[arg(long, default_value = "user")]
    source: String,
    /// Replacement positive example. Can be repeated.
    #[arg(long = "positive-example")]
    positive_examples: Vec<String>,
    /// Clear all positive examples.
    #[arg(long)]
    clear_positive_examples: bool,
    /// Replacement negative example. Can be repeated.
    #[arg(long = "negative-example")]
    negative_examples: Vec<String>,
    /// Clear all negative examples.
    #[arg(long)]
    clear_negative_examples: bool,
    /// Print the raw daemon JSON response.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Clone, Args)]
struct RulesStatusArgs {
    /// Stable rule ID.
    id: String,
    /// Site scope to update.
    #[arg(long, default_value = DEFAULT_SITE)]
    site: String,
    /// Source changing the rule status.
    #[arg(long, default_value = "user")]
    source: String,
    /// Print the raw daemon JSON response.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Clone, Args)]
struct RulesReorderArgs {
    /// Stable rule ID.
    id: String,
    /// New priority. Lower numbers run earlier.
    priority: i64,
    /// Site scope to update.
    #[arg(long, default_value = DEFAULT_SITE)]
    site: String,
    /// Source changing the priority.
    #[arg(long, default_value = "user")]
    source: String,
    /// Print the raw daemon JSON response.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Clone, Args)]
struct RulesValidateArgs {
    /// Stable rule ID.
    id: String,
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
struct RulesSuggestArgs {
    /// Site scope to inspect.
    #[arg(long, default_value = DEFAULT_SITE)]
    site: String,
    /// Minimum active feedback examples required for a suggestion.
    #[arg(long, default_value_t = 1)]
    min_feedback: usize,
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

pub(super) async fn run_rules(client: &DaemonClient, command: RulesCommand) -> CliResult<()> {
    match command.command.unwrap_or_else(|| {
        RulesSubcommand::List(RulesListArgs {
            site: DEFAULT_SITE.into(),
            status: None,
            limit: 100,
            offset: 0,
            json: false,
        })
    }) {
        RulesSubcommand::List(args) => run_rules_list(client, args).await?,
        RulesSubcommand::Show(args) => run_rules_show(client, args).await?,
        RulesSubcommand::Create(args) => run_rules_create(client, args).await?,
        RulesSubcommand::Update(args) => run_rules_update(client, args).await?,
        RulesSubcommand::Enable(args) => {
            run_rule_status(client, args, "active", "enabled").await?;
        }
        RulesSubcommand::Disable(args) => {
            run_rule_status(client, args, "disabled", "disabled").await?;
        }
        RulesSubcommand::Archive(args) => {
            run_rule_status(client, args, "archived", "archived").await?;
        }
        RulesSubcommand::Reorder(args) => run_rules_reorder(client, args).await?,
        RulesSubcommand::Validate(args) => run_rules_validate(client, args).await?,
        RulesSubcommand::Suggest(args) => run_rules_suggest(client, args).await?,
    }

    Ok(())
}

async fn run_rules_list(client: &DaemonClient, args: RulesListArgs) -> CliResult<()> {
    let mut query = vec![
        ("site", args.site),
        ("limit", args.limit.to_string()),
        ("offset", args.offset.to_string()),
    ];
    push_optional_query(&mut query, "status", args.status);
    let value = client.get_json("/v1/rules", &query).await?;

    if args.json {
        print_json(&value)?;
    } else {
        print_rules(&value);
    }

    Ok(())
}

async fn run_rules_show(client: &DaemonClient, args: RulesShowArgs) -> CliResult<()> {
    let value = client
        .get_json(&format!("/v1/rules/{}", args.id), &[("site", args.site)])
        .await?;
    if args.json {
        print_json(&value)?;
    } else {
        print_rule_detail(&value);
    }

    Ok(())
}

async fn run_rules_create(client: &DaemonClient, args: RulesCreateArgs) -> CliResult<()> {
    let mut body = json!({
        "title": args.title,
        "instruction": args.instruction,
        "source": args.source,
        "examples": {
            "positive": args.positive_examples,
            "negative": args.negative_examples
        }
    });
    if let Some(id) = args.id {
        body["id"] = json!(id);
    }
    if let Some(status) = args.status {
        body["status"] = json!(status);
    }
    if let Some(priority) = args.priority {
        body["priority"] = json!(priority);
    }

    let value = client
        .post_json("/v1/rules", &[("site", args.site)], body)
        .await?;
    if args.json {
        print_json(&value)?;
    } else {
        print_rule_saved(&value, "created");
    }

    Ok(())
}

async fn run_rules_update(client: &DaemonClient, args: RulesUpdateArgs) -> CliResult<()> {
    let mut body = json!({ "source": args.source });
    if let Some(status) = args.status {
        body["status"] = json!(status);
    }
    if let Some(priority) = args.priority {
        body["priority"] = json!(priority);
    }
    if let Some(title) = args.title {
        body["title"] = json!(title);
    }
    if let Some(instruction) = args.instruction {
        body["instruction"] = json!(instruction);
    }

    let mut examples = serde_json::Map::new();
    if args.clear_positive_examples || !args.positive_examples.is_empty() {
        examples.insert("positive".into(), json!(args.positive_examples));
    }
    if args.clear_negative_examples || !args.negative_examples.is_empty() {
        examples.insert("negative".into(), json!(args.negative_examples));
    }
    if !examples.is_empty() {
        body["examples"] = Value::Object(examples);
    }

    let value = client
        .post_json(
            &format!("/v1/rules/{}", args.id),
            &[("site", args.site)],
            body,
        )
        .await?;
    if args.json {
        print_json(&value)?;
    } else {
        print_rule_saved(&value, "updated");
    }

    Ok(())
}

async fn run_rule_status(
    client: &DaemonClient,
    args: RulesStatusArgs,
    status: &str,
    label: &str,
) -> CliResult<()> {
    let body = json!({
        "status": status,
        "source": args.source
    });
    let value = client
        .post_json(
            &format!("/v1/rules/{}/status", args.id),
            &[("site", args.site)],
            body,
        )
        .await?;
    if args.json {
        print_json(&value)?;
    } else {
        print_rule_saved(&value, label);
    }

    Ok(())
}

async fn run_rules_reorder(client: &DaemonClient, args: RulesReorderArgs) -> CliResult<()> {
    let body = json!({
        "priority": args.priority,
        "source": args.source
    });
    let value = client
        .post_json(
            &format!("/v1/rules/{}", args.id),
            &[("site", args.site)],
            body,
        )
        .await?;
    if args.json {
        print_json(&value)?;
    } else {
        print_rule_saved(&value, "reordered");
    }

    Ok(())
}

async fn run_rules_validate(client: &DaemonClient, args: RulesValidateArgs) -> CliResult<()> {
    let value = client
        .get_json(
            &format!("/v1/rules/{}/validate", args.id),
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
        print_rule_validation(&value);
    }

    Ok(())
}

async fn run_rules_suggest(client: &DaemonClient, args: RulesSuggestArgs) -> CliResult<()> {
    let value = client
        .get_json(
            "/v1/rule-suggestions",
            &[
                ("site", args.site),
                ("minFeedback", args.min_feedback.to_string()),
                ("limit", args.limit.to_string()),
                ("offset", args.offset.to_string()),
            ],
        )
        .await?;
    if args.json {
        print_json(&value)?;
    } else {
        print_rule_suggestions(&value);
    }

    Ok(())
}

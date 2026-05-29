use clap::{Args, Parser, Subcommand};
use reqwest::{Client as HttpClient, Url};
use serde_json::{json, Value};
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

#[derive(Debug, Clone, Args)]
struct RulesCommand {
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

#[derive(Debug, Clone, Args)]
struct ContentCommand {
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

#[derive(Debug, Clone, Args)]
struct FeedbackCommand {
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

#[derive(Debug, Clone, Args)]
struct AnnotationsCommand {
    #[command(subcommand)]
    command: AnnotationsSubcommand,
}

#[derive(Debug, Clone, Subcommand)]
enum AnnotationsSubcommand {
    /// List stored content annotations.
    List(AnnotationsListArgs),
    /// Create or update one content annotation.
    Put(AnnotationsPutArgs),
}

#[derive(Debug, Clone, Args)]
struct AnnotationsListArgs {
    /// Site scope to inspect.
    #[arg(long, default_value = DEFAULT_SITE)]
    site: String,
    /// Optional stable storage key filter.
    #[arg(long)]
    storage_key: Option<String>,
    /// Optional site-native content ID filter.
    #[arg(long)]
    content_id: Option<String>,
    /// Optional logical content kind filter.
    #[arg(long)]
    content_kind: Option<String>,
    /// Optional annotation category filter.
    #[arg(long)]
    annotation_type: Option<String>,
    /// Optional annotation key filter.
    #[arg(long)]
    key: Option<String>,
    /// Optional source filter.
    #[arg(long)]
    source: Option<String>,
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
struct AnnotationsPutArgs {
    /// Site scope to update.
    #[arg(long, default_value = DEFAULT_SITE)]
    site: String,
    /// Stable storage key returned by content inspection endpoints.
    #[arg(long)]
    storage_key: String,
    /// Logical content kind.
    #[arg(long, default_value = "post")]
    content_kind: String,
    /// Annotation category, such as tag, note, or topic.
    #[arg(long)]
    annotation_type: String,
    /// Annotation key within its category.
    #[arg(long, default_value = "")]
    key: String,
    /// Annotation value as JSON.
    #[arg(long)]
    value: String,
    /// Source that created or updated this annotation.
    #[arg(long)]
    source: String,
    /// Optional model confidence from 0.0 to 1.0.
    #[arg(long)]
    confidence: Option<f64>,
    /// Print the raw daemon JSON response.
    #[arg(long)]
    json: bool,
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

struct DaemonClient {
    origin: String,
    http: HttpClient,
}

impl DaemonClient {
    fn new(origin: String) -> Self {
        Self {
            origin,
            http: HttpClient::new(),
        }
    }

    async fn get_json(&self, path: &str, query: &[(&str, String)]) -> CliResult<Value> {
        let mut url = self.endpoint(path)?;
        if !query.is_empty() {
            let mut pairs = url.query_pairs_mut();
            for (key, value) in query {
                pairs.append_pair(key, value);
            }
        }

        self.json_response(self.http.get(url).send().await?).await
    }

    async fn post_json(
        &self,
        path: &str,
        query: &[(&str, String)],
        body: Value,
    ) -> CliResult<Value> {
        let mut url = self.endpoint(path)?;
        if !query.is_empty() {
            let mut pairs = url.query_pairs_mut();
            for (key, value) in query {
                pairs.append_pair(key, value);
            }
        }

        self.json_response(self.http.post(url).json(&body).send().await?)
            .await
    }

    async fn json_response(&self, response: reqwest::Response) -> CliResult<Value> {
        let status = response.status();
        let text = response.text().await?;
        if !status.is_success() {
            return Err(CliError::message(format!(
                "daemon returned HTTP {status}: {}",
                error_body_message(&text)
            )));
        }

        Ok(serde_json::from_str(&text)?)
    }

    fn endpoint(&self, path: &str) -> CliResult<Url> {
        let mut url =
            Url::parse(&self.origin).map_err(|error| CliError::message(error.to_string()))?;
        url.set_path(path);
        url.set_query(None);
        Ok(url)
    }
}

async fn run_status(client: &DaemonClient) -> CliResult<()> {
    match client.get_json("/health", &[]).await {
        Ok(value) => {
            let service = value_str(&value, "service").unwrap_or("weblayer-daemon");
            println!("{service} is reachable at {}", client.origin);
            println!("Run `weblayer --help` to see client commands.");
            Ok(())
        }
        Err(error) => Err(CliError::message(format!(
            "daemon is not reachable at {}: {error}\nRun `weblayer daemon` to start it.",
            client.origin
        ))),
    }
}

async fn run_rules(client: &DaemonClient, command: RulesCommand) -> CliResult<()> {
    match command.command.unwrap_or_else(|| {
        RulesSubcommand::List(RulesListArgs {
            site: DEFAULT_SITE.into(),
            status: None,
            limit: 100,
            offset: 0,
            json: false,
        })
    }) {
        RulesSubcommand::List(args) => {
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
        }
        RulesSubcommand::Show(args) => {
            let value = client
                .get_json(&format!("/v1/rules/{}", args.id), &[("site", args.site)])
                .await?;
            if args.json {
                print_json(&value)?;
            } else {
                print_rule_detail(&value);
            }
        }
        RulesSubcommand::Create(args) => {
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
        }
        RulesSubcommand::Update(args) => {
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
        }
        RulesSubcommand::Enable(args) => {
            run_rule_status(client, args, "active", "enabled").await?;
        }
        RulesSubcommand::Disable(args) => {
            run_rule_status(client, args, "disabled", "disabled").await?;
        }
        RulesSubcommand::Archive(args) => {
            run_rule_status(client, args, "archived", "archived").await?;
        }
        RulesSubcommand::Reorder(args) => {
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
        }
        RulesSubcommand::Validate(args) => {
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
        }
        RulesSubcommand::Suggest(args) => {
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
        }
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

async fn run_content(client: &DaemonClient, command: ContentCommand) -> CliResult<()> {
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

async fn run_feedback(client: &DaemonClient, command: FeedbackCommand) -> CliResult<()> {
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

async fn run_annotations(client: &DaemonClient, command: AnnotationsCommand) -> CliResult<()> {
    match command.command {
        AnnotationsSubcommand::List(args) => {
            let mut query = vec![
                ("site", args.site),
                ("limit", args.limit.to_string()),
                ("offset", args.offset.to_string()),
            ];
            push_optional_query(&mut query, "storageKey", args.storage_key);
            push_optional_query(&mut query, "contentId", args.content_id);
            push_optional_query(&mut query, "contentKind", args.content_kind);
            push_optional_query(&mut query, "annotationType", args.annotation_type);
            push_optional_query(&mut query, "key", args.key);
            push_optional_query(&mut query, "source", args.source);
            let value = client.get_json("/v1/content/annotations", &query).await?;

            if args.json {
                print_json(&value)?;
            } else {
                print_annotations(&value);
            }
        }
        AnnotationsSubcommand::Put(args) => {
            let value = parse_json_value(&args.value)?;
            let mut body = json!({
                "storageKey": args.storage_key,
                "contentKind": args.content_kind,
                "annotationType": args.annotation_type,
                "key": args.key,
                "value": value,
                "source": args.source
            });
            if let Some(confidence) = args.confidence {
                body["confidence"] = json!(confidence);
            }

            let response = client
                .post_json("/v1/content/annotations", &[("site", args.site)], body)
                .await?;
            if args.json {
                print_json(&response)?;
            } else {
                print_annotation_put(&response);
            }
        }
    }

    Ok(())
}

fn normalize_origin(origin: String) -> CliResult<String> {
    let url = Url::parse(origin.trim().trim_end_matches('/'))
        .map_err(|error| CliError::message(error.to_string()))?;
    if url.scheme() != "http" {
        return Err(CliError::message("daemon origin must use http"));
    }
    if url.host_str().is_none() {
        return Err(CliError::message("daemon origin must include a host"));
    }

    Ok(url.origin().ascii_serialization())
}

fn push_optional_query(query: &mut Vec<(&str, String)>, key: &'static str, value: Option<String>) {
    if let Some(value) = value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        query.push((key, value));
    }
}

fn print_json(value: &Value) -> CliResult<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn print_rules(value: &Value) {
    print_page_header("rules", value);
    println!("{:<36} {:<10} {:>8}  TITLE", "ID", "STATUS", "PRIORITY");
    for item in value_items(value) {
        println!(
            "{:<36} {:<10} {:>8}  {}",
            truncate(value_str(item, "id").unwrap_or(""), 36),
            value_str(item, "status").unwrap_or(""),
            value_i64(item, "priority")
                .map(|value| value.to_string())
                .unwrap_or_default(),
            value_str(item, "title").unwrap_or("")
        );
    }
}

fn print_rule_detail(value: &Value) {
    let rule = value.get("rule").unwrap_or(&Value::Null);
    println!("rule: {}", value_str(rule, "id").unwrap_or(""));
    println!("site: {}", value_str(value, "site").unwrap_or(""));
    println!("status: {}", value_str(rule, "status").unwrap_or(""));
    println!(
        "priority: {}",
        value_i64(rule, "priority")
            .map(|value| value.to_string())
            .unwrap_or_default()
    );
    println!("title: {}", value_str(rule, "title").unwrap_or(""));
    println!("source: {}", value_str(rule, "createdSource").unwrap_or(""));
    println!(
        "updated: {}",
        value_i64(rule, "updatedAtUnixMs")
            .map(|value| value.to_string())
            .unwrap_or_default()
    );
    println!(
        "instruction: {}",
        value_str(rule, "instruction").unwrap_or("")
    );

    let examples = rule.get("examples").unwrap_or(&Value::Null);
    print_string_array("positive examples", examples.get("positive"));
    print_string_array("negative examples", examples.get("negative"));

    let audit = value
        .get("audit")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    if !audit.is_empty() {
        println!("audit:");
        for event in audit.iter().take(10) {
            println!(
                "  {:<12} {:<18} {}",
                value_str(event, "eventKind").unwrap_or(""),
                value_str(event, "source").unwrap_or(""),
                value_i64(event, "createdAtUnixMs")
                    .map(|value| value.to_string())
                    .unwrap_or_default()
            );
        }
    }
}

fn print_rule_saved(value: &Value, action: &str) {
    let rule = value.get("rule").unwrap_or(&Value::Null);
    println!(
        "rule {} {} ({}, priority {})",
        value_str(rule, "id").unwrap_or("unknown"),
        action,
        value_str(rule, "status").unwrap_or("unknown"),
        value_i64(rule, "priority")
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".into())
    );
}

fn print_rule_validation(value: &Value) {
    let rule = value.get("rule").unwrap_or(&Value::Null);
    println!(
        "rule validation for {}: {} likely matches from {} stored items, limit {}, offset {}",
        value_str(rule, "id").unwrap_or(""),
        value_usize(value, "totalMatching").unwrap_or(0),
        value_usize(value, "totalStored").unwrap_or(0),
        value_usize(value, "limit").unwrap_or(0),
        value_usize(value, "offset").unwrap_or(0)
    );
    println!(
        "{:<28} {:<18} {:>5}  {:<24}  TEXT",
        "STORAGE KEY", "AUTHOR", "SCORE", "MATCHES"
    );
    for item in value_items(value) {
        let content = item.get("content").unwrap_or(&Value::Null);
        let matched_terms = value_string_array(item, "matchedTerms").join(",");
        let matched_examples = value_string_array(item, "matchedExamples").join(",");
        let matches = if matched_examples.is_empty() {
            matched_terms
        } else if matched_terms.is_empty() {
            matched_examples
        } else {
            format!("{matched_terms};{matched_examples}")
        };
        println!(
            "{:<28} {:<18} {:>5}  {:<24}  {}",
            truncate(value_str(content, "storageKey").unwrap_or(""), 28),
            truncate(value_str(content, "author").unwrap_or(""), 18),
            value_usize(item, "score")
                .map(|value| value.to_string())
                .unwrap_or_default(),
            truncate(&matches, 24),
            truncate(value_str(content, "text").unwrap_or(""), 72)
        );
    }
}

fn print_rule_suggestions(value: &Value) {
    print_page_header("rule suggestions", value);
    println!("{:<36} {:>8} {:>8}  TITLE", "ID", "FEEDBACK", "PRIORITY");
    for item in value_items(value) {
        println!(
            "{:<36} {:>8} {:>8}  {}",
            truncate(value_str(item, "id").unwrap_or(""), 36),
            value_usize(item, "feedbackCount")
                .map(|value| value.to_string())
                .unwrap_or_default(),
            value_i64(item, "priority")
                .map(|value| value.to_string())
                .unwrap_or_default(),
            value_str(item, "title").unwrap_or("")
        );
        println!(
            "  instruction: {}",
            value_str(item, "instruction").unwrap_or("")
        );
        let examples = item.get("examples").unwrap_or(&Value::Null);
        print_string_array("  positive examples", examples.get("positive"));
    }
}

fn print_content(value: &Value) {
    print_page_header("content", value);
    println!("{:<28} {:<18} {:>5}  TEXT", "STORAGE KEY", "AUTHOR", "SEEN");
    for item in value_items(value) {
        println!(
            "{:<28} {:<18} {:>5}  {}",
            truncate(value_str(item, "storageKey").unwrap_or(""), 28),
            truncate(value_str(item, "author").unwrap_or(""), 18),
            value_i64(item, "seenCount")
                .map(|value| value.to_string())
                .unwrap_or_default(),
            truncate(value_str(item, "text").unwrap_or(""), 88)
        );
    }
}

fn print_content_stats(value: &Value) {
    let stats = value.get("stats").unwrap_or(&Value::Null);
    println!("site: {}", value_str(value, "site").unwrap_or(""));
    println!(
        "content kind: {}",
        value_str(stats, "contentKind").unwrap_or("")
    );
    println!(
        "unique items: {}",
        value_usize(stats, "uniqueItems").unwrap_or(0)
    );
    println!(
        "total encounters: {}",
        value_usize(stats, "totalEncounters").unwrap_or(0)
    );
    println!(
        "items with stable id: {}",
        value_usize(stats, "itemsWithStableId").unwrap_or(0)
    );
    println!(
        "first seen: {}",
        value_i64(stats, "firstSeenAtUnixMs")
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".into())
    );
    println!(
        "last seen: {}",
        value_i64(stats, "lastSeenAtUnixMs")
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".into())
    );
}

fn print_feedback(value: &Value) {
    print_page_header("feedback", value);
    println!(
        "{:<28} {:<18} {:<8}  REASON",
        "STORAGE KEY", "AUTHOR", "ACTIVE"
    );
    for item in value_items(value) {
        println!(
            "{:<28} {:<18} {:<8}  {}",
            truncate(value_str(item, "storageKey").unwrap_or(""), 28),
            truncate(value_str(item, "author").unwrap_or(""), 18),
            value_bool(item, "active")
                .map(|value| value.to_string())
                .unwrap_or_default(),
            truncate(value_str(item, "reason").unwrap_or(""), 88)
        );
    }
}

fn print_annotations(value: &Value) {
    print_page_header("annotations", value);
    println!(
        "{:>5} {:<24} {:<12} {:<16} {:<18}  VALUE",
        "ID", "STORAGE KEY", "TYPE", "KEY", "SOURCE"
    );
    for item in value_items(value) {
        println!(
            "{:>5} {:<24} {:<12} {:<16} {:<18}  {}",
            value_i64(item, "id")
                .map(|value| value.to_string())
                .unwrap_or_default(),
            truncate(value_str(item, "storageKey").unwrap_or(""), 24),
            truncate(value_str(item, "annotationType").unwrap_or(""), 12),
            truncate(value_str(item, "key").unwrap_or(""), 16),
            truncate(value_str(item, "source").unwrap_or(""), 18),
            truncate(
                &item
                    .get("value")
                    .map(Value::to_string)
                    .unwrap_or_else(|| "null".into()),
                72,
            )
        );
    }
}

fn print_annotation_put(value: &Value) {
    let annotation = value.get("annotation").unwrap_or(&Value::Null);
    println!(
        "annotation {} upserted for {}",
        value_i64(annotation, "id")
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".into()),
        value_str(annotation, "storageKey").unwrap_or("unknown")
    );
}

fn print_page_header(label: &str, value: &Value) {
    let site = value_str(value, "site").unwrap_or("");
    let total = value_usize(value, "totalMatching").unwrap_or(0);
    let limit = value_usize(value, "limit").unwrap_or(0);
    let offset = value_usize(value, "offset").unwrap_or(0);
    println!("{label} for {site}: total {total}, limit {limit}, offset {offset}");
}

fn parse_json_value(text: &str) -> CliResult<Value> {
    serde_json::from_str(text).map_err(|error| {
        CliError::message(format!(
            "annotation value must be valid JSON: {error}. Example: --value '\"local-ai\"'"
        ))
    })
}

fn value_items(value: &Value) -> &[Value] {
    value
        .get("items")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn value_str<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn value_i64(value: &Value, key: &str) -> Option<i64> {
    value.get(key).and_then(Value::as_i64)
}

fn value_usize(value: &Value, key: &str) -> Option<usize> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
}

fn value_bool(value: &Value, key: &str) -> Option<bool> {
    value.get(key).and_then(Value::as_bool)
}

fn value_string_array(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn print_string_array(label: &str, value: Option<&Value>) {
    let items = value
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    if items.is_empty() {
        println!("{label}: none");
        return;
    }

    println!("{label}:");
    for item in items {
        if let Some(text) = item.as_str() {
            println!("  - {text}");
        }
    }
}

fn truncate(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

fn error_body_message(text: &str) -> String {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| text.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_origin_strips_paths_and_trailing_slashes() {
        assert_eq!(
            normalize_origin("http://127.0.0.1:17891/path/".into()).unwrap(),
            "http://127.0.0.1:17891"
        );
    }

    #[test]
    fn parse_json_value_requires_json() {
        assert!(parse_json_value("plain text").is_err());
        assert_eq!(
            parse_json_value("\"plain text\"").unwrap(),
            json!("plain text")
        );
    }
}

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
            Some(Command::Dislikes(command)) => ClientCommand::Dislikes(command),
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
        ClientCommand::Dislikes(command) => run_dislikes(&client, command).await,
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
    /// List stored dislike feedback.
    Dislikes(DislikesCommand),
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
struct DislikesCommand {
    #[command(subcommand)]
    command: Option<DislikesSubcommand>,
}

#[derive(Debug, Clone, Subcommand)]
enum DislikesSubcommand {
    /// List stored dislike feedback.
    List(DislikesListArgs),
}

#[derive(Debug, Clone, Args)]
struct DislikesListArgs {
    /// Site scope to inspect.
    #[arg(long, default_value = DEFAULT_SITE)]
    site: String,
    /// Filter by current active state.
    #[arg(long, default_value_t = true)]
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
    Dislikes(DislikesCommand),
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
    let args = match command.command {
        Some(RulesSubcommand::List(args)) => args,
        None => RulesListArgs {
            site: DEFAULT_SITE.into(),
            status: None,
            limit: 100,
            offset: 0,
            json: false,
        },
    };
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

async fn run_dislikes(client: &DaemonClient, command: DislikesCommand) -> CliResult<()> {
    let args = match command.command {
        Some(DislikesSubcommand::List(args)) => args,
        None => DislikesListArgs {
            site: DEFAULT_SITE.into(),
            active: true,
            limit: 20,
            offset: 0,
            json: false,
        },
    };
    let value = client
        .get_json(
            "/v1/dislikes",
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
        print_dislikes(&value);
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

fn print_dislikes(value: &Value) {
    print_page_header("dislikes", value);
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

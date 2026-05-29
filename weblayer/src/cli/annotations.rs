use super::{
    client::{push_optional_query, DaemonClient},
    output::{parse_json_value, print_annotation_put, print_annotations, print_json},
    CliResult, DEFAULT_SITE,
};
use clap::{Args, Subcommand};
use serde_json::json;

#[derive(Debug, Clone, Args)]
pub(super) struct AnnotationsCommand {
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

pub(super) async fn run_annotations(
    client: &DaemonClient,
    command: AnnotationsCommand,
) -> CliResult<()> {
    match command.command {
        AnnotationsSubcommand::List(args) => run_annotations_list(client, args).await?,
        AnnotationsSubcommand::Put(args) => run_annotations_put(client, args).await?,
    }

    Ok(())
}

async fn run_annotations_list(client: &DaemonClient, args: AnnotationsListArgs) -> CliResult<()> {
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

    Ok(())
}

async fn run_annotations_put(client: &DaemonClient, args: AnnotationsPutArgs) -> CliResult<()> {
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

    Ok(())
}

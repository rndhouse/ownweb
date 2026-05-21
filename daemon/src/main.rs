use axum::{
    http::{header, Method},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::{
    io::{self, Write},
    net::SocketAddr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::net::TcpListener;
use tower_http::cors::{Any, CorsLayer};
use tracing_subscriber::EnvFilter;

const DEFAULT_BIND_ADDR: &str = "127.0.0.1:17891";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse()?))
        .init();

    let bind_addr =
        std::env::var("PAIRPILOT_BIND_ADDR").unwrap_or_else(|_| DEFAULT_BIND_ADDR.into());
    let addr: SocketAddr = bind_addr.parse()?;

    let app = Router::new()
        .route("/health", get(health))
        .route("/v1/x-posts/analyze", post(analyze_posts))
        .layer(cors_layer());

    let listener = TcpListener::bind(addr).await?;
    eprintln!("pairpilot-daemon listening on http://{addr}");
    eprintln!("pairpilot-daemon logging captured posts to stdout as JSONL");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

fn cors_layer() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers([header::CONTENT_TYPE])
        .max_age(Duration::from_secs(60 * 60))
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        ok: true,
        service: "pairpilot-daemon",
    })
}

async fn analyze_posts(Json(request): Json<AnalyzeRequest>) -> Json<AnalyzeResponse> {
    if let Err(error) = log_request_to_stdout(&request) {
        eprintln!("failed to log captured posts: {error}");
    }
    let posts = request.posts.into_iter().map(classify_post).collect();
    Json(AnalyzeResponse { posts })
}

fn classify_post(post: IncomingPost) -> PostDecision {
    let normalized = post.text.to_lowercase();
    let spam_hits = count_matches(
        &normalized,
        &[
            "airdrop",
            "crypto giveaway",
            "guaranteed returns",
            "100x",
            "dm me",
            "link in bio",
            "free money",
        ],
    );
    let ai_hits = count_matches(
        &normalized,
        &[
            "as an ai",
            "delve",
            "unlock the power",
            "game-changer",
            "in today's fast-paced",
            "revolutionize",
            "seamlessly",
        ],
    );

    if post.text.trim().is_empty() {
        return PostDecision::keep(post.client_id);
    }

    if spam_hits >= 2 {
        return PostDecision {
            client_id: post.client_id,
            action: "hide",
            label: Some("Pairpilot: spam".into()),
            reason: Some("Matched promotional spam heuristics".into()),
            replacement_text: None,
            confidence: Some(0.9),
        };
    }

    if ai_hits >= 2 {
        return PostDecision {
            client_id: post.client_id,
            action: "dim",
            label: Some("Pairpilot: likely generated".into()),
            reason: Some("Matched generated-writing heuristics".into()),
            replacement_text: None,
            confidence: Some(0.72),
        };
    }

    PostDecision::keep(post.client_id)
}

fn count_matches(text: &str, terms: &[&str]) -> usize {
    terms.iter().filter(|term| text.contains(**term)).count()
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

fn log_request_to_stdout(request: &AnalyzeRequest) -> io::Result<()> {
    if request.posts.is_empty() {
        return Ok(());
    }

    let received_at_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let stdout = io::stdout();
    let mut handle = stdout.lock();

    for post in &request.posts {
        let captured_post = CapturedPost {
            received_at_unix_ms,
            source: request.source.as_deref(),
            post,
        };
        serde_json::to_writer(&mut handle, &captured_post)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        handle.write_all(b"\n")?;
    }

    handle.flush()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AnalyzeRequest {
    #[serde(default)]
    source: Option<String>,
    posts: Vec<IncomingPost>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct IncomingPost {
    client_id: String,
    #[serde(default)]
    post_id: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    author_handle: Option<String>,
    #[serde(default)]
    text: String,
    #[serde(default)]
    captured_at: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CapturedPost<'a> {
    received_at_unix_ms: u128,
    source: Option<&'a str>,
    post: &'a IncomingPost,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    ok: bool,
    service: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AnalyzeResponse {
    posts: Vec<PostDecision>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PostDecision {
    client_id: String,
    action: &'static str,
    label: Option<String>,
    reason: Option<String>,
    replacement_text: Option<String>,
    confidence: Option<f32>,
}

impl PostDecision {
    fn keep(client_id: String) -> Self {
        Self {
            client_id,
            action: "keep",
            label: None,
            reason: None,
            replacement_text: None,
            confidence: Some(1.0),
        }
    }
}

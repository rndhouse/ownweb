use reqwest::Client;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    error::Error,
    fs,
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::time::sleep;

type TestResult<T> = Result<T, Box<dyn Error>>;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires WEBLAYER_RUN_CODEX_E2E=1 and a working Codex app-server path"]
async fn codex_rule_proposal_from_curated_feedback() -> TestResult<()> {
    require_codex_e2e()?;

    let data_dir = TempDir::new("weblayer-codex-e2e-curated")?;
    let daemon = TestDaemon::start(data_dir.path().to_path_buf())?;
    daemon.wait_until_ready().await?;

    let fixtures = curated_feedback();
    let analyze_response = daemon.analyze(&fixtures).await?;
    let rule_stats = rule_decision_stats(daemon.data_dir_path())?;
    assert_rule_has_nonzero_counts(&rule_stats, "x-engagement-bait-reaction")?;

    let context_ids = feedback_context_ids_by_client_id(&analyze_response)?;
    for fixture in &fixtures {
        let context_id = context_ids
            .get(fixture.client_id)
            .ok_or_else(|| format!("missing feedback context for {}", fixture.client_id))?;
        daemon.post_feedback(fixture, context_id).await?;
    }

    let feedback = daemon.get("/v1/feedback?site=x.com&limit=20").await?;
    assert_eq!(feedback["totalMatching"], json!(fixtures.len()));

    let proposal = daemon.agent_rule_proposal(2, 20).await?;
    assert_proposal_invariants(&proposal, fixtures.len())?;
    assert_eq!(proposal["proposal"]["source"], json!("agent:codex-app"));

    let artifact = write_artifacts("curated", &feedback, &proposal, &rule_stats)?;
    eprintln!("wrote Codex E2E artifacts to {}", artifact.display());
    eprintln!("{}", proposal_summary(&proposal, &rule_stats));

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires WEBLAYER_RUN_CODEX_E2E=1, WEBLAYER_E2E_USE_LOCAL_DATA=1, and local feedback"]
async fn codex_rule_proposal_from_local_data_copy() -> TestResult<()> {
    require_codex_e2e()?;
    if !env_flag("WEBLAYER_E2E_USE_LOCAL_DATA") {
        eprintln!("skipping local-data Codex E2E; set WEBLAYER_E2E_USE_LOCAL_DATA=1 to enable");
        return Ok(());
    }

    let source_data_dir = local_data_dir()?;
    let source_db = source_data_dir.join("x.com/db.sqlite");
    if !source_db.exists() {
        eprintln!(
            "skipping local-data Codex E2E; no local X database at {}",
            source_db.display()
        );
        return Ok(());
    }

    let data_dir = TempDir::new("weblayer-codex-e2e-local")?;
    copy_sqlite_site_db(&source_data_dir, data_dir.path())?;
    let daemon = TestDaemon::start(data_dir.path().to_path_buf())?;
    daemon.wait_until_ready().await?;

    let feedback = daemon.get("/v1/feedback?site=x.com&limit=20").await?;
    let feedback_count = feedback["totalMatching"].as_u64().unwrap_or(0) as usize;
    if feedback_count == 0 {
        eprintln!("skipping local-data Codex E2E; copied database has no active feedback");
        return Ok(());
    }

    let proposal = daemon.agent_rule_proposal(1, 20).await?;
    assert_proposal_invariants(&proposal, feedback_count.min(10))?;
    assert_eq!(proposal["proposal"]["source"], json!("agent:codex-app"));

    let rule_stats = rule_decision_stats(daemon.data_dir_path())?;
    let artifact = write_artifacts("local-copy", &feedback, &proposal, &rule_stats)?;
    eprintln!("wrote Codex E2E artifacts to {}", artifact.display());
    eprintln!("{}", proposal_summary(&proposal, &rule_stats));

    Ok(())
}

#[derive(Clone, Copy)]
struct FeedbackFixture {
    client_id: &'static str,
    status_id: &'static str,
    reason: &'static str,
    text: &'static str,
}

fn curated_feedback() -> Vec<FeedbackFixture> {
    vec![
        FeedbackFixture {
            client_id: "curated-1",
            status_id: "9101",
            reason: "engagement bait",
            text: "Reply YES if you agree with this absurd viral clip and I will send the secret thread.",
        },
        FeedbackFixture {
            client_id: "curated-2",
            status_id: "9102",
            reason: "engagement bait",
            text: "Quote this with your hottest take about this ridiculous video, wrong answers only.",
        },
        FeedbackFixture {
            client_id: "curated-3",
            status_id: "9103",
            reason: "generic AI slop",
            text: "I asked ChatGPT to write a viral thread about productivity and the result will shock you.",
        },
        FeedbackFixture {
            client_id: "curated-4",
            status_id: "9104",
            reason: "generic AI slop",
            text: "This AI-generated thread reveals ten mind blowing habits every founder must copy today.",
        },
    ]
}

struct TestDaemon {
    origin: String,
    child: Child,
    data_dir: TempDir,
}

impl TestDaemon {
    fn start(data_dir_path: PathBuf) -> TestResult<Self> {
        let http_port = free_port()?;
        let codex_ws_port = free_port()?;
        let data_dir = TempDir::from_existing(data_dir_path);
        let child = Command::new(env!("CARGO_BIN_EXE_weblayer"))
            .arg("daemon")
            .env("WEBLAYER_BIND_ADDR", format!("127.0.0.1:{http_port}"))
            .env("WEBLAYER_DATA_DIR", data_dir.path())
            .env(
                "WEBLAYER_CODEX_APP_WS",
                format!("ws://127.0.0.1:{codex_ws_port}"),
            )
            .env(
                "WEBLAYER_CODEX_OPINION_TIMEOUT_MS",
                env::var("WEBLAYER_CODEX_OPINION_TIMEOUT_MS").unwrap_or_else(|_| "60000".into()),
            )
            .env(
                "WEBLAYER_CODEX_RULE_PROPOSAL_TIMEOUT_MS",
                env::var("WEBLAYER_CODEX_RULE_PROPOSAL_TIMEOUT_MS")
                    .unwrap_or_else(|_| "120000".into()),
            )
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;

        Ok(Self {
            origin: format!("http://127.0.0.1:{http_port}"),
            child,
            data_dir,
        })
    }

    fn data_dir_path(&self) -> &Path {
        self.data_dir.path()
    }

    async fn wait_until_ready(&self) -> TestResult<()> {
        let client = Client::new();
        for _ in 0..120 {
            if let Ok(response) = client.get(format!("{}/health", self.origin)).send().await {
                if response.status().is_success() {
                    return Ok(());
                }
            }
            sleep(Duration::from_millis(250)).await;
        }

        Err("daemon did not become ready".into())
    }

    async fn get(&self, path: &str) -> TestResult<Value> {
        let response = Client::new()
            .get(format!("{}{}", self.origin, path))
            .send()
            .await?;
        json_response(response).await
    }

    async fn post(&self, path: &str, body: Value) -> TestResult<Value> {
        let response = Client::new()
            .post(format!("{}{}", self.origin, path))
            .json(&body)
            .send()
            .await?;
        json_response(response).await
    }

    async fn analyze(&self, fixtures: &[FeedbackFixture]) -> TestResult<Value> {
        let elements = fixtures
            .iter()
            .map(|fixture| dom_element(fixture.client_id, fixture.status_id, fixture.text))
            .collect::<Vec<_>>();
        self.post(
            "/v1/dom/analyze",
            json!({
                "page": {
                    "url": "https://x.com/home",
                    "title": "X"
                },
                "elements": elements
            }),
        )
        .await
    }

    async fn post_feedback(&self, fixture: &FeedbackFixture, context_id: &str) -> TestResult<()> {
        self.post(
            "/v1/dom/feedback",
            json!({
                "feedback": "thumbsDown",
                "reason": fixture.reason,
                "feedbackContextId": context_id,
                "page": {
                    "url": "https://x.com/home",
                    "title": "X"
                },
                "element": dom_element(fixture.client_id, fixture.status_id, fixture.text)
            }),
        )
        .await?;
        Ok(())
    }

    async fn agent_rule_proposal(
        &self,
        min_feedback: usize,
        feedback_limit: usize,
    ) -> TestResult<Value> {
        let mut last_proposal = None;
        for attempt in 1..=2 {
            let proposal = self
                .post(
                    "/v1/rule-proposals?site=x.com",
                    json!({
                        "minFeedback": min_feedback,
                        "feedbackLimit": feedback_limit
                    }),
                )
                .await?;
            if proposal["proposal"]["source"] == json!("agent:codex-app") {
                return Ok(proposal);
            }
            eprintln!(
                "rule proposal attempt {attempt} used {}; retrying once if available",
                proposal["proposal"]["source"].as_str().unwrap_or("unknown")
            );
            last_proposal = Some(proposal);
        }

        Ok(last_proposal.expect("proposal attempts should record a result"))
    }
}

impl Drop for TestDaemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

async fn json_response(response: reqwest::Response) -> TestResult<Value> {
    let status = response.status();
    let text = response.text().await?;
    if !status.is_success() {
        return Err(format!("daemon returned HTTP {status}: {text}").into());
    }

    Ok(serde_json::from_str(&text)?)
}

fn dom_element(client_id: &str, status_id: &str, text: &str) -> Value {
    json!({
        "clientId": client_id,
        "text": text,
        "attributes": [
            {
                "name": "data-testid",
                "value": "tweet"
            }
        ],
        "links": [
            {
                "href": format!("https://x.com/user/status/{status_id}"),
                "text": "status"
            }
        ]
    })
}

fn feedback_context_ids_by_client_id(response: &Value) -> TestResult<BTreeMap<String, String>> {
    let mut context_ids = BTreeMap::new();
    for command in response["commands"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
    {
        if command["action"] != json!("insertFeedbackControl") {
            continue;
        }
        let client_id = command["target"]["clientId"]
            .as_str()
            .ok_or("feedback command missing target.clientId")?;
        let context_id = command["feedbackContextId"]
            .as_str()
            .ok_or("feedback command missing feedbackContextId")?;
        context_ids.insert(client_id.to_string(), context_id.to_string());
    }

    Ok(context_ids)
}

fn assert_proposal_invariants(proposal: &Value, expected_feedback_count: usize) -> TestResult<()> {
    let proposal = proposal
        .get("proposal")
        .ok_or("response missing proposal")?;
    assert_eq!(proposal["status"], json!("pending"));
    assert_eq!(
        proposal["feedbackCount"].as_u64().unwrap_or(0) as usize,
        expected_feedback_count
    );

    let changes = proposal["changes"]
        .as_array()
        .ok_or("proposal.changes must be an array")?;
    assert!(
        !changes.is_empty(),
        "proposal should contain at least one change"
    );

    let mut evidence_keys = BTreeSet::new();
    for change in changes {
        let action = change["action"].as_str().ok_or("change missing action")?;
        assert!(
            matches!(
                action,
                "createRule" | "updateRule" | "disableRule" | "noChange"
            ),
            "unexpected action: {action}"
        );
        assert!(
            change["rationale"]
                .as_str()
                .is_some_and(|rationale| !rationale.trim().is_empty()),
            "change should include a rationale"
        );
        if matches!(action, "updateRule" | "disableRule") {
            assert!(
                change["ruleId"]
                    .as_str()
                    .is_some_and(|rule_id| !rule_id.trim().is_empty()),
                "{action} should identify an existing rule"
            );
        }
        for key in change["evidenceStorageKeys"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[])
        {
            if let Some(key) = key.as_str() {
                evidence_keys.insert(key.to_string());
            }
        }
    }

    assert!(
        !evidence_keys.is_empty(),
        "proposal should cite at least one feedback evidence key"
    );

    Ok(())
}

#[derive(Debug, Clone)]
struct RuleStat {
    rule_id: String,
    matched_count: usize,
    hide_count: usize,
}

fn rule_decision_stats(data_dir: &Path) -> TestResult<Vec<RuleStat>> {
    let connection = Connection::open(data_dir.join("x.com/db.sqlite"))?;
    let mut statement = connection.prepare(
        "
        SELECT action, matched_rule_ids_json
        FROM content_decision_events
        ",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut stats = BTreeMap::<String, (usize, usize)>::new();

    for row in rows {
        let (action, matched_rule_ids_json) = row?;
        let matched_rule_ids: Vec<String> = serde_json::from_str(&matched_rule_ids_json)?;
        let unique_rule_ids = matched_rule_ids.into_iter().collect::<BTreeSet<_>>();

        for rule_id in unique_rule_ids {
            let entry = stats.entry(rule_id).or_default();
            entry.0 += 1;
            if action == "hide" {
                entry.1 += 1;
            }
        }
    }

    Ok(stats
        .into_iter()
        .map(|(rule_id, (matched_count, hide_count))| RuleStat {
            rule_id,
            matched_count,
            hide_count,
        })
        .collect())
}

fn assert_rule_has_nonzero_counts(stats: &[RuleStat], rule_id: &str) -> TestResult<()> {
    let stat = stats
        .iter()
        .find(|stat| stat.rule_id == rule_id)
        .ok_or_else(|| format!("missing decision stats for rule {rule_id}"))?;
    assert!(
        stat.matched_count > 0,
        "expected non-zero matched count for {rule_id}"
    );
    assert!(
        stat.hide_count > 0,
        "expected non-zero hide count for {rule_id}"
    );

    Ok(())
}

fn write_artifacts(
    label: &str,
    feedback: &Value,
    proposal: &Value,
    rule_stats: &[RuleStat],
) -> TestResult<PathBuf> {
    let dir = env::var("WEBLAYER_E2E_ARTIFACT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/codex-e2e"));
    fs::create_dir_all(&dir)?;

    let stem = format!("{label}-{}", unix_ms());
    let json_path = dir.join(format!("{stem}.json"));
    let md_path = dir.join(format!("{stem}.md"));
    let artifact = json!({
        "feedback": feedback,
        "proposal": proposal,
        "ruleStats": rule_stats_json(rule_stats)
    });
    fs::write(&json_path, serde_json::to_string_pretty(&artifact)?)?;
    fs::write(&md_path, proposal_summary(proposal, rule_stats))?;

    Ok(json_path)
}

fn rule_stats_json(rule_stats: &[RuleStat]) -> Value {
    Value::Array(
        rule_stats
            .iter()
            .map(|stat| {
                json!({
                    "ruleId": stat.rule_id,
                    "matchedCount": stat.matched_count,
                    "hideCount": stat.hide_count
                })
            })
            .collect(),
    )
}

fn proposal_summary(response: &Value, rule_stats: &[RuleStat]) -> String {
    let proposal = &response["proposal"];
    let mut text = String::new();
    text.push_str(&format!(
        "# Rule Proposal {}\n\n",
        proposal["id"].as_str().unwrap_or("")
    ));
    text.push_str(&format!(
        "- source: {}\n- status: {}\n- feedback: {}\n- active rules: {}\n\n",
        proposal["source"].as_str().unwrap_or(""),
        proposal["status"].as_str().unwrap_or(""),
        proposal["feedbackCount"].as_u64().unwrap_or(0),
        proposal["activeRuleCount"].as_u64().unwrap_or(0)
    ));
    text.push_str("## Rule Stats\n\n");
    if rule_stats.is_empty() {
        text.push_str("No rule decision stats were recorded before proposal generation.\n\n");
    } else {
        for stat in rule_stats {
            text.push_str(&format!(
                "- `{}`: matched {}, hid {}\n",
                stat.rule_id, stat.matched_count, stat.hide_count
            ));
        }
        text.push('\n');
    }
    text.push_str("## Changes\n\n");

    for change in proposal["changes"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
    {
        text.push_str(&format!(
            "- action: `{}`\n",
            change["action"].as_str().unwrap_or("")
        ));
        if let Some(rule_id) = change["ruleId"].as_str() {
            text.push_str(&format!("  rule: `{rule_id}`\n"));
        }
        if let Some(title) = change["title"].as_str() {
            text.push_str(&format!("  title: {title}\n"));
        }
        if let Some(instruction) = change["instruction"].as_str() {
            text.push_str(&format!("  instruction: {instruction}\n"));
        }
        text.push_str(&format!(
            "  rationale: {}\n",
            change["rationale"].as_str().unwrap_or("")
        ));
        let evidence = change["evidenceStorageKeys"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[])
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        if !evidence.is_empty() {
            text.push_str(&format!("  evidence: {}\n", evidence.join(", ")));
        }
        text.push('\n');
    }

    text
}

fn require_codex_e2e() -> TestResult<()> {
    if env_flag("WEBLAYER_RUN_CODEX_E2E") {
        Ok(())
    } else {
        Err("set WEBLAYER_RUN_CODEX_E2E=1 to run Codex E2E tests".into())
    }
}

fn env_flag(name: &str) -> bool {
    env::var(name)
        .map(|value| value != "0" && !value.eq_ignore_ascii_case("false"))
        .unwrap_or(false)
}

fn local_data_dir() -> TestResult<PathBuf> {
    if let Ok(path) = env::var("WEBLAYER_DATA_DIR") {
        let path = path.trim();
        if !path.is_empty() {
            return Ok(PathBuf::from(path));
        }
    }

    let home = env::var("HOME")?;
    Ok(PathBuf::from(home).join(".local/share/weblayer"))
}

fn copy_sqlite_site_db(source_data_dir: &Path, target_data_dir: &Path) -> TestResult<()> {
    let source_site = source_data_dir.join("x.com");
    let target_site = target_data_dir.join("x.com");
    fs::create_dir_all(&target_site)?;

    for suffix in ["", "-wal", "-shm"] {
        let file_name = format!("db.sqlite{suffix}");
        let source = source_site.join(&file_name);
        if source.exists() {
            fs::copy(&source, target_site.join(file_name))?;
        }
    }

    Ok(())
}

fn free_port() -> TestResult<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.port())
}

fn unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

struct TempDir {
    path: PathBuf,
    cleanup: bool,
}

impl TempDir {
    fn new(prefix: &str) -> TestResult<Self> {
        let path = env::temp_dir().join(format!("{prefix}-{}-{}", std::process::id(), unix_ms()));
        fs::create_dir_all(&path)?;
        Ok(Self {
            path,
            cleanup: true,
        })
    }

    fn from_existing(path: PathBuf) -> Self {
        Self {
            path,
            cleanup: true,
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        if self.cleanup {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

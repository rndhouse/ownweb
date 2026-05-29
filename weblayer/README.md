# WebLayer

WebLayer is a local web filtering daemon and CLI for user-controlled browsing
rules. It stores encountered site content locally, records feedback, manages
explicit filtering rules, and exposes a REST/WebSocket API for browser clients.
Current site-specific behavior focuses on X/Twitter posts.

## Install

Install from crates.io:

```sh
cargo install weblayer
```

This installs the `weblayer` command. It can run the local daemon or act as a
client for a daemon that is already running.

## Run the Daemon

```sh
weblayer daemon
```

The daemon binds to `127.0.0.1:17891` by default. Override it with:

```sh
WEBLAYER_BIND_ADDR=127.0.0.1:19000 weblayer daemon
```

Daemon output uses structured logs on stdout. The default log level is `debug`;
override it with `RUST_LOG`.

Incoming posts are not logged by default. To enable captured-content log events:

```sh
WEBLAYER_LOG_CAPTURED_CONTENT=1 weblayer daemon
```

## CLI

Without `daemon`, `weblayer` talks to a running local daemon. `weblayer` with no
subcommand behaves like `weblayer status`.

```sh
weblayer status
weblayer rules list --site x.com
weblayer rules show x-engagement-bait-reaction --site x.com
weblayer rules create \
  --site x.com \
  --id x-ai-slop \
  --title "AI slop" \
  --instruction "Hide generic AI engagement bait." \
  --positive-example "I asked ChatGPT to write this viral thread"
weblayer rules validate x-engagement-bait-reaction --site x.com
weblayer rules suggest --site x.com --min-feedback 2
weblayer rules enable x-ai-slop --site x.com
weblayer rules disable x-ai-slop --site x.com
weblayer content list --site x.com --limit 20
weblayer content search --site x.com codex
weblayer content stats --site x.com
weblayer feedback list --site x.com
weblayer annotations list --site x.com --storage-key x:id:123
weblayer annotations put \
  --site x.com \
  --storage-key x:id:123 \
  --annotation-type tag \
  --key topics \
  --value '["local-ai","tools"]' \
  --source agent:organizer
```

Client commands use `http://127.0.0.1:17891` by default. Override that with
`--daemon-origin` or `WEBLAYER_DAEMON_ORIGIN`.

## Local Data

Encountered site content is stored in per-site SQLite databases under the
WebLayer data directory. X posts are stored at:

```text
~/.local/share/weblayer/x.com/db.sqlite
```

X feedback is stored in the same database as both an append-only event log and
current feedback state. A stored active thumbs-down makes later scans hide that
post by X status ID.

Override the root data directory with `WEBLAYER_DATA_DIR`. The daemon uses
bundled SQLite through Rust dependencies, so no separate SQLite service or
system install is required.

Reset the X database on startup with:

```sh
WEBLAYER_X_RESET_DB=1 weblayer daemon
```

This removes `db.sqlite`, `db.sqlite-wal`, and `db.sqlite-shm` for `x.com`
before the daemon opens storage.

## Analysis

Codex app-server analysis is enabled by default. The daemon starts a local
Codex app-server process when needed, keeps one app-server thread alive across
requests, and asks it to evaluate captured X/Twitter posts against active
content rules. Every captured X post with text or a URL is sent to Codex so the
daemon can evaluate it against the active rule set.

Opinions are cached in memory by X status ID, a normalized fallback key, and the
active rule set. This lets the timeline view and single-post view reuse the same
AI decision when they capture the same post content under the same policy.

Cache hits and X posts sent to the Codex app-server are logged at debug level
on stdout. Repeated full captured post payloads from DOM extraction are logged
at trace level.

## Configuration

Useful environment variables:

```sh
WEBLAYER_DAEMON_ORIGIN=http://127.0.0.1:17891
WEBLAYER_CODEX_APP_ENABLED=0
WEBLAYER_CODEX_APP_WS=ws://127.0.0.1:39177
WEBLAYER_CODEX_MODEL=gpt-5.3-codex-spark
WEBLAYER_CODEX_EFFORT=low
WEBLAYER_CODEX_TIMEOUT_MS=8000
WEBLAYER_CODEX_CWD=/path/to/project
WEBLAYER_DATA_DIR=/home/user/.local/share/weblayer
WEBLAYER_LOG_CAPTURED_CONTENT=0
WEBLAYER_X_RESET_DB=0
WEBLAYER_X_SUMMARY_CACHE_MAX_ENTRIES=10000
WEBLAYER_X_SUMMARY_CACHE_TTL_SECS=86400
RUST_LOG=debug
```

## API Reference

- `GET /health`
- `GET /v1/events`
- `POST /v1/dom/analyze`
- `POST /v1/dom/feedback`
- `GET /v1/content?site=x.com&q=codex`
- `GET /v1/content/annotations?site=x.com&storageKey=x:id:123`
- `POST /v1/content/annotations?site=x.com`
- `GET /v1/content/stats?site=x.com`
- `GET /v1/feedback?site=x.com`
- `GET /v1/rule-suggestions?site=x.com`
- `GET /v1/rules?site=x.com`
- `POST /v1/rules?site=x.com`
- `GET /v1/rules/{id}?site=x.com`
- `POST /v1/rules/{id}?site=x.com`
- `POST /v1/rules/{id}/status?site=x.com`
- `GET /v1/rules/{id}/validate?site=x.com`

`/v1/events` is the primary extension path. The extension opens a WebSocket,
sends DOM analysis events, receives immediate `pending` commands that gate
identified posts, then receives `final` commands after local analysis finishes.

`/v1/dom/analyze` is the REST smoke-test path. It accepts the same DOM snapshot
shape and returns final DOM commands in one response. `/v1/dom/feedback`
records `thumbsDown`, `undoThumbsDown`, and `updateReason` signals for one DOM
region. Feedback controls include a `feedbackContext` snapshot of the active
rule set; the extension echoes it back so feedback can be tied to the rules
that were in play. Site-scoped inspection endpoints keep the path generic and
take the site scope through the `site` query parameter. `/v1/content` lists
recent stored content or searches it with SQLite FTS5 when `q` is provided.
`/v1/content/stats` returns unique stored content rows and total captured
encounters for the selected site. `/v1/feedback` lists stored user feedback
signals, such as active thumbs-down feedback for X posts.
`/v1/content/annotations` lets agents attach tags, notes, topics, or other JSON
metadata to stored content without changing the original captured content.
`/v1/rules` manages site-scoped filtering rules. New rules default to `draft`;
only `active` rules are sent to the AI analyzer. Rule status values are
`draft`, `active`, `disabled`, and `archived`.

Rule create request shape:

```json
{
  "title": "AI slop",
  "instruction": "Hide generic AI engagement bait.",
  "source": "user",
  "examples": {
    "positive": ["I asked ChatGPT to write this viral thread"],
    "negative": ["Detailed notes about local AI implementation"]
  }
}
```

Rule update requests accept any subset of `title`, `instruction`, `status`,
`priority`, `source`, and `examples`. Example arrays replace only the side
provided. Rule status changes can also use:

```json
{
  "status": "active",
  "source": "user"
}
```

Rule validation uses local stored X posts and reports likely matches from rule
terms and examples. It is a pre-activation blast-radius check, not an AI
classification pass.

Rule suggestions derive draft candidates from active feedback reasons:

```http
GET /v1/rule-suggestions?site=x.com&minFeedback=2&limit=20
```

Suggestions are not stored and are never active automatically. Use their title,
instruction, and examples to create an explicit draft rule after review.

Content annotation request shape:

```json
{
  "storageKey": "x:id:123",
  "contentKind": "post",
  "annotationType": "tag",
  "key": "topics",
  "value": ["local-ai", "tools"],
  "confidence": 0.82,
  "source": "agent:organizer"
}
```

DOM analysis request shape:

```json
{
  "page": {
    "url": "https://x.com/home",
    "title": "X",
    "capturedAt": "2026-05-22T10:00:00.000Z"
  },
  "elements": [
    {
      "clientId": "dom:1",
      "selector": "article:nth-of-type(1)",
      "tagName": "article",
      "role": "article",
      "text": "Post text",
      "html": "<article>...</article>",
      "attributes": [{ "name": "data-testid", "value": "tweet" }],
      "links": [
        {
          "href": "https://x.com/user/status/123",
          "text": "status",
          "ariaLabel": null
        }
      ],
      "snapshotHash": "abc123",
      "capturedAt": "2026-05-22T10:00:00.000Z"
    }
  ]
}
```

DOM analysis response shape:

```json
{
  "commands": [
    {
      "action": "insertLabel",
      "target": {
        "clientId": "dom:1",
        "selector": "article:nth-of-type(1)",
        "mustMatchSnapshotHash": "abc123"
      },
      "label": "Summary: Post summary",
      "text": null,
      "reason": "Codex app-server summary",
      "confidence": 0.8,
      "matchedRuleIds": []
    }
  ]
}
```

`insertFeedbackControl` commands include `feedbackContext`, with active rule
snapshots and item-specific decision metadata for later feedback.

WebSocket request shape:

```json
{
  "type": "analyzeDom",
  "requestId": "dom:1",
  "page": {
    "url": "https://x.com/home",
    "title": "X",
    "capturedAt": "2026-05-22T10:00:00.000Z"
  },
  "elements": []
}
```

WebSocket command event shape:

```json
{
  "type": "commands",
  "requestId": "dom:1",
  "phase": "pending",
  "commands": []
}
```

Supported command actions are `keep`, `hide`, `dim`, `insertLabel`,
`insertFeedbackControl`, and `replaceText`. Site-specific DOM interpretation
lives under `src/sites/`, and site-specific SQLite storage lives under
`src/storage/`; the extension stays generic and only captures DOM regions and
executes commands.

## License

MIT. See `LICENSE`.

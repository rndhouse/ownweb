# OwnWeb Daemon

Local REST service for the Chrome extension.

## Run

```sh
cargo run
```

The daemon binds to `127.0.0.1:17891` by default. Override it with:

```sh
OWNWEB_BIND_ADDR=127.0.0.1:19000 cargo run
```

Daemon output goes through structured logs on stdout. The default log level is
`debug`; override it with `RUST_LOG`.

Incoming posts are not logged by default. To enable captured-content log events:

```sh
OWNWEB_LOG_CAPTURED_CONTENT=1 cargo run
```

Encountered site content is stored in per-site SQLite databases under the local
OwnWeb data directory. X posts are stored at:

```text
~/.local/share/ownweb/x.com/db.sqlite
```

X feedback is stored in the same database as both an append-only event log and
current feedback state. A stored active thumbs-down makes later scans hide that
post by X status ID.

Override the root data directory with `OWNWEB_DATA_DIR`. The daemon uses
bundled SQLite through Rust dependencies, so no separate SQLite service or
system install is required.

For development, reset the X database on startup with:

```sh
OWNWEB_X_RESET_DB=1 cargo run
```

This removes `db.sqlite`, `db.sqlite-wal`, and `db.sqlite-shm` for `x.com`
before the daemon opens storage.

Codex app-server analysis is enabled by default. The daemon starts a local
Codex app-server process when needed, keeps one app-server thread alive across
requests, and asks it to evaluate captured X/Twitter posts against active
content rules. During development, every captured X post with text or a URL is
sent to Codex so the daemon can exercise the analysis loop.

Opinions are cached in memory by X status ID, a normalized fallback key, and the
active rule set. This lets the timeline view and single-post view reuse the same
AI decision when they capture the same post content under the same policy.

Cache hits and X posts sent to the Codex app-server are logged at debug level
on stdout. Repeated full captured post payloads from DOM extraction are trace
level:

```sh
cargo run
```

Useful environment variables:

```sh
OWNWEB_CODEX_APP_ENABLED=0
OWNWEB_CODEX_APP_WS=ws://127.0.0.1:39177
OWNWEB_CODEX_MODEL=gpt-5.3-codex-spark
OWNWEB_CODEX_EFFORT=low
OWNWEB_CODEX_TIMEOUT_MS=8000
OWNWEB_CODEX_CWD=/home/user/dev/ownweb/public
OWNWEB_DATA_DIR=/home/user/.local/share/ownweb
OWNWEB_LOG_CAPTURED_CONTENT=0
OWNWEB_X_RESET_DB=0
OWNWEB_X_SUMMARY_CACHE_MAX_ENTRIES=10000
OWNWEB_X_SUMMARY_CACHE_TTL_SECS=86400
RUST_LOG=debug
```

## Endpoints

- `GET /health`
- `GET /v1/events`
- `POST /v1/dom/analyze`
- `POST /v1/dom/feedback`
- `GET /v1/dislikes?site=x.com`
- `GET /v1/rules?site=x.com`

`/v1/events` is the primary extension path. The extension opens a WebSocket,
sends DOM analysis events, receives immediate `pending` commands that gate
identified posts, then receives `final` commands after local analysis finishes.

`/v1/dom/analyze` is the REST smoke-test path. It accepts the same DOM snapshot
shape and returns final DOM commands in one response. `/v1/dom/feedback`
records `thumbsDown`, `undoThumbsDown`, and `updateReason` signals for one DOM
region. Site-scoped inspection endpoints keep the path generic and take the
site scope through the `site` query parameter.

Request shape:

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

Response shape:

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
      "confidence": 0.8
    }
  ]
}
```

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

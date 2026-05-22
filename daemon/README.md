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

Override the root data directory with `OWNWEB_DATA_DIR`. The daemon uses
bundled SQLite through Rust dependencies, so no separate SQLite service or
system install is required.

Codex app-server summaries are enabled by default. The daemon starts a local
Codex app-server process when needed, keeps one app-server thread alive across
requests, asks for short X/Twitter post summaries, and attaches them as
`label` decisions next to posts. During development, every captured X post with
text or a URL is sent to Codex so the browser view visibly changes. If Codex is
unavailable or too slow, the daemon returns a visible summary-unavailable label
for posts that would have been summarized.

Summaries are cached in memory by X status ID plus a normalized text hash. This
lets the timeline view and single-post view reuse the same AI summary when they
capture the same post content.

Cache hits are logged at debug level on stdout:

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
OWNWEB_X_SUMMARY_CACHE_MAX_ENTRIES=10000
OWNWEB_X_SUMMARY_CACHE_TTL_SECS=86400
RUST_LOG=debug
```

To restore the faster filtered mode later, only send suspicious or
promotion-like posts through Codex:

```sh
OWNWEB_X_REVIEW_ALL=0 cargo run
```

## Endpoints

- `GET /health`
- `POST /v1/dom/analyze`

`/v1/dom/analyze` is the generic browser contract. The extension sends page
metadata plus candidate DOM region snapshots. The daemon dispatches by page
URL, interprets site-specific structures, stores normalized content, and
returns DOM commands for the extension to apply.

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

Supported command actions are `keep`, `hide`, `dim`, `insertLabel`, and
`replaceText`. Site-specific logic lives under `src/sites/`; the extension
stays generic and only captures DOM regions and executes commands.

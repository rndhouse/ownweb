# Pairpilot Daemon

Local REST service for the Chrome extension.

## Run

```sh
cargo run
```

The daemon binds to `127.0.0.1:17891` by default. Override it with:

```sh
PAIRPILOT_BIND_ADDR=127.0.0.1:19000 cargo run
```

Incoming posts are not logged to stdout by default. To enable JSONL capture:

```sh
PAIRPILOT_LOG_CAPTURED_CONTENT=1 cargo run > captured-posts.jsonl
```

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

Cache hits are logged at debug level on stderr:

```sh
RUST_LOG=pairpilot_daemon::summary_cache=debug cargo run
```

Useful environment variables:

```sh
PAIRPILOT_CODEX_APP_ENABLED=0
PAIRPILOT_CODEX_APP_WS=ws://127.0.0.1:39177
PAIRPILOT_CODEX_MODEL=gpt-5.3-codex-spark
PAIRPILOT_CODEX_EFFORT=low
PAIRPILOT_CODEX_TIMEOUT_MS=8000
PAIRPILOT_CODEX_CWD=/home/user/dev/pairpilot/public
PAIRPILOT_LOG_CAPTURED_CONTENT=0
PAIRPILOT_X_SUMMARY_CACHE_MAX_ENTRIES=10000
PAIRPILOT_X_SUMMARY_CACHE_TTL_SECS=86400
```

To restore the faster filtered mode later, only send suspicious or
promotion-like posts through Codex:

```sh
PAIRPILOT_X_REVIEW_ALL=0 cargo run
```

## Endpoints

- `GET /health`
- `POST /v1/content/analyze`
- `POST /v1/x-posts/analyze`

`/v1/content/analyze` is the generic endpoint. It accepts a `source` and
normalized `items`, then dispatches to the matching site module.

`/v1/x-posts/analyze` is the compatibility endpoint used by the current Chrome
extension. It maps X post payloads into the generic content model before
analysis.

Site-specific logic lives under `src/sites/`. The X analyzer currently uses
simple placeholder heuristics.

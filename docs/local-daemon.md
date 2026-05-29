# Local Daemon

The `weblayer` binary can run the local REST and WebSocket daemon used by the
browser extension.

```sh
weblayer daemon
```

The daemon binds to `127.0.0.1:17891` by default. Override the bind address with:

```sh
WEBLAYER_BIND_ADDR=127.0.0.1:19000 weblayer daemon
```

## Local Data

Encountered site content is stored in per-site SQLite databases under the
WebLayer data directory. X.com content is stored at:

```text
~/.local/share/weblayer/x.com/db.sqlite
```

Override the root data directory with:

```sh
WEBLAYER_DATA_DIR=/path/to/weblayer-data weblayer daemon
```

For development, reset the X.com database on startup with:

```sh
WEBLAYER_X_RESET_DB=1 weblayer daemon
```

## Logging

Daemon output goes through structured logs on stdout. The default log level is
`debug`; override it with `RUST_LOG`.

Incoming posts are not logged by default. To enable captured-content log events:

```sh
WEBLAYER_LOG_CAPTURED_CONTENT=1 weblayer daemon
```

## Endpoints

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

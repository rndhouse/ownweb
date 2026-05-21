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

Incoming posts are logged to stdout as JSONL. To save them to a file:

```sh
cargo run > captured-posts.jsonl
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

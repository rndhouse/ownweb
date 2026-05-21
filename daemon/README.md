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
- `POST /v1/x-posts/analyze`

The analyzer currently uses simple placeholder heuristics. Replace `classify_post` in `src/main.rs` with the real filtering logic.

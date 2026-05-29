# CLI

Without the `daemon` subcommand, the `weblayer` binary talks to a running local
daemon. Running `weblayer` with no subcommand behaves like `weblayer status`.

Use Cargo from the repository root:

```sh
cargo run --manifest-path weblayer/Cargo.toml -- status
```

## Daemon Origin

Client commands use `http://127.0.0.1:17891` by default. Override that with
`--daemon-origin` or `WEBLAYER_DAEMON_ORIGIN`.

```sh
cargo run --manifest-path weblayer/Cargo.toml -- \
  --daemon-origin http://127.0.0.1:19000 \
  status
```

## Content

```sh
cargo run --manifest-path weblayer/Cargo.toml -- content list --site x.com --limit 20
cargo run --manifest-path weblayer/Cargo.toml -- content search --site x.com codex
cargo run --manifest-path weblayer/Cargo.toml -- content stats --site x.com
```

## Feedback

```sh
cargo run --manifest-path weblayer/Cargo.toml -- feedback list --site x.com
```

## Annotations

```sh
cargo run --manifest-path weblayer/Cargo.toml -- \
  annotations list \
  --site x.com \
  --storage-key x:id:123

cargo run --manifest-path weblayer/Cargo.toml -- \
  annotations put \
  --site x.com \
  --storage-key x:id:123 \
  --annotation-type tag \
  --key topics \
  --value '["local-ai","tools"]' \
  --source agent:organizer
```

## Rules

```sh
cargo run --manifest-path weblayer/Cargo.toml -- rules list --site x.com
cargo run --manifest-path weblayer/Cargo.toml -- rules show x-ai-slop --site x.com
cargo run --manifest-path weblayer/Cargo.toml -- rules suggest --site x.com --min-feedback 2
cargo run --manifest-path weblayer/Cargo.toml -- rules validate x-ai-slop --site x.com
cargo run --manifest-path weblayer/Cargo.toml -- rules enable x-ai-slop --site x.com
cargo run --manifest-path weblayer/Cargo.toml -- rules disable x-ai-slop --site x.com
cargo run --manifest-path weblayer/Cargo.toml -- rules archive x-ai-slop --site x.com
```

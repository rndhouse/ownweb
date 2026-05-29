# CLI

Without the `daemon` subcommand, the `weblayer` binary talks to a running local
daemon. Running `weblayer` with no subcommand behaves like `weblayer status`.

```sh
weblayer status
```

## Daemon Origin

Client commands use `http://127.0.0.1:17891` by default. Override that with
`--daemon-origin` or `WEBLAYER_DAEMON_ORIGIN`.

```sh
weblayer --daemon-origin http://127.0.0.1:19000 status
```

## Content

```sh
weblayer content list --site x.com --limit 20
weblayer content search --site x.com codex
weblayer content stats --site x.com
```

## Feedback

```sh
weblayer feedback list --site x.com
```

## Annotations

```sh
weblayer annotations list \
  --site x.com \
  --storage-key x:id:123

weblayer annotations put \
  --site x.com \
  --storage-key x:id:123 \
  --annotation-type tag \
  --key topics \
  --value '["local-ai","tools"]' \
  --source agent:organizer
```

## Rules

```sh
weblayer rules list --site x.com
weblayer rules show x-ai-slop --site x.com
weblayer rules suggest --site x.com --min-feedback 2
weblayer rules validate x-ai-slop --site x.com
weblayer rules enable x-ai-slop --site x.com
weblayer rules disable x-ai-slop --site x.com
weblayer rules archive x-ai-slop --site x.com
```

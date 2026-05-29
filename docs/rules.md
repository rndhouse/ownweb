# Rules

Rules are site-scoped policy records stored by the daemon. For X.com, rules live
in the local SQLite database:

```text
~/.local/share/weblayer/x.com/db.sqlite
```

The daemon currently seeds one active X.com rule:

```json
{
  "id": "x-engagement-bait-reaction",
  "site": "x.com",
  "status": "active",
  "priority": 50,
  "title": "Engagement bait reaction posts"
}
```

## Lifecycle

Supported statuses are:

- `draft`: editable rule that is not used for filtering.
- `active`: rule sent to the AI analyzer for X.com post filtering.
- `disabled`: retained rule that is not used for filtering.
- `archived`: retained historical rule hidden from normal active management.

New rules created through the daemon default to `draft`. Activating a rule is an
explicit status change.

## CLI

Create a draft rule:

```sh
weblayer rules create \
  --site x.com \
  --id x-ai-slop \
  --title "AI slop" \
  --instruction "Hide generic AI engagement bait." \
  --positive-example "I asked ChatGPT to write this viral thread"
```

Inspect, test, and activate rules:

```sh
weblayer rules list --site x.com
weblayer rules show x-ai-slop --site x.com
weblayer rules validate x-ai-slop --site x.com
weblayer rules enable x-ai-slop --site x.com
```

Disable or archive a rule:

```sh
weblayer rules disable x-ai-slop --site x.com
weblayer rules archive x-ai-slop --site x.com
```

Suggest draft candidates from active feedback reasons:

```sh
weblayer rules suggest --site x.com --min-feedback 2
```

Suggestions are review material only. They are not inserted into `content_rules`
and they are not active.

## API

List rules:

```http
GET http://127.0.0.1:17891/v1/rules?site=x.com
```

Create a draft rule:

```http
POST http://127.0.0.1:17891/v1/rules?site=x.com
Content-Type: application/json
```

```json
{
  "id": "x-ai-slop",
  "title": "AI slop",
  "instruction": "Hide generic AI engagement bait.",
  "source": "user",
  "examples": {
    "positive": ["I asked ChatGPT to write this viral thread"],
    "negative": ["Detailed notes about local AI implementation"]
  }
}
```

Change status:

```http
POST http://127.0.0.1:17891/v1/rules/x-ai-slop/status?site=x.com
Content-Type: application/json
```

```json
{
  "status": "active",
  "source": "user"
}
```

Validate a rule against stored X.com posts:

```http
GET http://127.0.0.1:17891/v1/rules/x-ai-slop/validate?site=x.com&limit=20
```
